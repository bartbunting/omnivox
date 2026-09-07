#!/usr/bin/env python3
"""Build Linux ECI/DECtalk interfaces; never copy user-installed runtimes."""

from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
NAMES = {f"omnivox-{engine}-helper": engine for engine in ("eloquence", "dectalk")}


def replace_file(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{destination.name}-", dir=destination.parent)
    os.close(fd)
    try:
        shutil.copy2(source, temporary)
        os.replace(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--target")
    args = parser.parse_args()
    target = args.target or os.environ.get("CARGO_BUILD_TARGET", "")
    if sys.platform != "linux" or (target and "-linux-" not in target):
        parser.error("Linux helpers require a Linux build target")
    command = ["cargo", "build", "--locked", "--package", "omnivox-linux-helpers",
               "--bins", "--message-format=json-render-diagnostics"]
    if args.release:
        command.append("--release")
    if args.target:
        command.extend(["--target", args.target])
    outputs = {}
    print("+ " + " ".join(command), file=sys.stderr)
    with subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, text=True) as process:
        assert process.stdout is not None
        for line in process.stdout:
            message = json.loads(line)
            if message.get("reason") == "compiler-message":
                sys.stderr.write(message["message"].get("rendered") or "")
            name = message.get("target", {}).get("name")
            if (message.get("reason") == "compiler-artifact" and name in NAMES
                    and message.get("executable")):
                outputs[name] = Path(message["executable"]).resolve()
        status = process.wait()
    if status:
        return status
    if set(outputs) != set(NAMES):
        raise RuntimeError("Cargo did not report both Linux helper executables")
    for name, executable in outputs.items():
        destination = executable.parent / NAMES[name]
        replace_file(executable, destination / name)
        replace_file(ROOT / "linux-helpers" / "COPYING", destination / "OMNIVOX-HELPER-COPYING")
        replace_file(ROOT / "linux-helpers" / "README.md", destination / "OMNIVOX-HELPER-README.md")
        print(f"Staged {name} in {destination}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
