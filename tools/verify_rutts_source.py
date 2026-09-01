#!/usr/bin/env python3
"""Verify the deterministic RuTTS source and build-integration archive."""

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
import verify_release as common
from build_rutts import RUTTS_COMMIT, RUTTS_VERSION
from package_flite_source import extract_git_source, file_manifest


class SourceVerificationError(common.VerificationError):
    """The RuTTS source artifact violated its release contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SourceVerificationError(message)


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=release / f"omnivox-{version}-rutts-source.tar.gz",
    )
    parser.add_argument(
        "--checksums",
        type=Path,
        default=release / "rutts-source-sha256sums.txt",
    )
    parser.add_argument("--version", default=version)
    return parser.parse_args()


def verify_manifest(root: Path) -> None:
    manifest = json.loads((root / "SOURCE-MANIFEST.json").read_text(encoding="utf-8"))
    require(manifest.get("schema_version") == 1, "unknown source manifest schema")
    expected = manifest.get("files")
    require(isinstance(expected, dict), "source manifest files are missing")
    actual = file_manifest(root)
    require(expected == actual, "source manifest does not match the extracted files")


def verify(repository: Path, arguments: argparse.Namespace) -> None:
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"source archive does not exist: {archive}")
    require(checksums.is_file(), f"checksum file does not exist: {checksums}")
    common.verify_checksum(archive, checksums)
    with tempfile.TemporaryDirectory(prefix="Omnivox RuTTS source verification ") as temporary:
        working = Path(temporary)
        extracted = working / "extracted"
        extracted.mkdir()
        common.extract_tar(archive, extracted)
        root_name = f"omnivox-{arguments.version}-rutts-source"
        require(
            {path.name for path in extracted.iterdir()} == {root_name},
            "source archive has the wrong top-level directory",
        )
        root = extracted / root_name
        require(
            {path.name for path in root.iterdir()}
            == {
                "README.md",
                "SOURCE-MANIFEST.json",
                "SOURCE-PROVENANCE.json",
                "inputs",
                "omnivox",
            },
            "source artifact root entries are incomplete or unexpected",
        )
        verify_manifest(root)
        provenance = json.loads(
            (root / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
        )
        require(provenance.get("artifact") == "omnivox-rutts-source", "wrong artifact provenance")
        require(provenance.get("rutts_version") == RUTTS_VERSION, "wrong RuTTS version")
        require(provenance.get("rutts_commit") == RUTTS_COMMIT, "wrong RuTTS commit")
        require(
            provenance.get("built_in_voices") == ["male", "female"],
            "wrong built-in voice set",
        )
        require(provenance.get("rulex_included") is False, "RuLex exclusion is not recorded")
        commit = str(provenance.get("omnivox_commit", ""))
        require(re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "invalid Omnivox commit")

        reference = working / "reference"
        extract_git_source(repository, commit, reference)
        require(
            file_manifest(reference) == file_manifest(root / "omnivox"),
            "packaged Omnivox source does not match the recorded Git tree",
        )
        lock_path = root / "omnivox/omnivox-rutts-sys/source-inputs.json"
        lock = json.loads(lock_path.read_text(encoding="utf-8"))
        require(
            sha256_file(lock_path) == provenance.get("source_input_lock_sha256"),
            "source lock digest mismatch",
        )
        input_path = root / str(provenance.get("input_archive", ""))
        require(input_path.is_file(), "locked RuTTS input archive is missing")
        require(
            sha256_file(input_path) == lock.get("sha256"),
            "RuTTS input archive checksum mismatch",
        )
        require(
            provenance.get("source_tree_sha256") == lock.get("source_tree_sha256"),
            "RuTTS tree digest mismatch",
        )

        build_inputs = working / "build-inputs"
        downloads = build_inputs / "downloads"
        downloads.mkdir(parents=True)
        shutil.copy2(input_path, downloads / str(lock["archive"]))
        environment = dict(os.environ)
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        prepare = root / "omnivox/tools/prepare_rutts_inputs.py"
        subprocess.run(
            [sys.executable, str(prepare), "--output", str(build_inputs)],
            cwd=root / "omnivox",
            env=environment,
            check=True,
        )
        subprocess.run(
            [sys.executable, str(prepare), "--output", str(build_inputs), "--check"],
            cwd=root / "omnivox",
            env=environment,
            check=True,
        )
    print(f"PASS {archive.name}: manifest, Git tree, source lock, and offline preparation")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        verify(repository, parse_arguments(repository))
    except (
        OSError,
        SourceVerificationError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
        tarfile.TarError,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
