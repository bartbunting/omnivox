#!/usr/bin/env python3
"""Create a deterministic Piper corresponding-source and build-input archive."""

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
import urllib.parse

sys.dont_write_bytecode = True
from build_piper import ONNXRUNTIME_VERSION, PIPER_COMMIT, PIPER_VERSION
from prepare_piper_inputs import PreparationError, download


EXPECTED_TARGETS = {
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
}
EXPECTED_COMPONENTS = {"espeak_ng", "onnxruntime", "sonic"}
VENDOR_CONFIG = """[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
"""


class SourcePackagingError(RuntimeError):
    """The repository cannot form the required source artifact."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SourcePackagingError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def git_output(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    return result.stdout.strip()


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"expected a JSON object: {path}")
    return value


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=version)
    parser.add_argument("--commit", default=git_output(repository, "rev-parse", "HEAD"))
    parser.add_argument(
        "--cache",
        type=Path,
        default=repository / "target/piper-source-cache",
        help="verified download cache",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=release / f"omnivox-{version}-piper-source.tar.gz",
    )
    parser.add_argument(
        "--checksums",
        type=Path,
        default=release / "piper-source-sha256sums.txt",
    )
    parser.add_argument(
        "--source-date-epoch",
        type=int,
        default=int(os.environ.get("SOURCE_DATE_EPOCH", "0")),
        help="normalized archive timestamp (default: SOURCE_DATE_EPOCH or zero)",
    )
    return parser.parse_args()


def safe_archive_name(name: object) -> str:
    value = str(name)
    require(
        value == Path(value).name and value not in ("", ".", ".."),
        f"unsafe locked archive name: {value!r}",
    )
    return value


def validate_download_specification(
    name: str, specification: dict[str, object]
) -> tuple[str, str]:
    url = str(specification.get("url", ""))
    parsed = urllib.parse.urlparse(url)
    require(
        parsed.scheme == "https" and parsed.netloc == "github.com",
        f"{name} does not use a reviewed GitHub HTTPS source",
    )
    digest = str(specification.get("sha256", ""))
    require(
        re.fullmatch(r"[0-9a-f]{64}", digest) is not None,
        f"invalid locked checksum for {name}",
    )
    return url, digest


def locked_archives(repository: Path) -> tuple[dict[str, dict[str, object]], dict[str, str]]:
    native_path = repository / "omnivox-piper-sys/native-inputs.json"
    source_path = repository / "omnivox-piper-sys/source-inputs.json"
    native = load_json(native_path)
    source = load_json(source_path)
    require(native.get("schema_version") == 1, "unknown native-input schema")
    require(native.get("piper_version") == PIPER_VERSION, "wrong locked Piper version")
    targets = native.get("targets")
    require(isinstance(targets, dict), "native-input lock has no targets")
    require(set(targets) == EXPECTED_TARGETS, "native-input target set changed")

    archives: dict[str, dict[str, object]] = {}
    for target in sorted(targets):
        components = targets[target]
        require(isinstance(components, dict), f"invalid target lock: {target}")
        require(set(components) == EXPECTED_COMPONENTS, f"component set changed: {target}")
        for component in sorted(components):
            specification = components[component]
            require(isinstance(specification, dict), f"invalid component: {component}")
            archive = safe_archive_name(specification.get("archive"))
            url, digest = validate_download_specification(archive, specification)
            locked = archives.setdefault(
                archive,
                {
                    "kind": "native-build-input",
                    "sha256": digest,
                    "url": url,
                    "uses": [],
                },
            )
            require(
                locked["sha256"] == digest and locked["url"] == url,
                f"conflicting lock entries for {archive}",
            )
            uses = locked["uses"]
            assert isinstance(uses, list)
            uses.append({"component": component, "target": target})

    require(source.get("schema_version") == 1, "unknown source-input schema")
    sources = source.get("sources")
    require(
        isinstance(sources, dict) and set(sources) == {"onnxruntime"},
        "source-input component set changed",
    )
    onnx = sources["onnxruntime"]
    require(isinstance(onnx, dict), "invalid ONNX Runtime source lock")
    require(onnx.get("version") == ONNXRUNTIME_VERSION, "wrong ONNX Runtime source version")
    require(onnx.get("license") == "MIT", "wrong ONNX Runtime source licence")
    require(
        re.fullmatch(r"[0-9a-f]{40}", str(onnx.get("commit", ""))) is not None,
        "invalid ONNX Runtime source commit",
    )
    archive = safe_archive_name(onnx.get("archive"))
    url, digest = validate_download_specification(archive, onnx)
    size = onnx.get("size")
    require(isinstance(size, int) and size > 0, "invalid ONNX Runtime source size")
    require(archive not in archives, f"duplicate source archive name: {archive}")
    archives[archive] = {
        "kind": "corresponding-source",
        "sha256": digest,
        "size": size,
        "url": url,
        "uses": [{"component": "onnxruntime-source"}],
    }
    lock_digests = {
        native_path.relative_to(repository).as_posix(): sha256_file(native_path),
        source_path.relative_to(repository).as_posix(): sha256_file(source_path),
    }
    return archives, lock_digests


def ensure_downloads(
    cache: Path, specifications: dict[str, dict[str, object]]
) -> dict[str, Path]:
    cache.mkdir(parents=True, exist_ok=True)
    downloads: dict[str, Path] = {}
    for name in sorted(specifications):
        specification = specifications[name]
        path = cache / name
        expected = str(specification["sha256"])
        if path.is_file() and sha256_file(path) != expected:
            print(f"Replacing stale Piper source input {name}", file=sys.stderr)
            path.unlink()
        if not path.is_file():
            print(f"Downloading locked Piper source input {name}", file=sys.stderr)
            download(str(specification["url"]), path, expected)
        require(sha256_file(path) == expected, f"cached checksum mismatch: {name}")
        expected_size = specification.get("size")
        if expected_size is not None:
            require(path.stat().st_size == expected_size, f"cached size mismatch: {name}")
        downloads[name] = path
    return downloads


def safe_parts(name: str) -> tuple[str, ...]:
    require("\\" not in name, f"git archive member uses a backslash: {name!r}")
    path = PurePosixPath(name)
    require(not path.is_absolute(), f"git archive member is absolute: {name!r}")
    parts = tuple(part for part in path.parts if part not in ("", "."))
    require(parts and ".." not in parts, f"unsafe git archive member: {name!r}")
    return parts


def extract_git_source(repository: Path, commit: str, destination: Path) -> Path:
    archive = destination.parent / "repository.tar"
    with archive.open("wb") as output:
        subprocess.run(
            ["git", "-C", str(repository), "archive", "--format=tar", commit],
            check=True,
            stdout=output,
        )
    destination.mkdir(parents=True)
    with tarfile.open(archive, "r:") as bundle:
        seen: set[PurePosixPath] = set()
        for member in bundle.getmembers():
            relative = PurePosixPath(*safe_parts(member.name))
            require(relative not in seen, f"duplicate git archive member: {member.name}")
            seen.add(relative)
            target = destination.joinpath(*relative.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            require(member.isfile(), f"unsupported git archive member: {member.name}")
            source = bundle.extractfile(member)
            require(source is not None, f"cannot read git archive member: {member.name}")
            target.parent.mkdir(parents=True, exist_ok=True)
            with source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
            target.chmod(member.mode & 0o777)
    archive.unlink()
    return destination


def vendor_cargo_sources(source: Path) -> None:
    result = subprocess.run(
        ["cargo", "vendor", "--locked", "--versioned-dirs", "vendor"],
        cwd=source,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    require(result.returncode == 0, f"cargo vendor failed:\n{result.stderr}")
    configuration = source / ".cargo/config.toml"
    configuration.parent.mkdir(parents=True, exist_ok=True)
    configuration.write_text(VENDOR_CONFIG, encoding="utf-8")


def link_or_copy(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.link(source, destination)
    except OSError:
        shutil.copyfile(source, destination)


def source_readme(version: str, commit: str) -> str:
    return f"""# Omnivox {version} Piper source and build inputs

This archive accompanies the optional Piper binaries. It records Omnivox
commit `{commit}` and contains:

- `omnivox/`: the exact committed Omnivox source, including vendored libpiper;
- `omnivox/vendor/`: every Cargo registry package selected by `Cargo.lock`;
- `inputs/`: checksum-locked eSpeak NG and Sonic sources, ONNX Runtime binaries
  used by all four companions, and the corresponding ONNX Runtime source; and
- `SOURCE-MANIFEST.json`: an exhaustive size, mode, and SHA-256 manifest.

No Omnivox CI voice model is included. The model lock in the source tree is
metadata only.

For an offline native build, copy the files from `inputs/` into an empty
`build-inputs/downloads/` directory, then run from `omnivox/`:

```sh
python3 tools/prepare_piper_inputs.py --target TARGET --output ../build-inputs
OMNIVOX_PIPER_INPUTS_DIR="$PWD/../build-inputs" \\
  cargo build --locked --offline --release --package omnivox-piper-helper \\
  --features piper
```

Replace `TARGET` with one of the target triples recorded in
`omnivox/omnivox-piper-sys/native-inputs.json`. Native Piper builds require a
matching host and target plus the platform prerequisites documented by
Omnivox. The included ONNX Runtime source corresponds to the prebuilt 1.22.0
libraries; rebuilding ONNX Runtime itself may require its documented toolchain
and third-party source preparation.
"""


def file_manifest(root: Path) -> dict[str, dict[str, object]]:
    entries: dict[str, dict[str, object]] = {}
    for path in sorted(root.rglob("*")):
        require(not path.is_symlink(), f"source artifact symlink is not allowed: {path}")
        require(path.is_dir() or path.is_file(), f"unsupported source entry: {path}")
        if not path.is_file() or path.name == "SOURCE-MANIFEST.json":
            continue
        relative = path.relative_to(root).as_posix()
        entries[relative] = {
            "mode": "0755" if path.stat().st_mode & 0o111 else "0644",
            "sha256": sha256_file(path),
            "size": path.stat().st_size,
        }
    return entries


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


def write_tar_archive(
    source: Path, destination: Path, root_name: str, timestamp: int
) -> None:
    require(timestamp >= 0, "source date epoch cannot be negative")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        with temporary.open("wb") as raw:
            with gzip.GzipFile(
                filename="", mode="wb", fileobj=raw, mtime=timestamp, compresslevel=9
            ) as compressed:
                with tarfile.open(
                    fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT
                ) as archive:
                    archive.addfile(tar_info(root_name, 0o755, timestamp, True))
                    for path in sorted(source.rglob("*")):
                        relative = path.relative_to(source).as_posix()
                        name = f"{root_name}/{relative}"
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


def write_checksum(archive: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        temporary.write_text(
            f"{sha256_file(archive)}  {archive.name}\n", encoding="utf-8"
        )
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()


def package(repository: Path, arguments: argparse.Namespace) -> Path:
    require(
        re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", arguments.version)
        is not None,
        f"invalid release version: {arguments.version}",
    )
    require(arguments.version == repository_version(repository), "release version mismatch")
    require(
        re.fullmatch(r"[0-9a-f]{40}", arguments.commit) is not None,
        f"invalid source commit: {arguments.commit}",
    )
    git_output(repository, "cat-file", "-e", f"{arguments.commit}^{{commit}}")
    specifications, lock_digests = locked_archives(repository)
    downloads = ensure_downloads(arguments.cache.resolve(), specifications)
    target = repository / "target"
    target.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="piper-source-", dir=target) as temporary:
        staging = Path(temporary) / f"omnivox-{arguments.version}-piper-source"
        staging.mkdir()
        source = extract_git_source(repository, arguments.commit, staging / "omnivox")
        require(
            repository_version(source) == arguments.version,
            "committed source version does not match the archive version",
        )
        for relative, digest in lock_digests.items():
            require(
                sha256_file(source / relative) == digest,
                f"committed source lock does not match the packaging checkout: {relative}",
            )
        vendor_cargo_sources(source)
        inputs = staging / "inputs"
        for name, path in sorted(downloads.items()):
            link_or_copy(path, inputs / name)
            specifications[name]["size"] = path.stat().st_size
        (staging / "README.md").write_text(
            source_readme(arguments.version, arguments.commit), encoding="utf-8"
        )
        manifest = {
            "artifact": "omnivox-piper-source-and-build-inputs",
            "cargo_dependencies_vendored": True,
            "contents": file_manifest(staging),
            "input_archives": specifications,
            "locks": lock_digests,
            "native_targets": sorted(EXPECTED_TARGETS),
            "schema_version": 1,
            "source_commit": arguments.commit,
            "version": arguments.version,
            "voice_model_included": False,
        }
        (staging / "SOURCE-MANIFEST.json").write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        output = arguments.output.resolve()
        write_tar_archive(staging, output, staging.name, arguments.source_date_epoch)
    write_checksum(output, arguments.checksums.resolve())
    return output


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        output = package(repository, parse_arguments(repository))
        print(f"Packaged {output} ({output.stat().st_size / (1024 * 1024):.1f} MiB)")
    except (
        OSError,
        PreparationError,
        SourcePackagingError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
        tarfile.TarError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
