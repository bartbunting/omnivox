#!/usr/bin/env python3
"""Check owned local speech with a file earcon, retaining remote restrictions.

Run against a complete staged server, for example:
python3 omnivox-cli/tests/check_local_earcons.py target/debug/omnivox
All output uses null audio. Supports native Windows through WSL as well.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import tempfile
import threading
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    args = parser.parse_args()
    server = args.server.resolve()
    wsl = os.name != "nt" and server.suffix.lower() == ".exe"
    parent = None
    if wsl:
        native_temp = subprocess.check_output(["cmd.exe", "/d", "/c", "echo", "%TEMP%"],
                                              cwd=server.parent, text=True).strip()
        parent = subprocess.check_output(["wslpath", "-u", native_temp], text=True).strip()
    root = Path(tempfile.mkdtemp(prefix="omnivox-local-earcon-", dir=parent))

    def native(path):
        return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip() if wsl else str(path)

    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
    environment.update(OMNIVOX_VOICE_ROOT=native(root), OMNIVOX_AUDIO_OUTPUT="null", OMNIVOX_ENGINE="espeak")
    forwarded = ["OMNIVOX_VOICE_ROOT", "OMNIVOX_AUDIO_OUTPUT", "OMNIVOX_ENGINE", "OMNIVOX_REMOTE_WORKER"]
    environment["WSLENV"] = ":".join(
        [entry for entry in environment.get("WSLENV", "").split(":")
         if entry and entry.split("/", 1)[0] not in forwarded] + forwarded)
    path = root / "local-warning.wav"
    with wave.open(str(path), "wb") as output:
        output.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
        output.writeframes(b"\x00\x00" * 1600)

    def check(owned):
        env = dict(environment)
        if not owned:
            env["OMNIVOX_REMOTE_WORKER"] = "1"
        with tempfile.TemporaryFile() as diagnostics:
            process = subprocess.Popen([str(server)] + (["--voice-library-owner"] if owned else []),
                                       env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=diagnostics)
            messages = queue.Queue()

            def read():
                for line in iter(process.stdout.readline, b""):
                    messages.put(line)
                messages.put(None)

            reader = threading.Thread(target=read, daemon=True)
            reader.start()
            try:
                if not owned:
                    process.stdin.write(b"START\n")
                for identifier, with_file in [(1, False), (2, True)]:
                    document = {"protocol_version": 3, "generation": identifier,
                                "dispatch_id": identifier, "delivery_policy": "ordered",
                                "spans": [{"id": 1, "text": "Beginning of buffer"}], "actions": []}
                    if with_file:
                        document["actions"] = [{"id": "warning", "position": {
                            "position": "span_boundary", "span_id": 1, "affinity": "before"},
                            "lifecycle_anchor": "object", "type": "audio", "path": native(path),
                            "mode": "overlay", "volume": 1.0, "pan": 0.0, "effect_bus": "dry"}]
                    payload = base64.b64encode(json.dumps(document).encode())
                    process.stdin.write(b"emacsvox_timeline {" + payload + b"}\n")
                    process.stdin.flush()
                    prefix = f"__EMACSVOX_TRACKED__ {identifier} ".encode()
                    while True:
                        line = messages.get(timeout=30)
                        assert line is not None, "speech worker exited before completion"
                        if line.startswith(prefix):
                            status = line[len(prefix):].strip().decode()
                            expected = "completed" if owned or not with_file else "failed"
                            assert status == expected, ("local" if owned else "remote", with_file, status, expected)
                            break
                print(f"{'Local owner' if owned else 'Remote worker'}: speech and file-earcon policy passed", flush=True)
            finally:
                process.stdin.close()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                    raise
                reader.join(timeout=5)
                process.stdout.close()
                assert not reader.is_alive(), "speech reader did not retire"
                assert process.returncode == 0, "speech worker cleanup failed"

    try:
        check(True)
        check(False)
    finally:
        shutil.rmtree(root)


if __name__ == "__main__":
    main()
