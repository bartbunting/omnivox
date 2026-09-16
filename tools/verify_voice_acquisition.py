#!/usr/bin/env python3
# Copyright (C) 2026 Emacsvox contributors
# SPDX-License-Identifier: GPL-2.0-or-later
"""Exercise HTTPS acquisition and disabled installation in a private voice root."""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    parser.add_argument("--catalogue", type=Path)
    args = parser.parse_args()
    server = args.server.resolve()
    if args.catalogue:
        catalogue = json.loads(args.catalogue.read_text())
    else:
        lock = json.loads((Path(__file__).resolve().parents[1] / "omnivox-piper-sys/test-model.json").read_text())
        # Same exact upstream fixture as the existing synthesis acceptance test.
        revision = "39ab474be869e9181350af6a65e4953eef67aaa0"
        base = f"https://huggingface.co/rhasspy/piper-voices/resolve/{revision}/en/en_US/kristin/medium/"
        files = [("model", "en_US-kristin-medium.onnx", 63531379, "5849957f929cbf720c258f8458692d6103fff2f0e3d3b19c8259474bb06a18d4"),
                 ("config", "en_US-kristin-medium.onnx.json", 4968, "5681426d4aead22195de70531eeeeddb46493cfaffc5764b2ea3db73428b651c"),
                 ("model_card", "MODEL_CARD", 479, "8c181b1d5f7d8152b914faab941fcbe6b8f0495e63e5189d6f27b8c68c8393fc")]
        assert revision in json.dumps(lock), "Update the acquisition fixture with the reviewed model lock"
        catalogue = {"schema_version": 1, "revision": "ci-only", "entries": [{
            "id": "piper-acquisition-fixture", "provider": "piper", "name": "Kristin CI fixture", "language": "en-US",
            "description": "CI-only acquisition fixture; not a shipped voice catalogue", "source": "https://huggingface.co/rhasspy/piper-voices",
            "source_revision": revision, "licence": "CI-only approval; see model card", "licence_url": base + "MODEL_CARD",
            "files": [dict(role=role, url=base+name, bytes=size, sha256=digest) for role, name, size, digest in files],
            "voices": [{"physical_id": "piper:v1/c/piper-acquisition-fixture/0", "name": "Kristin", "speaker_index": 0}]}]}
    root = Path(tempfile.mkdtemp(prefix="omnivox-acquire-"))
    print(f"Retained private acquisition root: {root}", flush=True)
    environment = {key: value for key, value in os.environ.items() if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
    environment["OMNIVOX_VOICE_ROOT"] = str(root)

    def inspect():
        result = subprocess.run([str(server), "--voice-library-service"], env=environment,
                                input='{"request_id":1,"command":"inspect"}\n', capture_output=True, text=True, timeout=30, check=True)
        return json.loads(result.stdout.removeprefix("OMNIVOX-LOCAL "))

    def acquire(document, entry, cancel=False):
        events = queue.Queue()
        with subprocess.Popen([str(server), "--voice-library-acquire"], env=environment, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as process:
            def read():
                for line in process.stdout:
                    events.put(json.loads(line.removeprefix("OMNIVOX-LOCAL ")))
                events.put(None)
            reader = threading.Thread(target=read, daemon=True)
            reader.start()
            process.stdin.write(json.dumps(dict(request_id=1, command="acquire", voice=entry, plan_json=json.dumps(document))) + "\n")
            process.stdin.flush()
            last = None
            try:
                while True:
                    event = events.get(timeout=180)
                    if event is None:
                        break
                    last = event
                    if cancel and not process.stdin.closed:
                        process.stdin.close()
                assert process.wait(timeout=15) == 0, process.stderr.read()
            finally:
                if not process.stdin.closed:
                    process.stdin.close()
                process.wait(timeout=150)
                reader.join(timeout=5)
            return last

    before = inspect()
    entry = catalogue["entries"][0]["id"]
    cancelled = acquire(catalogue, entry, cancel=True)
    assert cancelled["progress"]["state"] == "stopped-needs-inspection", cancelled
    assert inspect()["sha256"] == before["sha256"]
    broken = json.loads(json.dumps(catalogue))
    # Reject Content-Length disagreement before native validation or registration.
    broken["entries"][0]["files"][0]["bytes"] += 1
    failed = acquire(broken, entry)
    assert failed["progress"]["state"] == "failed", failed
    assert inspect()["sha256"] == before["sha256"]
    for entry in catalogue["entries"]:
        result = acquire(catalogue, entry["id"])
        assert result["progress"]["state"] == "installed-disabled", result
    after = inspect()
    assert len(after["index"]["voices"]) == sum(len(e["voices"]) for e in catalogue["entries"])
    assert all(not voice["enabled"] for voice in after["index"]["voices"])
    assert all(package["ownership"] == "managed" for package in after["index"]["packages"])
    assert before["active"] == after["active"] is None
    duplicate = acquire(catalogue, catalogue["entries"][0]["id"])
    assert duplicate["type"] == "error" and "already installed" in duplicate["message"], duplicate
    assert inspect()["sha256"] == after["sha256"]
    print("PASS: cancellation, wrong size, verified downloads, native validation, disabled installation and duplicate rejection")


if __name__ == "__main__":
    main()
