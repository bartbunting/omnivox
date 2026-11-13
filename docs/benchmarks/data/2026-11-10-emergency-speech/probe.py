#!/usr/bin/env python3
"""Isolated native emergency recovery; null output, never installed speech."""
import argparse
import base64
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "2026-11-10-resident-reload"))
import resident


class Peer(resident.Peer):
    def __init__(self, command, env, root, label, engine):
        self.started = time.perf_counter_ns()
        self.stderr = open(root / f"{label}.stderr", "wb")
        self.p = subprocess.Popen(command, env=env, stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=self.stderr, bufsize=0)
        self.rows = []
        self.condition = threading.Condition()
        self.control = self.dispatch = 0
        self.engine = engine
        self.eof = False
        self.log = open(root / f"{label}.jsonl", "w")
        self.write_lock = threading.Lock()

        def read():
            for line in self.p.stdout:
                now = time.perf_counter_ns()
                if line.startswith(b"__OMNIVOX_CONTROL__ "):
                    value = json.loads(base64.b64decode(line.split()[1])); kind = "control"
                elif line.startswith(b"__EMACSVOX_MARKER__ "):
                    value = json.loads(base64.b64decode(line.split()[1])); kind = "marker"
                elif line.startswith(b"__EMACSVOX_TRACKED__ "):
                    fields = line.split()
                    value = dict(dispatch_id=int(fields[1]), outcome=fields[2].decode()); kind = "terminal"
                elif line.startswith(b"OMNIVOX-LOCAL "):
                    value = json.loads(line[len(b"OMNIVOX-LOCAL "):]); kind = "owner"
                else:
                    value = dict(line=line.decode(errors="replace")); kind = "other"
                row = dict(ns=now, kind=kind, value=value)
                self.log.write(json.dumps(row) + "\n")
                with self.condition:
                    self.rows.append(row); self.condition.notify_all()
            with self.condition:
                self.eof = True; self.condition.notify_all()

        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()
        self.owner = self.local("describe")
        assert not self.owner["startup_error"], self.owner
        self.capabilities = self.rpc("capabilities")
        self.ready = time.perf_counter_ns()

    def local(self, command, **fields):
        self.control += 1
        identity = self.control
        self.send(b"OMNIVOX-LOCAL " + json.dumps(dict(request_id=identity, command=command, **fields)).encode() + b"\n")
        return self.wait(lambda r: r["kind"] == "owner" and r["value"].get("request_id") == identity)["value"]

    def retire(self):
        receipt = self.local("retire-native", worker=self.owner["worker"])
        assert receipt["type"] == "native_retired" and receipt["worker"] == self.owner["worker"], receipt
        return receipt

    def close(self):
        self.retire()
        self.p.stdin.close()
        self.p.wait(timeout=10)
        self.reader.join(timeout=2)
        self.log.close(); self.p.stdout.close(); self.stderr.close()
        # Failed persistence may still make ordinary owner exit nonzero, after
        # its separately verified native tree/I/O retirement. Keep that status.
        return self.p.returncode


def fingerprints(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob("*")) if p.is_file()}


def run(args):
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    executable = args.exe.resolve()
    config = root / "config"; config.mkdir()
    voices = root / "voices"
    resident.save(config / "config.json", resident.configuration(engine="flite"))
    env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "WSL_INTEROP") if key in os.environ}
    env.update(OMNIVOX_CONFIG_DIR=str(config), OMNIVOX_VOICE_ROOT=str(voices),
               OMNIVOX_ENGINE="flite", OMNIVOX_AUDIO_OUTPUT="null")
    emergency = [str(executable), "--emergency-owner", "--audio-output", "null"]
    if args.windows_runtime:
        data = (args.windows_runtime.resolve() / "espeak-ng-data.path").read_text().strip()
        env.update(ESPEAK_NG_DATA=data, WSLENV="OMNIVOX_CONFIG_DIR/p:OMNIVOX_VOICE_ROOT/p:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA")
        emergency += ["--espeak-data", data]
    peers = []
    result = dict(success=False, executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                  checks=[], starts=[], speech=[], cleanup=[])
    denied = False
    try:
        main = Peer([str(executable), "--voice-library-owner"], env, root, "main", "flite"); peers.append(main)
        notify = Peer([str(executable), "--voice-library-owner"], env, root, "notify", "flite"); peers.append(notify)
        main.speech(); notify.speech()
        (config / "config.json").write_text("{broken")
        (voices / "host.json").write_text("{broken host")
        if args.windows_runtime:
            # A directory at the immutable receipt destination makes the actual
            # write fail on Windows. It is an obstruction, not a disk-full test.
            for p in [main, notify]:
                (voices / "sessions" / f"{p.owner['worker']}.retired").mkdir()
        else:
            (voices / "sessions").chmod(0o500); denied = True
        before = fingerprints(voices)
        assert main.reload()["outcome"] == "rejected"
        main.speech(); notify.speech()
        result["checks"].append("bad preferences and failed management writes preserve both working lanes")
        start = time.perf_counter_ns()
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            notification = pool.submit(notify.speech)
            receipt = main.retire()
            result["notification_during_retirement"] = notification.result()
        result["retirement_ms"] = (time.perf_counter_ns() - start) / 1e6
        assert receipt["retirement_recorded"] is False, receipt
        assert main.retire()["retirement_recorded"] is False
        result["cleanup"].append(receipt)
        result["checks"].append("live native retirement remains queryable without durable evidence")
        hostile = dict(env, OMNIVOX_ENGINE="unavailable", ESPEAK_NG_DATA="missing-data",
                       OMNIVOX_ESPEAK_VARIANTS="invalid!", OMNIVOX_OWNED_STARTUP="missing-startup")
        if args.windows_runtime:
            hostile["WSLENV"] += ":OMNIVOX_ESPEAK_VARIANTS:OMNIVOX_OWNED_STARTUP"
        for index in range(args.samples):
            p = Peer(emergency, hostile, root, f"emergency-{index}", "espeak"); peers.append(p)
            assert p.owner["emergency"] is True and p.owner["startup"] == ""
            assert "host_preferences_v1" not in p.capabilities["features"]
            assert p.rpc("host_preferences_status_v1")["type"] == "error"
            p.register()
            first = p.speech()
            result["starts"].append(dict(ready_ms=(p.ready-p.started)/1e6,
                                         first_source_ms=(first["start_ns"]-p.started)/1e6+first["source_ms"]))
            for _ in range(8):
                result["speech"].append(p.speech(workload="letter"))
                result["speech"].append(p.speech(workload="replaceable"))
            notify.speech()
            assert p.retire()["retirement_recorded"] is False
            assert p.close() == 0
            peers.remove(p)
            assert fingerprints(voices) == before, "emergency changed blocked management state"
        result["checks"].append("minimal starts ignore broken state and overrides, retain independent peer and all management bytes")
        result["checks"].append("character navigation and replacement complete on eSpeak; reload unavailable")
        for p in list(peers):
            result["cleanup"].append(p.retire())
            result["cleanup"].append(dict(owner_exit=p.close()))
            peers.remove(p)
        assert fingerprints(voices) == before
        # There cannot even be a management directory at this native path.
        invalid = root / "not-a-directory"; invalid.write_text("preserve me")
        hostile.update(OMNIVOX_CONFIG_DIR=str(invalid), OMNIVOX_VOICE_ROOT=str(invalid))
        p = Peer(emergency, hostile, root, "no-storage", "espeak"); peers.append(p)
        p.speech(); p.close(); peers.remove(p)
        assert invalid.read_text() == "preserve me"
        result["checks"].append("speech starts with configuration and management paths that are not directories")
        result["success"] = True
    finally:
        for p in peers:
            try: p.close()
            except Exception as error: result["cleanup"].append(dict(error=str(error)))
        if denied: (voices / "sessions").chmod(0o700)
        resident.save(root / "result.json", result)
    print(json.dumps({key: value for key, value in result.items() if key != "speech"}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("exe", type=Path); parser.add_argument("output", type=Path)
    parser.add_argument("--windows-runtime", type=Path)
    parser.add_argument("--samples", type=int, default=8)
    run(parser.parse_args())
