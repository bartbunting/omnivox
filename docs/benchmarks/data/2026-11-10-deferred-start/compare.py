#!/usr/bin/env python3
"""Sequential matched client comparison; alternate order and retain every run."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument("exe", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
rows = []
for pair in range(11):
    variants = ("legacy", "deferred") if pair % 2 == 0 else ("deferred", "legacy")
    for variant in variants:
        label = f"pair-{pair:02d}-{variant}"
        command = [sys.executable, str(Path(__file__).with_name("emacs.py")),
                   str(args.exe.resolve()), str(root / label)]
        if variant == "legacy":
            command.append("--legacy")
        if pair >= 9:
            command.append("--profile")
        result = subprocess.run(command, capture_output=True, timeout=150)
        (root / f"{label}.log").write_bytes(result.stdout + result.stderr)
        row = {"pair": pair, "variant": variant, "warmup": pair == 0,
               "profiled": pair >= 9, "command": command, "exit": result.returncode}
        rows.append(row)
        (root / "runs.json").write_text(json.dumps(rows, indent=2) + "\n")
        print(label, result.returncode, flush=True)
        if result.returncode:
            sys.exit(result.returncode)
