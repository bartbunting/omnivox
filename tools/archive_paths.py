"""Portable archive member validation and extraction containment."""

from pathlib import Path, PurePosixPath


def safe_parts(
    name: str, error_type: type[Exception] = ValueError
) -> tuple[str, ...]:
    # Colons include drive-relative paths (C:../file) and NTFS streams.
    # Reject them on every host so Unix validation also protects Windows users.
    if "\\" in name or ":" in name or "\0" in name:
        raise error_type(f"unsafe archive member: {name!r}")
    path = PurePosixPath(name)
    parts = tuple(part for part in path.parts if part not in ("", "."))
    if path.is_absolute() or not parts or ".." in parts:
        raise error_type(f"unsafe archive member: {name!r}")
    if any(part.endswith((".", " ")) for part in parts):
        raise error_type(f"ambiguous Windows archive member: {name!r}")
    return parts


def extraction_target(
    destination: Path,
    parts: tuple[str, ...],
    error_type: type[Exception] = ValueError,
) -> Path:
    target = destination.joinpath(*parts)
    # Check existing links too: a safe member name must not follow a parent or
    # final-file symlink outside the destination supplied by the caller.
    if not target.resolve().is_relative_to(destination.resolve()):
        raise error_type(f"archive member leaves extraction directory: {'/'.join(parts)!r}")
    return target
