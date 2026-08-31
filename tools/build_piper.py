#!/usr/bin/env python3
"""Build and atomically stage the native Piper companion runtime."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
from typing import Iterable


PIPER_VERSION = "1.7.0"
PIPER_COMMIT = "7b8e8f7197a480047677715f00d3d78903b55a2a"
ESPEAK_COMMIT = "212928b394a96e8fd2096616bfd54e17845c48f6"
SONIC_COMMIT = "fbf75c3d6d846bad3bb3d456cbc5d07d9fd8c104"
ONNXRUNTIME_VERSION = "1.22.0"
LINUX_TARGET = "x86_64-unknown-linux-gnu"
RUNTIME_FILES = (
    "libpiper.so",
    "libonnxruntime.so.1",
    "libonnxruntime_providers_shared.so",
)


class StagingError(RuntimeError):
    """A build output cannot form the requested companion payload."""


def usage() -> str:
    return (
        "usage: python tools/build_piper.py [cargo build arguments]\n\n"
        "Builds omnivox-piper-helper with locked dependencies and stages an "
        "isolated relocatable companion directory beside the Cargo profile "
        "output. Linux x64 is the first supported staging target.\n\n"
        "example:\n"
        "  python3 tools/build_piper.py --release"
    )


def render_cargo_message(message: dict[str, object]) -> None:
    if message.get("reason") != "compiler-message":
        return
    compiler_message = message.get("message")
    if not isinstance(compiler_message, dict):
        return
    rendered = compiler_message.get("rendered")
    if isinstance(rendered, str):
        sys.stderr.write(rendered)


def run_cargo_build(arguments: list[str]) -> tuple[list[Path], list[Path]]:
    forbidden = ("--message-format", "--package", "--features")
    for argument in arguments:
        if argument == "-p" or argument.startswith(forbidden):
            raise StagingError(
                "tools/build_piper.py owns Cargo package, feature, and message "
                f"selection: {argument}"
            )

    command = [
        "cargo",
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        "--package",
        "omnivox-piper-helper",
        "--features",
        "piper",
        *arguments,
    ]
    environment = dict(os.environ)
    environment["OMNIVOX_PIPER_RELOCATABLE"] = "1"
    print("+ " + " ".join(command), file=sys.stderr)
    process = subprocess.Popen(
        command,
        env=environment,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if process.stdout is None:
        raise StagingError("failed to capture Cargo build messages")

    executables: list[Path] = []
    library_directories: list[Path] = []
    for line in process.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(line)
            continue
        render_cargo_message(message)

        if message.get("reason") == "compiler-artifact":
            target = message.get("target")
            executable = message.get("executable")
            if (
                isinstance(target, dict)
                and target.get("name") == "omnivox-piper-helper"
                and isinstance(executable, str)
            ):
                executables.append(Path(executable).resolve())

        package_id = str(message.get("package_id", ""))
        if (
            message.get("reason") == "build-script-executed"
            and "/omnivox-piper-sys#" in package_id
        ):
            linked_paths = message.get("linked_paths")
            if isinstance(linked_paths, list):
                for value in linked_paths:
                    if isinstance(value, str) and value.startswith("native="):
                        library_directories.append(
                            Path(value.removeprefix("native=")).resolve()
                        )

    return_code = process.wait()
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)
    return executables, library_directories


def prepare_inputs(repository: Path) -> None:
    command = [
        sys.executable,
        str(repository / "tools/prepare_piper_inputs.py"),
        "--target",
        LINUX_TARGET,
    ]
    if configured := os.environ.get("OMNIVOX_PIPER_INPUTS_DIR"):
        command.extend(["--output", configured])
    print("+ " + " ".join(command), file=sys.stderr)
    subprocess.run(command, check=True)


def select_unique(paths: Iterable[Path], description: str) -> Path:
    candidates = sorted(set(paths))
    if len(candidates) != 1:
        rendered = "\n  ".join(str(path) for path in candidates) or "<none>"
        raise StagingError(
            f"Cargo reported {len(candidates)} {description} candidates; "
            "refusing to guess:\n"
            f"  {rendered}"
        )
    return candidates[0]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_tree_digest(root: Path, excluded: set[str] | None = None) -> str:
    excluded = excluded or set()
    digest = hashlib.sha256()
    paths: list[Path] = []
    for directory, names, filenames in os.walk(root):
        names[:] = sorted(name for name in names if name != ".git")
        for filename in sorted(filenames):
            path = Path(directory) / filename
            relative = path.relative_to(root).as_posix()
            if relative not in excluded:
                paths.append(path)
    for path in sorted(paths):
        relative = path.relative_to(root).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(sha256_file(path).encode("ascii"))
        digest.update(b"\n")
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


def copy_required(source: Path, destination: Path) -> None:
    if not source.is_file():
        raise StagingError(f"required Piper companion input is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def validate_native_layout(executable: Path, library_dir: Path) -> tuple[Path, Path]:
    if sys.platform != "linux" or platform.machine().lower() not in {
        "x86_64",
        "amd64",
    }:
        raise StagingError(
            "Piper companion staging currently supports native Linux x64 only"
        )
    if not executable.is_file():
        raise StagingError(f"Piper helper executable is missing: {executable}")

    install_dir = library_dir.parent
    native_root = install_dir.parent
    if (
        library_dir.name != "lib"
        or install_dir.name != "i1"
        or native_root.name != LINUX_TARGET
        or native_root.parent.name != PIPER_VERSION
    ):
        raise StagingError(f"unexpected Piper native output layout: {library_dir}")
    for filename in RUNTIME_FILES:
        if not (library_dir / filename).is_file():
            raise StagingError(f"required Piper runtime library is missing: {filename}")
    data_dir = install_dir / "share/espeak-ng-data"
    if not (data_dir / "phontab").is_file():
        raise StagingError(f"Piper eSpeak data is missing: {data_dir}")
    return native_root, data_dir


def native_inputs_root(native_root: Path) -> Path:
    configured = os.environ.get("OMNIVOX_PIPER_INPUTS_DIR")
    if configured:
        inputs = Path(configured).resolve()
    else:
        target_base = native_root.parent.parent.parent
        inputs = target_base / "piper-inputs" / PIPER_VERSION / native_root.name
    if not (inputs / "PREPARED.json").is_file():
        raise StagingError(f"verified Piper inputs are missing: {inputs}")
    return inputs


def stage_notices(repository: Path, native_root: Path, destination: Path) -> None:
    inputs = native_inputs_root(native_root)
    espeak_source = inputs / "sources/espeak-ng"
    sonic_source = inputs / "sources/sonic"
    onnx_source = inputs / "sources/onnxruntime"

    copy_required(
        repository / "omnivox-piper-helper/runtime-assets/THIRD-PARTY-NOTICES.md",
        destination / "THIRD-PARTY-NOTICES.md",
    )
    files = (
        (
            repository / "third-party/piper1-gpl/COPYING",
            "Piper-GPL-3.0-or-later.txt",
        ),
        (repository / "third-party/piper1-gpl/UPSTREAM.md", "Piper-UPSTREAM.md"),
        (espeak_source / "COPYING", "eSpeak-NG-GPL-3.0-or-later.txt"),
        (espeak_source / "COPYING.APACHE", "eSpeak-NG-Apache-2.0.txt"),
        (espeak_source / "COPYING.BSD2", "eSpeak-NG-BSD-2-Clause.txt"),
        (espeak_source / "COPYING.UCD", "Unicode-Data-License.txt"),
        (espeak_source / "src/ucd-tools/COPYING", "UCD-Tools-GPL-3.0.txt"),
        (
            espeak_source / "src/ucd-tools/COPYING.UCD",
            "UCD-Tools-Unicode-Data-License.txt",
        ),
        (espeak_source / "src/compat/getopt.c", "NetBSD-getopt.c"),
        (sonic_source / "LICENSE", "Sonic-Apache-2.0.txt"),
        (onnx_source / "LICENSE", "ONNX-Runtime-MIT.txt"),
        (
            onnx_source / "ThirdPartyNotices.txt",
            "ONNX-Runtime-ThirdPartyNotices.txt",
        ),
        (repository / "Cargo.lock", "omnivox-Cargo.lock"),
    )
    for source, filename in files:
        copy_required(source, destination / filename)


def provenance(repository: Path, native_root: Path) -> dict[str, object]:
    inputs = native_inputs_root(native_root)
    prepared = json.loads((inputs / "PREPARED.json").read_text(encoding="utf-8"))
    components = prepared.get("components")
    if not isinstance(components, dict):
        raise StagingError("prepared Piper input marker has no components object")
    espeak = components.get("espeak_ng")
    sonic = components.get("sonic")
    onnxruntime = components.get("onnxruntime")
    if not all(isinstance(value, dict) for value in (espeak, sonic, onnxruntime)):
        raise StagingError("prepared Piper input marker is incomplete")
    assert isinstance(espeak, dict)
    assert isinstance(sonic, dict)
    assert isinstance(onnxruntime, dict)
    if espeak.get("commit") != ESPEAK_COMMIT or sonic.get("commit") != SONIC_COMMIT:
        raise StagingError(
            "prepared Piper source commit does not match the staging lock"
        )
    if onnxruntime.get("version") != ONNXRUNTIME_VERSION:
        raise StagingError(
            "prepared ONNX Runtime version does not match the staging lock"
        )

    tracked_status = git_output(
        repository, "status", "--porcelain", "--untracked-files=no"
    )
    vendored_piper = repository / "third-party/piper1-gpl"
    return {
        "schema_version": 1,
        "artifact": "omnivox-piper-companion-linux-x64",
        "target": LINUX_TARGET,
        "omnivox": {
            "repository": "https://github.com/bartbunting/omnivox",
            "commit": git_output(repository, "rev-parse", "HEAD"),
            "tracked_worktree_dirty": bool(tracked_status),
        },
        "libpiper": {
            "repository": "https://github.com/OHF-Voice/piper1-gpl",
            "version": PIPER_VERSION,
            "commit": PIPER_COMMIT,
            "vendored_source_tree_sha256": source_tree_digest(
                vendored_piper, {"UPSTREAM.md"}
            ),
        },
        "espeak_ng": {
            "repository": "https://github.com/espeak-ng/espeak-ng",
            "commit": ESPEAK_COMMIT,
            "archive": espeak["archive"],
            "archive_sha256": espeak["sha256"],
            "source_tree_sha256": espeak["source_tree_sha256"],
            "verified_before_build": True,
        },
        "sonic": {
            "repository": "https://github.com/waywardgeek/sonic",
            "commit": SONIC_COMMIT,
            "archive": sonic["archive"],
            "archive_sha256": sonic["sha256"],
            "source_tree_sha256": sonic["source_tree_sha256"],
            "verified_before_build": True,
        },
        "onnxruntime": {
            "repository": "https://github.com/microsoft/onnxruntime",
            "version": ONNXRUNTIME_VERSION,
            "archive": onnxruntime["archive"],
            "archive_sha256": onnxruntime["sha256"],
            "source_tree_sha256": onnxruntime["source_tree_sha256"],
            "verified_before_build": True,
        },
        "native_input_lock_sha256": prepared["lock_file_sha256"],
        "voice_model_included": False,
    }


def write_checksums(directory: Path) -> int:
    checksum_path = directory / "SHA256SUMS"
    files = sorted(
        path
        for path in directory.rglob("*")
        if path.is_file() and path != checksum_path
    )
    checksum_path.write_text(
        "".join(
            f"{sha256_file(path)}  {path.relative_to(directory).as_posix()}\n"
            for path in files
        ),
        encoding="utf-8",
    )
    return len(files)


def run_checked(command: list[str], cwd: Path) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        env={
            key: value
            for key, value in os.environ.items()
            if key != "LD_LIBRARY_PATH"
        },
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode != 0:
        raise StagingError(
            f"command failed ({result.returncode}): "
            f"{' '.join(command)}\n{result.stdout}"
        )
    return result.stdout


def verify_linux_runtime(directory: Path) -> None:
    helper = directory / "omnivox-piper-helper"
    helper_dynamic = run_checked(["readelf", "-d", str(helper)], directory)
    if "$ORIGIN" not in helper_dynamic or "piper-native" in helper_dynamic:
        raise StagingError(
            f"staged helper has a non-relocatable RUNPATH:\n{helper_dynamic}"
        )
    piper_dynamic = run_checked(
        ["readelf", "-d", str(directory / "libpiper.so")], directory
    )
    if "$ORIGIN" not in piper_dynamic:
        raise StagingError(
            "staged libpiper does not search for ONNX Runtime beside itself"
        )

    dependencies = run_checked(["ldd", str(helper)], directory)
    if "not found" in dependencies:
        raise StagingError(
            f"staged helper has unresolved native dependencies:\n{dependencies}"
        )
    for filename in ("libpiper.so", "libonnxruntime.so.1"):
        expected = str((directory / filename).resolve())
        if expected not in dependencies:
            raise StagingError(
                f"staged helper did not resolve {filename} from its companion "
                "directory:\n"
                f"{dependencies}"
            )


def replace_directory(source: Path, destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    source.replace(destination)


def stage_companion(repository: Path, executable: Path, library_dir: Path) -> Path:
    native_root, data_source = validate_native_layout(executable, library_dir)
    profile_dir = executable.parent
    destination = profile_dir / "piper"
    temporary = Path(
        tempfile.mkdtemp(
            prefix=f".{destination.name}.tmp-{os.getpid()}-", dir=profile_dir
        )
    )
    try:
        copy_required(executable, temporary / "omnivox-piper-helper")
        for filename in RUNTIME_FILES:
            copy_required(library_dir / filename, temporary / filename)
        shutil.copytree(data_source, temporary / "espeak-ng-data")
        copy_required(repository / "LICENSE", temporary / "LICENSE")
        copy_required(repository / "docs/LICENSING.md", temporary / "LICENSING.md")
        copy_required(
            repository / "omnivox-piper-helper/runtime-assets/PIPER-COMPANION.md",
            temporary / "README.md",
        )
        stage_notices(repository, native_root, temporary / "third-party-licenses")
        (temporary / "SOURCE-PROVENANCE.json").write_text(
            json.dumps(provenance(repository, native_root), indent=2, sort_keys=True)
            + "\n",
            encoding="utf-8",
        )
        file_count = write_checksums(temporary)
        verify_linux_runtime(temporary)
        replace_directory(temporary, destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)

    byte_count = sum(
        path.stat().st_size
        for path in destination.rglob("*")
        if path.is_file()
    )
    print(
        f"Staged {file_count + 1} Piper companion files "
        f"({byte_count / (1024 * 1024):.1f} MiB) in {destination}",
        file=sys.stderr,
    )
    return destination


def main() -> int:
    if any(argument in {"-h", "--help"} for argument in sys.argv[1:]):
        print(usage())
        return 0
    repository = Path(__file__).resolve().parent.parent
    try:
        prepare_inputs(repository)
        executables, library_directories = run_cargo_build(sys.argv[1:])
        executable = select_unique(executables, "Piper helper executable")
        library_dir = select_unique(
            (
                path
                for path in library_directories
                if (path / "libpiper.so").is_file()
            ),
            "Piper native library directory",
        )
        stage_companion(repository, executable, library_dir)
    except (OSError, StagingError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
