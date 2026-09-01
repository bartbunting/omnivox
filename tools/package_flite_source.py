#!/usr/bin/env python3
"""Create the deterministic Flite source and build-integration archive."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

sys.dont_write_bytecode = True
from build_flite import FLITE_COMMIT, FLITE_VERSION
from prepare_piper_inputs import safe_parts


class SourcePackagingError(RuntimeError):
    """The repository cannot form the Flite source artifact."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SourcePackagingError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_output(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    return result.stdout.strip()


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=version)
    parser.add_argument("--commit", default=git_output(repository, "rev-parse", "HEAD"))
    parser.add_argument(
        "--output",
        type=Path,
        default=release / f"omnivox-{version}-flite-source.tar.gz",
    )
    parser.add_argument(
        "--checksums",
        type=Path,
        default=release / "flite-source-sha256sums.txt",
    )
    parser.add_argument(
        "--source-date-epoch",
        type=int,
        default=int(os.environ.get("SOURCE_DATE_EPOCH", "0")),
    )
    return parser.parse_args()


def extract_git_source(repository: Path, commit: str, destination: Path) -> None:
    archive_path = destination.parent / "omnivox-source.tar"
    with archive_path.open("wb") as output:
        subprocess.run(
            ["git", "-C", str(repository), "archive", "--format=tar", commit],
            check=True,
            stdout=output,
        )
    destination.mkdir()
    with tarfile.open(archive_path, "r:") as archive:
        seen: set[PurePosixPath] = set()
        for member in archive.getmembers():
            relative = PurePosixPath(*safe_parts(member.name))
            require(relative not in seen, f"duplicate Git archive member: {member.name}")
            seen.add(relative)
            target = destination.joinpath(*relative.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            require(member.isfile(), f"unsupported Git archive member: {member.name}")
            source = archive.extractfile(member)
            require(source is not None, f"cannot read Git archive member: {member.name}")
            target.parent.mkdir(parents=True, exist_ok=True)
            with source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
            target.chmod(member.mode & 0o777)
    archive_path.unlink()


def file_manifest(root: Path) -> dict[str, dict[str, object]]:
    entries: dict[str, dict[str, object]] = {}
    for path in sorted(root.rglob("*")):
        require(not path.is_symlink(), f"source artifact symlink is not allowed: {path}")
        require(path.is_dir() or path.is_file(), f"unsupported source entry: {path}")
        if not path.is_file() or path.name == "SOURCE-MANIFEST.json":
            continue
        entries[path.relative_to(root).as_posix()] = {
            "mode": "0755" if path.stat().st_mode & 0o111 else "0644",
            "sha256": sha256_file(path),
            "size": path.stat().st_size,
        }
    return entries


def source_readme(version: str, commit: str, archive_name: str) -> str:
    return f"""# Omnivox {version} Flite source

This artifact accompanies the optional Flite companion binaries. It contains:

- `omnivox/`: the exact Omnivox Git tree at commit `{commit}`;
- `inputs/{archive_name}`: the checksum-locked upstream Flite v{FLITE_VERSION}
  archive at commit `{FLITE_COMMIT}`; and
- `SOURCE-MANIFEST.json`: an exhaustive size, mode, and SHA-256 manifest.

No external `.flitevox` files are included. The sole compiled-in voice source
is `cmu_us_slt` inside the upstream archive.

To reproduce the native source preparation, copy the input archive into
`build-inputs/downloads/`, then run from `omnivox/`:

```sh
python3 tools/prepare_flite_inputs.py --output ../build-inputs
OMNIVOX_FLITE_SOURCE_DIR="$PWD/../build-inputs/source" \\
  cargo build --locked --release --package omnivox-flite-helper
```

Cargo registry dependencies and a platform C compiler are also required. The
build script itself performs no network access once both the locked Cargo
dependencies and verified Flite source are available.
"""


def tar_info(name: str, mode: int, timestamp: int, directory: bool) -> tarfile.TarInfo:
    info = tarfile.TarInfo(name)
    info.mode = mode
    info.mtime = timestamp
    info.uid = 0
    info.gid = 0
    info.uname = "root"
    info.gname = "root"
    if directory:
        info.type = tarfile.DIRTYPE
    return info


def write_archive(source: Path, destination: Path, root_name: str, timestamp: int) -> None:
    require(timestamp >= 0, "source date epoch cannot be negative")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        with temporary.open("wb") as raw:
            with gzip.GzipFile(
                filename="", mode="wb", fileobj=raw, mtime=timestamp, compresslevel=9
            ) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                    archive.addfile(tar_info(root_name, 0o755, timestamp, True))
                    for path in sorted(source.rglob("*")):
                        name = f"{root_name}/{path.relative_to(source).as_posix()}"
                        if path.is_dir():
                            archive.addfile(tar_info(name, 0o755, timestamp, True))
                        else:
                            mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                            info = tar_info(name, mode, timestamp, False)
                            info.size = path.stat().st_size
                            with path.open("rb") as content:
                                archive.addfile(info, content)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()


def package(repository: Path, arguments: argparse.Namespace) -> None:
    require(
        re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", arguments.version)
        is not None,
        f"invalid release version: {arguments.version}",
    )
    require(re.fullmatch(r"[0-9a-f]{40}", arguments.commit) is not None, "invalid Git commit")
    require(
        not git_output(repository, "status", "--porcelain", "--untracked-files=no"),
        "refusing to package while the tracked worktree is dirty",
    )
    lock_path = repository / "omnivox-flite-sys/source-inputs.json"
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    require(lock.get("flite_version") == FLITE_VERSION, "wrong locked Flite version")
    require(lock.get("commit") == FLITE_COMMIT, "wrong locked Flite commit")
    prepared = repository / "target/flite-inputs" / FLITE_VERSION
    subprocess.run(
        [sys.executable, str(repository / "tools/prepare_flite_inputs.py"), "--check"],
        check=True,
    )
    input_archive = prepared / "downloads" / str(lock["archive"])
    require(sha256_file(input_archive) == lock.get("sha256"), "Flite input checksum mismatch")

    with tempfile.TemporaryDirectory(prefix="Omnivox Flite source packaging ") as temporary:
        root = Path(temporary) / "stage"
        root.mkdir()
        extract_git_source(repository, arguments.commit, root / "omnivox")
        inputs = root / "inputs"
        inputs.mkdir()
        shutil.copy2(input_archive, inputs / input_archive.name)
        (root / "README.md").write_text(
            source_readme(arguments.version, arguments.commit, input_archive.name),
            encoding="utf-8",
        )
        provenance = {
            "schema_version": 1,
            "artifact": "omnivox-flite-source",
            "omnivox_commit": arguments.commit,
            "flite_version": FLITE_VERSION,
            "flite_commit": FLITE_COMMIT,
            "input_archive": f"inputs/{input_archive.name}",
            "input_archive_sha256": lock["sha256"],
            "source_tree_sha256": lock["source_tree_sha256"],
            "source_input_lock_sha256": sha256_file(lock_path),
            "external_voice_files_included": False,
        }
        (root / "SOURCE-PROVENANCE.json").write_text(
            json.dumps(provenance, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        (root / "SOURCE-MANIFEST.json").write_text(
            json.dumps(
                {"schema_version": 1, "files": file_manifest(root)},
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        output = arguments.output.resolve()
        root_name = f"omnivox-{arguments.version}-flite-source"
        write_archive(root, output, root_name, arguments.source_date_epoch)
    checksum_path = arguments.checksums.resolve()
    checksum_path.parent.mkdir(parents=True, exist_ok=True)
    checksum_path.write_text(f"{sha256_file(output)}  {output.name}\n", encoding="utf-8")
    print(f"Packaged {output} ({output.stat().st_size / (1024 * 1024):.1f} MiB)")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        package(repository, parse_arguments(repository))
    except (
        OSError,
        SourcePackagingError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
        tarfile.TarError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
