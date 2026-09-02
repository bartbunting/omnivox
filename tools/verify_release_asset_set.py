#!/usr/bin/env python3
"""Require the exact archive and checksum set for an Omnivox release."""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys


class AssetSetError(RuntimeError):
    """The candidate release contains a missing or unexpected asset."""


GENERIC_TARGETS = {
    "linux-x64": "tar.gz",
    "macos-arm64": "tar.gz",
    "macos-x64": "tar.gz",
    "windows-x64": "zip",
    "windows-arm64": "zip",
}
PORTABLE_COMPANION_TARGETS = {
    **GENERIC_TARGETS,
    "linux-arm64": "tar.gz",
}
PIPER_TARGETS = {
    target: extension
    for target, extension in GENERIC_TARGETS.items()
    if target != "windows-arm64"
}


def expected_asset_names(version: str) -> set[str]:
    names = {
        f"omnivox-{version}-{target}.{extension}"
        for target, extension in GENERIC_TARGETS.items()
    }
    for engine in ("flite", "rutts"):
        names.update(
            f"omnivox-{version}-{engine}-{target}.{extension}"
            for target, extension in PORTABLE_COMPANION_TARGETS.items()
        )
        names.add(f"omnivox-{version}-{engine}-source.tar.gz")
    names.update(
        f"omnivox-{version}-piper-{target}.{extension}"
        for target, extension in PIPER_TARGETS.items()
    )
    names.add(f"omnivox-{version}-piper-source.tar.gz")
    names.add("sha256sums.txt")
    return names


def require_exact_names(actual: list[str], version: str) -> None:
    duplicates = sorted(name for name in set(actual) if actual.count(name) > 1)
    expected = expected_asset_names(version)
    actual_set = set(actual)
    missing = sorted(expected - actual_set)
    unexpected = sorted(actual_set - expected)
    if duplicates or missing or unexpected:
        details = []
        if duplicates:
            details.append(f"duplicate assets: {duplicates}")
        if missing:
            details.append(f"missing assets: {missing}")
        if unexpected:
            details.append(f"unexpected assets: {unexpected}")
        raise AssetSetError("; ".join(details))


def checksum_names(path: Path) -> list[str]:
    names: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"[0-9a-fA-F]{64}\s+\*?(.+)", line)
        if match is None:
            raise AssetSetError(f"invalid checksum line: {line!r}")
        name = match.group(1)
        if Path(name).name != name or "/" in name or "\\" in name:
            raise AssetSetError(f"checksum entry is not a file name: {name!r}")
        names.append(name)
    return names


def verify_directory(directory: Path, version: str) -> None:
    if not directory.is_dir():
        raise AssetSetError(f"release asset directory does not exist: {directory}")
    names = sorted(path.name for path in directory.iterdir() if path.is_file())
    require_exact_names(names, version)
    checksum_path = directory / "sha256sums.txt"
    expected_archives = expected_asset_names(version) - {"sha256sums.txt"}
    actual_checksums = checksum_names(checksum_path)
    duplicate_checksums = sorted(
        name for name in set(actual_checksums) if actual_checksums.count(name) > 1
    )
    if duplicate_checksums:
        raise AssetSetError(f"duplicate checksum entries: {duplicate_checksums}")
    checksum_set = set(actual_checksums)
    missing = sorted(expected_archives - checksum_set)
    unexpected = sorted(checksum_set - expected_archives)
    if missing or unexpected:
        raise AssetSetError(
            f"checksum manifest mismatch: missing={missing}, unexpected={unexpected}"
        )


def verify_names_file(path: Path, version: str) -> None:
    names = [line for line in path.read_text(encoding="utf-8").splitlines() if line]
    require_exact_names(names, version)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--directory", type=Path)
    source.add_argument("--names-file", type=Path)
    return parser.parse_args()


def main() -> int:
    arguments = parse_arguments()
    try:
        if arguments.directory is not None:
            verify_directory(arguments.directory, arguments.version)
        else:
            verify_names_file(arguments.names_file, arguments.version)
    except (AssetSetError, OSError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(f"PASS: exact Omnivox {arguments.version} release asset set")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
