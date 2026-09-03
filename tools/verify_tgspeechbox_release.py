#!/usr/bin/env python3
"""Verify the Windows x64 TGSpeechBox companion and real synthesis."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

sys.dont_write_bytecode = True
import verify_release as common
from build_tgspeechbox import (
    COMMIT,
    DEFAULT_SAMPLE_RATE,
    EXPECTED_VOICE_COUNT,
    RELEASE,
    SUPPORTED_TARGETS,
    SUPPORTED_SAMPLE_RATES,
    VOICE_INVENTORY_FILENAME,
    VOICE_INVENTORY_FILENAMES,
)
from package_tgspeechbox import (
    EXPECTED_NOTICES,
    EXPECTED_ROOT,
    RELEASE_SUFFIX,
    RELEASE_TARGET,
)


class TGSpeechBoxVerificationError(common.VerificationError):
    """A TGSpeechBox companion violated its release contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise TGSpeechBoxVerificationError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/x86_64-pc-windows-gnu/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=release / f"omnivox-{version}-tgspeechbox-{RELEASE_SUFFIX}.zip",
    )
    parser.add_argument(
        "--checksums", type=Path, default=release / "tgspeechbox-sha256sums.txt"
    )
    parser.add_argument("--version", default=version)
    parser.add_argument("--target", default=RELEASE_TARGET)
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--omnivox-archive", type=Path)
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


def verify_inventory(
    path: Path, source_identity: str, sample_rate: int
) -> dict[str, object]:
    inventory = json.loads(path.read_text(encoding="utf-8"))
    require(inventory.get("schema_version") == 1, f"wrong inventory schema: {path.name}")
    require(inventory.get("engine_id") == "tgspeechbox", f"wrong engine inventory: {path.name}")
    require(
        inventory.get("source_identity") == source_identity,
        f"wrong inventory source identity: {path.name}",
    )
    descriptor = inventory.get("descriptor")
    require(isinstance(descriptor, dict), f"missing descriptor: {path.name}")
    require(descriptor.get("id") == "tgspeechbox", f"wrong descriptor: {path.name}")
    require(
        f"native {sample_rate} Hz" in str(descriptor.get("version", "")),
        f"wrong inventory sample rate: {path.name}",
    )
    voices = descriptor.get("voices")
    require(
        isinstance(voices, list) and len(voices) == EXPECTED_VOICE_COUNT,
        f"wrong voice count: {path.name}",
    )
    return inventory


def verify_layout(extracted: Path, target: str) -> Path:
    require(target == RELEASE_TARGET, f"unsupported release target: {target}")
    stage_suffix, helper_name = SUPPORTED_TARGETS[target]
    require(
        {path.name for path in extracted.iterdir()} == {"tgspeechbox"},
        "archive must contain exactly one top-level tgspeechbox directory",
    )
    directory = extracted / "tgspeechbox"
    require(
        {path.name for path in directory.iterdir()} == EXPECTED_ROOT | {helper_name},
        "TGSpeechBox companion root entries are incomplete or unexpected",
    )
    notices = directory / "third-party-licenses"
    require(
        {path.name for path in notices.iterdir()} == EXPECTED_NOTICES,
        "TGSpeechBox notice set is incomplete or unexpected",
    )
    require((directory / "packs/phonemes.yaml").is_file(), "TGSpeechBox packs are missing")
    require((directory / "espeak-ng-data/phontab").is_file(), "eSpeak data is missing")
    verify_inner_checksums(directory)

    source_lock = directory / "source-inputs.json"
    source_identity = sha256_file(source_lock)
    for sample_rate in SUPPORTED_SAMPLE_RATES:
        verify_inventory(
            directory / VOICE_INVENTORY_FILENAMES[sample_rate],
            source_identity,
            sample_rate,
        )
    default_inventory = verify_inventory(
        directory / VOICE_INVENTORY_FILENAME,
        source_identity,
        DEFAULT_SAMPLE_RATE,
    )
    require(
        default_inventory
        == json.loads(
            (directory / VOICE_INVENTORY_FILENAMES[DEFAULT_SAMPLE_RATE]).read_text(
                encoding="utf-8"
            )
        ),
        "default voice inventory does not match the default sample rate",
    )

    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(provenance.get("schema_version") == 1, "unknown provenance schema")
    require(
        provenance.get("artifact") == f"omnivox-tgspeechbox-companion-{stage_suffix}",
        "wrong artifact provenance",
    )
    require(provenance.get("target") == target, "wrong target provenance")
    require(provenance.get("markers_advertised") is True, "marker support is missing")
    require(
        provenance.get("marker_support") == "exact_requested_anchors",
        "requested-anchor support is missing",
    )
    require(
        provenance.get("rate_mapping") == "calibrated_eloquence_v1",
        "calibrated rate status is missing",
    )
    tgspeechbox = provenance.get("tgspeechbox")
    require(isinstance(tgspeechbox, dict), "TGSpeechBox provenance is missing")
    require(tgspeechbox.get("release") == RELEASE, "wrong source revision")
    require(tgspeechbox.get("commit") == COMMIT, "wrong source commit")
    require(tgspeechbox.get("verified_before_build") is True, "source was not verified")
    for field in ("archive_sha256", "source_tree_sha256"):
        require(
            re.fullmatch(r"[0-9a-f]{64}", str(tgspeechbox.get(field, ""))) is not None,
            f"invalid TGSpeechBox {field}",
        )
    return directory / helper_name


def verify_helper(
    helper: Path, repository: Path, working: Path, iterations: int
) -> None:
    if os.name != "nt":
        helper.chmod(helper.stat().st_mode | 0o755)
    command = [
        sys.executable,
        str(repository / "tools/stress_helper.py"),
        str(helper.resolve()),
        "--engine-id",
        "tgspeechbox",
        "--iterations",
        str(iterations),
        "--cancel-probe",
        "--health-every",
        "5",
        "--require-streaming",
    ]
    for dimension in ("rate", "average_pitch", "pitch_range", "volume"):
        command.extend(["--require-acss", dimension])
    common.run(command, working, common.clean_environment())


def verify_omnivox_synthesis(
    companion: Path,
    archive: Path,
    checksums: Path,
    version: str,
    working: Path,
) -> None:
    require(archive.is_file(), f"matching Omnivox archive is missing: {archive}")
    require(
        archive.name == f"omnivox-{version}-windows-x64.zip",
        f"unexpected Omnivox archive name: {archive.name}",
    )
    common.verify_checksum(archive, checksums)
    installed = working.parent / "Installed Omnivox with TGSpeechBox"
    installed.mkdir()
    common.extract_zip(archive, installed)
    omnivox = common.verify_layout(installed, "windows", version)
    common.verify_architecture(omnivox, "windows", "x86_64")
    if os.name != "nt":
        omnivox.chmod(omnivox.stat().st_mode | 0o755)
    shutil.copytree(companion, installed / "tgspeechbox")

    environment = common.clean_environment()
    command = str(omnivox.resolve())
    voices = common.run(
        [command, "--engine", "tgspeechbox", "--list-voices"], working, environment
    )
    require("Found 154 voices" in voices, "TGSpeechBox returned the wrong inventory")
    require("[en-us/adam]" in voices, "release voice is missing")
    wav = working / "tgspeechbox release probe.wav"
    common.run(
        [
            command,
            "--engine",
            "tgspeechbox",
            "--dump-wav",
            "en-us/adam",
            str(wav),
            "TGSpeechBox relocated release verification.",
        ],
        working,
        environment,
    )
    raw = wav.with_name("tgspeechbox release probe_raw.wav")
    require(wav.is_file() and raw.is_file(), "TGSpeechBox did not write both WAV outputs")
    common.read_wav(raw, canonical=False)
    common.read_wav(wav, canonical=True)


def verify(arguments: argparse.Namespace, repository: Path) -> None:
    require(arguments.target == RELEASE_TARGET, f"unsupported release target: {arguments.target}")
    require(arguments.iterations > 0, "iterations must be positive")
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"archive does not exist: {archive}")
    require(checksums.is_file(), f"checksum file does not exist: {checksums}")
    require(
        archive.name
        == f"omnivox-{arguments.version}-tgspeechbox-{RELEASE_SUFFIX}.zip",
        f"unexpected TGSpeechBox archive name: {archive.name}",
    )
    common.verify_checksum(archive, checksums)
    with tempfile.TemporaryDirectory(prefix="Omnivox TGSpeechBox verification ") as temporary:
        root = Path(temporary)
        extracted = root / "Extracted companion with spaces"
        working = root / "Unrelated working directory"
        extracted.mkdir()
        working.mkdir()
        common.extract_zip(archive, extracted)
        helper = verify_layout(extracted, arguments.target)
        common.verify_architecture(helper, "windows", "x86_64")
        verify_helper(helper, repository, working, arguments.iterations)
        if arguments.omnivox_archive is not None:
            verify_omnivox_synthesis(
                extracted / "tgspeechbox",
                arguments.omnivox_archive.resolve(),
                checksums,
                arguments.version,
                working,
            )
    mode = "structure, relocation, inventory, and helper synthesis"
    if arguments.omnivox_archive is not None:
        mode += " with exact Omnivox routing"
    print(f"PASS {archive.name}: {mode}")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        verify(parse_arguments(repository), repository)
    except (
        OSError,
        common.VerificationError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
