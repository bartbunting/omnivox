#!/usr/bin/env python3
"""Check managed Flite startup and SLT enablement in owned helpers, without playback."""
import argparse
import base64
import json
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from stress_helper import HelperSession


def verify(helper, path, version, enabled):
    session = HelperSession([str(helper), "--voice-library", path])
    try:
        def send(request_id, kind, **fields):
            session.send({"protocol_version": version, "request_id": request_id, "type": kind, **fields})

        def receive(request_id, deadline):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError("Flite request exceeded its deadline")
            try:
                line = session.responses.get(timeout=remaining)
            except queue.Empty as error:
                raise RuntimeError("Flite response timed out") from error
            if line is None:
                raise RuntimeError(f"Flite exited: {session.stderr_lines[-3:]}")
            response = json.loads(line)
            assert response["protocol_version"] == version and response["request_id"] == request_id, response
            return response

        send(1, "hello", supported_protocol_versions=[version])
        assert receive(1, time.monotonic() + 10)["type"] == "hello"
        send(2, "describe")
        response = receive(2, time.monotonic() + 10)
        if enabled:
            descriptor = response["descriptor"]
            assert len(descriptor["voices"]) == 1 and descriptor["default_voice_id"] == "cmu_us_slt", descriptor
        else:
            assert response["type"] == "error" and response["code"] == "not_available", response
            assert "excluded by configuration" in response["message"], response
        fields = {"text": "Managed Flite selection works.", "settings": {"voice_id": "cmu_us_slt", "rate": 0.5, "pitch": 1.0, "volume": 1.0}}
        if version >= 2:
            fields["anchors"] = []
        send(3, "synthesize", **fields)
        size = 0
        sequence = 0
        started = False
        deadline = time.monotonic() + 20
        while True:
            response = receive(3, deadline)
            kind = response["type"]
            if kind == "synthesis_started":
                assert enabled and not started and response["actual_voice_id"] == "cmu_us_slt", response
                assert response["format"]["channels"] == 2
                started = True
            elif kind == "audio_chunk":
                assert enabled and started and response["chunk"]["sequence"] == sequence
                sequence += 1
                size += len(base64.b64decode(response["chunk"]["data_base64"], validate=True))
            elif kind == "markers":
                assert enabled and started
            elif kind == "synthesis_completed":
                assert enabled and size > 0 and size == response["frame_count"] * 4
                break
            elif kind == "error":
                assert not enabled and size == 0 and response["code"] == "not_available", response
                break
            else:
                raise RuntimeError(f"Unexpected Flite response: {response}")
        send(4, "ping")
        assert receive(4, time.monotonic() + 10)["type"] == "pong"
        send(5, "shutdown")
        assert receive(5, time.monotonic() + 10)["type"] == "shutting_down"
        assert session.process.wait(timeout=10) == 0
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
    with tempfile.TemporaryDirectory(prefix="omnivox-flite-library-wire-") as temporary:
        path = Path(temporary) / "generation.json"
        native = str(path)
        if helper.suffix.lower() == ".exe" and sys.platform == "linux":
            native = subprocess.check_output(["wslpath", "-w", native], text=True).strip()
        for enabled in (False, True):
            path.write_text(json.dumps({"schema_version": 1,
                "target_id": "11111111-1111-4111-8111-111111111111", "profile_id": "22222222-2222-4222-8222-222222222222",
                "generation_id": "33333333-3333-4333-8333-333333333333", "disabled_physical_ids": [], "piper": None,
                "flite": {"builtin_slt": enabled, "files": []}}), encoding="utf-8")
            for version in range(1, 6):
                verify(helper, native, version, enabled)
                print(f"Flite protocol {version}: SLT {'enabled' if enabled else 'disabled'}, PCM gating and shutdown passed")


if __name__ == "__main__":
    main()
