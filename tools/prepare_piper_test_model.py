#!/usr/bin/env python3
"""Download and verify the locked, CI-only Piper synthesis model."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
import urllib.parse

sys.dont_write_bytecode = True
from prepare_piper_inputs import PreparationError, download, require, sha256_file


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        help="model directory (default: target/piper-test-model)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="make no network requests or repairs; only verify the prepared model",
    )
    return parser.parse_args()


def load_lock(path: Path) -> dict[str, object]:
    lock = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(lock, dict), "Piper test-model lock must be an object")
    require(lock.get("schema_version") == 1, "unknown Piper test-model schema")
    require(
        lock.get("release_artifact_included") is False,
        "Piper test model must remain outside release artifacts",
    )
    require(
        lock.get("licensing_review") == "ci_only_approved",
        "Piper test model is not approved for CI-only use",
    )
    require(
        lock.get("licensing_evidence")
        == {
            "declared_dataset": "LibriVox",
            "declared_license": "public domain",
            "source_file": "MODEL_CARD",
            "training_lineage": "trained from scratch",
        },
        "Piper test-model licensing evidence changed without review",
    )
    revision = str(lock.get("revision", ""))
    require(
        re.fullmatch(r"[0-9a-f]{40}", revision) is not None,
        "Piper test-model revision is not an immutable commit",
    )
    files = lock.get("files")
    require(isinstance(files, dict) and files, "Piper test-model files are missing")
    return lock


def source_url(lock: dict[str, object], filename: str) -> str:
    repository = str(lock["repository"])
    parsed = urllib.parse.urlparse(repository)
    require(
        parsed.scheme == "https" and parsed.netloc == "huggingface.co",
        "Piper test model must use the reviewed HTTPS source host",
    )
    source_path = str(lock["source_path"]).strip("/")
    revision = str(lock["revision"])
    quoted_path = "/".join(
        urllib.parse.quote(part, safe="")
        for part in (source_path + "/" + filename).split("/")
    )
    return f"{repository.rstrip('/')}/resolve/{revision}/{quoted_path}"


def expected_marker(lock_path: Path, lock: dict[str, object]) -> dict[str, object]:
    return {
        "schema_version": 1,
        "lock_file": "omnivox-piper-sys/test-model.json",
        "lock_file_sha256": sha256_file(lock_path),
        "model_id": lock["model_id"],
        "repository": lock["repository"],
        "revision": lock["revision"],
        "release_artifact_included": False,
        "licensing_review": lock["licensing_review"],
        "licensing_evidence": lock["licensing_evidence"],
        "files": lock["files"],
    }


def prepare(arguments: argparse.Namespace) -> Path:
    repository = Path(__file__).resolve().parent.parent
    lock_path = repository / "omnivox-piper-sys/test-model.json"
    lock = load_lock(lock_path)
    output = (
        arguments.output.resolve()
        if arguments.output
        else repository / "target/piper-test-model"
    )
    files = lock["files"]
    assert isinstance(files, dict)
    for filename in sorted(files):
        require(
            filename == Path(filename).name and filename not in ("", ".", ".."),
            f"unsafe Piper test-model filename: {filename!r}",
        )
        specification = files[filename]
        require(isinstance(specification, dict), f"invalid model file: {filename}")
        expected_sha256 = str(specification.get("sha256", ""))
        expected_size = specification.get("size")
        require(
            re.fullmatch(r"[0-9a-f]{64}", expected_sha256) is not None,
            f"invalid model checksum: {filename}",
        )
        require(
            isinstance(expected_size, int) and expected_size > 0,
            f"invalid model size: {filename}",
        )
        destination = output / filename
        needs_download = not destination.is_file()
        if destination.is_file():
            if destination.stat().st_size != expected_size:
                if arguments.check:
                    raise PreparationError(f"cached model size mismatch: {filename}")
                print(f"Replacing outdated Piper test file {filename}", file=sys.stderr)
                needs_download = True
            elif sha256_file(destination) != expected_sha256:
                if arguments.check:
                    raise PreparationError(
                        f"cached model checksum mismatch: {filename}"
                    )
                print(f"Replacing outdated Piper test file {filename}", file=sys.stderr)
                needs_download = True
        elif arguments.check:
            raise PreparationError(f"cached model file is missing: {destination}")
        if needs_download:
            print(f"Downloading locked Piper test file {filename}", file=sys.stderr)
            download(source_url(lock, filename), destination, expected_sha256)
            require(
                destination.stat().st_size == expected_size,
                f"downloaded model size mismatch: {filename}",
            )

    marker_path = output / "PREPARED.json"
    marker = expected_marker(lock_path, lock)
    if arguments.check:
        require(marker_path.is_file(), f"model marker is missing: {marker_path}")
        require(
            json.loads(marker_path.read_text(encoding="utf-8")) == marker,
            "model marker does not match the locked files",
        )
    else:
        output.mkdir(parents=True, exist_ok=True)
        marker_path.write_text(
            json.dumps(marker, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    print(f"Prepared and verified Piper test model in {output}", file=sys.stderr)
    return output


def main() -> int:
    try:
        prepare(parse_arguments())
    except (OSError, PreparationError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
