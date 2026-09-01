#!/usr/bin/env python3
"""Build and atomically stage the dynamically loaded RHVoice helper."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


class StagingError(RuntimeError):
    """A Cargo output cannot form the RHVoice companion directory."""


def usage() -> str:
    return (
        "usage: python tools/build_rhvoice.py [cargo build arguments]\n\n"
        "Builds omnivox-rhvoice-helper with locked dependencies and stages "
        "it in an isolated rhvoice/ directory beside the Cargo profile output.\n\n"
        "examples:\n"
        "  python3 tools/build_rhvoice.py --release\n"
        "  python3 tools/build_rhvoice.py --release --target aarch64-apple-darwin"
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


def build(arguments: list[str]) -> Path:
    for argument in arguments:
        if argument == "-p" or argument.startswith(("--package", "--message-format")):
            raise StagingError(
                "tools/build_rhvoice.py owns Cargo's package and message-format selection"
            )

    command = [
        "cargo",
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        "--package",
        "omnivox-rhvoice-helper",
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
        raise StagingError("failed to capture Cargo build messages")

    executables: set[Path] = set()
    for line in process.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(line)
            continue
        render_cargo_message(message)
        target = message.get("target")
        executable = message.get("executable")
        if (
            message.get("reason") == "compiler-artifact"
            and isinstance(target, dict)
            and target.get("name") == "omnivox-rhvoice-helper"
            and isinstance(executable, str)
        ):
            executables.add(Path(executable).resolve())

    return_code = process.wait()
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)
    if len(executables) != 1:
        rendered = "\n  ".join(str(path) for path in sorted(executables)) or "<none>"
        raise StagingError(
            f"Cargo reported {len(executables)} RHVoice helper executables; "
            f"refusing to guess:\n  {rendered}"
        )
    executable = executables.pop()
    if not executable.is_file():
        raise StagingError(f"Cargo-reported helper is missing: {executable}")
    return executable


def stage(executable: Path) -> Path:
    profile_directory = executable.parent
    destination = profile_directory / "rhvoice"
    temporary = Path(
        tempfile.mkdtemp(
            prefix=f".{destination.name}.tmp-{os.getpid()}-",
            dir=profile_directory,
        )
    )
    try:
        shutil.copy2(executable, temporary / executable.name)
        if destination.exists():
            if not destination.is_dir():
                raise StagingError(
                    f"RHVoice companion destination is not a directory: {destination}"
                )
            shutil.rmtree(destination)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    print(f"Staged RHVoice helper in {destination}", file=sys.stderr)
    return destination


def main() -> int:
    if any(argument in {"-h", "--help"} for argument in sys.argv[1:]):
        print(usage())
        return 0
    try:
        stage(build(sys.argv[1:]))
    except (OSError, StagingError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
