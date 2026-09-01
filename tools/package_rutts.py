#!/usr/bin/env python3
"""Create a deterministic RuTTS companion release archive."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tomllib
import zipfile

sys.dont_write_bytecode = True
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
EXPECTED_NOTICES = {"RuTTS-LICENSE.txt", "omnivox-Cargo.lock"}


class PackagingError(RuntimeError):
    """The staged RuTTS companion cannot form a release archive."""


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


def parse_arguments(repository: Path, target: str) -> argparse.Namespace:
    version = repository_version(repository)
    suffix, _ = SUPPORTED_TARGETS[target]
    extension = "zip" if suffix.startswith("windows-") else "tar.gz"
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=version)
    parser.add_argument("--staged", type=Path, default=release / "rutts")
    parser.add_argument(
        "--output",
        type=Path,
        default=release / f"omnivox-{version}-rutts-{suffix}.{extension}",
    )
    parser.add_argument(
        "--checksums", type=Path, default=release / "rutts-sha256sums.txt"
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


def validate_stage(directory: Path, repository: Path, target: str) -> None:
    suffix, helper_name = SUPPORTED_TARGETS[target]
    require(directory.is_dir(), f"staged RuTTS directory is missing: {directory}")
    require(
        {path.name for path in directory.iterdir()} == EXPECTED_ROOT | {helper_name},
        "staged RuTTS root entries are incomplete or unexpected",
    )
    notices = directory / "third-party-licenses"
    require(
        {path.name for path in notices.iterdir()} == EXPECTED_NOTICES,
        "staged RuTTS notice set is incomplete or unexpected",
    )
    for path in directory.rglob("*"):
        require(not path.is_symlink(), f"staged RuTTS symlink is not allowed: {path}")
        require(path.is_dir() or path.is_file(), f"unsupported staged entry: {path}")
    helper = directory / helper_name
    if not suffix.startswith("windows-"):
        require(helper.stat().st_mode & 0o111 != 0, "RuTTS helper is not executable")
    expected = inner_checksums(directory)
    actual = {
        path.relative_to(directory).as_posix(): sha256_file(path)
        for path in directory.rglob("*")
        if path.is_file() and path.name != "SHA256SUMS"
    }
    require(expected == actual, "staged RuTTS SHA256SUMS does not match every file")

    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(
        provenance.get("artifact") == f"omnivox-rutts-companion-{suffix}",
        "wrong RuTTS artifact provenance",
    )
    require(provenance.get("target") == target, "wrong RuTTS target provenance")
    require(
        provenance.get("built_in_voices") == ["male", "female"],
        "wrong RuTTS built-in voice set",
    )
    require(provenance.get("rulex_included") is False, "RuLex exclusion is not recorded")
    rutts = provenance.get("rutts")
    require(isinstance(rutts, dict), "RuTTS source provenance is missing")
    require(rutts.get("version") == RUTTS_VERSION, "wrong RuTTS source version")
    require(rutts.get("commit") == RUTTS_COMMIT, "wrong RuTTS source commit")
    require(rutts.get("verified_before_build") is True, "RuTTS source was not verified")
    omnivox = provenance.get("omnivox")
    require(isinstance(omnivox, dict), "Omnivox provenance is missing")
    require(
        omnivox.get("tracked_worktree_dirty") is False,
        "refusing to package a RuTTS companion staged from a dirty worktree",
    )
    require(
        not git_output(repository, "status", "--porcelain", "--untracked-files=no"),
        "refusing to package while the current tracked worktree is dirty",
    )
    require(
        omnivox.get("commit") == git_output(repository, "rev-parse", "HEAD"),
        "staged RuTTS companion does not match the current source commit",
    )


def tar_info(name: str, mode: int, timestamp: int, directory: bool) -> tarfile.TarInfo:
    info = tarfile.TarInfo(name)
    info.mode = mode
    info.mtime = timestamp
    info.uid = 0
    info.gid = 0
    info.uname = "root"
    info.gname = "root"
    if directory:
        info.type = tarfile.DIRTYPE
    return info


def write_tar(source: Path, destination: Path, timestamp: int) -> None:
    require(timestamp >= 0, "source date epoch cannot be negative")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        with temporary.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=timestamp) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                    archive.addfile(tar_info("rutts", 0o755, timestamp, True))
                    for path in sorted(source.rglob("*")):
                        name = f"rutts/{path.relative_to(source).as_posix()}"
                        if path.is_dir():
                            archive.addfile(tar_info(name, 0o755, timestamp, True))
                        else:
                            mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                            info = tar_info(name, mode, timestamp, False)
                            info.size = path.stat().st_size
                            with path.open("rb") as content:
                                archive.addfile(info, content)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()


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
        with zipfile.ZipFile(temporary, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            archive.writestr(zip_info("rutts", 0o755, timestamp, True), b"")
            for path in sorted(source.rglob("*")):
                name = f"rutts/{path.relative_to(source).as_posix()}"
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
        target = host_target()
        require(target in SUPPORTED_TARGETS, f"unsupported native RuTTS target: {target}")
        arguments = parse_arguments(repository, target)
        require(
            re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", arguments.version)
            is not None,
            f"invalid release version: {arguments.version}",
        )
        staged = arguments.staged.resolve()
        validate_stage(staged, repository, target)
        output = arguments.output.resolve()
        suffix, _ = SUPPORTED_TARGETS[target]
        if suffix.startswith("windows-"):
            write_zip(staged, output, arguments.source_date_epoch)
        else:
            write_tar(staged, output, arguments.source_date_epoch)
        write_checksum(output, arguments.checksums.resolve())
        print(f"Packaged {output} ({output.stat().st_size / (1024 * 1024):.1f} MiB)")
    except (
        OSError,
        PackagingError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
