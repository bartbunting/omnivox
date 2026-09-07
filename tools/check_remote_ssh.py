#!/usr/bin/env python3
"""Opt-in real SSH/Emacs acceptance; see docs/REMOTE.md for prerequisites.

Only this check's private service, tunnel, token, and remote snapshot are owned.
No installed configuration, checkout, runtime, or existing tunnel is changed.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import io
import json
import os
from pathlib import Path
import queue
import re
import shlex
import subprocess
import tarfile
import tempfile
import threading
import time

from create_remote_token import create_token

ROOT = Path(__file__).resolve().parents[1]
STAGE = "OMNIVOX-SSH-CHECK "
SSH_OPTIONS = ["-a", "-x", "-T", "-o", "BatchMode=yes",
               "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8",
               "-o", "ConnectionAttempts=1", "-o", "ControlMaster=no",
               "-o", "ControlPath=none", "-o", "ServerAliveInterval=5",
               "-o", "ServerAliveCountMax=2"]

# The separate client connection survives an intentional forwarding loss.
# EOF on that connection or a deadline also retires the owned remote Emacs.
REMOTE_RUNNER = '''import os, signal, subprocess, sys, threading
root, emacs, port, engine = sys.argv[1:]
os.environ.update(HOME=root + "/home", XDG_CONFIG_HOME=root + "/home/.config",
                  XDG_DATA_HOME=root + "/home/.local/share",
                  XDG_STATE_HOME=root + "/home/.local/state", XDG_CACHE_HOME=root + "/home/.cache",
                  EMACSVOX_DIR=root, OMNIVOX_REMOTE_TEST_PORT=port,
                  OMNIVOX_REMOTE_TEST_TOKEN=root + "/token",
                  OMNIVOX_REMOTE_TEST_ENGINE=engine)
os.makedirs(os.environ["HOME"], mode=0o700, exist_ok=True)
child = subprocess.Popen([emacs, "-Q", "--batch", "-l", root + "/check.el"],
                         stdin=subprocess.DEVNULL, start_new_session=True)
lock = threading.Lock()
def retire():
    with lock:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
def watch_input():
    while os.read(0, 4096):
        pass
    retire()
threading.Thread(target=watch_input, daemon=True).start()
timer = threading.Timer(180, retire)
timer.start()
try:
    sys.exit(child.wait())
finally:
    timer.cancel()
    retire()
'''


class LoggedProcess:
    """Drain output continuously and retire only the child created here."""

    def __init__(self, command, *, env=None):
        self.lines = []
        self.events = queue.Queue()
        self.process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, encoding="utf-8",
            errors="replace", env=env)
        self.reader = threading.Thread(target=self._read)
        self.reader.start()

    def _read(self):
        try:
            for line in self.process.stdout:
                self.lines.append(line.rstrip())
                self.events.put(line.rstrip())
        finally:
            self.events.put(None)

    def expect(self, pattern, timeout=30):
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"Timed out waiting for {pattern}")
            try:
                line = self.events.get(timeout=remaining)
            except queue.Empty:
                raise TimeoutError(f"Timed out waiting for {pattern}") from None
            if line is None:
                raise RuntimeError(f"Process ended before {pattern}: " + "\n".join(self.lines[-8:]))
            match = re.search(pattern, line)
            if match:
                return match

    def close(self, *, quit_service=False):
        process = self.process
        if process.stdin and not process.stdin.closed:
            try:
                if quit_service and process.poll() is None:
                    process.stdin.write("quit\n")
                    process.stdin.flush()
                process.stdin.close()
            except (BrokenPipeError, OSError):
                pass
        try:
            code = process.wait(timeout=10)
            if quit_service and code != 0:
                raise RuntimeError(f"Service exited unsuccessfully: {code}")
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            if quit_service:
                raise RuntimeError("Service did not shut down its owned workers")
        finally:
            self.reader.join(timeout=10)
            process.stdout.close()


def ssh_command(args, words, *, forward=None):
    command = [args.ssh, *SSH_OPTIONS]
    if forward:
        command += ["-o", "ExitOnForwardFailure=yes", "-R", forward]
    else:
        command += ["-o", "ClearAllForwardings=yes"]
    # OpenSSH passes the command through a remote shell, even with list argv.
    return command + [args.host, shlex.join(words)]


def remote(args, words, *, data=None):
    result = subprocess.run(ssh_command(args, words), input=data,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=40)
    if result.returncode:
        raise RuntimeError("SSH command failed: " + result.stderr.decode(errors="replace"))
    return result.stdout.decode().strip()


def archive_client(checkout, token):
    revision = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"],
                                       text=True).strip()
    archive = io.BytesIO(subprocess.check_output(
        ["git", "-C", str(checkout), "archive", revision, "lisp", "etc", "sounds", "VERSION", "COPYING"]))
    with tarfile.open(fileobj=archive, mode="a") as bundle:
        for name, data, mode in [
            ("check.el", (ROOT / "tools/remote_ssh_check.el").read_bytes(), 0o600),
            ("runner.py", REMOTE_RUNNER.encode(), 0o600),
            ("token", token, 0o600),
        ]:
            entry = tarfile.TarInfo(name)
            entry.size, entry.mode = len(data), mode
            bundle.addfile(entry, io.BytesIO(data))
    return revision, archive.getvalue()


def check(args, report):
    secret = ""
    logs = {}
    started = time.monotonic()
    try:
        with ExitStack() as stack:
            temp = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="omnivox-ssh-")))
            token = temp / "token"
            create_token(token)
            secret = token.read_text().strip()
            # Reject inherited forwards: this check must own every listener it opens.
            config = subprocess.check_output([args.ssh, "-G", *SSH_OPTIONS, args.host], text=True)
            if re.search(r"^(?:localforward|remoteforward|dynamicforward) ", config, re.M):
                raise ValueError("SSH host has configured forwards; use an alias without forwards")
            report["remote_emacs"] = remote(args, [args.remote_emacs, "--version"]).splitlines()[0]
            report["remote_system"] = remote(args, ["uname", "-sm"])
            revision, bundle = archive_client(Path(args.emacsvox), token.read_bytes())
            report["emacsvox_commit"] = revision
            # Hash the public fixture, never the credential-containing archive.
            report["fixture_sha256"] = hashlib.sha256((ROOT / "tools/remote_ssh_check.el").read_bytes()).hexdigest()
            remote_root = remote(args, ["python3", "-c",
                "import tempfile; print(tempfile.mkdtemp(prefix='omnivox-ssh-', dir='/tmp'))"])
            if not re.fullmatch(r"/tmp/omnivox-ssh-[A-Za-z0-9_-]+", remote_root):
                raise RuntimeError("Unexpected remote temporary directory")
            def remove_snapshot():
                remote(args, ["python3", "-c", "import shutil,sys; shutil.rmtree(sys.argv[1])", remote_root])
                report["remote_snapshot_removed"] = True
            stack.callback(remove_snapshot)
            remote(args, ["tar", "-xmf", "-", "-C", remote_root], data=bundle)
            del bundle

            def native(path):
                return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip() if args.windows else str(path)

            environment = dict(os.environ, OMNIVOX_ENGINE=args.engine,
                               OMNIVOX_LOG_SYNTHESIS_TEXT="0",
                               OMNIVOX_LOG_DIRECTORY=str(temp / "logs"))
            service = LoggedProcess([args.program, "--serve", "--listen", "127.0.0.1:0",
                "--token-file", native(token), "--sound-root", native(Path(args.emacsvox) / "sounds"),
                "--audio-output", args.audio_output], env=environment)
            logs["service"] = service.lines
            stack.callback(service.close, quit_service=True)
            port = int(service.expect(r"listening on 127\.0\.0\.1:(\d+)", 15)[1])
            report["service_port"] = port

            def tunnel(remote_port, name):
                forward = f"127.0.0.1:{remote_port}:127.0.0.1:{port}"
                process = LoggedProcess(ssh_command(args, ["python3", "-u", "-c",
                    "import sys; print('OMNIVOX-FORWARD-READY', flush=True); sys.stdin.buffer.read()"],
                    forward=forward))
                logs[name] = process.lines
                stack.callback(process.close)
                if remote_port == 0:
                    remote_port = int(process.expect(r"Allocated port (\d+) for remote forward", 20)[1])
                process.expect(r"^OMNIVOX-FORWARD-READY$", 20)
                listeners = remote(args, ["ss", "-H", "-ltn", f"sport = :{remote_port}"])
                addresses = [line.split()[3] for line in listeners.splitlines()]
                if addresses != [f"127.0.0.1:{remote_port}"]:
                    raise RuntimeError(f"Forward must bind only IPv4 loopback: {addresses}")
                return process, remote_port

            forward, remote_port = tunnel(0, "tunnel_initial")
            report["remote_port"] = remote_port
            print(f"SSH forward ready; {report['remote_emacs']}; client {revision[:12]}", flush=True)
            client = LoggedProcess(ssh_command(args, ["python3", "-u", remote_root + "/runner.py",
                remote_root, args.remote_emacs, str(remote_port), args.engine]))
            logs["client"] = client.lines
            stack.callback(client.close)
            client.expect("^" + STAGE + "idle-passed$", 70)
            print("Both speech lanes and idle heartbeat passed.", flush=True)
            client.expect("^" + STAGE + "interrupt-now$")
            interrupted = time.monotonic()
            forward.process.terminate()
            forward.close()
            client.expect("^" + STAGE + "disconnected$", 25)
            report["disconnect_seconds"] = round(time.monotonic() - interrupted, 3)
            print("Interrupted both pending requests; restoring the owned tunnel.", flush=True)
            restored = time.monotonic()
            tunnel(remote_port, "tunnel_restored")
            client.expect("^" + STAGE + "passed$", 60)
            if client.process.wait(timeout=10) != 0:
                raise RuntimeError("Remote Emacs returned failure after acceptance")
            report["recovery_and_speech_seconds"] = round(time.monotonic() - restored, 3)
            report["checks"] = ["both_lane_routing", "realized_engine", "marked_completion",
                                "idle_heartbeat", "ssh_loss_pending_speech",
                                "automatic_two_lane_reconnect", "fresh_speech_without_old_backlog"]
        report["passed"] = True
    except BaseException as error:
        report["error"] = str(error).replace(secret, "[redacted]") if secret else str(error)
        raise
    finally:
        report["elapsed_seconds"] = round(time.monotonic() - started, 3)
        report["logs"] = {name: [line.replace(secret, "[redacted]") if secret else line for line in lines]
                          for name, lines in logs.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True, help="existing trusted SSH host alias")
    parser.add_argument("--ssh", default="ssh", help="Windows OpenSSH path for Windows service")
    parser.add_argument("--program", required=True, help="staged Omnivox executable or Emacsvox launcher")
    parser.add_argument("--windows", action="store_true", help="run a Windows service from WSL")
    parser.add_argument("--emacsvox", default=str(ROOT.parent / "emacsvox"), help="checkout to snapshot at HEAD")
    parser.add_argument("--remote-emacs", required=True, help="absolute supported Emacs executable on host")
    parser.add_argument("--engine", required=True, help="expected workstation engine ID")
    parser.add_argument("--audio-output", choices=["null", "device", "pulse"], default="null")
    parser.add_argument("--report-dir", type=Path, required=True, help="new directory for credential-free evidence")
    args = parser.parse_args()
    if args.host.startswith("-") or re.search(r"\s|[\x00-\x1f]", args.host):
        parser.error("host must be a single SSH destination")
    if not args.remote_emacs.startswith("/"):
        parser.error("--remote-emacs must be an absolute remote path")
    args.report_dir.mkdir(parents=True, mode=0o700, exist_ok=False)
    report = {"passed": False, "host": args.host, "program": args.program,
              "engine": args.engine, "audio_output": args.audio_output,
              "windows": args.windows, "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    try:
        check(args, report)
    except (Exception, KeyboardInterrupt) as error:
        # Detailed output is redacted inside check; keep the terminal failure
        # concise and avoid serializing Python locals containing credentials.
        detail = (report.get("error") or type(error).__name__).splitlines()[0]
        print(f"SSH acceptance failed: {detail}\nSee {args.report_dir}/report.json.")
    finally:
        (args.report_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    if report["passed"]:
        print(f"Real SSH acceptance passed ({args.audio_output}, {args.engine}).")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
