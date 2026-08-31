#!/usr/bin/env python3
"""Verify a Linux x64 Piper companion archive and optional real synthesis."""

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
import tarfile
import tempfile
import tomllib

sys.dont_write_bytecode = True
import verify_release as common


TARGET = "x86_64-unknown-linux-gnu"
PIPER_VERSION = "1.7.0"
PIPER_COMMIT = "7b8e8f7197a480047677715f00d3d78903b55a2a"
RUNTIME_BINARIES = (
    "omnivox-piper-helper",
    "libpiper.so",
    "libonnxruntime.so.1",
    "libonnxruntime_providers_shared.so",
)
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
EXPECTED_NOTICES = {
    "NetBSD-getopt.c",
    "ONNX-Runtime-MIT.txt",
    "ONNX-Runtime-ThirdPartyNotices.txt",
    "Piper-GPL-3.0-or-later.txt",
    "Piper-UPSTREAM.md",
    "Sonic-Apache-2.0.txt",
    "THIRD-PARTY-NOTICES.md",
    "UCD-Tools-GPL-3.0.txt",
    "UCD-Tools-Unicode-Data-License.txt",
    "Unicode-Data-License.txt",
    "eSpeak-NG-Apache-2.0.txt",
    "eSpeak-NG-BSD-2-Clause.txt",
    "eSpeak-NG-GPL-3.0-or-later.txt",
    "omnivox-Cargo.lock",
}


class PiperVerificationError(common.VerificationError):
    """A Piper companion artifact violated its release contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PiperVerificationError(message)


def repository_version(repository: Path) -> str:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def parse_arguments(repository: Path) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    model_default = os.environ.get("PIPER_MODEL") or None
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=release / f"omnivox-{version}-piper-linux-x64.tar.gz",
    )
    parser.add_argument(
        "--checksums", type=Path, default=release / "piper-sha256sums.txt"
    )
    parser.add_argument("--version", default=version)
    parser.add_argument(
        "--omnivox",
        type=Path,
        help="matching Piper-enabled main binary for end-to-end synthesis",
    )
    parser.add_argument(
        "--model",
        type=Path,
        default=Path(model_default) if model_default else None,
        help="licence-reviewed test model (or set PIPER_MODEL)",
    )
    return parser.parse_args()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_checksum_path(value: str) -> str:
    require("\\" not in value, f"inner checksum path uses a backslash: {value!r}")
    path = PurePosixPath(value)
    require(not path.is_absolute(), f"inner checksum path is absolute: {value!r}")
    parts = tuple(part for part in path.parts if part not in ("", "."))
    require(parts and ".." not in parts, f"unsafe inner checksum path: {value!r}")
    return PurePosixPath(*parts).as_posix()


def verify_inner_checksums(directory: Path) -> None:
    entries: dict[str, str] = {}
    for line in (directory / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        require(match is not None, f"invalid inner checksum line: {line!r}")
        relative = safe_checksum_path(match.group(2))
        require(relative not in entries, f"duplicate inner checksum: {relative}")
        entries[relative] = match.group(1)

    actual_paths = {
        path.relative_to(directory).as_posix(): path
        for path in directory.rglob("*")
        if path.is_file() and path != directory / "SHA256SUMS"
    }
    require(
        entries.keys() == actual_paths.keys(),
        "inner checksums do not cover exactly every companion file",
    )
    for relative, path in actual_paths.items():
        require(
            sha256_file(path) == entries[relative],
            f"inner checksum mismatch: {relative}",
        )


def packaged_workspace_versions(lock_path: Path) -> dict[str, str]:
    contents = lock_path.read_text(encoding="utf-8")
    versions: dict[str, str] = {}
    for package in re.split(r"^\[\[package\]\]\s*$", contents, flags=re.MULTILINE):
        name = re.search(r'^name = "([^"]+)"$', package, flags=re.MULTILINE)
        version = re.search(r'^version = "([^"]+)"$', package, flags=re.MULTILINE)
        if name and version and name.group(1).startswith("omnivox-"):
            versions[name.group(1)] = version.group(1)
    return versions


def verify_provenance(directory: Path, version: str) -> None:
    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(provenance.get("schema_version") == 1, "unknown provenance schema")
    require(
        provenance.get("artifact") == "omnivox-piper-companion-linux-x64",
        "wrong artifact provenance",
    )
    require(provenance.get("target") == TARGET, "wrong target provenance")
    require(
        provenance.get("voice_model_included") is False,
        "voice model exclusion is not recorded",
    )

    omnivox = provenance.get("omnivox")
    require(isinstance(omnivox, dict), "Omnivox provenance is missing")
    require(
        re.fullmatch(r"[0-9a-f]{40}", str(omnivox.get("commit", ""))) is not None,
        "invalid Omnivox source commit",
    )
    require(
        omnivox.get("tracked_worktree_dirty") is False,
        "archive was staged from a dirty worktree",
    )

    libpiper = provenance.get("libpiper")
    require(isinstance(libpiper, dict), "libpiper provenance is missing")
    require(libpiper.get("version") == PIPER_VERSION, "wrong libpiper version")
    require(libpiper.get("commit") == PIPER_COMMIT, "wrong libpiper commit")

    for component in ("espeak_ng", "sonic", "onnxruntime"):
        value = provenance.get(component)
        require(isinstance(value, dict), f"{component} provenance is missing")
        require(
            value.get("verified_before_build") is True,
            f"{component} was not verified before build",
        )
        for field in ("archive_sha256", "source_tree_sha256"):
            require(
                re.fullmatch(r"[0-9a-f]{64}", str(value.get(field, ""))) is not None,
                f"invalid {component} {field}",
            )
    require(
        re.fullmatch(r"[0-9a-f]{64}", str(provenance.get("native_input_lock_sha256", "")))
        is not None,
        "invalid native input lock digest",
    )

    versions = packaged_workspace_versions(
        directory / "third-party-licenses/omnivox-Cargo.lock"
    )
    require(versions, "packaged Cargo.lock has no Omnivox packages")
    wrong = sorted(
        f"{name}={package_version}"
        for name, package_version in versions.items()
        if package_version != version
    )
    require(not wrong, f"packaged workspace version mismatch: {wrong}")


def verify_layout(extracted: Path, version: str) -> Path:
    require(
        {path.name for path in extracted.iterdir()} == {"piper"},
        "archive must contain exactly one top-level piper directory",
    )
    directory = extracted / "piper"
    require(directory.is_dir(), "archive piper directory is missing")
    actual_root = {path.name for path in directory.iterdir()}
    require(
        actual_root == EXPECTED_ROOT,
        f"unexpected Piper root entries: {sorted(actual_root)}",
    )
    notices = directory / "third-party-licenses"
    require(
        {path.name for path in notices.iterdir()} == EXPECTED_NOTICES,
        "Piper third-party notice set is incomplete or unexpected",
    )
    for name in ("LICENSE", "LICENSING.md", "README.md"):
        require((directory / name).stat().st_size > 100, f"{name} is empty")
    data = directory / "espeak-ng-data"
    require((data / "phontab").is_file(), "Piper espeak-ng-data/phontab is missing")
    require(
        sum(path.is_file() for path in data.rglob("*")) >= 100,
        "Piper eSpeak data payload is unexpectedly small",
    )
    helper = directory / "omnivox-piper-helper"
    require(helper.stat().st_mode & 0o111 != 0, "Piper helper is not executable")
    require(
        not any(path.name.endswith((".onnx", ".onnx.json")) for path in directory.rglob("*")),
        "companion archive unexpectedly contains a voice model or configuration",
    )
    verify_inner_checksums(directory)
    verify_provenance(directory, version)
    return directory


def verify_runpath(binary: Path, directory: Path) -> None:
    environment = common.clean_environment()
    dynamic = common.run(["readelf", "-d", str(binary)], directory, environment)
    paths = re.findall(r"\((?:RPATH|RUNPATH)\).*\[([^]]+)\]", dynamic)
    require(paths == ["$ORIGIN"], f"unexpected runtime search path for {binary.name}: {paths}")


def verify_native_runtime(directory: Path) -> None:
    for name in RUNTIME_BINARIES:
        common.verify_architecture(directory / name, "linux", "x86_64")
    verify_runpath(directory / "omnivox-piper-helper", directory)
    verify_runpath(directory / "libpiper.so", directory)

    environment = common.clean_environment()
    for name in RUNTIME_BINARIES:
        dependencies = common.run(
            ["ldd", str((directory / name).resolve())], directory, environment
        )
        require(
            "not found" not in dependencies,
            f"unresolved dependency for {name}:\n{dependencies}",
        )
    helper_dependencies = common.run(
        ["ldd", str((directory / "omnivox-piper-helper").resolve())],
        directory,
        environment,
    )
    for name in ("libpiper.so", "libonnxruntime.so.1"):
        require(
            str((directory / name).resolve()) in helper_dependencies,
            f"helper did not resolve {name} from its relocated companion directory",
        )


def verify_synthesis(
    directory: Path, omnivox: Path, model: Path, version: str, working: Path
) -> None:
    require(omnivox.is_file(), f"matching Omnivox binary is missing: {omnivox}")
    require(model.is_file(), f"Piper model is missing: {model}")
    require(
        model.with_suffix(model.suffix + ".json").is_file()
        or model.with_suffix(".json").is_file(),
        f"Piper model configuration is missing beside {model}",
    )
    installed = directory.parent / "omnivox"
    shutil.copy2(omnivox, installed)
    installed.chmod(installed.stat().st_mode | 0o755)
    common.verify_architecture(installed, "linux", "x86_64")

    environment = common.clean_environment()
    environment["OMNIVOX_PIPER_MODEL"] = str(model.resolve())
    command = str(installed.resolve())
    output = common.run([command, "--version"], working, environment).strip()
    require(output == f"omnivox {version}", f"unexpected Omnivox version: {output!r}")
    voices = common.run(
        [command, "--engine", "piper", "--list-voices"], working, environment
    )
    match = re.search(r"Found\s+(\d+)\s+voices", voices)
    require(match is not None and int(match.group(1)) > 0, "Piper returned no voices")
    voice = re.search(r"\[(piper:[^\]\s]+)\]", voices)
    require(voice is not None, "Piper voice inventory has no physical voice ID")

    wav = working / "piper release probe.wav"
    common.run(
        [
            command,
            "--engine",
            "piper",
            "--dump-wav",
            voice.group(1),
            str(wav),
            "Relocated Piper release verification with a moderately long sentence.",
        ],
        working,
        environment,
    )
    raw = wav.with_name("piper release probe_raw.wav")
    require(wav.is_file() and raw.is_file(), "Piper did not write both WAV outputs")
    common.read_wav(raw, canonical=False)
    common.read_wav(wav, canonical=True)


def verify(arguments: argparse.Namespace) -> None:
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"archive does not exist: {archive}")
    require(checksums.is_file(), f"checksum file does not exist: {checksums}")
    expected_name = f"omnivox-{arguments.version}-piper-linux-x64.tar.gz"
    require(
        archive.name == expected_name,
        f"unexpected Piper archive name: {archive.name}",
    )
    common.verify_checksum(archive, checksums)
    require(archive.name.endswith(".tar.gz"), "Linux Piper release must be a .tar.gz")

    if arguments.omnivox is not None:
        require(arguments.model is not None, "--omnivox requires --model")
    if arguments.model is not None and arguments.omnivox is None:
        default_binary = Path(__file__).resolve().parent.parent / "target/release/omnivox"
        arguments.omnivox = default_binary

    with tempfile.TemporaryDirectory(prefix="Omnivox Piper verification ") as temporary:
        root = Path(temporary)
        extracted = root / "Extracted companion with spaces"
        working = root / "Unrelated working directory"
        extracted.mkdir()
        working.mkdir()
        common.extract_tar(archive, extracted)
        directory = verify_layout(extracted, arguments.version)
        verify_native_runtime(directory)
        if arguments.model is not None:
            assert arguments.omnivox is not None
            verify_synthesis(
                directory,
                arguments.omnivox.resolve(),
                arguments.model.resolve(),
                arguments.version,
                working,
            )

    mode = "structural and native-runtime"
    if arguments.model is not None:
        mode += " with real synthesis"
    print(f"PASS {archive.name}: {mode} verification")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        verify(parse_arguments(repository))
    except (
        OSError,
        common.VerificationError,
        json.JSONDecodeError,
        subprocess.TimeoutExpired,
        tarfile.TarError,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
