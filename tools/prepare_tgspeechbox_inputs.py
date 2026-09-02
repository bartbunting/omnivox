#!/usr/bin/env python3
"""Download, verify, and safely extract the locked TGSpeechBox source input."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile

sys.dont_write_bytecode = True
from prepare_piper_inputs import (
    PreparationError,
    download,
    extract_archive,
    require,
    sha256_file,
    tree_digest,
)


RELEASE = "v-310b802"
REQUIRED_PATHS = (
    "LICENSE",
    "packs/phonemes.yaml",
    "packs/lang/default.yaml",
    "src/frame.cpp",
    "src/speechPlayer.cpp",
    "src/speechWaveGenerator.cpp",
    "src/frontend/nvspFrontend.cpp",
    "src/frontend/nvspFrontend.h",
)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Prepare the checksum-locked TGSpeechBox v3.10 Beta 8.02 source tree."
    )
    parser.add_argument(
        "--output",
        type=Path,
        help=f"prepared input directory (default: target/tgspeechbox-inputs/{RELEASE})",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="make no network requests or repairs; only verify the prepared cache",
    )
    return parser.parse_args()


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"expected a JSON object: {path}")
    return value


def verify_source(source: Path, expected_digest: str) -> str:
    require(source.is_dir(), f"prepared TGSpeechBox source is missing: {source}")
    for relative in REQUIRED_PATHS:
        require((source / relative).is_file(), f"TGSpeechBox source is missing {relative}")
    actual = tree_digest(source)
    require(
        actual == expected_digest,
        f"prepared TGSpeechBox tree checksum mismatch: found {actual}, "
        f"expected {expected_digest}",
    )
    return actual


def prepare(arguments: argparse.Namespace) -> Path:
    repository = Path(__file__).resolve().parent.parent
    lock_path = repository / "omnivox-tgspeechbox-sys/source-inputs.json"
    lock = load_json(lock_path)
    output = (
        arguments.output.resolve()
        if arguments.output
        else repository / "target/tgspeechbox-inputs" / str(lock["release"])
    )
    archive = output / "downloads" / str(lock["archive"])
    source = output / "source"
    marker_path = output / "PREPARED.json"

    expected_archive = str(lock["sha256"])
    if archive.is_file():
        actual_archive = sha256_file(archive)
        require(
            actual_archive == expected_archive,
            f"cached TGSpeechBox archive checksum mismatch: found {actual_archive}, "
            f"expected {expected_archive}",
        )
    elif arguments.check:
        raise PreparationError(f"cached TGSpeechBox archive is missing: {archive}")
    else:
        print("Downloading locked TGSpeechBox source", file=sys.stderr)
        download(str(lock["url"]), archive, expected_archive)

    expected_tree = str(lock["source_tree_sha256"])
    try:
        actual_tree = verify_source(source, expected_tree)
    except PreparationError:
        if arguments.check:
            raise
        output.mkdir(parents=True, exist_ok=True)
        temporary = Path(tempfile.mkdtemp(prefix=".tgspeechbox.tmp-", dir=output))
        try:
            extracted = extract_archive(
                archive,
                temporary,
                str(lock["archive_root"]),
                materialize_symlinks=False,
            )
            if source.exists():
                shutil.rmtree(source)
            extracted.replace(source)
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
        actual_tree = verify_source(source, expected_tree)

    marker = {
        "schema_version": 1,
        "lock_file": str(lock_path.relative_to(repository)),
        "lock_file_sha256": sha256_file(lock_path),
        "archive_path": str(archive.resolve()),
        "source_path": str(source.resolve()),
        "archive_sha256": expected_archive,
        "source_tree_sha256": actual_tree,
        "commit": str(lock["commit"]),
        "release": str(lock["release"]),
    }
    if arguments.check:
        require(marker_path.is_file(), f"prepared-input marker is missing: {marker_path}")
        require(load_json(marker_path) == marker, "prepared-input marker does not match inputs")
    else:
        marker_path.write_text(
            json.dumps(marker, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    print(f"Prepared and verified TGSpeechBox inputs in {output}", file=sys.stderr)
    return output


def main() -> int:
    try:
        prepare(parse_arguments())
    except (OSError, PreparationError, json.JSONDecodeError, tarfile.TarError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
