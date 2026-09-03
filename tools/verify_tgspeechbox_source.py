#!/usr/bin/env python3
"""Verify the deterministic TGSpeechBox corresponding-source artifact."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

sys.dont_write_bytecode = True
from build_tgspeechbox import COMMIT, RELEASE
import verify_release as common
from verify_piper_source import (
    repository_version,
    sha256_file,
    verify_committed_source,
    verify_offline_cargo,
)
from package_piper_source import git_output
from package_tgspeechbox import RELEASE_TARGET


class SourceVerificationError(common.VerificationError):
    """The TGSpeechBox source artifact violated its release contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SourceVerificationError(message)


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=release / f"omnivox-{version}-tgspeechbox-source.tar.gz",
    )
    parser.add_argument(
        "--checksums",
        type=Path,
        default=release / "tgspeechbox-source-sha256sums.txt",
    )
    parser.add_argument("--version", default=version)
    parser.add_argument("--commit", default=git_output(repository, "rev-parse", "HEAD"))
    return parser.parse_args()


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"expected a JSON object: {path}")
    return value


def verify_manifest(root: Path, version: str, commit: str) -> dict[str, object]:
    manifest_path = root / "SOURCE-MANIFEST.json"
    manifest = load_json(manifest_path)
    require(manifest.get("schema_version") == 1, "unknown source manifest schema")
    require(
        manifest.get("artifact") == "omnivox-tgspeechbox-source-and-build-inputs",
        "wrong source artifact identity",
    )
    require(manifest.get("version") == version, "source manifest version mismatch")
    require(manifest.get("source_commit") == commit, "source commit mismatch")
    require(manifest.get("tgspeechbox_revision") == RELEASE, "source revision mismatch")
    require(manifest.get("tgspeechbox_commit") == COMMIT, "TGSpeechBox commit mismatch")
    require(
        manifest.get("cargo_dependencies_vendored") is True,
        "Cargo dependency source boundary is missing",
    )
    require(
        set(manifest.get("native_targets", [])) == {RELEASE_TARGET},
        "source manifest native target set changed",
    )
    declared = manifest.get("contents")
    require(isinstance(declared, dict) and declared, "source content manifest is empty")
    actual = {
        path.relative_to(root).as_posix(): path
        for path in root.rglob("*")
        if path.is_file() and path != manifest_path
    }
    require(declared.keys() == actual.keys(), "source manifest is not exhaustive")
    for relative, path in actual.items():
        specification = declared[relative]
        require(isinstance(specification, dict), f"invalid content entry: {relative}")
        expected_mode = "0755" if path.stat().st_mode & 0o111 else "0644"
        require(specification.get("mode") == expected_mode, f"mode mismatch: {relative}")
        require(specification.get("size") == path.stat().st_size, f"size mismatch: {relative}")
        require(
            specification.get("sha256") == sha256_file(path),
            f"checksum mismatch: {relative}",
        )
    return manifest


def verify_input(
    root: Path, source: Path, manifest: dict[str, object], working: Path
) -> None:
    lock_path = source / "omnivox-tgspeechbox-sys/source-inputs.json"
    lock = load_json(lock_path)
    require(lock.get("schema_version") == 1, "unknown packaged source lock schema")
    require(lock.get("release") == RELEASE, "wrong packaged source revision")
    require(lock.get("commit") == COMMIT, "wrong packaged source commit")
    require(
        manifest.get("source_input_lock_sha256") == sha256_file(lock_path),
        "source lock digest mismatch",
    )
    input_specification = manifest.get("input_archive")
    require(isinstance(input_specification, dict), "input provenance is missing")
    name = str(lock.get("archive", ""))
    require(name == Path(name).name, "unsafe packaged input name")
    input_archive = root / "inputs" / name
    require(
        {path.name for path in (root / "inputs").iterdir() if path.is_file()} == {name},
        "source input archive set mismatch",
    )
    require(sha256_file(input_archive) == lock.get("sha256"), "input checksum mismatch")
    require(input_specification.get("name") == name, "input provenance name mismatch")
    require(
        input_specification.get("sha256") == lock.get("sha256"),
        "input provenance checksum mismatch",
    )
    require(
        input_specification.get("source_tree_sha256") == lock.get("source_tree_sha256"),
        "input provenance tree mismatch",
    )
    require(
        input_specification.get("size") == input_archive.stat().st_size,
        "input provenance size mismatch",
    )

    prepared = working / "prepared-inputs"
    downloads = prepared / "downloads"
    downloads.mkdir(parents=True)
    shutil.copy2(input_archive, downloads / name)
    command = [
        sys.executable,
        str(source / "tools/prepare_tgspeechbox_inputs.py"),
        "--output",
        str(prepared),
    ]
    subprocess.run(command, cwd=source, check=True)
    subprocess.run([*command, "--check"], cwd=source, check=True)


def verify(arguments: argparse.Namespace, repository: Path) -> None:
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"source archive does not exist: {archive}")
    require(checksums.is_file(), f"source checksum file does not exist: {checksums}")
    require(
        archive.name == f"omnivox-{arguments.version}-tgspeechbox-source.tar.gz",
        f"unexpected source archive name: {archive.name}",
    )
    require(
        re.fullmatch(r"[0-9a-f]{40}", arguments.commit) is not None,
        f"invalid expected source commit: {arguments.commit}",
    )
    common.verify_checksum(archive, checksums)
    with tempfile.TemporaryDirectory(prefix="Omnivox TGSpeechBox source verification ") as temporary:
        working = Path(temporary)
        extracted = working / "Extracted source with spaces"
        extracted.mkdir()
        common.extract_tar(archive, extracted)
        root_name = f"omnivox-{arguments.version}-tgspeechbox-source"
        require(
            {path.name for path in extracted.iterdir()} == {root_name},
            "source archive must have one versioned top-level directory",
        )
        root = extracted / root_name
        require(
            {path.name for path in root.iterdir()}
            == {"README.md", "SOURCE-MANIFEST.json", "inputs", "omnivox"},
            "unexpected source archive root entries",
        )
        require((root / "README.md").stat().st_size > 500, "source README is incomplete")
        source = root / "omnivox"
        require(
            repository_version(source) == arguments.version,
            "packaged source version mismatch",
        )
        require(
            (source / "vendor/espeak-rs-sys-0.1.9/espeak-ng/src/ucd-tools/COPYING").stat().st_size
            > 30_000,
            "vendored eSpeak NG corresponding source is missing",
        )
        manifest = verify_manifest(root, arguments.version, arguments.commit)
        verify_committed_source(source, repository, arguments.commit)
        verify_input(root, source, manifest, working)
        verify_offline_cargo(source, working)
    print(f"PASS {archive.name}: source, input, manifest, and offline Cargo verification")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        verify(parse_arguments(repository), repository)
    except (
        OSError,
        common.VerificationError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
        tarfile.TarError,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
