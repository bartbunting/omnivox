#!/usr/bin/env python3
"""WSL-only silent smoke check of two packaged Windows device-output processes."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import wave


def windows_path(path):
    return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--program", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    program = args.program.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    native_temp = subprocess.check_output(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", "[IO.Path]::GetTempPath()"], text=True
    ).strip()
    temporary_parent = subprocess.check_output(["wslpath", "-u", native_temp], text=True).strip()
    results = {"started_utc": datetime.now(timezone.utc).isoformat(),
               "program": str(program), "program_sha256": hashlib.sha256(program.read_bytes()).hexdigest(),
               "audio": "8820 zero-valued stereo i16 frames at 44100 Hz; no acoustic measurement",
               "processes": []}
    processes = []
    try:
        with tempfile.TemporaryDirectory(prefix="omnivox-output-check-", dir=temporary_parent) as temporary:
            directory = Path(temporary)
            audio = directory / "zero.wav"
            with wave.open(str(audio), "wb") as output:
                output.setnchannels(2)
                output.setsampwidth(2)
                output.setframerate(44100)
                output.writeframes(bytes(8820 * 4))
            shutil.copyfile(audio, args.output / "zero.wav")
            for channel in ["left", "right"]:
                config = directory / channel
                config.mkdir()
                command = [str(program), "--config-dir", windows_path(config),
                           "--audio-output", "device", "--audio-target", channel,
                           "--play-wav", windows_path(audio)]
                processes.append((channel, command, subprocess.Popen(
                    command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)))
            for channel, command, process in processes:
                stdout, stderr = process.communicate(timeout=30)
                results["processes"].append({"channel": channel, "command": command,
                    "exit_code": process.returncode, "stdout": stdout, "stderr": stderr})
            assert all(result["exit_code"] == 0 for result in results["processes"]), results
            print("Both packaged Windows output processes played and drained zero-valued PCM successfully.")
    finally:
        for _, _, process in processes:
            if process.poll() is None:
                process.kill()
                process.communicate()
        results["finished_utc"] = datetime.now(timezone.utc).isoformat()
        (args.output / "results.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
