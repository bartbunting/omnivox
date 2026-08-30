#!/usr/bin/env python3
"""Build Omnivox and stage the matching eSpeak NG runtime data."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from typing import Iterable


# Keep this exact so a dependency upgrade cannot silently reuse notices that
# were reviewed for a different bundled eSpeak NG source snapshot.
ESPEAK_PACKAGE = "#espeak-rs-sys@0.1.9"
NOTICE_FILES = (
    ("espeak-ng/src/ucd-tools/COPYING", "eSpeak-NG-GPL-3.0.txt", True),
    ("espeak-ng/src/ucd-tools/COPYING.UCD", "Unicode-Data-License.txt", True),
    ("espeak-ng/src/compat/getopt.c", "NetBSD-getopt.c", True),
    ("build/_deps/sonic-git-src/LICENSE", "Sonic-Apache-2.0.txt", False),
)
PROJECT_LICENSE_FILES = (
    ("LICENSE", "LICENSE"),
    ("docs/LICENSING.md", "LICENSING.md"),
)


def usage() -> str:
    return (
        "usage: python tools/build.py [cargo build arguments]\n\n"
        "Runs `cargo build --locked`, stages generated espeak-ng-data and "
        "project and third-party license notices beside the resulting "
        "executable, and rejects ambiguous dependency outputs.\n\n"
        "examples:\n"
        "  python tools/build.py --release\n"
        "  python tools/build.py --release --target aarch64-apple-darwin"
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


def run_cargo_build(arguments: list[str]) -> list[Path]:
    command = [
        "cargo",
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        *arguments,
    ]
    print("+ " + " ".join(command), file=sys.stderr)
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if process.stdout is None:
        raise RuntimeError("failed to capture Cargo build messages")

    outputs: list[Path] = []
    for line in process.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(line)
            continue
        render_cargo_message(message)
        if (
            message.get("reason") == "build-script-executed"
            and ESPEAK_PACKAGE in str(message.get("package_id", ""))
        ):
            out_dir = message.get("out_dir")
            if isinstance(out_dir, str):
                outputs.append(Path(out_dir).resolve())

    return_code = process.wait()
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)
    return outputs


def file_manifest(directory: Path) -> list[tuple[str, str]]:
    manifest: list[tuple[str, str]] = []
    for path in sorted(item for item in directory.rglob("*") if item.is_file()):
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest.append((path.relative_to(directory).as_posix(), digest))
    return manifest


def select_output(outputs: Iterable[Path]) -> Path:
    candidates = sorted(
        {
            output
            for output in outputs
            if (output / "share" / "espeak-ng-data" / "phontab").is_file()
        }
    )
    if not candidates:
        raise RuntimeError(
            "Cargo completed without reporting a usable espeak-rs-sys data output"
        )

    reference = file_manifest(candidates[0] / "share" / "espeak-ng-data")
    for candidate in candidates[1:]:
        if file_manifest(candidate / "share" / "espeak-ng-data") != reference:
            rendered = "\n  ".join(str(path) for path in candidates)
            raise RuntimeError(
                "Cargo reported non-identical eSpeak NG data outputs; refusing "
                f"to guess which one to package:\n  {rendered}"
            )
    return candidates[0]


def replace_directory(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(
        tempfile.mkdtemp(
            prefix=f".{destination.name}.tmp-{os.getpid()}-",
            dir=destination.parent,
        )
    )
    try:
        shutil.copytree(source, temporary, dirs_exist_ok=True)
        if destination.exists():
            shutil.rmtree(destination)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def stage_notices(repository: Path, output: Path, profile_dir: Path) -> None:
    temporary = Path(
        tempfile.mkdtemp(
            prefix=f".third-party-licenses.tmp-{os.getpid()}-",
            dir=profile_dir,
        )
    )
    destination = profile_dir / "third-party-licenses"
    try:
        shutil.copy2(
            repository
            / "omnivox-tts"
            / "runtime-assets"
            / "THIRD-PARTY-NOTICES.md",
            temporary / "THIRD-PARTY-NOTICES.md",
        )
        shutil.copy2(repository / "Cargo.lock", temporary / "omnivox-Cargo.lock")

        for relative_source, filename, required in NOTICE_FILES:
            source = output / relative_source
            if source.is_file():
                shutil.copy2(source, temporary / filename)
            elif required:
                raise RuntimeError(f"required eSpeak NG notice is missing: {source}")

        if destination.exists():
            shutil.rmtree(destination)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def stage_project_licenses(repository: Path, profile_dir: Path) -> None:
    for source_name, destination_name in PROJECT_LICENSE_FILES:
        source = repository / source_name
        if not source.is_file():
            raise RuntimeError(f"required project license file is missing: {source}")
        shutil.copy2(source, profile_dir / destination_name)


def stage_runtime_assets(repository: Path, output: Path) -> Path:
    if output.name != "out" or output.parent.parent.name != "build":
        raise RuntimeError(f"unexpected espeak-rs-sys OUT_DIR layout: {output}")
    profile_dir = output.parent.parent.parent
    data_source = output / "share" / "espeak-ng-data"
    data_destination = profile_dir / "espeak-ng-data"

    replace_directory(data_source, data_destination)
    stage_notices(repository, output, profile_dir)
    stage_project_licenses(repository, profile_dir)

    file_count = sum(path.is_file() for path in data_destination.rglob("*"))
    byte_count = sum(
        path.stat().st_size for path in data_destination.rglob("*") if path.is_file()
    )
    print(
        f"Staged {file_count} eSpeak NG data files "
        f"({byte_count / (1024 * 1024):.1f} MiB) in {profile_dir}",
        file=sys.stderr,
    )
    return profile_dir


def main() -> int:
    if any(argument in {"-h", "--help"} for argument in sys.argv[1:]):
        print(usage())
        return 0
    if any(argument.startswith("--message-format") for argument in sys.argv[1:]):
        print("error: tools/build.py owns Cargo's --message-format", file=sys.stderr)
        return 2

    repository = Path(__file__).resolve().parent.parent
    try:
        output = select_output(run_cargo_build(sys.argv[1:]))
        stage_runtime_assets(repository, output)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
