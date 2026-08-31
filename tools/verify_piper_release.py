#!/usr/bin/env python3
"""Verify a native Piper companion archive and optional real synthesis."""

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
import zipfile

sys.dont_write_bytecode = True
import verify_release as common
from build_piper import PlatformConfig, StagingError, native_platform


PIPER_VERSION = "1.7.0"
PIPER_COMMIT = "7b8e8f7197a480047677715f00d3d78903b55a2a"
EXPECTED_COMMON_ROOT = {
    "LICENSE",
    "LICENSING.md",
    "README.md",
    "SHA256SUMS",
    "SOURCE-PROVENANCE.json",
    "espeak-ng-data",
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


def parse_arguments(
    repository: Path, configuration: PlatformConfig
) -> argparse.Namespace:
    version = repository_version(repository)
    release = repository / "target/release"
    model_default = os.environ.get("PIPER_MODEL") or None
    extension = "zip" if configuration.platform_name == "windows" else "tar.gz"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive",
        type=Path,
        default=(
            release
            / f"omnivox-{version}-piper-{configuration.artifact_suffix}.{extension}"
        ),
    )
    parser.add_argument(
        "--checksums", type=Path, default=release / "piper-sha256sums.txt"
    )
    parser.add_argument("--version", default=version)
    main_binary = parser.add_mutually_exclusive_group()
    main_binary.add_argument(
        "--omnivox",
        type=Path,
        help="matching Piper-enabled main binary for end-to-end synthesis",
    )
    main_binary.add_argument(
        "--omnivox-archive",
        type=Path,
        help="matching generic release archive for end-to-end synthesis",
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


def verify_provenance(
    directory: Path, version: str, configuration: PlatformConfig
) -> None:
    provenance = json.loads(
        (directory / "SOURCE-PROVENANCE.json").read_text(encoding="utf-8")
    )
    require(provenance.get("schema_version") == 1, "unknown provenance schema")
    require(
        provenance.get("artifact")
        == f"omnivox-piper-companion-{configuration.artifact_suffix}",
        "wrong artifact provenance",
    )
    require(
        provenance.get("target") == configuration.target,
        "wrong target provenance",
    )
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
        re.fullmatch(
            r"[0-9a-f]{64}",
            str(provenance.get("native_input_lock_sha256", "")),
        )
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


def verify_layout(
    extracted: Path, version: str, configuration: PlatformConfig
) -> Path:
    require(
        {path.name for path in extracted.iterdir()} == {"piper"},
        "archive must contain exactly one top-level piper directory",
    )
    directory = extracted / "piper"
    require(directory.is_dir(), "archive piper directory is missing")
    actual_root = {path.name for path in directory.iterdir()}
    expected_root = EXPECTED_COMMON_ROOT | {
        configuration.helper,
        *configuration.runtime_files,
    }
    require(
        actual_root == expected_root,
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
    helper = directory / configuration.helper
    if configuration.platform_name != "windows":
        require(helper.stat().st_mode & 0o111 != 0, "Piper helper is not executable")
    require(
        not any(
            path.name.endswith((".onnx", ".onnx.json"))
            for path in directory.rglob("*")
        ),
        "companion archive unexpectedly contains a voice model or configuration",
    )
    verify_inner_checksums(directory)
    verify_provenance(directory, version, configuration)
    return directory


def verify_runpath(binary: Path, directory: Path) -> None:
    environment = common.clean_environment()
    dynamic = common.run(["readelf", "-d", str(binary)], directory, environment)
    paths = re.findall(r"\((?:RPATH|RUNPATH)\).*\[([^]]+)\]", dynamic)
    require(
        paths == ["$ORIGIN"],
        f"unexpected runtime search path for {binary.name}: {paths}",
    )


def verify_linux_runtime(directory: Path, configuration: PlatformConfig) -> None:
    for name in (configuration.helper, *configuration.runtime_files):
        common.verify_architecture(directory / name, "linux", "x86_64")
    verify_runpath(directory / configuration.helper, directory)
    verify_runpath(directory / configuration.libpiper, directory)

    environment = common.clean_environment()
    for name in (configuration.helper, *configuration.runtime_files):
        dependencies = common.run(
            ["ldd", str((directory / name).resolve())], directory, environment
        )
        require(
            "not found" not in dependencies,
            f"unresolved dependency for {name}:\n{dependencies}",
        )
    helper_dependencies = common.run(
        ["ldd", str((directory / configuration.helper).resolve())],
        directory,
        environment,
    )
    for name in ("libpiper.so", "libonnxruntime.so.1"):
        require(
            str((directory / name).resolve()) in helper_dependencies,
            f"helper did not resolve {name} from its relocated companion directory",
        )


def verify_macos_runtime(directory: Path, configuration: PlatformConfig) -> None:
    architecture = "aarch64" if configuration.architecture == "arm64" else "x86_64"
    for name in (configuration.helper, *configuration.runtime_files):
        common.verify_architecture(directory / name, "macos", architecture)
        output = common.run(
            ["lipo", "-archs", str(directory / name)],
            directory,
            common.clean_environment(),
        )
        require(
            output.split() == [configuration.architecture],
            f"unexpected Mach-O architectures for {name}: {output.strip()}",
        )

    for name in (configuration.helper, configuration.libpiper):
        load_commands = common.run(
            ["otool", "-l", str(directory / name)],
            directory,
            common.clean_environment(),
        )
        require(
            "path @loader_path" in load_commands,
            f"{name} has no @loader_path LC_RPATH",
        )

    helper_dependencies = common.run(
        ["otool", "-L", str(directory / configuration.helper)],
        directory,
        common.clean_environment(),
    )
    require(
        any(
            value in helper_dependencies
            for value in ("@rpath/libpiper.dylib", "@loader_path/libpiper.dylib")
        ),
        "helper does not use relocated libpiper",
    )
    onnx_name = configuration.runtime_files[-1]
    piper_dependencies = common.run(
        ["otool", "-L", str(directory / configuration.libpiper)],
        directory,
        common.clean_environment(),
    )
    require(
        any(
            value in piper_dependencies
            for value in (f"@rpath/{onnx_name}", f"@loader_path/{onnx_name}")
        ),
        "libpiper does not use relocated ONNX Runtime",
    )


def pe_imports(path: Path) -> set[str]:
    data = path.read_bytes()

    def unsigned(offset: int, size: int) -> int:
        require(offset >= 0 and offset + size <= len(data), f"truncated PE: {path}")
        return int.from_bytes(data[offset : offset + size], "little")

    require(len(data) >= 64 and data[:2] == b"MZ", f"not a PE binary: {path}")
    pe_offset = unsigned(0x3C, 4)
    require(data[pe_offset : pe_offset + 4] == b"PE\0\0", f"missing PE header: {path}")
    coff = pe_offset + 4
    section_count = unsigned(coff + 2, 2)
    optional_size = unsigned(coff + 16, 2)
    optional = coff + 20
    require(unsigned(optional, 2) == 0x20B, f"PE is not 64-bit: {path}")
    import_rva = unsigned(optional + 112 + 8, 4)
    section_table = optional + optional_size

    sections: list[tuple[int, int, int]] = []
    for index in range(section_count):
        section = section_table + index * 40
        virtual_size = unsigned(section + 8, 4)
        virtual_address = unsigned(section + 12, 4)
        raw_size = unsigned(section + 16, 4)
        raw_offset = unsigned(section + 20, 4)
        sections.append((virtual_address, max(virtual_size, raw_size), raw_offset))

    def file_offset(rva: int) -> int:
        for virtual_address, size, raw_offset in sections:
            if virtual_address <= rva < virtual_address + size:
                return raw_offset + rva - virtual_address
        raise PiperVerificationError(f"PE RVA is outside sections: {path}")

    if import_rva == 0:
        return set()
    imports: set[str] = set()
    descriptor = file_offset(import_rva)
    for _ in range(4096):
        values = tuple(unsigned(descriptor + offset, 4) for offset in range(0, 20, 4))
        if values == (0, 0, 0, 0, 0):
            return imports
        name_offset = file_offset(values[3])
        end = data.find(b"\0", name_offset, min(len(data), name_offset + 1024))
        require(end != -1, f"unterminated PE import name: {path}")
        imports.add(data[name_offset:end].decode("ascii").lower())
        descriptor += 20
    raise PiperVerificationError(f"unbounded PE import table: {path}")


def verify_windows_runtime(directory: Path, configuration: PlatformConfig) -> None:
    for name in (configuration.helper, *configuration.runtime_files):
        common.verify_architecture(directory / name, "windows", "x86_64")
    helper_imports = pe_imports(directory / configuration.helper)
    require("piper.dll" in helper_imports, "helper does not import adjacent piper.dll")
    piper_imports = pe_imports(directory / configuration.libpiper)
    require(
        "onnxruntime.dll" in piper_imports,
        "piper.dll does not import adjacent onnxruntime.dll",
    )


def verify_native_runtime(directory: Path, configuration: PlatformConfig) -> None:
    if configuration.platform_name == "linux":
        verify_linux_runtime(directory, configuration)
    elif configuration.platform_name == "macos":
        verify_macos_runtime(directory, configuration)
    else:
        verify_windows_runtime(directory, configuration)


def verify_synthesis(
    directory: Path,
    omnivox: Path,
    model: Path,
    version: str,
    working: Path,
    configuration: PlatformConfig,
) -> None:
    require(omnivox.is_file(), f"matching Omnivox binary is missing: {omnivox}")
    require(model.is_file(), f"Piper model is missing: {model}")
    require(
        model.with_suffix(model.suffix + ".json").is_file()
        or model.with_suffix(".json").is_file(),
        f"Piper model configuration is missing beside {model}",
    )
    binary_name = (
        "omnivox.exe" if configuration.platform_name == "windows" else "omnivox"
    )
    installed = directory.parent / binary_name
    shutil.copy2(omnivox, installed)
    if configuration.platform_name != "windows":
        installed.chmod(installed.stat().st_mode | 0o755)
    architecture = "aarch64" if configuration.architecture == "arm64" else "x86_64"
    common.verify_architecture(
        installed, configuration.platform_name, architecture
    )

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

    def verify_espeak_fallback(candidate: Path, label: str) -> None:
        fallback_environment = common.clean_environment()
        fallback_environment["OMNIVOX_PIPER_MODEL"] = str(candidate.resolve())
        fallback_voices = common.run(
            [command, "--engine", "piper", "--list-voices"],
            working,
            fallback_environment,
        )
        require(
            re.search(r"\[espeak:[^\]\s]+\]", fallback_voices) is not None,
            f"{label} Piper model did not preserve the eSpeak fallback",
        )
        require(
            "[piper:" not in fallback_voices,
            f"{label} Piper model unexpectedly registered a Piper voice",
        )

    verify_espeak_fallback(working / "missing model.onnx", "missing")
    corrupt_model = working / "corrupt model.onnx"
    corrupt_model.write_bytes(b"not an ONNX model\n")
    source_config = model.with_suffix(model.suffix + ".json")
    if not source_config.is_file():
        source_config = model.with_suffix(".json")
    shutil.copy2(source_config, corrupt_model.with_suffix(".onnx.json"))
    verify_espeak_fallback(corrupt_model, "corrupt")


def verify(arguments: argparse.Namespace, configuration: PlatformConfig) -> None:
    archive = arguments.archive.resolve()
    checksums = arguments.checksums.resolve()
    require(archive.is_file(), f"archive does not exist: {archive}")
    require(checksums.is_file(), f"checksum file does not exist: {checksums}")
    extension = "zip" if configuration.platform_name == "windows" else "tar.gz"
    expected_name = (
        f"omnivox-{arguments.version}-piper-{configuration.artifact_suffix}."
        f"{extension}"
    )
    require(
        archive.name == expected_name,
        f"unexpected Piper archive name: {archive.name}",
    )
    common.verify_checksum(archive, checksums)

    if arguments.omnivox is not None or arguments.omnivox_archive is not None:
        require(
            arguments.model is not None,
            "--omnivox or --omnivox-archive requires --model",
        )
    if (
        arguments.model is not None
        and arguments.omnivox is None
        and arguments.omnivox_archive is None
    ):
        binary_name = (
            "omnivox.exe" if configuration.platform_name == "windows" else "omnivox"
        )
        default_binary = (
            Path(__file__).resolve().parent.parent / "target/release" / binary_name
        )
        arguments.omnivox = default_binary

    with tempfile.TemporaryDirectory(prefix="Omnivox Piper verification ") as temporary:
        root = Path(temporary)
        extracted = root / "Extracted companion with spaces"
        working = root / "Unrelated working directory"
        extracted.mkdir()
        working.mkdir()
        common.extract_archive(archive, extracted, configuration.platform_name)
        directory = verify_layout(extracted, arguments.version, configuration)
        verify_native_runtime(directory, configuration)
        if arguments.model is not None:
            omnivox = arguments.omnivox
            if arguments.omnivox_archive is not None:
                main_archive = arguments.omnivox_archive.resolve()
                require(
                    main_archive.is_file(),
                    f"matching Omnivox archive is missing: {main_archive}",
                )
                expected_main_name = (
                    f"omnivox-{arguments.version}-{configuration.artifact_suffix}."
                    f"{extension}"
                )
                require(
                    main_archive.name == expected_main_name,
                    f"unexpected Omnivox archive name: {main_archive.name}",
                )
                common.verify_checksum(main_archive, checksums)
                main_extracted = root / "Extracted main release with spaces"
                main_extracted.mkdir()
                common.extract_archive(
                    main_archive, main_extracted, configuration.platform_name
                )
                omnivox = common.verify_layout(
                    main_extracted, configuration.platform_name, arguments.version
                )
                architecture = (
                    "aarch64"
                    if configuration.architecture == "arm64"
                    else "x86_64"
                )
                common.verify_architecture(
                    omnivox,
                    configuration.platform_name,
                    architecture,
                )
            assert omnivox is not None
            verify_synthesis(
                directory,
                omnivox.resolve(),
                arguments.model.resolve(),
                arguments.version,
                working,
                configuration,
            )

    mode = "structural and native-runtime"
    if arguments.model is not None:
        mode += " with real synthesis and model-failure fallback"
    print(f"PASS {archive.name}: {mode} verification")


def main() -> int:
    repository = Path(__file__).resolve().parent.parent
    try:
        configuration = native_platform()
        verify(parse_arguments(repository, configuration), configuration)
    except (
        OSError,
        common.VerificationError,
        StagingError,
        json.JSONDecodeError,
        subprocess.TimeoutExpired,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
