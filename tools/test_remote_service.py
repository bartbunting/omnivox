#!/usr/bin/env python3
"""Acceptance tests against a staged Omnivox remote service (make dev first).

Set OMNIVOX_REMOTE_TEST_PROGRAM to a staged binary or Emacsvox launcher.
Set OMNIVOX_REMOTE_TEST_WINDOWS=1 when that launcher runs a Windows payload
from WSL. OMNIVOX_REMOTE_TEST_SLOW=1 also exercises heartbeat expiry.
"""
from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import queue
import re
import secrets
import socket
import subprocess
import tempfile
import threading
import time
import unittest

from create_remote_token import create_token

ROOT = Path(__file__).resolve().parents[1]
PROGRAM = os.environ.get("OMNIVOX_REMOTE_TEST_PROGRAM", str(ROOT / "target/debug/omnivox"))
WINDOWS = os.environ.get("OMNIVOX_REMOTE_TEST_WINDOWS") == "1"


def native_path(path: Path) -> str:
    if WINDOWS:
        return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()
    return str(path)


class Peer:
    def __init__(self, port: int):
        self.socket = socket.create_connection(("127.0.0.1", port), timeout=5)
        self.pending = b""
        self.received = []

    def close(self):
        self.socket.close()

    def send(self, data: str):
        self.socket.sendall(data.encode())

    def line(self, timeout=10):
        deadline = time.monotonic() + timeout
        while b"\n" not in self.pending:
            self.socket.settimeout(max(0.01, deadline - time.monotonic()))
            data = self.socket.recv(8192)
            if not data:
                raise EOFError("remote connection closed")
            self.pending += data
        line, self.pending = self.pending.split(b"\n", 1)
        line = line.decode().rstrip("\r")
        self.received.append(line)
        return line

    def until(self, prefix: str, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            line = self.line(deadline - time.monotonic())
            if line.startswith(prefix):
                return line
        raise TimeoutError(prefix)

    def control(self, kind: str, request_id=1):
        payload = {"protocol_version": 1, "request_id": request_id, "type": kind}
        self.send("omnivox_control " + base64.b64encode(json.dumps(payload).encode()).decode() + "\n")
        response = self.until("__OMNIVOX_CONTROL__ ")
        return json.loads(base64.b64decode(response.split(" ", 1)[1]))


class RemoteServiceTests(unittest.TestCase):
    def setUp(self):
        directory = None
        # WSL's translated Windows TEMP is used without depending on a username.
        if WINDOWS:
            win_temp = subprocess.check_output(
                ["cmd.exe", "/c", "echo", "%TEMP%"], text=True).strip()
            directory = subprocess.check_output(["wslpath", "-u", win_temp], text=True).strip()
        self.temp = tempfile.TemporaryDirectory(prefix="omnivox-remote-test-", dir=directory)
        self.addCleanup(self.temp.cleanup)
        token_path = Path(self.temp.name) / "token"
        self.token_path = token_path
        create_token(token_path)
        self.token = token_path.read_text().strip()
        self.session = secrets.token_hex(16)
        self.lines: queue.Queue[str] = queue.Queue()
        self.peers: list[Peer] = []
        self.log = []
        self.process = subprocess.Popen(
            [PROGRAM, "--serve", "--listen", "127.0.0.1:0", "--token-file", native_path(token_path),
             "--sound-root", native_path(ROOT / "test-sounds"), "--audio-output", "null"],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace")
        self.addCleanup(self.stop)

        def read_log():
            for line in self.process.stderr:
                self.lines.put(line)
                self.log.append(line)

        self.reader = threading.Thread(target=read_log)
        self.reader.start()
        deadline = time.monotonic() + 10
        while True:
            line = self.lines.get(timeout=max(0.01, deadline - time.monotonic()))
            match = re.search(r"listening on 127\.0\.0\.1:(\d+)", line)
            if match:
                self.port = int(match[1])
                break

    def stop(self):
        for peer in self.peers:
            peer.close()
        if self.process.poll() is None:
            self.process.stdin.write("quit\n")
            self.process.stdin.flush()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                self.fail("service did not retire workers during shutdown")
        self.process.stdin.close()
        self.reader.join(timeout=10)
        self.process.stderr.close()
        self.assertNotIn(self.token, "".join(self.log))

    def connect(self, lane="speaker", session=None, token=None, expected="ready"):
        peer = Peer(self.port)
        self.peers.append(peer)
        peer.send(f"OMNIVOX-REMOTE 1 {token or self.token} {session or self.session} {lane}\n")
        self.assertEqual(peer.line(), f"OMNIVOX-REMOTE 1 {expected}")
        return peer

    def reconnect(self, lane="speaker", session=None):
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline:
            peer = Peer(self.port)
            self.peers.append(peer)
            peer.send(f"OMNIVOX-REMOTE 1 {self.token} {session or self.session} {lane}\n")
            line = peer.line()
            if line.endswith(" ready"):
                return peer
            self.assertEqual(line, "OMNIVOX-REMOTE 1 error busy")
            peer.close()
            time.sleep(0.05)
        self.fail("lane was not released")

    def test_authentication_and_exclusive_two_lane_session(self):
        bad = self.connect(token="0" * 64, expected="error authentication")
        with self.assertRaises(EOFError):
            bad.line()
        speaker = self.connect()
        notify = self.connect("notification")
        self.connect(expected="error busy")
        self.connect(session="c" * 32, expected="error busy")
        speaker.send("OMNIVOX-REMOTE ping\n")
        self.assertEqual(speaker.line(), "OMNIVOX-REMOTE pong")
        speaker.close()
        # Losing the foreground connection leaves notifications usable.
        self.assertEqual(notify.control("capabilities")["type"], "capabilities")
        self.reconnect().close()
        notify.close()
        self.reconnect(session="d" * 32)

    def test_protocol_burst_unicode_icons_markers_and_stop(self):
        peer = self.connect()
        self.assertEqual(peer.control("capabilities")["type"], "capabilities")
        inventory = peer.control("inventory", 2)
        self.assertTrue(inventory["engines"])
        peer.send("tts_set_speech_rate 300\n" * 100)
        peer.send('a "omnivox-icon:complete.ogg"\nq Remote café.\nemacsvox_marker_dispatch 31\n')
        terminal = peer.until("__EMACSVOX_TRACKED__ 31 ")
        self.assertTrue(terminal.endswith(" completed"), terminal)
        markers = [json.loads(base64.b64decode(line.split(" ", 1)[1]))
                   for line in peer.received if line.startswith("__EMACSVOX_MARKER__ ")]
        self.assertTrue(any(event["type"] == "utterance_started" for event in markers))
        long_text = "This obsolete paragraph should be interrupted. " * 1000
        started = time.monotonic()
        peer.send(f"q {long_text}\nemacsvox_marker_dispatch 32\ns\nq Next.\nemacsvox_marker_dispatch 33\n")
        self.assertTrue(peer.until("__EMACSVOX_TRACKED__ 33 ").endswith(" completed"))
        self.assertLess(time.monotonic() - started, 4)
        cancelled = "__EMACSVOX_TRACKED__ 32 cancelled"
        if cancelled not in peer.received:
            self.assertEqual(peer.until("__EMACSVOX_TRACKED__ 32 "), cancelled)

    def test_disconnect_discards_partial_input_and_old_backlog(self):
        peer = self.connect()
        peer.control("capabilities")
        peer.send("sh 10000\nemacsvox_marker_dispatch 41\nq incomplete")
        peer.close()
        replacement = self.reconnect()
        replacement.control("capabilities")
        started = time.monotonic()
        replacement.send("q Fresh.\nemacsvox_marker_dispatch 42\n")
        self.assertTrue(replacement.until("__EMACSVOX_TRACKED__ 42 ").endswith(" completed"))
        self.assertLess(time.monotonic() - started, 4)

    def test_invalid_framing_closes_lane_and_allows_reconnect(self):
        for data in (b"q bad\0\n", b"q \xff\n", b"q " + b"x" * (512 * 1024)):
            peer = self.reconnect()
            peer.socket.sendall(data)
            with self.assertRaises((EOFError, ConnectionResetError)):
                peer.line()
            peer.close()

    @unittest.skipUnless(os.environ.get("OMNIVOX_REMOTE_TEST_EMACS"), "optional live Emacsvox acceptance")
    def test_live_emacsvox_filters_inventory_completion_and_recovery(self):
        emacsvox = Path(os.environ["OMNIVOX_REMOTE_TEST_EMACSVOX"])
        environment = dict(os.environ, OMNIVOX_REMOTE_TEST_PORT=str(self.port),
                           OMNIVOX_REMOTE_TEST_TOKEN=str(self.token_path))
        result = subprocess.run(
            [os.environ["OMNIVOX_REMOTE_TEST_EMACS"], "-Q", "--batch", "-l",
             str(emacsvox / "test/verify-remote-omnivox.el")],
            env=environment, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=80)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("lane recovery passed", result.stdout)

    @unittest.skipUnless(os.environ.get("OMNIVOX_REMOTE_TEST_SLOW"), "optional 20-second lease test")
    def test_partial_bytes_do_not_renew_the_heartbeat_lease(self):
        peer = self.connect()
        started = time.monotonic()
        while time.monotonic() - started < 22:
            try:
                peer.socket.sendall(b"x")
            except (BrokenPipeError, ConnectionResetError):
                break
            time.sleep(0.05)
        with self.assertRaises((EOFError, ConnectionResetError)):
            peer.line()
        self.reconnect()


if __name__ == "__main__":
    unittest.main()
