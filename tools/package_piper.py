#!/usr/bin/env python3
"""Create a deterministic Linux x64 Piper companion release archive."""

from __future__ import annotations

import argparse
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


TARGET = "x86_64-unknown-linux-gnu"
PLATFORM_SUFFIX = "piper-linux-x64"
EXPECTED_ROOT = {
    "LICENSE",
    "LICENSING.md",
    "README.md",
    "SHA256SUMS",
    "SOURCE-PROVENANCE.json",
    "espeak-ng-data",
    "libonnxruntime.so.1",
    "libonnxruntime_providers_shared.so",
    "libpiper.so",
    "omnivox-piper-helper",
    "third-party-licenses",
}


class PackagingError(RuntimeError):
    """The staged companion cannot form a release archive."""


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


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    default_archive = (
        repository / "target/release" / f"omnivox-{version}-{PLATFORM_SUFFIX}.tar.gz"
    )
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=version)
    parser.add_argument("--staged", type=Path, default=repository / "target/release/piper")
    parser.add_argument("--output", type=Path, default=default_archive)
    parser.add_argument(
        "--checksums",
        type=Path,
        default=repository / "target/release/piper-sha256sums.txt",
    )
    parser.add_argument(
        "--source-date-epoch",
        type=int,
        default=int(os.environ.get("SOURCE_DATE_EPOCH", "0")),
        help="normalized archive timestamp (default: SOURCE_DATE_EPOCH or zero)",
    )
    return parser.parse_args()


def inner_checksums(directory: Path) -> dict[str, str]:
    checksum_path = directory / "SHA256SUMS"
    entries: dict[str, str] = {}
    for line in checksum_path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^\\]+)", line)
        require(match is not None, f"invalid inner checksum line: {line!r}")
        relative = match.group(2)
        require(relative not in entries, f"duplicate inner checksum: {relative}")
        entries[relative] = match.group(1)
    return entries


def git_output(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    return result.stdout.strip()


def validate_stage(directory: Path, version: str, repository: Path) -> None:
    require(directory.is_dir(), f"staged Piper directory is missing: {directory}")
    actual_root = {path.name for path in directory.iterdir()}
    require(
        actual_root == EXPECTED_ROOT,
        f"unexpected staged Piper root entries: {sorted(actual_root)}",
    )
    for path in directory.rglob("*"):
        require(not path.is_symlink(), f"staged Piper symlink is not allowed: {path}")
        require(path.is_dir() or path.is_file(), f"unsupported staged entry: {path}")

    helper = directory / "omnivox-piper-helper"
    require(helper.stat().st_mode & 0o111 != 0, "Piper helper is not executable")
    require((directory / "espeak-ng-data/phontab").is_file(), "Piper phontab is missing")

    expected = inner_checksums(directory)
    actual = {
        path.relative_to(directory).as_posix(): sha256_file(path)
        for path in directory.rglob("*")
        if path.is_file() and path != directory / "SHA256SUMS"
    }
    require(expected == actual, "staged Piper SHA256SUMS does not match every file")

    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(
        provenance.get("artifact") == "omnivox-piper-companion-linux-x64",
        "wrong artifact provenance",
    )
    require(provenance.get("target") == TARGET, "wrong target provenance")
    omnivox = provenance.get("omnivox")
    require(isinstance(omnivox, dict), "Omnivox provenance is missing")
    require(
        omnivox.get("tracked_worktree_dirty") is False,
        "refusing to package a Piper companion staged from a dirty worktree",
    )
    require(
        not git_output(repository, "status", "--porcelain", "--untracked-files=no"),
        "refusing to package while the current tracked worktree is dirty",
    )
    require(
        omnivox.get("commit") == git_output(repository, "rev-parse", "HEAD"),
        "staged Piper companion does not match the current source commit",
    )
    require(
        provenance.get("voice_model_included") is False,
        "voice model boundary is not explicit",
    )
    require(
        not any(path.name.endswith((".onnx", ".onnx.json")) for path in directory.rglob("*")),
        "staged companion unexpectedly contains a voice model or model configuration",
    )

    lock_text = (
        directory / "third-party-licenses/omnivox-Cargo.lock"
    ).read_text(encoding="utf-8")
    versions = re.findall(
        r'^name = "omnivox-[^"]+"\nversion = "([^"]+)"$',
        lock_text,
        flags=re.MULTILINE,
    )
    require(versions, "packaged Cargo.lock has no Omnivox packages")
    require(
        all(package_version == version for package_version in versions),
        f"staged Cargo.lock does not match release version {version}",
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


def write_archive(source: Path, destination: Path, timestamp: int) -> None:
    require(timestamp >= 0, "source date epoch cannot be negative")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        with temporary.open("wb") as raw:
            with gzip.GzipFile(
                filename="", mode="wb", fileobj=raw, mtime=timestamp
            ) as compressed:
                with tarfile.open(
                    fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT
                ) as archive:
                    archive.addfile(tar_info("piper", 0o755, timestamp, True))
                    for path in sorted(source.rglob("*")):
                        relative = path.relative_to(source).as_posix()
                        name = f"piper/{relative}"
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


def write_checksum(archive: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        temporary.write_text(
            f"{sha256_file(archive)}  {archive.name}\n", encoding="utf-8"
        )
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    arguments = parse_arguments(repository)
    try:
        require(
            re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", arguments.version)
            is not None,
            f"invalid release version: {arguments.version}",
        )
        validate_stage(arguments.staged.resolve(), arguments.version, repository)
        output = arguments.output.resolve()
        write_archive(arguments.staged.resolve(), output, arguments.source_date_epoch)
        write_checksum(output, arguments.checksums.resolve())
        print(f"Packaged {output} ({output.stat().st_size / (1024 * 1024):.1f} MiB)")
        print(f"Wrote {arguments.checksums.resolve()}")
    except (
        OSError,
        PackagingError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
        tarfile.TarError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
