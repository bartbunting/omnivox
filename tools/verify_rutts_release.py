#!/usr/bin/env python3
"""Verify a RuTTS companion archive and exercise both built-in voices."""

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
from build_rutts import RUTTS_COMMIT, RUTTS_VERSION, SUPPORTED_TARGETS, host_target


EXPECTED_ROOT = {
    "LICENSE",
    "LICENSING.md",
    "README.md",
    "SHA256SUMS",
    "SOURCE-PROVENANCE.json",
    "source-inputs.json",
    "third-party-licenses",
}


class RuttsVerificationError(common.VerificationError):
    """A RuTTS companion violated its release contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuttsVerificationError(message)


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
        default=release / f"omnivox-{version}-rutts-{suffix}.{extension}",
    )
    parser.add_argument(
        "--checksums", type=Path, default=release / "rutts-sha256sums.txt"
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
        {path.name for path in extracted.iterdir()} == {"rutts"},
        "archive must contain exactly one top-level rutts directory",
    )
    directory = extracted / "rutts"
    require(
        {path.name for path in directory.iterdir()} == EXPECTED_ROOT | {helper_name},
        "RuTTS companion root entries are incomplete or unexpected",
    )
    notices = directory / "third-party-licenses"
    require(
        {path.name for path in notices.iterdir()}
        == {"RuTTS-LICENSE.txt", "omnivox-Cargo.lock"},
        "RuTTS notice set is incomplete or unexpected",
    )
    require((notices / "RuTTS-LICENSE.txt").stat().st_size > 500, "RuTTS licence is empty")
    require(
        not any("rulex" in path.name.lower() for path in directory.rglob("*")),
        "RuLex file was packaged",
    )
    verify_inner_checksums(directory)

    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(provenance.get("schema_version") == 1, "unknown provenance schema")
    require(
        provenance.get("artifact") == f"omnivox-rutts-companion-{suffix}",
        "wrong artifact provenance",
    )
    require(provenance.get("target") == target, "wrong target provenance")
    require(
        provenance.get("built_in_voices") == ["male", "female"],
        "wrong built-in voice set",
    )
    require(provenance.get("rulex_included") is False, "RuLex exclusion is not recorded")
    rutts = provenance.get("rutts")
    require(isinstance(rutts, dict), "RuTTS provenance is missing")
    require(rutts.get("version") == RUTTS_VERSION, "wrong RuTTS version")
    require(rutts.get("commit") == RUTTS_COMMIT, "wrong RuTTS commit")
    require(rutts.get("verified_before_build") is True, "RuTTS source was not verified")
    for field in ("archive_sha256", "source_tree_sha256"):
        require(
            re.fullmatch(r"[0-9a-f]{64}", str(rutts.get(field, ""))) is not None,
            f"invalid RuTTS {field}",
        )
    helper = directory / helper_name
    if not suffix.startswith("windows-"):
        require(helper.stat().st_mode & 0o111 != 0, "RuTTS helper is not executable")
    return helper


def stress_command(
    repository: Path, helper: Path, iterations: int, voice: str
) -> list[str]:
    return [
        sys.executable,
        str(repository / "tools/stress_helper.py"),
        str(helper.resolve()),
        "--engine-id",
        "rutts",
        "--voice-id",
        voice,
        "--iterations",
        str(iterations),
        "--cancel-probe",
        "--require-acss",
        "rate",
        "--require-acss",
        "average_pitch",
        "--require-acss",
        "pitch_range",
        "--require-acss",
        "volume",
    ]


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

    with tempfile.TemporaryDirectory(prefix="Omnivox RuTTS verification ") as temporary:
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
        common.run(stress_command(repository, helper, arguments.iterations, "male"), working, environment)
        common.run(
            stress_command(repository, helper, min(arguments.iterations, 5), "female"),
            working,
            environment,
        )
    print(f"PASS {archive.name}: structure, relocation, and both RuTTS voices")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        target = host_target()
        require(target in SUPPORTED_TARGETS, f"unsupported native RuTTS target: {target}")
        verify(parse_arguments(repository, target), repository, target)
    except (
        OSError,
        RuttsVerificationError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
