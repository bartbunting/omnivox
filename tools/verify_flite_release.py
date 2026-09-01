#!/usr/bin/env python3
"""Verify a Flite companion archive and exercise real SLT synthesis."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tempfile
import tomllib

sys.dont_write_bytecode = True
import verify_release as common
from build_flite import FLITE_COMMIT, FLITE_VERSION, SUPPORTED_TARGETS, host_target


EXPECTED_ROOT = {
    "LICENSE",
    "LICENSING.md",
    "README.md",
    "SHA256SUMS",
    "SOURCE-PROVENANCE.json",
    "source-inputs.json",
    "third-party-licenses",
}


class FliteVerificationError(common.VerificationError):
    """A Flite companion violated its release contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise FliteVerificationError(message)


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_arguments(repository: Path, target: str) -> argparse.Namespace:
    version = repository_version(repository)
    suffix, _ = SUPPORTED_TARGETS[target]
    extension = "zip" if suffix.startswith("windows-") else "tar.gz"
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=release / f"omnivox-{version}-flite-{suffix}.{extension}",
    )
    parser.add_argument(
        "--checksums", type=Path, default=release / "flite-sha256sums.txt"
    )
    parser.add_argument("--version", default=version)
    parser.add_argument("--iterations", type=int, default=5)
    return parser.parse_args()


def safe_checksum_path(value: str) -> str:
    require("\\" not in value, f"checksum path uses a backslash: {value!r}")
    path = PurePosixPath(value)
    require(not path.is_absolute(), f"checksum path is absolute: {value!r}")
    parts = tuple(part for part in path.parts if part not in ("", "."))
    require(parts and ".." not in parts, f"unsafe checksum path: {value!r}")
    return PurePosixPath(*parts).as_posix()


def verify_inner_checksums(directory: Path) -> None:
    entries: dict[str, str] = {}
    for line in (directory / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, f"invalid inner checksum line: {line!r}")
        relative = safe_checksum_path(match.group(2))
        require(relative not in entries, f"duplicate inner checksum: {relative}")
        entries[relative] = match.group(1)
    actual = {
        path.relative_to(directory).as_posix(): sha256_file(path)
        for path in directory.rglob("*")
        if path.is_file() and path.name != "SHA256SUMS"
    }
    require(entries == actual, "inner checksums do not cover the exact payload")


def verify_layout(extracted: Path, target: str) -> Path:
    suffix, helper_name = SUPPORTED_TARGETS[target]
    require(
        {path.name for path in extracted.iterdir()} == {"flite"},
        "archive must contain exactly one top-level flite directory",
    )
    directory = extracted / "flite"
    require(
        {path.name for path in directory.iterdir()} == EXPECTED_ROOT | {helper_name},
        "Flite companion root entries are incomplete or unexpected",
    )
    notices = directory / "third-party-licenses"
    require(
        {path.name for path in notices.iterdir()}
        == {"Flite-COPYING.txt", "omnivox-Cargo.lock"},
        "Flite notice set is incomplete or unexpected",
    )
    require((notices / "Flite-COPYING.txt").stat().st_size > 1_000, "Flite licence is empty")
    require(not any(path.suffix == ".flitevox" for path in directory.rglob("*")), "external voice file was packaged")
    verify_inner_checksums(directory)

    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(provenance.get("schema_version") == 1, "unknown provenance schema")
    require(
        provenance.get("artifact") == f"omnivox-flite-companion-{suffix}",
        "wrong artifact provenance",
    )
    require(provenance.get("target") == target, "wrong target provenance")
    require(provenance.get("compiled_voice") == "cmu_us_slt", "wrong bundled voice")
    require(provenance.get("external_voice_files_included") is False, "voice exclusion is not recorded")
    flite = provenance.get("flite")
    require(isinstance(flite, dict), "Flite provenance is missing")
    require(flite.get("version") == FLITE_VERSION, "wrong Flite version")
    require(flite.get("commit") == FLITE_COMMIT, "wrong Flite commit")
    require(flite.get("verified_before_build") is True, "Flite source was not verified")
    for field in ("archive_sha256", "source_tree_sha256"):
        require(
            re.fullmatch(r"[0-9a-f]{64}", str(flite.get(field, ""))) is not None,
            f"invalid Flite {field}",
        )
    helper = directory / helper_name
    if not suffix.startswith("windows-"):
        require(helper.stat().st_mode & 0o111 != 0, "Flite helper is not executable")
    return helper


def verify(arguments: argparse.Namespace, repository: Path, target: str) -> None:
    suffix, _ = SUPPORTED_TARGETS[target]
    platform = suffix.split("-", 1)[0]
    arch = "aarch64" if suffix.endswith("arm64") else "x86_64"
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"archive does not exist: {archive}")
    require(checksums.is_file(), f"checksum file does not exist: {checksums}")
    require(arguments.iterations > 0, "iterations must be positive")
    common.verify_checksum(archive, checksums)

    with tempfile.TemporaryDirectory(prefix="Omnivox Flite verification ") as temporary:
        root = Path(temporary)
        extracted = root / "Extracted companion with spaces"
        working = root / "Unrelated working directory"
        extracted.mkdir()
        working.mkdir()
        common.extract_archive(archive, extracted, platform)
        helper = verify_layout(extracted, target)
        common.verify_architecture(helper, platform, arch)
        environment = common.clean_environment()
        if platform == "linux":
            dependencies = common.run(["ldd", str(helper.resolve())], working, environment)
            require("not found" not in dependencies, f"unresolved helper dependency:\n{dependencies}")
        command = [
            sys.executable,
            str(repository / "tools/stress_helper.py"),
            str(helper.resolve()),
            "--engine-id",
            "flite",
            "--iterations",
            str(arguments.iterations),
            "--cancel-probe",
            "--require-acss",
            "rate",
            "--require-acss",
            "average_pitch",
            "--require-acss",
            "volume",
        ]
        common.run(command, working, environment)
    print(f"PASS {archive.name}: structure, relocation, and SLT synthesis")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        target = host_target()
        require(target in SUPPORTED_TARGETS, f"unsupported native Flite target: {target}")
        verify(parse_arguments(repository, target), repository, target)
    except (
        OSError,
        FliteVerificationError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
