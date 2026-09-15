#!/usr/bin/env python3
"""Verify model/speaker routing and failure isolation in owned Piper helpers.

Uses the tiny owned fixture graphs, never plays audio, and preserves installed
voices and live speech sessions. Pass a freshly staged native helper executable.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import queue
import struct
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from stress_helper import HelperSession


def native_path(path, windows):
    path = str(path.resolve())
    if windows and sys.platform == "linux":
        return subprocess.check_output(["wslpath", "-w", path], text=True).strip()
    return path


def receive(session, version, request_id, deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise RuntimeError("Piper request exceeded its deadline")
    try:
        line = session.responses.get(timeout=remaining)
    except queue.Empty as error:
        raise RuntimeError("Piper response timed out") from error
    if line is None:
        raise RuntimeError(f"Piper exited: {session.stderr_lines[-3:]}")
    response = json.loads(line)
    assert response["protocol_version"] == version, response
    assert response["request_id"] == request_id, response
    return response


def verify(helper, version, library, expected):
    session = HelperSession([str(helper), "--voice-library", library])
    try:
        def send(request_id, kind, **fields):
            session.send({"protocol_version": version, "request_id": request_id, "type": kind, **fields})

        send(1, "hello", supported_protocol_versions=[version])
        assert receive(session, version, 1, time.monotonic() + 30)["type"] == "hello"
        send(2, "describe")
        description = receive(session, version, 2, time.monotonic() + 10)
        assert len(description["descriptor"]["voices"]) == 6, description
        # Both buffered (v1-v4) and streaming (v5) paths use exactly one worker.
        for request_id, (voice, sample) in enumerate(expected, 3):
            fields = {"text": "a", "settings": {"voice_id": voice, "rate": 0.5, "pitch": 1.0, "volume": 1.0}}
            if version >= 2:
                fields["anchors"] = []
            send(request_id, "synthesize", **fields)
            samples = []
            sequence = 0
            started = False
            deadline = time.monotonic() + 20
            while True:
                response = receive(session, version, request_id, deadline)
                kind = response["type"]
                if kind == "synthesis_started":
                    assert not started
                    started = True
                    assert response["actual_voice_id"] == voice, response
                    assert response["format"]["channels"] == 2
                elif kind == "audio_chunk":
                    assert started and response["chunk"]["sequence"] == sequence
                    sequence += 1
                    raw = base64.b64decode(response["chunk"]["data_base64"], validate=True)
                    samples.extend(struct.unpack(f"<{len(raw) // 2}h", raw))
                elif kind == "error":
                    assert sample is None and not samples, response
                    assert response["code"] == "voice_not_found" and not response["retryable"], response
                    break
                elif kind == "synthesis_completed":
                    assert sample is not None and started and samples, response
                    assert response["frame_count"] * 2 == len(samples)
                    assert all(abs(value - sample) <= 1 for value in samples), samples[:8]
                    break
                else:
                    raise RuntimeError(f"Unexpected Piper response: {response}")
        send(100, "ping")
        assert receive(session, version, 100, time.monotonic() + 10)["type"] == "pong"
        send(101, "shutdown")
        assert receive(session, version, 101, time.monotonic() + 10)["type"] == "shutting_down"
        assert session.process.wait(timeout=10) == 0
        print(f"Piper protocol {version}: model/speaker PCM, bad-model isolation and shutdown passed")
    finally:
        session.stop()
        session.process.stdin.close()
        session.stdout_thread.join(timeout=5)
        session.stderr_thread.join(timeout=5)
        session.process.stdout.close()
        session.process.stderr.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("helper", type=Path)
    args = parser.parse_args()
    helper = args.helper.resolve()
    windows = helper.suffix.lower() == ".exe"
    source = Path(__file__).resolve().parent.parent / "test-fixtures/piper-speakers"
    with tempfile.TemporaryDirectory(prefix="omnivox-piper-library-wire-") as temporary:
        root = Path(temporary)
        (root / "bad.onnx").write_bytes(b"not an ONNX model")

        def asset(path):
            data = path.read_bytes()
            return {"path": native_path(path, windows), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}

        models = []
        for key, path in [("alpha", source / "alpha.onnx"), ("beta", source / "beta.onnx"), ("z-bad", root / "bad.onnx")]:
            models.append({"identity": {"catalogue_key": key}, "model": asset(path), "config": asset(source / "config.json"),
                           "voices": [{"physical_id": f"piper:v1/c/{key}/{speaker}", "speaker_index": speaker,
                                       "display_name": f"{key} {speaker}", "language": None} for speaker in (0, 1)]})
        library = root / "generation.json"
        library.write_text(json.dumps({"schema_version": 1,
            "target_id": "11111111-1111-4111-8111-111111111111", "profile_id": "22222222-2222-4222-8222-222222222222",
            "generation_id": "33333333-3333-4333-8333-333333333333", "disabled_physical_ids": [],
            "piper": {"models": models}, "flite": None}), encoding="utf-8")
        expected = [(f"piper:v1/c/{name}/{speaker}", sample) for name, speaker, sample in
                    [("alpha", 0, 4096), ("alpha", 1, 8192), ("beta", 1, 16384),
                     ("z-bad", 0, None), ("z-bad", 1, None), ("beta", 0, 12288), ("alpha", 0, 4096)]]
        for version in range(1, 6):
            verify(helper, version, native_path(library, windows), expected)


if __name__ == "__main__":
    main()
