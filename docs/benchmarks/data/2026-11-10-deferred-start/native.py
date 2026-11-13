#!/usr/bin/env python3
"""Deferred emergency owner checks in private state, using null output only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "2026-11-10-emergency-speech"))
import probe as emergency


class Prepared(emergency.Peer):
    def __init__(self, command, env, root, label):
        self.preparing = True
        super().__init__(command + ["--prepare"], env, root, label, "espeak")
        assert self.owner["prepared"] and self.owner["deferred_start_version"] == 1
        assert not self.owner["retired"]
        if not windows:
            assert not children(self.p.pid), "prepared owner created a native worker"

    def rpc(self, kind, **fields):
        if self.preparing:
            assert kind == "capabilities"
            return None  # The base reader starts; a prepared owner has no speech protocol yet.
        return super().rpc(kind, **fields)

    def release(self):
        reply = self.local("start-emergency", worker=self.owner["worker"])
        assert reply["type"] == "started" and reply["worker"] == self.owner["worker"], reply
        self.preparing = False
        self.capabilities = self.rpc("capabilities")
        self.register()


def children(pid):
    return Path(f"/proc/{pid}/task/{pid}/children").read_text().split()


parser = argparse.ArgumentParser()
parser.add_argument("exe", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--windows-runtime", type=Path)
args = parser.parse_args()
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
exe = args.exe.resolve()
windows = args.windows_runtime is not None
env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "WSL_INTEROP") if key in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root), OMNIVOX_VOICE_ROOT=str(root / "voices"),
           OMNIVOX_ENGINE="flite", OMNIVOX_AUDIO_OUTPUT="null", OMNIVOX_STARTUP_TIMING="1")
command = [str(exe), "--emergency-owner", "--audio-output", "null"]
if windows:
    data = (args.windows_runtime / "espeak-ng-data.path").read_text().strip()
    env.update(ESPEAK_NG_DATA=data,
               WSLENV="OMNIVOX_CONFIG_DIR/p:OMNIVOX_VOICE_ROOT/p:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA:OMNIVOX_STARTUP_TIMING")
    command += ["--espeak-data", data]
emergency.resident.save(root / "config.json", emergency.resident.configuration(engine="flite"))
result = dict(success=False, executable_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), checks=[], speech=[])
peers = []
try:
    main = emergency.Peer([str(exe), "--voice-library-owner"], env, root, "main", "flite"); peers.append(main)
    notify = emergency.Peer([str(exe), "--voice-library-owner"], env, root, "notify", "flite"); peers.append(notify)
    main.speech(); notify.speech()
    cancelled = Prepared(command, env, root, "cancelled"); peers.append(cancelled)
    wrong = cancelled.local("start-emergency", worker="00000000-0000-4000-8000-000000000000")
    assert wrong["type"] == "error"
    assert cancelled.local("describe")["prepared"]
    # Ordinary speech input before release must not be retained for replay.
    cancelled.send(b"q {Never replay this preparation text}\nd\n")
    assert not cancelled.local("retire-native", worker=cancelled.owner["worker"])["retirement_recorded"]
    assert cancelled.local("start-emergency", worker=cancelled.owner["worker"])["type"] == "error"
    cancelled.close(); peers.remove(cancelled)
    main.speech(); notify.speech()
    result["checks"].append("prepare has no worker; wrong identity rejected; cancellation prevents release")

    for label, bad_command in [
        ("bad-output", [str(exe), "--emergency-owner", "--prepare", "--audio-output", "unsupported"]),
        ("bad-data", [str(exe), "--emergency-owner", "--prepare", "--audio-output", "null", "--espeak-data",
                      (subprocess.check_output(["wslpath", "-w", str(root / 'missing')], text=True).strip()
                       if windows else str(root / 'missing'))])]:
        run = subprocess.run(bad_command, env=env,
                             input=b'OMNIVOX-LOCAL {"request_id":1,"command":"describe"}\n',
                             capture_output=True, timeout=10)
        (root / f"{label}.stdout").write_bytes(run.stdout); (root / f"{label}.stderr").write_bytes(run.stderr)
        owner = json.loads(run.stdout.removeprefix(b"OMNIVOX-LOCAL "))
        assert owner["startup_error"] and owner["retired"] and not owner["prepared"]
        main.speech(); notify.speech()
    result["checks"].append("invalid output or missing data fails preparation while original speech remains usable")

    candidate = Prepared(command, env, root, "released"); peers.append(candidate)
    result["speech"].append(dict(phase="old-while-prepared", **main.speech()))
    result["speech"].append(dict(phase="notification-while-prepared", **notify.speech()))
    old_retired = main.retire()
    assert old_retired["type"] == "native_retired"
    start = time.perf_counter_ns()
    candidate.release()
    identity_before = None if windows else children(candidate.p.pid)
    reply = candidate.local("start-emergency", worker=candidate.owner["worker"])
    assert reply["type"] == "started"
    if not windows:
        assert len(identity_before) == 1 and children(candidate.p.pid) == identity_before
    first = candidate.speech(workload="letter")
    result["release_to_first_source_ms"] = (first["start_ns"] - start)/1e6 + first["source_ms"]
    result["speech"].append(first)
    result["speech"].append(notify.speech())
    result["checks"].append("release follows actual old retirement; repeated release reuses exactly one worker; both lanes speak")

    fake = root / "disappearing-data" / "espeak-ng-data"
    fake.mkdir(parents=True)
    (fake / "phontab").write_bytes(b"preparation presence fixture; never loaded")
    fake_parent = str(fake.parent)
    if windows:
        fake_parent = subprocess.check_output(["wslpath", "-w", fake_parent], text=True).strip()
    failed = Prepared([str(exe), "--emergency-owner", "--audio-output", "null", "--espeak-data", fake_parent], env, root, "failed-initialization")
    peers.append(failed)
    (fake / "phontab").rename(fake / "phontab.retained")
    assert failed.local("start-emergency", worker=failed.owner["worker"])["type"] == "started"
    proof = failed.wait(lambda row: row["kind"] == "owner" and row["value"]["type"] == "retired")
    assert proof["value"]["worker"] == failed.owner["worker"]
    assert failed.local("start-emergency", worker=failed.owner["worker"])["type"] == "error"
    result["checks"].append("post-prepare initialization failure proves native cleanup and cannot relaunch on retry")
    candidate.speech(); notify.speech()
    result["success"] = True
finally:
    for peer in reversed(peers):
        peer.close()
    emergency.resident.save(root / "result.json", result)
print(json.dumps(result, indent=2))
