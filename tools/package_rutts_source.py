#!/usr/bin/env python3
"""Create the deterministic RuTTS source and build-integration archive."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

sys.dont_write_bytecode = True
from build_rutts import RUTTS_COMMIT, RUTTS_VERSION
from package_flite_source import extract_git_source, file_manifest, write_archive


class SourcePackagingError(RuntimeError):
    """The repository cannot form the RuTTS source artifact."""


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
        default=release / f"omnivox-{version}-rutts-source.tar.gz",
    )
    parser.add_argument(
        "--checksums",
        type=Path,
        default=release / "rutts-source-sha256sums.txt",
    )
    parser.add_argument(
        "--source-date-epoch",
        type=int,
        default=int(os.environ.get("SOURCE_DATE_EPOCH", "0")),
    )
    return parser.parse_args()


def source_readme(version: str, commit: str, archive_name: str) -> str:
    return f"""# Omnivox {version} RuTTS source

This artifact accompanies the optional RuTTS companion binaries. It contains:

- `omnivox/`: the exact Omnivox Git tree at commit `{commit}`;
- `inputs/{archive_name}`: the checksum-locked upstream RuTTS v{RUTTS_VERSION}
  archive at commit `{RUTTS_COMMIT}`; and
- `SOURCE-MANIFEST.json`: an exhaustive size, mode, and SHA-256 manifest.

The upstream archive contains RuTTS's built-in male and female voice data.
RuLex source, libraries, and dictionary databases are not included.

To reproduce the native source preparation, copy the input archive into
`build-inputs/downloads/`, then run from `omnivox/`:

```sh
python3 tools/prepare_rutts_inputs.py --output ../build-inputs
OMNIVOX_RUTTS_SOURCE_DIR="$PWD/../build-inputs/source" \
  cargo build --locked --release --package omnivox-rutts-helper
```

Cargo registry dependencies and a platform C compiler are also required. The
build script itself performs no network access once both the locked Cargo
dependencies and verified RuTTS source are available.
"""


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
    lock_path = repository / "omnivox-rutts-sys/source-inputs.json"
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    require(lock.get("rutts_version") == RUTTS_VERSION, "wrong locked RuTTS version")
    require(lock.get("commit") == RUTTS_COMMIT, "wrong locked RuTTS commit")
    prepared = repository / "target/rutts-inputs" / RUTTS_VERSION
    subprocess.run(
        [sys.executable, str(repository / "tools/prepare_rutts_inputs.py"), "--check"],
        check=True,
    )
    input_archive = prepared / "downloads" / str(lock["archive"])
    require(sha256_file(input_archive) == lock.get("sha256"), "RuTTS input checksum mismatch")

    with tempfile.TemporaryDirectory(prefix="Omnivox RuTTS source packaging ") as temporary:
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
            "artifact": "omnivox-rutts-source",
            "omnivox_commit": arguments.commit,
            "rutts_version": RUTTS_VERSION,
            "rutts_commit": RUTTS_COMMIT,
            "input_archive": f"inputs/{input_archive.name}",
            "input_archive_sha256": lock["sha256"],
            "source_tree_sha256": lock["source_tree_sha256"],
            "source_input_lock_sha256": sha256_file(lock_path),
            "built_in_voices": ["male", "female"],
            "rulex_included": False,
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
        root_name = f"omnivox-{arguments.version}-rutts-source"
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
