#!/usr/bin/env python3
"""Create the deterministic Windows x64 TGSpeechBox companion archive."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import zipfile

sys.dont_write_bytecode = True
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


RELEASE_TARGET = "x86_64-pc-windows-gnu"
RELEASE_SUFFIX = "windows-x64"
EXPECTED_ROOT = {
    "LICENSE",
    "LICENSING.md",
    "README.md",
    "SHA256SUMS",
    "SOURCE-PROVENANCE.json",
    "VOICE-INVENTORY-22050.json",
    "VOICE-INVENTORY-44100.json",
    "VOICE-INVENTORY.json",
    "espeak-ng-data",
    "packs",
    "source-inputs.json",
    "third-party-licenses",
}
EXPECTED_NOTICES = {
    "TGSpeechBox-LICENSE.txt",
    "Unicode-Data-License.txt",
    "eSpeak-NG-GPL-3.0.txt",
    "omnivox-Cargo.lock",
}


class PackagingError(RuntimeError):
    """The staged TGSpeechBox companion cannot form a release archive."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PackagingError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def git_output(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    return result.stdout.strip()


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/x86_64-pc-windows-gnu/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=version)
    parser.add_argument("--target", default=RELEASE_TARGET)
    parser.add_argument("--staged", type=Path, default=release / "tgspeechbox")
    parser.add_argument(
        "--output",
        type=Path,
        default=release / f"omnivox-{version}-tgspeechbox-{RELEASE_SUFFIX}.zip",
    )
    parser.add_argument(
        "--checksums", type=Path, default=release / "tgspeechbox-sha256sums.txt"
    )
    parser.add_argument(
        "--source-date-epoch",
        type=int,
        default=int(os.environ.get("SOURCE_DATE_EPOCH", "0")),
    )
    return parser.parse_args()


def inner_checksums(directory: Path) -> dict[str, str]:
    entries: dict[str, str] = {}
    for line in (directory / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^\\]+)", line)
        require(match is not None, f"invalid inner checksum line: {line!r}")
        relative = match.group(2)
        require(relative not in entries, f"duplicate inner checksum: {relative}")
        entries[relative] = match.group(1)
    return entries


def validate_inventory(path: Path, source_identity: str, sample_rate: int) -> None:
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


def validate_stage(directory: Path, repository: Path, target: str) -> None:
    require(target == RELEASE_TARGET, f"unsupported release target: {target}")
    stage_suffix, helper_name = SUPPORTED_TARGETS[target]
    require(directory.is_dir(), f"staged TGSpeechBox directory is missing: {directory}")
    require(
        {path.name for path in directory.iterdir()} == EXPECTED_ROOT | {helper_name},
        "staged TGSpeechBox root entries are incomplete or unexpected",
    )
    notices = directory / "third-party-licenses"
    require(
        {path.name for path in notices.iterdir()} == EXPECTED_NOTICES,
        "staged TGSpeechBox notice set is incomplete or unexpected",
    )
    for path in directory.rglob("*"):
        require(not path.is_symlink(), f"staged symlink is not allowed: {path}")
        require(path.is_dir() or path.is_file(), f"unsupported staged entry: {path}")
    require(inner_checksums(directory) == {
        path.relative_to(directory).as_posix(): sha256_file(path)
        for path in directory.rglob("*")
        if path.is_file() and path.name != "SHA256SUMS"
    }, "staged TGSpeechBox SHA256SUMS does not match every file")

    lock_path = repository / "omnivox-tgspeechbox-sys/source-inputs.json"
    source_identity = sha256_file(lock_path)
    require(
        sha256_file(directory / "source-inputs.json") == source_identity,
        "staged source lock does not match the checkout",
    )
    for sample_rate in SUPPORTED_SAMPLE_RATES:
        validate_inventory(
            directory / VOICE_INVENTORY_FILENAMES[sample_rate],
            source_identity,
            sample_rate,
        )
    validate_inventory(
        directory / VOICE_INVENTORY_FILENAME,
        source_identity,
        DEFAULT_SAMPLE_RATE,
    )

    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(
        provenance.get("artifact") == f"omnivox-tgspeechbox-companion-{stage_suffix}",
        "wrong TGSpeechBox artifact provenance",
    )
    require(provenance.get("target") == target, "wrong TGSpeechBox target provenance")
    require(provenance.get("markers_advertised") is False, "marker exclusion is not recorded")
    require(provenance.get("rate_mapping") == "provisional", "rate status is not recorded")
    tgspeechbox = provenance.get("tgspeechbox")
    require(isinstance(tgspeechbox, dict), "TGSpeechBox source provenance is missing")
    require(tgspeechbox.get("release") == RELEASE, "wrong TGSpeechBox source revision")
    require(tgspeechbox.get("commit") == COMMIT, "wrong TGSpeechBox source commit")
    require(tgspeechbox.get("verified_before_build") is True, "source was not verified")
    omnivox = provenance.get("omnivox")
    require(isinstance(omnivox, dict), "Omnivox provenance is missing")
    require(
        omnivox.get("tracked_worktree_dirty") is False,
        "refusing to package a companion staged from a dirty worktree",
    )
    require(
        not git_output(repository, "status", "--porcelain", "--untracked-files=no"),
        "refusing to package while the current tracked worktree is dirty",
    )
    require(
        omnivox.get("commit") == git_output(repository, "rev-parse", "HEAD"),
        "staged companion does not match the current source commit",
    )


def zip_info(name: str, mode: int, timestamp: int, directory: bool) -> zipfile.ZipInfo:
    normalized = datetime.fromtimestamp(max(timestamp, 315_532_800), tz=timezone.utc)
    info = zipfile.ZipInfo(
        name + ("/" if directory and not name.endswith("/") else ""),
        (
            normalized.year,
            normalized.month,
            normalized.day,
            normalized.hour,
            normalized.minute,
            normalized.second,
        ),
    )
    info.create_system = 3
    info.external_attr = ((0o040000 if directory else 0o100000) | mode) << 16
    info.compress_type = zipfile.ZIP_DEFLATED
    return info


def write_zip(source: Path, destination: Path, timestamp: int) -> None:
    require(timestamp >= 0, "source date epoch cannot be negative")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        with zipfile.ZipFile(
            temporary, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
        ) as archive:
            archive.writestr(zip_info("tgspeechbox", 0o755, timestamp, True), b"")
            for path in sorted(source.rglob("*")):
                name = f"tgspeechbox/{path.relative_to(source).as_posix()}"
                if path.is_dir():
                    archive.writestr(zip_info(name, 0o755, timestamp, True), b"")
                else:
                    mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                    with path.open("rb") as content, archive.open(
                        zip_info(name, mode, timestamp, False), "w", force_zip64=True
                    ) as output:
                        for chunk in iter(lambda: content.read(1024 * 1024), b""):
                            output.write(chunk)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()


def write_checksum(archive: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        temporary.write_text(f"{sha256_file(archive)}  {archive.name}\n", encoding="utf-8")
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        arguments = parse_arguments(repository)
        require(
            re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", arguments.version)
            is not None,
            f"invalid release version: {arguments.version}",
        )
        validate_stage(arguments.staged.resolve(), repository, arguments.target)
        output = arguments.output.resolve()
        write_zip(arguments.staged.resolve(), output, arguments.source_date_epoch)
        write_checksum(output, arguments.checksums.resolve())
        print(f"Packaged {output} ({output.stat().st_size / (1024 * 1024):.1f} MiB)")
    except (
        OSError,
        PackagingError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
        zipfile.BadZipFile,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
