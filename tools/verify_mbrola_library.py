#!/usr/bin/env python3
"""Check MBROLA downloads, enablement, independent voices and native validation.

All acquisition and speech use a retained private root and null audio output.
The caller explicitly supplies the reviewed catalogue and staged companion.
"""
import argparse
import base64
import contextlib
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time
import uuid

from stress_helper import HelperSession, request
from verify_voice_library_startup import native, server

EN1 = "mbrola:v1/mb-en1/en1"
US1 = "mbrola:v1/mb-us1/us1"


def synthesize(session, identifier, voice, text="focus"):
    before = time.monotonic()
    session.send(request(identifier, "synthesize", text=text,
                         settings=dict(voice_id=voice, rate=0.5, pitch=1.0, volume=1.0), anchors=[]))
    pcm = bytearray()
    while True:
        response = session.receive(identifier)
        if response["type"] == "synthesis_started":
            assert response["actual_voice_id"] == voice, response
        elif response["type"] == "audio_chunk":
            pcm.extend(base64.b64decode(response["chunk"]["data_base64"], validate=True))
        elif response["type"] == "synthesis_completed":
            assert pcm and len(pcm) == response["frame_count"] * 4, response
            return bytes(pcm), round((time.monotonic() - before) * 1000, 3)
        else:
            assert response["type"] == "markers" and not response.get("markers"), response


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--catalogue", type=Path, required=True)
    parser.add_argument("--scratch-dir", type=Path)
    parser.add_argument("--espeak-data")
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    program, helper = args.server.resolve(), args.helper.resolve()
    windows = program.suffix == ".exe"
    root = Path(tempfile.mkdtemp(prefix="mbrola-library-", dir=args.scratch_dir))
    print(f"Retained isolated test root: {root}", flush=True)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("OMNIVOX_", "LD_", "DYLD_")) and key != "ESPEAK_NG_DATA"}
    environment.update(OMNIVOX_VOICE_ROOT=native(root, windows), OMNIVOX_MBROLA_HELPER=native(helper, windows))
    if args.espeak_data:
        environment["ESPEAK_NG_DATA"] = args.espeak_data
    forwarded = ["OMNIVOX_VOICE_ROOT", "OMNIVOX_MBROLA_HELPER", "ESPEAK_NG_DATA"]
    environment["WSLENV"] = ":".join([entry for entry in environment.get("WSLENV", "").split(":")
                                       if entry and entry.split("/")[0] not in forwarded] + forwarded)
    catalogue = json.loads(args.catalogue.read_text())
    assert catalogue["schema_version"] == 2

    def service(command, **fields):
        result = subprocess.run([str(program), "--voice-library-service"], env=environment,
                                input=json.dumps(dict(request_id=1, command=command, **fields)) + "\n",
                                capture_output=True, text=True, timeout=45, check=True)
        reply = json.loads(result.stdout.removeprefix("OMNIVOX-LOCAL "))
        assert reply["type"] != "error", reply
        return reply

    def acquire(entry, cancel=False):
        events = queue.Queue()
        with subprocess.Popen([str(program), "--voice-library-acquire"], env=environment, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as process:
            def read():
                for line in process.stdout:
                    events.put(json.loads(line.removeprefix("OMNIVOX-LOCAL ")))
                events.put(None)
            reader = threading.Thread(target=read, daemon=True)
            reader.start()
            process.stdin.write(json.dumps(dict(request_id=1, command="acquire", voice=entry,
                                                 plan_json=json.dumps(catalogue))) + "\n")
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
            if last.get("progress", {}).get("state") == "failed":
                log = root / "acquisitions" / last["progress"]["operation_id"] / "validation.log"
                if log.exists():
                    print(log.read_text(errors="replace")[-12000:])
            return last

    host = service("host")
    assert "mbrola" in host["catalogue_providers"]
    before = service("inspect")
    cancelled = acquire("mbrola-us1", cancel=True)
    assert cancelled["progress"]["state"] == "stopped-needs-inspection", cancelled
    assert service("inspect")["sha256"] == before["sha256"]
    for entry in catalogue["entries"]:
        result = acquire(entry["id"])
        assert result["progress"]["state"] == "installed-disabled", result
        print(f"Downloaded and natively validated {entry['id']}", flush=True)
    installed = service("inspect")
    assert installed["active"] is None and installed["index"]["schema_version"] == 2
    assert len(installed["index"]["voices"]) == 4
    assert [voice["physical_id"] for voice in installed["index"]["voices"] if voice["enabled"]] == [EN1]
    assert len(list((root / "packages").glob("*/*/LICENSE"))) == 3
    assert len(list((root / "packages").glob("*/*/README"))) == 3
    duplicate = acquire("mbrola-us1")
    assert duplicate["type"] == "error" and "already installed" in duplicate["message"]

    def enable(voice, enabled):
        index = service("inspect")
        service("enable", engine="mbrola", voice=voice, enabled=enabled, expected_sha256=index["sha256"])

    def stage():
        index = service("inspect")
        return service("stage", generation=str(uuid.uuid4()), expected_sha256=index["sha256"], mbrola=True)

    for voice in installed["index"]["voices"]:
        enable(voice["physical_id"], True)
    candidate = stage()
    generation_path = candidate["path"]
    # Four real native loads and exact companion evidence, with no playback.
    validation_environment = {key: value for key, value in environment.items() if key != "ESPEAK_NG_DATA"}
    with subprocess.Popen([str(program), "--validate-voice-library", generation_path, "--mbrola-helper", native(helper, windows),
                           "--validation-report", native(root / "all-voices-validation.json", windows)],
                          env=validation_environment, stdin=subprocess.PIPE) as validation:
        # EOF requests cancellation; retain the input pipe until native cleanup.
        assert validation.wait(timeout=180) == 0
        validation.stdin.close()
    session = HelperSession([str(helper), "--voice-library", generation_path])
    measurements = []
    try:
        session.send(request(1, "hello", supported_protocol_versions=[5]))
        assert session.receive(1)["type"] == "hello"
        session.send(request(2, "describe"))
        descriptor = session.receive(2)["descriptor"]
        assert len(descriptor["voices"]) == 4
        order = [EN1, US1, "mbrola:v1/mb-us2/us2", "mbrola:v1/mb-us3/us3", EN1, US1]
        audio = []
        for identifier, voice in enumerate(order, 10):
            pcm, elapsed = synthesize(session, identifier, voice)
            audio.append(pcm)
            measurements.append(dict(voice=voice, synthesis_ms=elapsed, pcm_sha256=hashlib.sha256(pcm).hexdigest()))
        assert audio[0] == audio[4] and audio[1] == audio[5] and audio[0] != audio[1]
        terminated, _ = synthesize(session, 30, EN1, "focus\n")
        chopped, _ = synthesize(session, 31, EN1, "focu\n")
        assert audio[0] == terminated and audio[0] != chopped
    finally:
        session.stop()
    with contextlib.ExitStack() as stack:
        lanes = [stack.enter_context(server(program, ["--voice-library", generation_path], environment)) for _ in range(2)]
        def preview(pair):
            lane, voice = pair
            result = lane.control("preview", text="Different MBROLA voices can speak on each stream.",
                                  selector=dict(kind="exact", engine_id="mbrola", voice_id=voice))
            assert result["status"] == "completed" and result["realized"]["voice_id"] == voice, result
        with ThreadPoolExecutor(max_workers=2) as pool:
            list(pool.map(preview, zip(lanes, [EN1, US1])))
        # Changing desired state leaves already-started workers on their generation.
        enable(US1, False)
        preview((lanes[1], US1))
    disabled = stage()
    with server(program, ["--voice-library", disabled["path"]], environment) as lane:
        result = lane.control("preview", text="A disabled voice must not speak.", selector=dict(kind="exact", engine_id="mbrola", voice_id=US1))
        assert result["status"] != "completed" and result.get("realized") is None, result
        preview((lane, EN1))
    args.report.write_text(json.dumps(dict(schema_version=1, platform="windows" if windows else "linux", root=str(root),
        server_sha256=hashlib.sha256(program.read_bytes()).hexdigest(), helper_sha256=hashlib.sha256(helper.read_bytes()).hexdigest(),
        catalogue_sha256=hashlib.sha256(args.catalogue.read_bytes()).hexdigest(), cancellation=True, native_downloads=3,
        installed_disabled=True, en1_preserved=True, notices_retained=True, native_validation=True,
        per_request_switching=measurements, two_lane_distinct_voices=True, disabled_exact_preview=True,
        existing_workers_pinned=True, final_text_preserved=True), indent=2) + "\n")
    print("PASS: acquisition, validation, independent voices, two lanes, disabled voices and final text", flush=True)


if __name__ == "__main__":
    main()
