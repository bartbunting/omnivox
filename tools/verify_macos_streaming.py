#!/usr/bin/env python3
"""Run existing full-server probes on two independent, simultaneous Mac lanes."""

from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import platform
import re
import subprocess
import sys


def main() -> None:
    server, probe_log, output = map(Path, sys.argv[1:])
    server = server.resolve()
    output.mkdir(parents=True, exist_ok=False)
    voices = re.findall(r"^VOICE (\S+) ", probe_log.read_text(), re.MULTILINE)
    if not voices or "PASS native streaming" not in probe_log.read_text():
        raise RuntimeError("the native adapter probe supplied no accepted exact voice")
    tools = Path(__file__).resolve().parent
    provenance = {
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "binary_sha256": hashlib.sha256(server.read_bytes()).hexdigest(),
        "host": platform.platform(),
        "architecture": platform.machine(),
        "voice": voices[0],
        "audio_output": "null",
    }
    (output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")

    def lane(name: str) -> None:
        common = [str(server), "--engine", "macos", "--expected-engine-id", "macos",
                  "--voice-id", voices[0], "--timeout", "30"]
        commands = [
            ("benchmark", [sys.executable, str(tools / "benchmark_server.py"), *common,
                           "--null-audio", "--mode", "warm", "--iterations", "2", "--warmups", "1"]),
            ("stress", [sys.executable, str(tools / "stress_server.py"), *common,
                        "--server-arg=--audio-output", "--server-arg=null",
                        "--iterations", "6", "--stop-every", "2"]),
        ]
        for kind, command in commands:
            command += ["--json-output", str(output / f"{name}-{kind}.json")]
            with (output / f"{name}-{kind}.log").open("w") as log:
                subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
        print(f"PASS {name}: all six benchmark workloads, replacement and hard-stop stress", flush=True)

    # Each harness owns and retires its own server; a stop in one process must
    # not prevent the other process using Apple's speech service.
    with ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(lane, ("main", "notification")))


if __name__ == "__main__":
    main()
