#!/usr/bin/env python3
"""Exercise private MBROLA helpers and two owned null-output speech workers."""
import argparse
import base64
import contextlib
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import shutil

from stress_helper import HelperSession, request
from verify_voice_library_startup import native, server

VOICE = "mbrola:v1/mb-en1/en1"
TEXT = "The quick brown fox jumps over the lazy dog. This sentence contains every letter of the alphabet and helps compare speech rates."


@contextlib.contextmanager
def helper(program):
    session = HelperSession([str(program)])
    try:
        session.send(request(1, "hello", supported_protocol_versions=[5]))
        assert session.receive(1)["type"] == "hello"
        session.send(request(2, "describe"))
        descriptor = session.receive(2)["descriptor"]
        assert descriptor["id"] == "mbrola" and descriptor["default_voice_id"] == VOICE, descriptor
        assert descriptor["capabilities"]["audio_output"] == "buffered_pcm"
        assert not any(v for k, v in descriptor["capabilities"]["markers"].items() if k != "requested_anchors")
        assert descriptor["capabilities"]["markers"]["requested_anchors"] == "none"
        yield session
    finally:
        session.stop()


def start(session, identifier, text=TEXT, rate=0.5, pitch=1.0, volume=1.0):
    session.send(request(identifier, "synthesize", text=text,
                         settings=dict(voice_id=VOICE, rate=rate, pitch=pitch, volume=volume), anchors=[]))


def finish(session, identifier):
    pcm = bytearray()
    started = False
    while True:
        response = session.receive(identifier)
        if response["type"] == "synthesis_started":
            assert response["actual_voice_id"] == VOICE
            assert response["format"]["sample_rate"] == 44100 and response["format"]["channels"] == 2
            started = True
        elif response["type"] == "audio_chunk":
            assert started
            pcm.extend(base64.b64decode(response["chunk"]["data_base64"], validate=True))
        elif response["type"] == "synthesis_completed":
            assert pcm and len(pcm) == response["frame_count"] * 4
            return bytes(pcm)
        else:
            assert response["type"] == "markers" and not response.get("markers"), response


def wait_pid(path):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if path.exists():
            parts = path.read_text().split()
            if len(parts) == 2:
                return tuple(map(int, parts))
        time.sleep(0.005)
    raise AssertionError("native fault child did not start")


def assert_gone(pid, windows, utility):
    if windows:
        subprocess.run([str(utility), "--wait", str(pid)], check=True, timeout=5)
    else:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            stat = Path(f"/proc/{pid}/stat")
            if not stat.exists() or stat.read_text().split(") ", 1)[1][0] == "Z":
                return
            time.sleep(0.005)
        raise AssertionError(f"native process {pid} survived helper retirement")


def faults(program, windows, scratch_dir=None):
    # Substitute only an owned temporary frontend, retaining the real MBROLA
    # database/runtime. The normal builder never stages this fault executable.
    with tempfile.TemporaryDirectory(prefix="omnivox-mbrola-fault-", dir=scratch_dir) as temporary:
        root = Path(temporary)
        bundle = root / "bundle"
        shutil.copytree(program.parent, bundle)
        manifest_path = bundle / "prototype.json"
        manifest = json.loads(manifest_path.read_text())
        frontend = bundle / manifest["frontend"]
        compiler = "x86_64-w64-mingw32-gcc" if windows else "cc"
        subprocess.run([compiler, "-O2", "-static", str(Path(__file__).parent / "mbrola/fault-child.c"),
                        "-o", str(frontend)], check=True, timeout=30)
        manifest["files"][manifest["frontend"]] = hashlib.sha256(frontend.read_bytes()).hexdigest()
        manifest_path.write_text(json.dumps(manifest))
        pid_file = root / "child.pid"
        old = os.environ.get("OMNIVOX_MBROLA_TEST_PID_FILE")
        old_wslenv = os.environ.get("WSLENV")
        os.environ["OMNIVOX_MBROLA_TEST_PID_FILE"] = native(pid_file, windows)
        os.environ["WSLENV"] = (old_wslenv or "") + ":OMNIVOX_MBROLA_TEST_PID_FILE"
        try:
            with helper(bundle / program.name) as session:
                for identifier in (10, 20, 30):
                    pid_file.unlink(missing_ok=True)
                    start(session, identifier, "Hang until cancelled.")
                    child, _parent = wait_pid(pid_file)
                    assert session.receive(identifier)["type"] == "synthesis_started"
                    before = time.monotonic()
                    session.send(request(identifier + 1, "cancel", target_request_id=identifier))
                    seen = set()
                    while len(seen) < 2:
                        response = session.receive_any(timeout=3)
                        assert response["type"] in ("cancel_accepted", "synthesis_cancelled"), response
                        seen.add(response["type"])
                    assert_gone(child, windows, frontend)
                    assert time.monotonic() - before < 3
                    start(session, identifier + 2, "Replacement works.")
                    finish(session, identifier + 2)
                database = bundle / manifest["database"]
                original = database.read_bytes()
                database.write_bytes(b"corrupt" + original[7:])
                try:
                    start(session, 35, "Changed data must fail before PCM.")
                    assert session.receive(35)["type"] == "synthesis_started"
                    try:
                        session.receive(35)
                    except RuntimeError as error:
                        assert "hash mismatch" in str(error), error
                    else:
                        raise AssertionError("changed database produced output")
                finally:
                    database.write_bytes(original)
                start(session, 36, "Valid data works after restoration.")
                finish(session, 36)
                extra = bundle / "espeak-ng-data/voices/mb-en1"
                extra.write_text("name unexpected alias\nlanguage en\n")
                try:
                    start(session, 37, "An added native alias must fail.")
                    assert session.receive(37)["type"] == "synthesis_started"
                    try:
                        session.receive(37)
                    except RuntimeError as error:
                        assert "unverified file" in str(error), error
                    else:
                        raise AssertionError("unverified alias produced output")
                finally:
                    extra.unlink()
                # Forced helper death must also retire its currently blocked child.
                pid_file.unlink(missing_ok=True)
                start(session, 40, "Hang while the helper is killed.")
                child, parent = wait_pid(pid_file)
                if windows:
                    subprocess.run([str(frontend), "--kill", str(parent)], check=True, timeout=5)
                else:
                    os.kill(parent, signal.SIGKILL)
                assert_gone(child, windows, frontend)
            with helper(bundle / program.name) as recovered:
                start(recovered, 50, "Recovery after forced retirement.")
                finish(recovered, 50)
        finally:
            for key, value in (("OMNIVOX_MBROLA_TEST_PID_FILE", old), ("WSLENV", old_wslenv)):
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("helper", type=Path)
    parser.add_argument("--server", type=Path)
    parser.add_argument("--espeak-data")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--scratch-dir", type=Path, help="Native Windows tests should use a directory on the Windows drive")
    parser.add_argument("--server-only", action="store_true", help="Check a newly staged server after native helper checks have passed")
    args = parser.parse_args()
    if args.server_only and not args.server:
        parser.error("--server-only requires --server")
    program = args.helper.resolve()
    windows = program.suffix == ".exe"
    report = dict(schema_version=1, platform="windows" if windows else "linux", corpus=TEXT,
                  helper_sha256=hashlib.sha256(program.read_bytes()).hexdigest(),
                  manifest_sha256=hashlib.sha256((program.parent / "prototype.json").read_bytes()).hexdigest(), rates=[])
    if not args.server_only:
        with helper(program) as session:
            for identifier, rate in enumerate((0.0, 0.5, 1.0, 1.5, 2.0), 10):
                before = time.monotonic()
                start(session, identifier, rate=rate)
                pcm = finish(session, identifier)
                seconds = len(pcm) / 4 / 44100
                report["rates"].append(dict(host_rate=rate, audio_seconds=seconds,
                                            wall_seconds=time.monotonic() - before,
                                            pcm_sha256=hashlib.sha256(pcm).hexdigest()))
            assert all(a["audio_seconds"] > b["audio_seconds"] for a, b in zip(report["rates"], report["rates"][1:]))
            start(session, 20, volume=0)
            assert not any(finish(session, 20))
            start(session, 21, pitch=1.4)
            assert hashlib.sha256(finish(session, 21)).hexdigest() != report["rates"][1]["pcm_sha256"]
        print("Native rate, pitch and mute checks passed", flush=True)
        faults(program, windows, args.scratch_dir)
        report["cancellation_replacement_and_forced_retirement"] = "passed"
    if args.server:
        report["server_sha256"] = hashlib.sha256(args.server.read_bytes()).hexdigest()
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
        environment["OMNIVOX_MBROLA_HELPER"] = native(program, windows)
        if args.espeak_data:
            environment["ESPEAK_NG_DATA"] = args.espeak_data
        forwarded = ["OMNIVOX_MBROLA_HELPER", "ESPEAK_NG_DATA"]
        environment["WSLENV"] = ":".join([entry for entry in environment.get("WSLENV", "").split(":")
                                            if entry and entry.split("/")[0] not in forwarded] + forwarded)
        with contextlib.ExitStack() as stack:
            lanes = [stack.enter_context(server(args.server.resolve(), ["--engine", "mbrola"], environment)) for _ in range(2)]
            def check_lane(lane):
                inventory = lane.control("inventory")
                descriptor = next(e for e in inventory["engines"] if e["id"] == "mbrola")
                assert descriptor["default_voice_id"] == VOICE
                base = next(e for e in inventory["engines"] if e["id"] == "espeak")["default_voice_id"]
                result = lane.control("preview", text="Exact MBROLA preview.", selector=dict(kind="exact", engine_id="mbrola", voice_id=VOICE))
                assert result["status"] == "completed" and result["realized"]["voice_id"] == VOICE, result
                result = lane.control("preview", text="Wrong voice must fail.", selector=dict(kind="exact", engine_id="mbrola", voice_id="mb-en1"))
                assert result["status"] != "completed" and result.get("realized") is None, result
                result = lane.control("preview_voice", text="Fallback remains available.",
                                      preferences=[dict(kind="exact", engine_id="mbrola", voice_id="missing"),
                                                   dict(kind="exact", engine_id="espeak", voice_id=base)],
                                      disabled_engine_ids=[], fallback_policy=dict(preferred_engines=[],
                                      allow_same_language_on_requested_engine=False, global_default=None, fallback_engines=[]))
                assert result["status"] == "completed" and result["realized"]["engine_id"] == "espeak", result
            with ThreadPoolExecutor(max_workers=2) as pool:
                list(pool.map(check_lane, lanes))
        report["two_lane_exact_preview_and_fallback"] = "passed"
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
