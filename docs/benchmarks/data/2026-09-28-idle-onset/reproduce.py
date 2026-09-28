#!/usr/bin/env python3
"""Measure release mixer samples in an isolated git archive; no audio device opens."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import tomllib


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[4])
    parser.add_argument("--revision", default="v1.13.0")
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory")
    parser.add_argument("--windows", action="store_true", help="also build and run Windows x64 via WSL interop")
    args = parser.parse_args()
    repo = args.repo.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = Path(tempfile.mkdtemp(prefix="omnivox-idle-onset-release-"))
    commit = subprocess.check_output(
        ["git", "rev-parse", "--verify", args.revision + "^{commit}"], cwd=repo, text=True
    ).strip()
    with tempfile.TemporaryFile() as archive:
        subprocess.run(["git", "archive", commit], cwd=repo, stdout=archive, check=True)
        archive.seek(0)
        subprocess.run(["tar", "-xf", "-", "-C", str(source)], stdin=archive, check=True)
    fixture = Path(__file__).with_name("idle_onset_repro.rs")
    shutil.copyfile(fixture, source / "omnivox-audio/src/idle_onset_repro.rs")
    with (source / "omnivox-audio/src/output.rs").open("a") as destination:
        destination.write('\n#[cfg(test)]\n#[path = "idle_onset_repro.rs"]\nmod idle_onset_repro;\n')
    toolchain = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    provenance = {
        "revision": args.revision,
        "commit": commit,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "host": platform.platform(),
        "source_directory": str(source),
        "fixture_sha256": sha256(fixture),
        "runner_sha256": sha256(Path(__file__)),
        "cargo_lock_sha256": sha256(source / "Cargo.lock"),
        "rustc": subprocess.check_output(["rustc", "+" + toolchain, "-vV"], cwd=source, text=True),
        "measurement": "44.1 kHz stereo Rodio dynamic mixer samples, no device or wall-clock onset measurement",
        "runs": [],
    }
    if args.windows:
        provenance["windows_version"] = subprocess.check_output(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", "[Environment]::OSVersion.VersionString"],
            text=True,
        ).strip()
    print(f"Source: {source}\nEvidence: {output}", flush=True)
    try:
        targets = [("host", None)]
        if args.windows:
            targets.append(("windows", "x86_64-pc-windows-gnu"))
        for label, target in targets:
            command = ["cargo", "+" + toolchain, "test", "--locked", "-p", "omnivox-audio", "--lib", "--no-run", "--message-format=json"]
            if target:
                command.extend(["--target", target])
            with (output / f"{label}-build.jsonl").open("w") as stdout, (output / f"{label}-build.log").open("w") as stderr:
                subprocess.run(command, cwd=source, stdout=stdout, stderr=stderr, check=True, timeout=600)
            artifacts = [json.loads(line) for line in (output / f"{label}-build.jsonl").read_text().splitlines()]
            executable = next(Path(item["executable"]) for item in artifacts
                              if item.get("reason") == "compiler-artifact" and item.get("executable")
                              and item["target"]["name"] == "omnivox_audio")
            test_command = [str(executable), "idle_onset_repro", "--nocapture", "--test-threads=1"]
            with (output / f"{label}-tests.log").open("w") as log:
                result = subprocess.run(test_command, cwd=source, stdout=log, stderr=subprocess.STDOUT, timeout=60)
            provenance["runs"].append({
                "label": label, "target": target, "build_command": command,
                "test_command": test_command, "test_executable_sha256": sha256(executable),
                "exit_code": result.returncode,
            })
            result.check_returncode()
            print(f"{label}: diagnostic assertions passed", flush=True)
    finally:
        provenance["finished_utc"] = datetime.now(timezone.utc).isoformat()
        (output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")


if __name__ == "__main__":
    main()
