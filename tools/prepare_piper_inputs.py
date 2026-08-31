#!/usr/bin/env python3
"""Download, verify, and safely extract locked Piper native inputs."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import posixpath
import shutil
import sys
import tarfile
import tempfile
import urllib.request
import zipfile


class PreparationError(RuntimeError):
    """A native input did not satisfy the checked-in lock."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PreparationError(message)


def detected_target() -> str:
    machine = platform.machine().lower()
    if sys.platform == "linux" and machine in {"x86_64", "amd64"}:
        return "x86_64-unknown-linux-gnu"
    if sys.platform == "darwin" and machine in {"arm64", "aarch64"}:
        return "aarch64-apple-darwin"
    if sys.platform == "darwin" and machine in {"x86_64", "amd64"}:
        return "x86_64-apple-darwin"
    if sys.platform == "win32" and machine in {"x86_64", "amd64"}:
        return "x86_64-pc-windows-msvc"
    return ""


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Prepare checksum-locked native inputs for the Piper helper."
    )
    parser.add_argument(
        "--target",
        default=detected_target(),
        help="native Rust target triple (default: detected supported target)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="exact target-specific input directory (default: target/piper-inputs/...)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="make no network requests or repairs; only verify the prepared cache",
    )
    return parser.parse_args()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tree_digest(root: Path) -> str:
    digest = hashlib.sha256()
    entries = sorted(path for path in root.rglob("*") if not path.is_dir())
    for path in entries:
        relative = path.relative_to(root).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        if path.is_symlink():
            digest.update(b"symlink\0")
            digest.update(os.readlink(path).encode("utf-8"))
        elif path.is_file():
            digest.update(b"file\0")
            digest.update(sha256_file(path).encode("ascii"))
        else:
            raise PreparationError(f"unsupported extracted input entry: {path}")
        digest.update(b"\n")
    return digest.hexdigest()


def safe_parts(name: str) -> tuple[str, ...]:
    require("\\" not in name, f"archive member uses a backslash: {name!r}")
    path = PurePosixPath(name)
    require(not path.is_absolute(), f"archive member is absolute: {name!r}")
    parts = tuple(part for part in path.parts if part not in ("", "."))
    require(parts and ".." not in parts, f"unsafe archive member: {name!r}")
    return parts


def symlink_target(member: tarfile.TarInfo, archive_root: str) -> PurePosixPath:
    require(
        not PurePosixPath(member.linkname).is_absolute(),
        f"absolute link: {member.name}",
    )
    normalized = posixpath.normpath(
        str(PurePosixPath(member.name).parent / member.linkname)
    )
    parts = safe_parts(normalized)
    require(parts[0] == archive_root, f"link leaves archive root: {member.name}")
    return PurePosixPath(*parts)


def extract_tar_archive(
    archive: Path,
    destination: Path,
    archive_root: str,
    materialize_symlinks: bool,
) -> Path:
    with tarfile.open(archive, "r:gz") as bundle:
        members = bundle.getmembers()
        member_paths: set[PurePosixPath] = set()
        link_paths: set[PurePosixPath] = set()
        link_targets: dict[PurePosixPath, PurePosixPath] = {}
        for member in members:
            parts = safe_parts(member.name)
            require(parts[0] == archive_root, f"unexpected archive root: {member.name}")
            member_path = PurePosixPath(*parts)
            require(
                member_path not in member_paths,
                f"duplicate archive member: {member.name}",
            )
            member_paths.add(member_path)
            require(
                member.isdir() or member.isfile() or member.issym(),
                f"unsupported archive member: {member.name}",
            )
            if member.issym():
                link_targets[member_path] = symlink_target(member, archive_root)
                link_paths.add(member_path)

        for member_path in member_paths:
            require(
                not any(parent in link_paths for parent in member_path.parents),
                f"archive member traverses a link: {member_path}",
            )

        for member in members:
            target = destination.joinpath(*safe_parts(member.name))
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                source = bundle.extractfile(member)
                require(
                    source is not None,
                    f"cannot read archive member: {member.name}",
                )
                target.parent.mkdir(parents=True, exist_ok=True)
                with source, target.open("wb") as output:
                    shutil.copyfileobj(source, output)
                target.chmod(member.mode & 0o777)
            elif not member.issym():
                raise PreparationError(f"unsupported archive member: {member.name}")

        for member in members:
            if not member.issym():
                continue
            target = destination.joinpath(*safe_parts(member.name))
            target.parent.mkdir(parents=True, exist_ok=True)
            if materialize_symlinks:
                member_path = PurePosixPath(*safe_parts(member.name))
                source = destination.joinpath(*link_targets[member_path].parts)
                require(source.is_file(), f"cannot materialize archive link: {member.name}")
                shutil.copy2(source, target)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.symlink_to(member.linkname)

    extracted = destination / archive_root
    require(extracted.is_dir(), f"archive root was not extracted: {archive_root}")
    return extracted


def extract_zip_archive(archive: Path, destination: Path, archive_root: str) -> Path:
    with zipfile.ZipFile(archive) as bundle:
        member_paths: set[PurePosixPath] = set()
        for member in bundle.infolist():
            parts = safe_parts(member.filename)
            require(parts[0] == archive_root, f"unexpected archive root: {member.filename}")
            member_path = PurePosixPath(*parts)
            require(
                member_path not in member_paths,
                f"duplicate archive member: {member.filename}",
            )
            member_paths.add(member_path)
            mode = member.external_attr >> 16
            require(
                (mode & 0o170000) != 0o120000,
                f"zip symlink is not allowed: {member.filename}",
            )

        for member in bundle.infolist():
            target = destination.joinpath(*safe_parts(member.filename))
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)

    extracted = destination / archive_root
    require(extracted.is_dir(), f"archive root was not extracted: {archive_root}")
    return extracted


def extract_archive(
    archive: Path,
    destination: Path,
    archive_root: str,
    materialize_symlinks: bool,
) -> Path:
    if archive.suffix.lower() == ".zip":
        require(not materialize_symlinks, "zip inputs cannot request link materialization")
        return extract_zip_archive(archive, destination, archive_root)
    return extract_tar_archive(
        archive, destination, archive_root, materialize_symlinks
    )


def download(url: str, destination: Path, expected_sha256: str) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "Omnivox-build"})
        with urllib.request.urlopen(request, timeout=120) as source, temporary.open(
            "wb"
        ) as output:
            shutil.copyfileobj(source, output)
        actual = sha256_file(temporary)
        require(
            actual == expected_sha256,
            f"download checksum mismatch for {url}: found {actual}, "
            f"expected {expected_sha256}",
        )
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()


def ensure_archive(
    component: str, specification: dict[str, object], downloads: Path, check: bool
) -> Path:
    archive = downloads / str(specification["archive"])
    expected = str(specification["sha256"])
    if archive.is_file():
        actual = sha256_file(archive)
        require(
            actual == expected,
            f"cached {component} checksum mismatch: found {actual}, "
            f"expected {expected}",
        )
    elif check:
        raise PreparationError(f"cached {component} archive is missing: {archive}")
    else:
        print(f"Downloading locked {component} input", file=sys.stderr)
        download(str(specification["url"]), archive, expected)
    return archive


def replace_directory(source: Path, destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    source.replace(destination)


def ensure_source(
    component: str,
    specification: dict[str, object],
    archive: Path,
    sources: Path,
    check: bool,
) -> tuple[Path, str]:
    destination = sources / str(specification["destination"])
    expected_digest = str(specification["source_tree_sha256"])
    if destination.is_dir():
        actual = tree_digest(destination)
        if actual == expected_digest:
            return destination, actual
        if check:
            raise PreparationError(
                f"prepared {component} tree checksum mismatch: found {actual}, "
                f"expected {expected_digest}"
            )
    elif check:
        raise PreparationError(f"prepared {component} source is missing: {destination}")

    sources.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{component}.tmp-{os.getpid()}-", dir=sources)
    )
    try:
        extracted = extract_archive(
            archive,
            temporary,
            str(specification["archive_root"]),
            bool(specification.get("materialize_symlinks", False)),
        )
        replace_directory(extracted, destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    actual = tree_digest(destination)
    require(
        actual == expected_digest,
        f"extracted {component} tree checksum mismatch: found {actual}, "
        f"expected {expected_digest}",
    )
    return destination, actual


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"expected a JSON object: {path}")
    return value


def prepare(arguments: argparse.Namespace) -> Path:
    repository = Path(__file__).resolve().parent.parent
    lock_path = repository / "omnivox-piper-sys/native-inputs.json"
    lock = load_json(lock_path)
    targets = lock.get("targets")
    require(isinstance(targets, dict), "native input lock has no targets object")
    require(
        arguments.target in targets,
        f"unsupported Piper input target: {arguments.target}",
    )
    specifications = targets[arguments.target]
    require(isinstance(specifications, dict), "target input lock must be an object")

    output = (
        arguments.output.resolve()
        if arguments.output
        else repository
        / "target/piper-inputs"
        / str(lock["piper_version"])
        / arguments.target
    )
    downloads = output / "downloads"
    sources = output / "sources"
    marker_path = output / "PREPARED.json"
    prepared: dict[str, object] = {}
    for component in sorted(specifications):
        specification = specifications[component]
        require(isinstance(specification, dict), f"invalid lock entry: {component}")
        archive = ensure_archive(component, specification, downloads, arguments.check)
        source, source_digest = ensure_source(
            component,
            specification,
            archive,
            sources,
            arguments.check,
        )
        prepared[component] = {
            **specification,
            "archive_path": str(archive.resolve()),
            "source_path": str(source.resolve()),
            "source_tree_sha256": source_digest,
        }

    marker = {
        "schema_version": 1,
        "target": arguments.target,
        "lock_file": str(lock_path.relative_to(repository)),
        "lock_file_sha256": sha256_file(lock_path),
        "components": prepared,
    }
    if arguments.check:
        require(
            marker_path.is_file(),
            f"prepared-input marker is missing: {marker_path}",
        )
        require(
            load_json(marker_path) == marker,
            "prepared-input marker does not match inputs",
        )
    else:
        output.mkdir(parents=True, exist_ok=True)
        marker_path.write_text(
            json.dumps(marker, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    print(f"Prepared and verified Piper inputs in {output}", file=sys.stderr)
    return output


def main() -> int:
    try:
        prepare(parse_arguments())
    except (
        OSError,
        PreparationError,
        json.JSONDecodeError,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
