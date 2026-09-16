#!/usr/bin/env python3
"""Exercise native local ownership and profile leases without playback."""
import argparse
import base64
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import tempfile
import threading


class Peer:
    def __init__(self, server, mode, environment):
        self.diagnostics = tempfile.TemporaryFile()
        self.process = subprocess.Popen([str(server), mode], env=environment,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.diagnostics)
        self.messages = queue.Queue()
        self.sequence = 0
        self.service = mode == "--voice-library-service"

        def read():
            try:
                while line := self.process.stdout.readline(2 * 1024 * 1024 + 1):
                    assert len(line) <= 2 * 1024 * 1024
                    self.messages.put(line)
            finally:
                self.messages.put(None)

        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()

    def request(self, command, control=False, **fields):
        self.sequence += 1
        message = dict(request_id=self.sequence, **fields)
        if control:
            message.update(protocol_version=1, type=command)
            encoded = base64.b64encode(json.dumps(message).encode())
            line = b"omnivox_control {" + encoded + b"}\n"
        else:
            message["command"] = command
            line = (b"" if self.service else b"OMNIVOX-LOCAL ") + json.dumps(message).encode() + b"\n"
        self.process.stdin.write(line)
        self.process.stdin.flush()
        while True:
            line = self.messages.get(timeout=45)
            assert line is not None, "native peer closed before its receipt"
            if line.startswith(b"OMNIVOX-LOCAL "):
                answer = json.loads(line[len(b"OMNIVOX-LOCAL "):])
            elif line.startswith(b"__OMNIVOX_CONTROL__ "):
                answer = json.loads(base64.b64decode(line.split(None, 1)[1]))
            else:
                raise AssertionError("unexpected native output")
            if answer["request_id"] == self.sequence:
                return answer

    def close(self):
        self.process.stdin.close()
        code = self.process.wait(timeout=15)
        self.reader.join(timeout=5)
        assert not self.reader.is_alive(), "native output reader did not retire"
        self.process.stdout.close()
        self.diagnostics.seek(0)
        diagnostics = self.diagnostics.read(4096).decode(errors="replace")
        self.diagnostics.close()
        assert code == 0, (code, diagnostics)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    args = parser.parse_args()
    server = args.server.resolve()
    if os.name != "nt" and server.suffix.lower() == ".exe":
        native_temp = subprocess.check_output(["cmd.exe", "/d", "/c", "echo", "%TEMP%"], text=True).strip()
        parent = subprocess.check_output(["wslpath", "-u", native_temp], text=True).strip()
        root = Path(tempfile.mkdtemp(prefix="omnivox-local-owner-", dir=parent))
        native_root = subprocess.check_output(["wslpath", "-w", str(root)], text=True).strip()
    else:
        root = Path(tempfile.mkdtemp(prefix="omnivox-local-owner-"))
        native_root = str(root)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
    environment.update(OMNIVOX_VOICE_ROOT=native_root, OMNIVOX_AUDIO_OUTPUT="null", OMNIVOX_ENGINE="espeak")
    # All these inputs are already native values; WSL must not translate them.
    forwarded = ["OMNIVOX_VOICE_ROOT", "OMNIVOX_AUDIO_OUTPUT", "OMNIVOX_ENGINE",
                 "OMNIVOX_OWNED_STARTUP", "OMNIVOX_OWNED_STARTUP_SHA256"]
    environment["WSLENV"] = ":".join([entry for entry in environment.get("WSLENV", "").split(":")
                                      if entry and entry.split("/", 1)[0] not in forwarded] + forwarded)
    peers = []

    def peer(mode, env=environment):
        result = Peer(server, mode, env)
        peers.append(result)
        return result

    try:
        service = peer("--voice-library-service")
        host = service.request("host")
        assert host["type"] == "host"
        initial = service.request("inspect")
        assert initial["active"] is None and not initial["index"]["voices"]
        assert service.request("include-flite-slt", expected_sha256=initial["sha256"])["state"] == "pending"
        indexed = service.request("inspect")
        assert indexed["index"]["voices"][0]["enabled"] is True and indexed["active"] is None
        assert service.request("enable", engine="flite", voice="cmu_us_slt", enabled=False,
                               expected_sha256=indexed["sha256"])["state"] == "pending"
        import uuid
        indexed = service.request("inspect")
        generation = str(uuid.uuid4())
        assert service.request("stage", generation=generation, flite=True,
                               expected_sha256=indexed["sha256"])["type"] == "candidate"
        assert service.request("begin", generation=generation, operation=str(uuid.uuid4()), plan_json="{}")["type"] == "candidate"
        competitor = peer("--voice-library-service")
        assert competitor.request("inspect")["type"] == "error"
        assert service.request("finish", state="cancelled")["state"] == "cancelled"
        assert competitor.request("inspect")["active"] is None

        owner = peer("--voice-library-owner")
        description = owner.request("describe")
        assert description["type"] == "owner" and not description["retired"]
        assert owner.request("retire", worker=str(uuid.uuid4()))["type"] == "error"
        capabilities = owner.request("capabilities", control=True)
        assert "voice_library_v1" in capabilities["features"]
        status = owner.request("voice_library_status_v1", control=True)
        assert status["configuration"] is None
        assert owner.request("retire", worker=description["worker"])["type"] == "retired"

        failed = peer("--voice-library-owner", dict(environment,
                      OMNIVOX_OWNED_STARTUP=native_root + "/missing-startup.json",
                      OMNIVOX_OWNED_STARTUP_SHA256="0" * 64))
        description = failed.request("describe")
        assert description["retired"] and description["startup_error"]
        assert failed.request("retire", worker=description["worker"])["type"] == "retired"
        # Closing a healthy owner's input also waits for confirmed tree cleanup.
        closing = peer("--voice-library-owner")
        assert not closing.request("describe")["retired"]
        print("Native local identity, profile exclusion, desired enablement, owned startup and retirement passed without playback")
    finally:
        errors = []
        for item in reversed(peers):
            try:
                item.close()
            except Exception as error:
                errors.append(error)
        if errors:
            raise RuntimeError(f"native cleanup failed; retained {root}") from errors[0]
        shutil.rmtree(root)


if __name__ == "__main__":
    main()
