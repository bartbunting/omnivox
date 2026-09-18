#!/usr/bin/env python3
"""Exercise staged library startup and development status in owned silent servers."""
import argparse
import base64
import collections
import contextlib
import hashlib
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import tempfile
import threading
import time


def native(path, windows):
    if windows and sys.platform == "linux":
        return subprocess.check_output(["wslpath", "-w", str(path)], text=True, timeout=10).strip()
    return str(path)


class Server:
    def __init__(self, program, arguments, environment):
        engine = [] if "--engine" in arguments else ["--engine", "espeak"]
        self.process = subprocess.Popen(
            [str(program), "--audio-output", "null", *engine, *arguments],
            env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace", start_new_session=os.name == "posix",
        )
        self.lines = queue.Queue()
        self.errors = collections.deque(maxlen=20)
        def output():
            for line in self.process.stdout:
                self.lines.put(line)
            self.lines.put(None)
        def errors():
            for line in self.process.stderr:
                self.errors.append(line.rstrip())
        self.readers = [threading.Thread(target=output), threading.Thread(target=errors)]
        for thread in self.readers:
            thread.start()
        self.sequence = 0

    def control(self, kind, **fields):
        self.sequence += 1
        payload = {"protocol_version": 1, "request_id": self.sequence, "type": kind, **fields}
        encoded = base64.b64encode(json.dumps(payload).encode()).decode()
        self.process.stdin.write("omnivox_control " + encoded + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 75
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty as error:
                raise AssertionError(f"server response timed out: {list(self.errors)}") from error
            if line is None:
                raise AssertionError(f"server exited before responding: {list(self.errors)}")
            if line.startswith("__OMNIVOX_CONTROL__ "):
                response = json.loads(base64.b64decode(line.split(" ", 1)[1]))
                assert response["request_id"] == self.sequence, response
                return response
        raise AssertionError("server response deadline expired")

    def close(self):
        with contextlib.suppress(BrokenPipeError):
            self.process.stdin.close()
        try:
            self.process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            if os.name == "posix":
                os.killpg(self.process.pid, signal.SIGKILL)
            else:
                self.process.kill()
            self.process.wait(timeout=10)
            raise AssertionError("server did not retire after stdin closed")
        finally:
            for thread in self.readers:
                thread.join(timeout=5)
            for pipe in [self.process.stdout, self.process.stderr]:
                pipe.close()


@contextlib.contextmanager
def server(program, arguments, environment):
    instance = Server(program, arguments, environment)
    try:
        yield instance
    finally:
        instance.close()


def degraded_speech(session, provider, voice_id, reason=None):
    """Require truthful unavailability, exact-audition failure and real fallback PCM."""
    inventory = session.control("inventory")
    engine = next(engine for engine in inventory["engines"] if engine["id"] == provider)
    assert engine["availability"]["status"] == "unavailable", engine
    if reason:
        assert reason in engine["availability"]["reason"], engine
    status = session.control("voice_library_status_v1")
    assert not any(voice["engine_id"] == provider for voice in status["eligible_voices"]), status
    exact = {"kind": "exact", "engine_id": provider, "voice_id": voice_id}
    result = session.control("preview", text="Voice fallback.", selector=exact)
    assert result["status"] != "completed" and result.get("realized") is None, result
    result = session.control("preview_voice", text="Voice fallback.", preferences=[exact],
                             language="en", disabled_engine_ids=[], fallback_policy={
                                 "preferred_engines": [provider],
                                 "allow_same_language_on_requested_engine": True,
                                 "global_default": None, "fallback_engines": ["espeak"]})
    assert result["status"] == "completed" and result["realized"]["engine_id"] == "espeak", result
    return inventory, status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    parser.add_argument("--flite-helper", type=Path, required=True)
    parser.add_argument("--piper-helper", type=Path)
    args = parser.parse_args()
    program = args.server.resolve()
    windows = program.suffix.lower() == ".exe"
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="omnivox-library-startup-") as temporary:
        temporary = Path(temporary)
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
        if windows and os.environ.get("ESPEAK_NG_DATA"):
            environment["ESPEAK_NG_DATA"] = os.environ["ESPEAK_NG_DATA"]
        for name in ["PIPER", "FLITE", "RHVOICE", "RUTTS", "TGSPEECHBOX", "ELOQUENCE", "DECTALK"]:
            environment[f"OMNIVOX_{name}_HELPER"] = native(temporary / "absent-helper", windows)
        environment["OMNIVOX_FLITE_HELPER"] = native(args.flite_helper.resolve(), windows)
        if args.piper_helper:
            environment["OMNIVOX_PIPER_HELPER"] = native(args.piper_helper.resolve(), windows)
        if windows and sys.platform == "linux":
            forwarded = [key for key in environment if key.startswith("OMNIVOX_")]
            forwarded += ["OMNIVOX_VOICE_LIBRARY", "ESPEAK_NG_DATA"]
            retained = [entry for entry in environment.get("WSLENV", "").split(":")
                        if entry and entry.split("/")[0] not in forwarded]
            environment["WSLENV"] = ":".join(retained + forwarded)
        document = {"schema_version": 1,
                    "target_id": "11111111-1111-4111-8111-111111111111",
                    "profile_id": "22222222-2222-4222-8222-222222222222",
                    "generation_id": "33333333-3333-4333-8333-333333333333",
                    "disabled_physical_ids": [], "piper": {"models": []},
                    "flite": {"builtin_slt": False, "files": []}}
        path = temporary / "generation with spaces.json"
        def write():
            path.write_text(json.dumps(document), encoding="utf-8")
            return hashlib.sha256(path.read_bytes()).hexdigest()
        digest = write()
        arguments = ["--voice-library", native(path, windows)]
        with server(program, [], environment) as session:
            status = session.control("voice_library_status_v1")
            assert status["configuration"] is None, status
            inventory = session.control("inventory")
            espeak = next(engine for engine in inventory["engines"] if engine["id"] == "espeak")
            excluded = {"engine_id": "espeak", "voice_id": espeak["default_voice_id"]}

        document["disabled_physical_ids"] = [excluded]
        digest = write()
        # CLI wins over an invalid environment path; no silent legacy fallback.
        with server(program, arguments, {**environment, "OMNIVOX_VOICE_LIBRARY": native(temporary / "missing.json", windows)}) as session:
            status = session.control("voice_library_status_v1")
            inventory = session.control("inventory")
            assert status["configuration"]["sha256"] == digest, status
            assert status["inventory_generation"] == inventory["inventory_generation"], (status, inventory)
            assert excluded not in status["eligible_voices"], status
            for engine in inventory["engines"]:
                if engine["id"] == "espeak":
                    assert engine["default_voice_id"] != excluded["voice_id"], engine
                if engine["id"] in ("flite", "piper"):
                    assert engine["default_voice_id"] is None and not engine["voices"], engine
                    assert "excluded" in engine["availability"]["reason"], engine
            response = session.control("preview", text="a", selector={"kind": "exact", **excluded})
            assert response.get("realized") is None and response["status"] != "completed", response
            assert "voice_library_v1" in session.control("capabilities")["features"]
            result = session.control("set_routing_policy", routing_policy_generation=7,
                                     preferred_engine_ids=["espeak"], fallback_engine_ids=[], disabled_engine_ids=["espeak"])
            assert result["type"] == "routing_policy_applied", result
            status = session.control("voice_library_status_v1")
            inventory = session.control("inventory")
            assert status["inventory_generation"] == inventory["inventory_generation"], (status, inventory)
            assert not any(voice["engine_id"] == "espeak" for voice in status["eligible_voices"]), status
        print("Legacy status, CLI precedence, empty helpers, exclusions and exact preview passed", flush=True)

        document["flite"]["builtin_slt"] = True
        digest = write()
        with server(program, [], {**environment, "OMNIVOX_VOICE_LIBRARY": native(path, windows)}) as session:
            status = session.control("voice_library_status_v1")
            assert status["configuration"]["sha256"] == digest, status
            assert {"engine_id": "flite", "voice_id": "cmu_us_slt"} in status["eligible_voices"], status
            result = session.control("preview", text="Voice library.", selector={"kind": "exact", "engine_id": "flite", "voice_id": "cmu_us_slt"})
            assert result["status"] == "completed" and result["realized"]["engine_id"] == "flite", result
        print("Environment selection, managed Flite and actual preview identity passed", flush=True)

        with server(program, arguments, {**environment, "OMNIVOX_FLITE_HELPER": native(temporary / "missing-helper", windows)}) as session:
            inventory, status = degraded_speech(session, "flite", "cmu_us_slt")
            assert status["configuration"]["sha256"] == digest, status
            assert excluded not in status["eligible_voices"], status
        print("Missing managed helper retains speech, exclusions and truthful status", flush=True)

        for arguments_override, environment_override in [
            (["--voice-library", native(temporary / "missing.json", windows)], environment),
            (["--voice-library", ""], environment),
        ]:
            result = subprocess.run([str(program), "--audio-output", "null", *arguments_override],
                                    env=environment_override, input="", text=True, capture_output=True, timeout=90)
            assert result.returncode != 0, result
        result = subprocess.run([str(args.flite_helper.resolve()), "--voice-library", native(path, windows),
                                 "--voice-library-sha256", "0" * 64], input="", text=True, capture_output=True, timeout=30)
        assert result.returncode != 0 and "SHA-256" in result.stderr, result
        print("Malformed selection and changed-generation rejection passed", flush=True)

        damaged = temporary / "damaged.flitevox"
        damaged.write_bytes(b"bad")
        document["flite"]["files"] = [{"physical_id": "flitevox:test",
            "file": {"path": native(damaged, windows), "bytes": 3,
                     "sha256": hashlib.sha256(b"abc").hexdigest()},
            "display_name": "Test", "language": "en"}]
        write()
        with server(program, arguments, environment) as session:
            degraded_speech(session, "flite", "cmu_us_slt", "SHA-256")
        document["flite"]["files"] = []
        write()
        print("Damaged managed assets retain speech through other engines", flush=True)

        # A saved Piper selection must also work on a build without Piper support.
        # These tiny deterministic fixtures need no native Piper model loading.
        source = root / "test-fixtures/piper-speakers"
        model = temporary / "alpha.onnx"
        config = temporary / "alpha.onnx.json"
        model.write_bytes((source / "alpha.onnx").read_bytes())
        config.write_bytes((source / "config.json").read_bytes())
        def asset(file):
            data = file.read_bytes()
            return {"path": native(file, windows), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        document["piper"]["models"] = [{"identity": {"catalogue_key": "alpha"}, "model": asset(model), "config": asset(config),
            "voices": [{"physical_id": "piper:v1/c/alpha/1", "speaker_index": 1, "display_name": "Alpha speaker 1", "language": None}]}]
        write()
        if args.piper_helper:
            with server(program, arguments, environment) as session:
                result = session.control("preview", text="a", selector={"kind": "exact", "engine_id": "piper", "voice_id": "piper:v1/c/alpha/1"})
                assert result["status"] == "completed" and result["realized"]["voice_id"] == "piper:v1/c/alpha/1", result
        else:
            with server(program, arguments + ["--engine", "piper"], environment) as session:
                _, status = degraded_speech(session, "piper", "piper:v1/c/alpha/1")
                assert excluded not in status["eligible_voices"], status
            print("Unavailable preferred Piper falls back to real eSpeak synthesis", flush=True)
            return
        # Changed managed assets disable only their provider; an explicit
        # model override replaces that provider's declared load set.
        document["piper"]["models"][0]["model"]["sha256"] = "0" * 64
        write()
        with server(program, arguments, environment) as session:
            degraded_speech(session, "piper", "piper:v1/c/alpha/1", "SHA-256")
        with server(program, arguments + ["--piper-model", native(model, windows)], environment) as session:
            status = session.control("voice_library_status_v1")
            assert status["overridden_engines"] == ["piper"], status
            assert {"engine_id": "piper", "voice_id": "piper:alpha"} in status["eligible_voices"], status
            assert {"engine_id": "piper", "voice_id": "piper:v1/c/alpha/1"} not in status["eligible_voices"], status
        document["disabled_physical_ids"].append({"engine_id": "piper", "voice_id": "piper:alpha"})
        write()
        with server(program, arguments + ["--piper-model", native(model, windows)], environment) as session:
            status = session.control("voice_library_status_v1")
            assert status["overridden_engines"] == ["piper"], status
            assert not any(voice["engine_id"] == "piper" for voice in status["eligible_voices"]), status
            result = session.control("preview", text="a", selector={"kind": "exact", "engine_id": "piper", "voice_id": "piper:alpha"})
            assert result.get("realized") is None and result["status"] != "completed", result
        print("Managed Piper speaker preview and explicit model override passed", flush=True)


if __name__ == "__main__":
    main()
