#!/usr/bin/env python3
"""Bounded Linux owner faults, using only newly spawned null-output workers.

The fixture retains pidfds for cleanup; those are not product recovery evidence.
No stored process identifiers, installed state or unrelated processes are used.
"""
import argparse
import base64
import ctypes
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "2026-11-10-emergency-speech"))
import probe as emergency

parser = argparse.ArgumentParser()
parser.add_argument("exe", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
assert sys.platform == "linux"
assert ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) == 0  # Dedicated fixture subreaper.
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
exe = args.exe.resolve()
env = {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root), OMNIVOX_VOICE_ROOT=str(root / "voices"),
           OMNIVOX_ENGINE="espeak", OMNIVOX_AUDIO_OUTPUT="null")
emergency.resident.save(root / "config.json", emergency.resident.configuration(engine="espeak"))


def exited(fd):
    return bool(select.select([fd], [], [], 0)[0])


def send_signal(fd, sig):
    if not exited(fd):
        signal.pidfd_send_signal(fd, sig)


def threads(pid):
    rows = []
    for task in Path(f"/proc/{pid}/task").glob("*"):
        try:
            rows.append(dict(tid=int(task.name), name=(task / "comm").read_text().strip(),
                             wait=(task / "wchan").read_text().strip()))
        except FileNotFoundError:
            pass
    return rows


class Owner:
    def __init__(self, label):
        self.label = label
        self.stderr = (root / f"{label}.stderr").open("wb")
        self.p = subprocess.Popen([str(exe), "--voice-library-owner"], env=env,
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=self.stderr, bufsize=0)
        self.owner_fd = os.pidfd_open(self.p.pid)
        self.worker_fd = None
        self.buffer = b""
        self.lines = []
        os.set_blocking(self.p.stdin.fileno(), False)
        os.set_blocking(self.p.stdout.fileno(), False)
        self.local("describe", 1)
        self.owner = self.wait(lambda v: v.get("request_id") == 1)
        assert self.owner["type"] == "owner" and not self.owner["startup_error"]
        children = Path(f"/proc/{self.p.pid}/task/{self.p.pid}/children").read_text().split()
        assert len(children) == 1, children
        self.worker = int(children[0])
        self.worker_fd = os.pidfd_open(self.worker)
        assert Path(f"/proc/{self.worker}/exe").resolve() == exe
        assert str(self.worker) in Path(f"/proc/{self.p.pid}/task/{self.p.pid}/children").read_text().split()
        self.control(2)
        self.wait(lambda v: v.get("request_id") == 2)
        assert not Path(f"/proc/{self.worker}/task/{self.worker}/children").read_text().strip()

    def send(self, data):
        deadline = time.monotonic() + 2
        while data:
            assert time.monotonic() < deadline, "fixture write blocked"
            if select.select([], [self.p.stdin], [], .01)[1]:
                try:
                    data = data[os.write(self.p.stdin.fileno(), data):]
                except BlockingIOError:
                    pass

    def local(self, command, identity, **fields):
        self.send(b"OMNIVOX-LOCAL " + json.dumps(dict(command=command, request_id=identity, **fields)).encode() + b"\n")

    def control(self, identity):
        self.send(b"omnivox_control " + emergency.resident.encoded(dict(protocol_version=1, request_id=identity, type="capabilities")) + b"\n")

    def read(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if not select.select([self.p.stdout], [], [], min(.01, max(0, deadline-time.monotonic())))[0]:
                continue
            data = os.read(self.p.stdout.fileno(), 65536)
            if not data:
                break
            self.buffer += data
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                self.lines.append(line)

    def values(self):
        for line in self.lines:
            if line.startswith(b"OMNIVOX-LOCAL "):
                yield json.loads(line[len(b"OMNIVOX-LOCAL "):])
            elif line.startswith(b"__OMNIVOX_CONTROL__ "):
                yield json.loads(base64.b64decode(line.split()[1]))

    def wait(self, predicate):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            self.read(.01)
            for value in self.values():
                if predicate(value):
                    return value
        raise AssertionError("fixture startup response missing")

    def close(self):
        # An exact retained handle is only test cleanup, never a fabricated receipt.
        if self.worker_fd is not None:
            send_signal(self.worker_fd, signal.SIGKILL)
        try:
            self.p.stdin.close()
            self.read(.5)
            if self.p.poll() is None:
                send_signal(self.owner_fd, signal.SIGKILL)
            self.p.wait(timeout=3)
            self.read(.1)
            if self.worker_fd is not None:
                assert select.select([self.worker_fd], [], [], 3)[0], "fixture worker survived cleanup"
                try:
                    os.waitid(os.P_PIDFD, self.worker_fd, os.WEXITED | os.WNOHANG)
                except ChildProcessError:
                    pass  # Its actual native owner reaped it.
            (root / f"{self.label}.stdout").write_bytes(b"\n".join(self.lines) + b"\n" + self.buffer)
        finally:
            self.p.stdout.close()
            self.stderr.close()
            os.close(self.owner_fd)
            if self.worker_fd is not None:
                os.close(self.worker_fd)


result = dict(success=False, executable_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), cases=[])
peer = emergency.Peer([str(exe), "--voice-library-owner"], env, root, "healthy", "espeak")
try:
    peer.register()
    peer.speech()
    for case in ("blocked-input", "blocked-output", "owner-crash", "owner-crash-paused-worker"):
        owner = Owner(case)
        try:
            started = time.perf_counter_ns()
            if case in ("blocked-input", "owner-crash-paused-worker"):
                send_signal(owner.worker_fd, signal.SIGSTOP)
                deadline = time.monotonic() + 1
                while "State:\tT" not in Path(f"/proc/{owner.worker}/status").read_text():
                    assert time.monotonic() < deadline
                    time.sleep(.005)
            if case == "blocked-input":
                owner.send(b"q {" + b"x" * 65536 + b"}\n")
            if case == "blocked-output":
                for identity in range(10, 26):
                    owner.control(identity)
                time.sleep(.1)  # Let the deliberately unread output pipe fill.
            if case.startswith("owner-crash"):
                send_signal(owner.owner_fd, signal.SIGKILL)
            else:
                owner.local("retire-native", 100, worker=owner.owner["worker"])
            notification = peer.speech(workload="letter")
            deadline = time.monotonic() + 6.25
            while time.monotonic() < deadline:
                if exited(owner.owner_fd):
                    time.sleep(.05)
                elif case != "blocked-output":
                    owner.read(.05)
                else:
                    time.sleep(.05)
            receipt = Path(owner.owner["startup"]).with_suffix(".retired")
            inspection = subprocess.run(
                [str(exe), "--voice-library-service"], env=env,
                input=json.dumps(dict(
                    request_id=1, command="retirement-status", worker=owner.owner["worker"],
                    startup=owner.owner["startup"], startup_sha256=owner.owner["startup_sha256"])).encode() + b"\n",
                capture_output=True, timeout=5)
            (root / f"{case}.inspection.stdout").write_bytes(inspection.stdout)
            (root / f"{case}.inspection.stderr").write_bytes(inspection.stderr)
            assert inspection.returncode == 0
            status = json.loads(inspection.stdout.removeprefix(b"OMNIVOX-LOCAL "))
            row = dict(case=case, observation_ms=(time.perf_counter_ns()-started)/1e6,
                       owner_exited=exited(owner.owner_fd), worker_exited=exited(owner.worker_fd),
                       native_reply=any(v.get("type") == "native_retired" and v.get("request_id") == 100 for v in owner.values()),
                       durable_receipt=receipt.is_file(), inspection=status, notification=notification,
                       owner_threads=threads(owner.p.pid), worker_threads=threads(owner.worker))
            result["cases"].append(row)
            print(json.dumps(row), flush=True)
        finally:
            owner.close()
    result["success"] = True
finally:
    peer.close()
    emergency.resident.save(root / "result.json", result)
