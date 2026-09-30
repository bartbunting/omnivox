#!/usr/bin/env python3
"""Capture an isolated Windows Omnivox process tree; plays diagnostic signals."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

HERE = Path(__file__).resolve().parent
CASES = {
    "mono22050": ("helper-22050-1-streaming-fast", 22050, 1, "streaming", False),
    "mono16000": ("helper-16000-1-streaming-fast", 16000, 1, "streaming", False),
    "lead": ("helper-22050-1-streaming-lead", 22050, 1, "streaming", True),
    "buffered": ("helper-22050-1-buffered-fast", 22050, 1, "buffered", False),
    "canonical": ("helper-44100-2-streaming-fast", 44100, 2, "streaming", False),
}


def native(path):
    return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()


def read_log(path):
    try:
        return path.read_text()
    except OSError:
        return ""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--omnivox", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=CASES, action="append")
    parser.add_argument("--count", type=int, default=10)
    args = parser.parse_args()
    if not 1 <= args.count <= 100:
        parser.error("--count must be between 1 and 100")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    with (output / "compile.log").open("w") as log:
        subprocess.run(["x86_64-w64-mingw32-g++", "-std=c++17", "-O2", "-static", "-municode",
                        str(HERE / "capture.cpp"), "-o", str(output / "capture.exe"),
                        "-lole32", "-luuid"], stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["/mnt/c/Windows/Microsoft.NET/Framework64/v4.0.30319/csc.exe",
                        "/nologo", "/target:exe", "/reference:System.Web.Extensions.dll",
                        "/out:" + native(output / "helper.exe"), native(HERE / "helper.cs")],
                       cwd=output, stdout=log, stderr=subprocess.STDOUT, check=True)
    env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "USER") if key in os.environ}
    env.update(OMNIVOX_VOICE_ROOT=native(output / "voices"), RUST_LOG="info",
               WSLENV="OMNIVOX_VOICE_ROOT:RUST_LOG")
    for case in args.case or CASES:
        label, rate, channels, mode, lead = CASES[case]
        config = output / label
        (config / "helpers.d").mkdir(parents=True)
        shipped = ["espeak", "winrt", "macos", "piper", "rhvoice", "flite", "rutts",
                   "tgspeechbox", "eloquence", "dectalk"]
        (config / "config.json").write_text(json.dumps({
            "schema": 2, "audio": {"backend": "device", "target": "both"},
            "engine_overrides": {name: {"enabled": False} for name in shipped},
            "routing": {"preferred_engine_ids": ["org.onset"]}}))
        descriptor = json.loads((HERE / "descriptor.json").read_text())
        descriptor["capabilities"]["audio_output"] = mode + "_pcm"
        (config / "descriptor.json").write_text(json.dumps(descriptor))
        pcm_name = f"chirp-{rate}-{channels}" + ("-lead" if lead else "") + ".pcm"
        shutil.copyfile(HERE / pcm_name, config / "input.pcm")
        options = ["--descriptor", native(config / "descriptor.json"),
                   "--pcm", native(config / "input.pcm"),
                   "--record", native(config / "argv.jsonl"),
                   "--record-synthesis", native(config / "synthesis.jsonl"),
                   "--sample-rate", str(rate), "--channels", str(channels),
                   "--chunk-frames", "256", "--chunk-delay-ms", "0"]
        (config / "helpers.d/onset.json").write_text(json.dumps({
            "schema": 1, "engine_id": "org.onset", "program": native(output / "helper.exe"),
            "arguments": options}))
        common = ["--config-dir", native(config), "--engine", "org.onset"]
        with (output / (label + ".dump.log")).open("wb") as log:
            subprocess.run([str(args.omnivox.resolve()), *common, "--audio-output", "null",
                            "--dump-wav", "test", native(output / (label + ".reference.wav")), "e"],
                           env=env, cwd=output, stdout=log, stderr=subprocess.STDOUT,
                           check=True, timeout=30)
        seconds = args.count + 8
        command = [str(output / "capture.exe"), native(output / (label + ".f32")), str(seconds),
                   native(args.omnivox.resolve()), *common, "--audio-output", "device"]
        with (output / (label + ".stdout")).open("wb") as out, \
                (output / (label + ".stderr")).open("wb") as err:
            child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=out,
                                     stderr=err, env=env, cwd=output)
            try:
                deadline = time.monotonic() + 6
                while "Ready to accept" not in read_log(output / (label + ".stderr")):
                    if child.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError(read_log(output / (label + ".stderr")))
                    time.sleep(.02)
                time.sleep(1.1)
                sent = []
                for _ in range(args.count):
                    sent.append(time.time())
                    child.stdin.write(b"l {e}\n")
                    child.stdin.flush()
                    time.sleep(1)
                child.stdin.close()
                result = child.wait(timeout=seconds + 5)
                if result:
                    raise RuntimeError(f"capture exited {result}")
            finally:
                if child.poll() is None:
                    child.kill()  # Recorder's job handle also retires only its test children.
                    child.wait()
        (output / (label + ".run.json")).write_text(json.dumps({
            "command": command, "returncode": result, "sent_unix": sent}, indent=2) + "\n")
        print(label, "captured", args.count, "utterances", flush=True)
    subprocess.run(["python3", str(HERE / "analyze.py"), "--directory", str(output)], check=True)


if __name__ == "__main__":
    main()
