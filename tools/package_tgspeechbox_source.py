#!/usr/bin/env python3
"""Create the deterministic TGSpeechBox corresponding-source archive."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from build_tgspeechbox import COMMIT, RELEASE
from package_piper_source import (
    SourcePackagingError,
    extract_git_source,
    file_manifest,
    git_output,
    repository_version,
    sha256_file,
    vendor_cargo_sources,
    write_checksum,
    write_tar_archive,
)
from package_tgspeechbox import RELEASE_TARGET
from prepare_piper_inputs import PreparationError


class TGSpeechBoxSourcePackagingError(SourcePackagingError):
    """The repository cannot form the TGSpeechBox source artifact."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise TGSpeechBoxSourcePackagingError(message)


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=version)
    parser.add_argument("--commit", default=git_output(repository, "rev-parse", "HEAD"))
    parser.add_argument(
        "--output",
        type=Path,
        default=release / f"omnivox-{version}-tgspeechbox-source.tar.gz",
    )
    parser.add_argument(
        "--checksums",
        type=Path,
        default=release / "tgspeechbox-source-sha256sums.txt",
    )
    parser.add_argument(
        "--source-date-epoch",
        type=int,
        default=int(os.environ.get("SOURCE_DATE_EPOCH", "0")),
    )
    return parser.parse_args()


def source_readme(version: str, commit: str, archive_name: str) -> str:
    return f"""# Omnivox {version} TGSpeechBox source and build inputs

This archive accompanies the optional experimental TGSpeechBox Windows x64
companion. It contains:

- `omnivox/`: the exact Omnivox Git tree at commit `{commit}`;
- `omnivox/vendor/`: every Cargo registry package selected by `Cargo.lock`,
  including the pinned eSpeak NG source;
- `inputs/{archive_name}`: TGSpeechBox revision `{RELEASE}` at commit
  `{COMMIT}`; and
- `SOURCE-MANIFEST.json`: an exhaustive size, mode, and SHA-256 manifest.

For an offline Windows GNU build, copy the input archive into an empty
`build-inputs/downloads/` directory, then run from `omnivox/`:

```sh
CARGO_NET_OFFLINE=true python3 tools/prepare_tgspeechbox_inputs.py \\
  --output ../build-inputs
OMNIVOX_TGSPEECHBOX_INPUTS_DIR="$PWD/../build-inputs" \\
  CARGO_NET_OFFLINE=true python3 tools/build_tgspeechbox.py --release \\
  --target {RELEASE_TARGET}
```

The build requires the pinned Rust toolchain, Python, CMake, and a matching
MinGW-w64 C/C++ toolchain. The input archive and vendored Cargo graph make no
network request during this reproduction.
"""


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

    lock_path = repository / "omnivox-tgspeechbox-sys/source-inputs.json"
    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    require(lock.get("schema_version") == 1, "unknown TGSpeechBox source lock schema")
    require(lock.get("release") == RELEASE, "wrong locked TGSpeechBox revision")
    require(lock.get("commit") == COMMIT, "wrong locked TGSpeechBox commit")
    subprocess.run(
        [sys.executable, str(repository / "tools/prepare_tgspeechbox_inputs.py"), "--check"],
        check=True,
    )
    prepared = repository / "target/tgspeechbox-inputs" / RELEASE
    input_archive = prepared / "downloads" / str(lock["archive"])
    require(input_archive.is_file(), "prepared TGSpeechBox archive is missing")
    require(
        sha256_file(input_archive) == lock.get("sha256"),
        "TGSpeechBox input checksum mismatch",
    )

    target = repository / "target"
    target.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="tgspeechbox-source-", dir=target) as temporary:
        staging = Path(temporary) / f"omnivox-{arguments.version}-tgspeechbox-source"
        staging.mkdir()
        source = extract_git_source(repository, arguments.commit, staging / "omnivox")
        require(
            repository_version(source) == arguments.version,
            "committed source version does not match the archive version",
        )
        require(
            sha256_file(source / lock_path.relative_to(repository)) == sha256_file(lock_path),
            "committed source lock does not match the packaging checkout",
        )
        vendor_cargo_sources(source)
        inputs = staging / "inputs"
        inputs.mkdir()
        shutil.copy2(input_archive, inputs / input_archive.name)
        (staging / "README.md").write_text(
            source_readme(arguments.version, arguments.commit, input_archive.name),
            encoding="utf-8",
        )
        manifest = {
            "artifact": "omnivox-tgspeechbox-source-and-build-inputs",
            "cargo_dependencies_vendored": True,
            "contents": file_manifest(staging),
            "input_archive": {
                "name": input_archive.name,
                "sha256": lock["sha256"],
                "size": input_archive.stat().st_size,
                "source_tree_sha256": lock["source_tree_sha256"],
                "url": lock["url"],
            },
            "native_targets": [RELEASE_TARGET],
            "schema_version": 1,
            "source_commit": arguments.commit,
            "source_input_lock_sha256": sha256_file(lock_path),
            "tgspeechbox_commit": COMMIT,
            "tgspeechbox_revision": RELEASE,
            "version": arguments.version,
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
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
