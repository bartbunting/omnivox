#!/usr/bin/env python3
"""Verify an Omnivox release archive and, where native, exercise its engines."""

from __future__ import annotations

import argparse
import hashlib
import math
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import zipfile

import archive_paths


class VerificationError(RuntimeError):
    """A release artifact violated the published contract."""


LEGACY_RELEASES_WITHOUT_PROJECT_LICENSES = {"1.4.1"}
FIRST_RELEASE_WITH_RHVOICE_HELPER = (1, 5, 1)
FIRST_RELEASE_WITH_WINDOWS_RUNTIME_HELPERS = (1, 7, 1)
MAX_ARCHIVE_MEMBERS = 100_000
MAX_ARCHIVE_UNCOMPRESSED_BYTES = 4 * 1024 * 1024 * 1024


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationError(message)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Verify checksums, payload layout, architecture, relocation, voice "
            "discovery, and headless synthesis for an Omnivox release archive."
        )
    )
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--checksums", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument(
        "--platform", required=True, choices=("linux", "macos", "windows")
    )
    parser.add_argument("--arch", required=True, choices=("x86_64", "aarch64"))
    parser.add_argument(
        "--engines",
        default="",
        help="Comma-separated engines to execute, or empty for structural checks only",
    )
    return parser.parse_args()


def verify_checksum(archive: Path, checksums: Path) -> None:
    expected: str | None = None
    for line in checksums.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64})\s+\*?(.+)", line)
        if match and Path(match.group(2)).name == archive.name:
            expected = match.group(1).lower()
            break
    require(expected is not None, f"checksum entry is missing for {archive.name}")

    digest = hashlib.sha256()
    with archive.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    require(digest.hexdigest() == expected, f"checksum mismatch for {archive.name}")


def safe_parts(name: str) -> tuple[str, ...]:
    return archive_paths.safe_parts(name, VerificationError)


def extract_tar(archive: Path, destination: Path) -> None:
    with tarfile.open(archive, "r:gz") as bundle:
        seen: set[PurePosixPath] = set()
        total_bytes = 0
        for index, member in enumerate(bundle, start=1):
            require(
                index <= MAX_ARCHIVE_MEMBERS,
                f"tar archive exceeds the {MAX_ARCHIVE_MEMBERS}-member limit",
            )
            member_path = PurePosixPath(*safe_parts(member.name))
            require(member_path not in seen, f"duplicate tar member: {member.name!r}")
            seen.add(member_path)
            target = archive_paths.extraction_target(
                destination, member_path.parts, VerificationError
            )
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            require(member.isfile(), f"unsupported tar member: {member.name!r}")
            total_bytes += member.size
            require(
                total_bytes <= MAX_ARCHIVE_UNCOMPRESSED_BYTES,
                "tar archive exceeds the "
                f"{MAX_ARCHIVE_UNCOMPRESSED_BYTES}-byte uncompressed limit",
            )
            source = bundle.extractfile(member)
            require(source is not None, f"cannot read tar member: {member.name!r}")
            target.parent.mkdir(parents=True, exist_ok=True)
            with source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
            target.chmod(member.mode & 0o777)


def extract_zip(archive: Path, destination: Path) -> None:
    with zipfile.ZipFile(archive) as bundle:
        members = bundle.infolist()
        require(
            len(members) <= MAX_ARCHIVE_MEMBERS,
            f"zip archive exceeds the {MAX_ARCHIVE_MEMBERS}-member limit",
        )
        total_bytes = sum(member.file_size for member in members if not member.is_dir())
        require(
            total_bytes <= MAX_ARCHIVE_UNCOMPRESSED_BYTES,
            "zip archive exceeds the "
            f"{MAX_ARCHIVE_UNCOMPRESSED_BYTES}-byte uncompressed limit",
        )
        seen: set[PurePosixPath] = set()
        for member in members:
            member_path = PurePosixPath(*safe_parts(member.filename))
            require(
                member_path not in seen,
                f"duplicate zip member: {member.filename!r}",
            )
            seen.add(member_path)
            target = archive_paths.extraction_target(
                destination, member_path.parts, VerificationError
            )
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            mode = member.external_attr >> 16
            require(
                member.flag_bits & 0x1 == 0,
                f"encrypted zip member is not allowed: {member.filename!r}",
            )
            require(
                (mode & 0o170000) != 0o120000,
                f"zip symlink is not allowed: {member.filename!r}",
            )
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)


def extract_archive(archive: Path, destination: Path, platform: str) -> None:
    if platform == "windows":
        require(archive.suffix.lower() == ".zip", "Windows release must be a .zip")
        extract_zip(archive, destination)
    else:
        require(archive.name.endswith(".tar.gz"), "Unix release must be a .tar.gz")
        extract_tar(archive, destination)


def verify_layout(root: Path, platform: str, version: str) -> Path:
    binary_name = "omnivox.exe" if platform == "windows" else "omnivox"
    project_license_entries = {"LICENSE", "LICENSING.md"}
    expected_root = {
        binary_name,
        "omnivox-voices.el",
        "espeak-ng-data",
        "third-party-licenses",
    }
    if version not in LEGACY_RELEASES_WITHOUT_PROJECT_LICENSES:
        expected_root.update(project_license_entries)
    version_numbers = tuple(int(value) for value in version.split("-", 1)[0].split(".")[:3])
    if version_numbers >= FIRST_RELEASE_WITH_RHVOICE_HELPER:
        expected_root.add("rhvoice")
    if (
        platform == "windows"
        and version_numbers >= FIRST_RELEASE_WITH_WINDOWS_RUNTIME_HELPERS
    ):
        expected_root.update(
            {
                "OmnivoxDectalkHelper32.exe",
                "OmnivoxEloquenceHelper32.exe",
                "WINDOWS-HELPERS-COPYING",
                "windows-helpers-source",
            }
        )
    actual_root = {path.name for path in root.iterdir()}
    allowed_roots = {frozenset(expected_root)}
    if version in LEGACY_RELEASES_WITHOUT_PROJECT_LICENSES:
        allowed_roots.add(frozenset(expected_root | project_license_entries))
    require(
        frozenset(actual_root) in allowed_roots,
        f"unexpected archive root entries: {sorted(actual_root)}",
    )

    binary = root / binary_name
    require(
        binary.is_file() and binary.stat().st_size > 0,
        "main binary is missing or empty",
    )
    if platform != "windows":
        require(binary.stat().st_mode & 0o111 != 0, "Unix binary is not executable")

    adapter = root / "omnivox-voices.el"
    require(
        adapter.is_file() and adapter.stat().st_size > 100,
        "Emacspeak adapter is missing or empty",
    )

    for name in sorted(project_license_entries & actual_root):
        project_license = root / name
        require(
            project_license.is_file() and project_license.stat().st_size > 100,
            f"{name} is missing or empty",
        )

    data = root / "espeak-ng-data"
    require((data / "phontab").is_file(), "espeak-ng-data/phontab is missing")
    data_files = sum(path.is_file() for path in data.rglob("*"))
    require(data_files >= 100, f"eSpeak data payload is unexpectedly small ({data_files} files)")

    notices = root / "third-party-licenses"
    for name in (
        "THIRD-PARTY-NOTICES.md",
        "omnivox-Cargo.lock",
        "eSpeak-NG-GPL-3.0.txt",
        "Unicode-Data-License.txt",
        "NetBSD-getopt.c",
    ):
        require((notices / name).is_file(), f"third-party-licenses/{name} is missing")

    lock_text = (notices / "omnivox-Cargo.lock").read_text(encoding="utf-8")
    workspace_versions: dict[str, str] = {}
    for package in re.split(r"^\[\[package\]\]\s*$", lock_text, flags=re.MULTILINE):
        name_match = re.search(r'^name = "([^"]+)"$', package, flags=re.MULTILINE)
        version_match = re.search(r'^version = "([^"]+)"$', package, flags=re.MULTILINE)
        if name_match and version_match and name_match.group(1).startswith("omnivox-"):
            workspace_versions[name_match.group(1)] = version_match.group(1)
    require(
        workspace_versions,
        "packaged Cargo.lock has no Omnivox workspace packages",
    )
    wrong_versions = sorted(
        f"{name}={package_version}"
        for name, package_version in workspace_versions.items()
        if package_version != version
    )
    require(not wrong_versions, f"packaged workspace version mismatch: {wrong_versions}")

    if "rhvoice" in actual_root:
        helper_name = (
            "omnivox-rhvoice-helper.exe"
            if platform == "windows"
            else "omnivox-rhvoice-helper"
        )
        rhvoice = root / "rhvoice"
        require(
            {path.name for path in rhvoice.iterdir()} == {helper_name},
            "RHVoice helper directory is incomplete or unexpected",
        )
        helper = rhvoice / helper_name
        require(helper.stat().st_size > 0, "RHVoice helper is empty")
        if platform != "windows":
            require(helper.stat().st_mode & 0o111 != 0, "RHVoice helper is not executable")
    if (
        platform == "windows"
        and version_numbers >= FIRST_RELEASE_WITH_WINDOWS_RUNTIME_HELPERS
    ):
        for name in (
            "OmnivoxDectalkHelper32.exe",
            "OmnivoxEloquenceHelper32.exe",
        ):
            helper = root / name
            require(
                helper.is_file() and helper.stat().st_size > 0,
                f"{name} is missing or empty",
            )
            verify_windows_x86_architecture(helper)
        copying = root / "WINDOWS-HELPERS-COPYING"
        require(
            copying.is_file() and copying.stat().st_size > 10_000,
            "Windows helper GPL notice is missing or incomplete",
        )
        verify_windows_helper_source(root / "windows-helpers-source")
    return binary


def verify_windows_helper_source(source: Path) -> None:
    """Require the exact buildable source shipped for Windows bridge helpers."""
    expected = {
        "COPYING",
        "Makefile",
        "README.md",
        "build.ps1",
        "common/OmnivoxHelperHost.cs",
        "common/OmnivoxNativeLibrary.cs",
        "dectalk/OmnivoxDectalkCapture.cs",
        "dectalk/OmnivoxDectalkHelper.cs",
        "eloquence/OmnivoxEloquenceCapture.cs",
        "eloquence/OmnivoxEloquenceHelper.cs",
    }
    require(source.is_dir(), "Windows helper corresponding source is missing")
    actual = {
        path.relative_to(source).as_posix()
        for path in source.rglob("*")
        if path.is_file()
    }
    require(
        actual == expected,
        f"unexpected Windows helper source entries: {sorted(actual)}",
    )


def verify_windows_x86_architecture(binary: Path) -> None:
    """Require BINARY to be a 32-bit x86 Windows PE executable."""
    data = binary.read_bytes()[:4096]
    require(
        len(data) >= 64 and data[:2] == b"MZ",
        f"Windows helper is not PE: {binary}",
    )
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    require(
        pe_offset + 6 <= len(data)
        and data[pe_offset : pe_offset + 4] == b"PE\0\0",
        f"Windows helper PE header is invalid: {binary}",
    )
    machine = struct.unpack_from("<H", data, pe_offset + 4)[0]
    require(
        machine == 0x14C,
        f"Windows helper architecture mismatch: found 0x{machine:x}, expected x86",
    )


def verify_architecture(binary: Path, platform: str, arch: str) -> None:
    data = binary.read_bytes()[:4096]
    require(len(data) >= 64, f"binary is too short: {binary}")
    if platform == "linux":
        require(data[:4] == b"\x7fELF", "Linux binary is not ELF")
        require(data[4:6] == b"\x02\x01", "Linux binary is not 64-bit little-endian ELF")
        machine = struct.unpack_from("<H", data, 18)[0]
        expected = {"x86_64": 62, "aarch64": 183}[arch]
    elif platform == "windows":
        require(data[:2] == b"MZ", "Windows binary is not PE")
        pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
        require(
            pe_offset + 6 <= len(data),
            "Windows PE header is outside the inspected prefix",
        )
        require(
            data[pe_offset : pe_offset + 4] == b"PE\0\0",
            "Windows PE signature is missing",
        )
        machine = struct.unpack_from("<H", data, pe_offset + 4)[0]
        expected = {"x86_64": 0x8664, "aarch64": 0xAA64}[arch]
    else:
        require(
            data[:4] == b"\xcf\xfa\xed\xfe",
            "macOS binary is not little-endian Mach-O 64-bit",
        )
        machine = struct.unpack_from("<I", data, 4)[0]
        expected = {"x86_64": 0x01000007, "aarch64": 0x0100000C}[arch]
    require(
        machine == expected,
        f"binary architecture mismatch: found 0x{machine:x}, expected 0x{expected:x}",
    )


def run(command: list[str], cwd: Path, environment: dict[str, str]) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=120,
        check=False,
    )
    if result.returncode != 0:
        rendered = " ".join(command)
        raise VerificationError(
            f"command failed ({result.returncode}): {rendered}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result.stdout


def read_wav(path: Path, canonical: bool) -> None:
    content = path.read_bytes()
    require(len(content) >= 44, f"WAV is too short: {path}")
    require(
        content[:4] == b"RIFF" and content[8:12] == b"WAVE",
        f"invalid WAV header: {path}",
    )

    format_chunk: bytes | None = None
    audio_data: bytes | None = None
    offset = 12
    while offset + 8 <= len(content):
        chunk_name = content[offset : offset + 4]
        chunk_size = struct.unpack_from("<I", content, offset + 4)[0]
        chunk_start = offset + 8
        chunk_end = chunk_start + chunk_size
        require(chunk_end <= len(content), f"truncated WAV chunk in {path}")
        if chunk_name == b"fmt ":
            format_chunk = content[chunk_start:chunk_end]
        elif chunk_name == b"data":
            audio_data = content[chunk_start:chunk_end]
        offset = chunk_end + (chunk_size & 1)

    require(
        format_chunk is not None and len(format_chunk) >= 16,
        f"WAV format chunk is missing: {path}",
    )
    require(
        audio_data is not None and len(audio_data) > 0,
        f"WAV audio data is missing: {path}",
    )
    audio_format, channels, sample_rate, _, block_align, bits = struct.unpack_from(
        "<HHIIHH", format_chunk
    )
    require(audio_format == 3 and bits == 32, f"WAV is not IEEE float32: {path}")
    require(channels in (1, 2), f"unexpected WAV channel count {channels}: {path}")
    require(8_000 <= sample_rate <= 96_000, f"unexpected WAV sample rate {sample_rate}: {path}")
    require(block_align == channels * 4, f"invalid WAV block alignment: {path}")
    if canonical:
        require(
            channels == 2 and sample_rate == 44_100,
            f"processed WAV is not canonical: {path}",
        )

    require(len(audio_data) % 4 == 0, f"unaligned float samples: {path}")
    peak = 0.0
    for (sample,) in struct.iter_unpack("<f", audio_data):
        require(math.isfinite(sample), f"non-finite sample in {path}")
        peak = max(peak, abs(sample))
    frames = len(audio_data) // block_align
    duration = frames / sample_rate
    require(0.05 <= duration <= 60.0, f"unexpected WAV duration {duration:.3f}s: {path}")
    require(peak > 0.0001, f"WAV is effectively silent: {path}")


def clean_environment() -> dict[str, str]:
    environment = dict(os.environ)
    for key in list(environment):
        if key.upper() == "ESPEAK_NG_DATA" or key.upper().startswith("OMNIVOX_"):
            del environment[key]
    return environment


def verify_execution(
    binary: Path,
    version: str,
    engines: list[str],
    working: Path,
    platform: str,
) -> None:
    environment = clean_environment()
    command = str(binary.resolve())
    version_output = run([command, "--version"], working, environment).strip()
    require(
        version_output == f"omnivox {version}",
        f"unexpected version output: {version_output!r}",
    )
    require(
        "USAGE:" in run([command, "--help"], working, environment),
        "help output is incomplete",
    )

    if platform == "linux":
        dependencies = run(["ldd", command], working, environment)
        require(
            "not found" not in dependencies,
            f"Linux binary has unresolved dependencies:\n{dependencies}",
        )

    probe_text = "Release verification: café, naïve, Ελληνικά, 日本語."
    for engine in engines:
        voices = run([command, "--engine", engine, "--list-voices"], working, environment)
        match = re.search(r"Found\s+(\d+)\s+voices", voices)
        require(
            match is not None and int(match.group(1)) > 0,
            f"{engine} returned no voices",
        )

        output = working / f"{engine}-release-probe.wav"
        run(
            [command, "--engine", engine, "--dump-wav", "", str(output), probe_text],
            working,
            environment,
        )
        raw_output = output.with_name(output.name.replace(".wav", "_raw.wav"))
        require(
            output.is_file() and raw_output.is_file(),
            f"{engine} did not write both WAV outputs",
        )
        read_wav(raw_output, canonical=False)
        read_wav(output, canonical=True)


def verify(arguments: argparse.Namespace) -> None:
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"archive does not exist: {archive}")
    require(checksums.is_file(), f"checksum file does not exist: {checksums}")
    verify_checksum(archive, checksums)

    engines = [
        engine.strip() for engine in arguments.engines.split(",") if engine.strip()
    ]
    with tempfile.TemporaryDirectory(prefix="Omnivox release verification ") as temporary:
        temporary_root = Path(temporary)
        extracted = temporary_root / "Extracted release with spaces"
        working = temporary_root / "Unrelated working directory"
        extracted.mkdir()
        working.mkdir()
        extract_archive(archive, extracted, arguments.platform)
        binary = verify_layout(extracted, arguments.platform, arguments.version)
        verify_architecture(binary, arguments.platform, arguments.arch)
        rhvoice_helpers = list((extracted / "rhvoice").glob("omnivox-rhvoice-helper*"))
        if rhvoice_helpers:
            require(len(rhvoice_helpers) == 1, "ambiguous RHVoice helper payload")
            verify_architecture(rhvoice_helpers[0], arguments.platform, arguments.arch)
            if arguments.platform == "linux":
                dependencies = run(
                    ["ldd", str(rhvoice_helpers[0].resolve())],
                    working,
                    clean_environment(),
                )
                require(
                    "not found" not in dependencies,
                    f"RHVoice helper has unresolved dependencies:\n{dependencies}",
                )
        if engines:
            verify_execution(binary, arguments.version, engines, working, arguments.platform)
        version_numbers = tuple(
            int(value)
            for value in arguments.version.split("-", 1)[0].split(".")[:3]
        )
        if (
            arguments.platform == "windows"
            and version_numbers >= FIRST_RELEASE_WITH_WINDOWS_RUNTIME_HELPERS
        ):
            run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("test_windows_helper_startup.py")),
                    "--helpers",
                    str(extracted),
                ],
                working,
                clean_environment(),
            )

    mode = "structural" if not engines else f"structural and engine ({', '.join(engines)})"
    print(f"PASS {archive.name}: {mode} verification")


def main() -> int:
    try:
        verify(parse_arguments())
    except (
        OSError,
        VerificationError,
        subprocess.TimeoutExpired,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
