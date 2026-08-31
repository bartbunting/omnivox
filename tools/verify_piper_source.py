#!/usr/bin/env python3
"""Verify the deterministic Piper corresponding-source release artifact."""

from __future__ import annotations

import argparse
from io import BytesIO
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib

sys.dont_write_bytecode = True
import verify_release as common


EXPECTED_TARGETS = {
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
}
EXPECTED_COMPONENTS = {"espeak_ng", "onnxruntime", "sonic"}
VENDOR_CONFIG = """[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
"""


class SourceVerificationError(common.VerificationError):
    """The source artifact violated its release contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SourceVerificationError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_output(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    return result.stdout.strip()


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=release / f"omnivox-{version}-piper-source.tar.gz",
    )
    parser.add_argument(
        "--checksums",
        type=Path,
        default=release / "piper-source-sha256sums.txt",
    )
    parser.add_argument("--version", default=version)
    parser.add_argument("--commit", default=git_output(repository, "rev-parse", "HEAD"))
    return parser.parse_args()


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"expected a JSON object: {path}")
    return value


def verify_manifest(root: Path, version: str, commit: str) -> dict[str, object]:
    manifest_path = root / "SOURCE-MANIFEST.json"
    manifest = load_json(manifest_path)
    require(manifest.get("schema_version") == 1, "unknown source manifest schema")
    require(
        manifest.get("artifact") == "omnivox-piper-source-and-build-inputs",
        "wrong source artifact identity",
    )
    require(manifest.get("version") == version, "source manifest version mismatch")
    require(manifest.get("source_commit") == commit, "source commit mismatch")
    require(
        manifest.get("cargo_dependencies_vendored") is True,
        "Cargo dependency source boundary is missing",
    )
    require(
        manifest.get("voice_model_included") is False,
        "voice model exclusion is not recorded",
    )
    require(
        set(manifest.get("native_targets", [])) == EXPECTED_TARGETS,
        "source manifest native target set changed",
    )

    declared = manifest.get("contents")
    require(isinstance(declared, dict) and declared, "source content manifest is empty")
    actual = {
        path.relative_to(root).as_posix(): path
        for path in root.rglob("*")
        if path.is_file() and path != manifest_path
    }
    require(declared.keys() == actual.keys(), "source manifest is not exhaustive")
    for relative, path in actual.items():
        specification = declared[relative]
        require(isinstance(specification, dict), f"invalid content entry: {relative}")
        expected_mode = "0755" if path.stat().st_mode & 0o111 else "0644"
        require(specification.get("mode") == expected_mode, f"mode mismatch: {relative}")
        require(specification.get("size") == path.stat().st_size, f"size mismatch: {relative}")
        require(
            specification.get("sha256") == sha256_file(path),
            f"checksum mismatch: {relative}",
        )
    return manifest


def git_archive_files(repository: Path, commit: str) -> dict[str, tuple[int, str]]:
    result = subprocess.run(
        ["git", "-C", str(repository), "archive", "--format=tar", commit],
        check=True,
        stdout=subprocess.PIPE,
    )
    files: dict[str, tuple[int, str]] = {}
    with tarfile.open(fileobj=BytesIO(result.stdout), mode="r:") as archive:
        for member in archive.getmembers():
            if member.isdir():
                continue
            require(member.isfile(), f"unsupported committed source entry: {member.name}")
            source = archive.extractfile(member)
            require(source is not None, f"cannot read committed source: {member.name}")
            digest = hashlib.sha256(source.read()).hexdigest()
            files[member.name] = (0o755 if member.mode & 0o111 else 0o644, digest)
    return files


def verify_committed_source(source: Path, repository: Path, commit: str) -> None:
    generated = {".cargo/config.toml"}
    packaged = {
        path.relative_to(source).as_posix(): path
        for path in source.rglob("*")
        if path.is_file()
        and not path.relative_to(source).as_posix().startswith("vendor/")
        and path.relative_to(source).as_posix() not in generated
    }
    committed = git_archive_files(repository, commit)
    require(packaged.keys() == committed.keys(), "committed Omnivox source set mismatch")
    for relative, path in packaged.items():
        expected_mode, expected_digest = committed[relative]
        actual_mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
        require(actual_mode == expected_mode, f"committed mode mismatch: {relative}")
        require(sha256_file(path) == expected_digest, f"committed checksum mismatch: {relative}")


def locked_input_digests(source: Path) -> tuple[dict[str, str], dict[str, str]]:
    native_path = source / "omnivox-piper-sys/native-inputs.json"
    source_path = source / "omnivox-piper-sys/source-inputs.json"
    native = load_json(native_path)
    sources = load_json(source_path)
    require(native.get("schema_version") == 1, "unknown packaged native-input schema")
    targets = native.get("targets")
    require(isinstance(targets, dict), "packaged native-input targets are missing")
    require(set(targets) == EXPECTED_TARGETS, "packaged native target set changed")
    archives: dict[str, str] = {}
    for target, components in targets.items():
        require(isinstance(components, dict), f"invalid packaged target: {target}")
        require(set(components) == EXPECTED_COMPONENTS, f"component set changed: {target}")
        for component, value in components.items():
            require(isinstance(value, dict), f"invalid packaged component: {component}")
            name = str(value.get("archive", ""))
            digest = str(value.get("sha256", ""))
            require(
                name == Path(name).name and re.fullmatch(r"[0-9a-f]{64}", digest) is not None,
                f"invalid packaged input lock: {target}/{component}",
            )
            previous = archives.setdefault(name, digest)
            require(previous == digest, f"conflicting packaged input lock: {name}")

    require(sources.get("schema_version") == 1, "unknown packaged source-input schema")
    source_entries = sources.get("sources")
    require(
        isinstance(source_entries, dict) and set(source_entries) == {"onnxruntime"},
        "packaged source-input set changed",
    )
    onnx = source_entries["onnxruntime"]
    require(isinstance(onnx, dict), "invalid packaged ONNX Runtime source lock")
    require(onnx.get("version") == "1.22.0", "wrong packaged ONNX Runtime source version")
    require(onnx.get("license") == "MIT", "wrong packaged ONNX Runtime source licence")
    require(
        re.fullmatch(r"[0-9a-f]{40}", str(onnx.get("commit", ""))) is not None,
        "invalid packaged ONNX Runtime source commit",
    )
    name = str(onnx.get("archive", ""))
    digest = str(onnx.get("sha256", ""))
    require(
        name == Path(name).name and re.fullmatch(r"[0-9a-f]{64}", digest) is not None,
        "invalid packaged ONNX Runtime source archive",
    )
    require(name not in archives, "duplicate packaged source archive name")
    archives[name] = digest
    locks = {
        "omnivox-piper-sys/native-inputs.json": sha256_file(native_path),
        "omnivox-piper-sys/source-inputs.json": sha256_file(source_path),
    }
    return archives, locks


def verify_inputs(root: Path, source: Path, manifest: dict[str, object]) -> None:
    expected, lock_digests = locked_input_digests(source)
    inputs = root / "inputs"
    actual = {path.name: path for path in inputs.iterdir() if path.is_file()}
    require(actual.keys() == expected.keys(), "source input archive set mismatch")
    for name, path in actual.items():
        require(sha256_file(path) == expected[name], f"source input checksum mismatch: {name}")
    require(manifest.get("locks") == lock_digests, "source lock digest mismatch")
    described = manifest.get("input_archives")
    require(isinstance(described, dict), "source input provenance is missing")
    require(described.keys() == expected.keys(), "source input provenance set mismatch")
    for name, digest in expected.items():
        specification = described[name]
        require(isinstance(specification, dict), f"invalid source input provenance: {name}")
        require(specification.get("sha256") == digest, f"input provenance checksum mismatch: {name}")
        require(specification.get("size") == actual[name].stat().st_size, f"input provenance size mismatch: {name}")


def verify_offline_cargo(source: Path, working: Path) -> None:
    configuration = source / ".cargo/config.toml"
    require(configuration.read_text(encoding="utf-8") == VENDOR_CONFIG, "wrong Cargo vendor configuration")
    require((source / "vendor").is_dir(), "vendored Cargo sources are missing")
    environment = dict(os.environ)
    environment["CARGO_HOME"] = str(working / "cargo-home")
    environment["CARGO_NET_OFFLINE"] = "true"
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"],
        cwd=source,
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    require(result.returncode == 0, f"vendored Cargo graph is not offline-complete:\n{result.stderr}")


def verify(arguments: argparse.Namespace, repository: Path) -> None:
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"source archive does not exist: {archive}")
    require(checksums.is_file(), f"source checksum file does not exist: {checksums}")
    require(
        archive.name == f"omnivox-{arguments.version}-piper-source.tar.gz",
        f"unexpected source archive name: {archive.name}",
    )
    require(
        re.fullmatch(r"[0-9a-f]{40}", arguments.commit) is not None,
        f"invalid expected source commit: {arguments.commit}",
    )
    common.verify_checksum(archive, checksums)
    with tempfile.TemporaryDirectory(prefix="Omnivox source verification ") as temporary:
        working = Path(temporary)
        extracted = working / "Extracted source with spaces"
        extracted.mkdir()
        common.extract_tar(archive, extracted)
        root_name = f"omnivox-{arguments.version}-piper-source"
        require(
            {path.name for path in extracted.iterdir()} == {root_name},
            "source archive must have one versioned top-level directory",
        )
        root = extracted / root_name
        require(
            {path.name for path in root.iterdir()}
            == {"README.md", "SOURCE-MANIFEST.json", "inputs", "omnivox"},
            "unexpected source archive root entries",
        )
        require((root / "README.md").stat().st_size > 500, "source README is incomplete")
        source = root / "omnivox"
        require(repository_version(source) == arguments.version, "packaged source version mismatch")
        require((source / "third-party/piper1-gpl/COPYING").stat().st_size > 30000, "libpiper GPL source notice is missing")
        require((source / "third-party/piper1-gpl/libpiper/include/piper.h").is_file(), "vendored libpiper source is missing")
        manifest = verify_manifest(root, arguments.version, arguments.commit)
        verify_committed_source(source, repository, arguments.commit)
        verify_inputs(root, source, manifest)
        model_lock = load_json(source / "omnivox-piper-sys/test-model.json")
        model_files = model_lock.get("files")
        require(isinstance(model_files, dict), "packaged CI model lock is invalid")
        for name in model_files:
            if name != "MODEL_CARD":
                require(not any(source.rglob(name)), f"CI model payload is present: {name}")
                require(not (root / "inputs" / name).exists(), f"CI model input is present: {name}")
        verify_offline_cargo(source, working)
    print(f"PASS {archive.name}: source, inputs, manifest, and offline Cargo verification")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        verify(parse_arguments(repository), repository)
    except (
        OSError,
        common.VerificationError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
        tarfile.TarError,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
