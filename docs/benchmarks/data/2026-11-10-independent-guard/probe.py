#!/usr/bin/env python3
"""Independent Rust guard over real Linux Flite, with retained-handle faults.

This is an extraction experiment. It neither changes installed speech nor claims
that current Omnivox/Emacsvox automatically negotiate or use this guard.
"""
import argparse
import base64
import array
import ctypes
import fcntl
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import threading
import time

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "2026-11-10-emergency-speech"))
import probe as emergency

parser = argparse.ArgumentParser()
parser.add_argument("exe", type=Path)
parser.add_argument("guard", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
assert sys.platform == "linux"
assert ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) == 0
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
exe, guard = args.exe.resolve(), args.guard.resolve()
env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG") if key in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root), OMNIVOX_VOICE_ROOT=str(root / "voices"),
           OMNIVOX_ENGINE="flite", OMNIVOX_AUDIO_OUTPUT="null")
emergency.resident.save(root / "config.json", emergency.resident.configuration(engine="flite"))


def exited(fd):
    return bool(select.select([fd], [], [], 0)[0])


def stop(fd, sig):
    if not exited(fd):
        signal.pidfd_send_signal(fd, sig)


def wait(predicate, timeout=5):
    deadline = time.monotonic() + timeout
    while not predicate():
        assert time.monotonic() < deadline, "bounded fixture wait expired"
        time.sleep(.005)


def forwarding_waits(process):
    return [(task / "wchan").read_text().strip()
            for task in Path(f"/proc/{process.pid}/task").glob("*")]


def descriptors(pid):
    return {os.readlink(fd) for fd in Path(f"/proc/{pid}/fd").glob("*")}


class Lines:
    def __init__(self, stream):
        self.stream = stream
        self.pending = b""
        self.lines = []
        self.eof = False
        os.set_blocking(stream.fileno(), False)

    def read(self):
        if select.select([self.stream], [], [], .005)[0]:
            data = os.read(self.stream.fileno(), 65536)
            self.eof |= not data
            self.pending += data
            while b"\n" in self.pending:
                line, self.pending = self.pending.split(b"\n", 1)
                self.lines.append(line.decode())

    def until(self, predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.read()
            for i, line in enumerate(self.lines):
                if predicate(line):
                    return self.lines.pop(i)
            assert not self.eof, "unexpected fixture EOF"
        raise AssertionError("response deadline")


class Case:
    def __init__(self, label, startup, partial=False):
        self.label = label
        self.root = root / label
        self.root.mkdir()
        self.handles = []
        self.writers = []
        self.responses = []
        self.retained = []
        read_input, write_input = os.pipe()
        read_output, write_output = os.pipe()
        life_read, life_write = os.pipe()
        io_read, io_write = os.pipe()
        self.error = (self.root / "worker.stderr").open("wb")
        inherited = [read_input, write_output, self.error.fileno(), life_read, io_read]
        lifetime_pipes = {os.readlink(f"/proc/self/fd/{fd}") for fd in (life_read, io_read)}
        speech_pipes = {os.readlink(f"/proc/self/fd/{fd}") for fd in (read_input, write_output)}
        native_env = {os.fsdecode(bytes(k["Unix"])): os.fsdecode(bytes(v["Unix"]))
                      for k, v in startup["environment"]}
        self.guard_error = (self.root / "guard.stderr").open("wb")
        self.g = subprocess.Popen([str(guard), *map(str, inherited), str(exe), *startup["arguments"]],
                                  env=native_env, cwd=startup["working_directory"], pass_fds=inherited,
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.guard_error,
                                  bufsize=0)
        self.guard_fd = os.pidfd_open(self.g.pid)
        self.handles.append(self.guard_fd)
        for fd in (read_input, write_output, life_read, io_read):
            os.close(fd)
        self.control = Lines(self.g.stdout)
        ready = self.control.until(lambda line: line.startswith("READY "))
        self.worker = int(ready.split()[1])
        self.worker_fd = os.pidfd_open(self.worker)
        self.handles.append(self.worker_fd)
        assert Path(f"/proc/{self.worker}/exe").resolve() == exe
        assert str(self.worker) in Path(f"/proc/{self.g.pid}/task/{self.g.pid}/children").read_text().split()
        self.retained.append(self.worker_fd)
        assert not speech_pipes.intersection(descriptors(self.g.pid)), "guard retained a speech endpoint"
        assert not lifetime_pipes.intersection(descriptors(self.worker)), "worker inherited guard lifetime"
        self.f = subprocess.Popen([sys.executable, str(Path(__file__).with_name("forward.py")),
                                   *map(str, (write_input, read_output, life_write, io_write))],
                                  pass_fds=(write_input, read_output, life_write, io_write),
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=(self.root / "forward.stderr").open("wb"), bufsize=0)
        self.forward_fd = os.pidfd_open(self.f.pid)
        self.handles.append(self.forward_fd)
        for fd in (write_input, read_output, life_write, io_write):
            os.close(fd)
        self.speech = Lines(self.f.stdout)
        if partial:
            self.f.stdin.write(b"START1\n")
        else:
            snapshot = json.dumps(startup["engines"], separators=(",", ":")).encode()
            self.f.stdin.write(b"START1\n" + len(snapshot).to_bytes(4, "big") + snapshot)
            request = emergency.resident.encoded(dict(protocol_version=1, request_id=1, type="capabilities"))
            self.f.stdin.write(b"omnivox_control " + request + b"\n")
            self.speech.until(lambda line: line.startswith("__OMNIVOX_CONTROL__ "))
            payload = dict(protocol_version=3, generation=1, dispatch_id=1, delivery_policy="ordered",
                           spans=[dict(id=1, text="Guarded speech works.")], actions=[])
            self.f.stdin.write(b"emacsvox_timeline " + emergency.resident.encoded(payload) + b"\n")
            marker = self.speech.until(lambda line: line.startswith("__EMACSVOX_MARKER__ ")
                                      and json.loads(base64.b64decode(line.split()[1]))["type"] == "utterance_started")
            assert json.loads(base64.b64decode(marker.split()[1]))["engine_id"] == "flite"
            self.speech.until(lambda line: line.startswith("__EMACSVOX_TRACKED__ 1 completed"))
            children = Path(f"/proc/{self.worker}/task/{self.worker}/children").read_text().split()
            assert children, "real Flite helper missing"
            for pid in children:
                handle = os.pidfd_open(int(pid))
                assert pid in Path(f"/proc/{self.worker}/task/{self.worker}/children").read_text().split()
                self.handles.append(handle)
                self.retained.append(handle)
                assert not lifetime_pipes.intersection(descriptors(int(pid))), "helper inherited guard lifetime"

    def write_background(self, data):
        def write():
            try:
                remaining = data
                while remaining:
                    remaining = remaining[os.write(self.f.stdin.fileno(), remaining):]
            except BrokenPipeError:
                pass
        thread = threading.Thread(target=write, daemon=True)
        thread.start()
        self.writers.append(thread)

    def hold_status(self):
        capacity = fcntl.fcntl(self.g.stdout.fileno(), 1031, 4096)  # F_SETPIPE_SZ
        def fill():
            data = b"Q" * 65536
            try:
                while data:
                    data = data[os.write(self.g.stdin.fileno(), data):]
            except BrokenPipeError:
                pass
        thread = threading.Thread(target=fill, daemon=True)
        thread.start()
        self.writers.append(thread)
        def full():
            size = array.array("i", [0])
            fcntl.ioctl(self.g.stdout.fileno(), 0x541B, size, True)  # FIONREAD
            return size[0] >= capacity - 128
        wait(full)
        # Keep the reader withheld beyond the first nearly-full observation;
        # small nonblocking pipe writes are atomic and can leave padding unused.
        time.sleep(.1)

    def status(self):
        self.g.stdin.write(b"Q")
        line = self.control.until(lambda value: value.startswith("STATUS "))
        _, seq, worker, tree, io, stalls = line.split()
        assert int(worker) == self.worker
        result = dict(sequence=int(seq), tree=tree == "true", io=io == "true",
                      output_stalls=int(stalls), ns=time.perf_counter_ns())
        self.responses.append(result)
        return result

    def await_tree(self):
        wait(lambda: self.status()["tree"])

    def await_complete(self):
        wait(lambda: (lambda r: r["tree"] and r["io"])(self.status()))

    def close(self):
        if not exited(self.guard_fd):
            self.g.stdin.write(b"R")
        stop(self.forward_fd, signal.SIGKILL)
        self.f.wait(timeout=5)
        if not exited(self.guard_fd):
            self.await_complete()
            self.g.stdin.close()
            self.g.wait(timeout=5)
        else:
            # Only fixture cleanup after guard loss; never product retirement evidence.
            for fd in self.retained:
                stop(fd, signal.SIGKILL)
            for fd in self.retained:
                wait(lambda fd=fd: exited(fd))
            self.g.wait(timeout=5)
            while True:
                try:
                    if os.waitpid(-self.worker, os.WNOHANG)[0] == 0:
                        time.sleep(.005)
                        continue
                except ChildProcessError:
                    break
        for writer in self.writers:
            writer.join(timeout=2)
            assert not writer.is_alive()
        for fd in self.handles:
            os.close(fd)
        self.error.close()
        self.guard_error.close()
        (self.root / "status.json").write_text(json.dumps(self.responses, indent=2) + "\n")
        (self.root / "remaining-speech-lines.json").write_text(json.dumps(self.speech.lines, indent=2) + "\n")


peers = []
cases = []
result = dict(success=False, executable_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),
              guard_sha256=hashlib.sha256(guard.read_bytes()).hexdigest(), rows=[])
try:
    seed = emergency.Peer([str(exe), "--voice-library-owner"], env, root, "snapshot", "flite")
    peers.append(seed)
    startup = json.loads(Path(seed.owner["startup"]).read_text())
    seed.close()
    peers.remove(seed)
    peer = emergency.Peer([str(exe), "--voice-library-owner"], env, root, "notification", "flite")
    peers.append(peer)
    peer.register()
    (root / "config.json").write_text("{broken preferences")
    (root / "voices" / "host.json").write_text("{broken history")
    (root / "voices" / "sessions").chmod(0o500)
    before = emergency.fingerprints(root / "voices")
    for label in ("blocked-input", "blocked-output", "blocked-status", "owner-loss-paused-worker",
                  "worker-first-paused-helper", "partial-start", "guard-loss"):
        case = Case(label, startup, partial=label == "partial-start")
        cases.append(case)
        row = dict(case=label)
        if label in ("blocked-input", "owner-loss-paused-worker", "guard-loss"):
            stop(case.worker_fd, signal.SIGSTOP)
        if label == "blocked-input":
            case.write_background(b"q {" + b"x" * 65536 + b"}\n")
            wait(lambda: "anon_pipe_write" in forwarding_waits(case.f))
            row["forwarder_waits"] = forwarding_waits(case.f)
        elif label == "blocked-output":
            packet = b"omnivox_control " + emergency.resident.encoded(dict(protocol_version=1, request_id=2, type="capabilities")) + b"\n"
            case.write_background(packet * 5000)
            wait(lambda: "anon_pipe_write" in forwarding_waits(case.f))
            row["forwarder_waits"] = forwarding_waits(case.f)
        elif label == "worker-first-paused-helper":
            for fd in case.retained[1:]:
                stop(fd, signal.SIGSTOP)
            stop(case.worker_fd, signal.SIGKILL)
            wait(lambda: exited(case.worker_fd))
            assert any(not exited(fd) for fd in case.retained[1:])
        elif label == "blocked-status":
            case.hold_status()
        start = time.perf_counter_ns()
        if label == "guard-loss":
            stop(case.guard_fd, signal.SIGKILL)
            wait(lambda: exited(case.guard_fd))
            assert not exited(case.worker_fd)
            row.update(replacement="blocked: no live native authority", tree_proof=False, io_proof=False)
        else:
            if label in ("blocked-status", "owner-loss-paused-worker", "worker-first-paused-helper", "partial-start"):
                stop(case.forward_fd, signal.SIGKILL)
                if label == "blocked-status":
                    wait(lambda: all(exited(fd) for fd in case.retained))
                    row["native_exit_while_status_held_ms"] = (time.perf_counter_ns() - start) / 1e6
            else:
                case.g.stdin.write(b"R")
            case.await_tree()
            row["tree_ms"] = (time.perf_counter_ns() - start) / 1e6
            if label == "blocked-output":
                status = case.status()
                assert not status["io"], "blocked forwarding incorrectly claimed I/O retirement"
                row["io_held_until_forwarder_exit"] = True
            stop(case.forward_fd, signal.SIGKILL)
            case.await_complete()
            if label == "blocked-status":
                assert case.responses[-1]["output_stalls"] > 0
                row["output_stalls"] = case.responses[-1]["output_stalls"]
            assert all(exited(fd) for fd in case.retained)
            row["complete_ms"] = (time.perf_counter_ns() - start) / 1e6
            replacement = emergency.Peer([str(exe), "--emergency-owner", "--audio-output", "null"],
                                          env, case.root, "replacement", "espeak")
            peers.append(replacement)
            row["replacement_speech"] = replacement.speech()
            replacement.close()
            peers.remove(replacement)
        row["notification"] = peer.speech(workload="letter")
        assert emergency.fingerprints(root / "voices") == before
        case.close()
        cases.remove(case)
        result["rows"].append(row)
        print(json.dumps(row), flush=True)
    result["success"] = True
finally:
    for case in cases:
        case.close()
    for peer in peers:
        peer.close()
    (root / "voices" / "sessions").chmod(0o700)
    emergency.resident.save(root / "result.json", result)
