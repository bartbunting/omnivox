"""Test-only WSL loopback relay using an existing Windows C# compiler.

Production remote use connects via Windows OpenSSH, not this relay.
"""
import contextlib
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading


class WindowsLoopbackBridge:
    def __init__(self, target_port):
        self.target_port = target_port
        win_temp = subprocess.check_output(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
             "[System.IO.Path]::GetTempPath()"], text=True).strip()
        directory = subprocess.check_output(["wslpath", "-u", win_temp], text=True).strip()
        self.temp = tempfile.TemporaryDirectory(prefix="omnivox-bridge-test-", dir=directory)
        self.program = Path(self.temp.name) / "bridge.exe"
        native = lambda path: subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()
        try:
            subprocess.run(
                ["/mnt/c/Windows/Microsoft.NET/Framework64/v4.0.30319/csc.exe",
                 "/nologo", "/target:exe", "/out:" + native(self.program),
                 native(Path(__file__).with_suffix(".cs"))], check=True,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30,
                cwd=directory)
        except Exception:
            self.temp.cleanup()
            raise
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.listener.settimeout(0.1)
        self.port = self.listener.getsockname()[1]
        self.stopped = threading.Event()
        self.connections = []
        self.acceptor = threading.Thread(target=self.accept)
        self.acceptor.start()

    def accept(self):
        while not self.stopped.is_set():
            try:
                peer, _ = self.listener.accept()
            except TimeoutError:
                continue
            worker = threading.Thread(target=self.relay, args=(peer,))
            self.connections.append(worker)
            worker.start()

    def relay(self, peer):
        process = subprocess.Popen([str(self.program), str(self.target_port)],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL)

        def write_input():
            try:
                while data := peer.recv(8192):
                    process.stdin.write(data)
                    process.stdin.flush()
            except OSError:
                pass
            finally:
                with contextlib.suppress(OSError):
                    process.stdin.close()

        writer = threading.Thread(target=write_input)
        writer.start()
        try:
            while data := os.read(process.stdout.fileno(), 8192):
                peer.sendall(data)
        except OSError:
            pass
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
            with contextlib.suppress(OSError):
                peer.shutdown(socket.SHUT_RDWR)
            writer.join(timeout=5)
            peer.close()
            process.stdout.close()
            if writer.is_alive():
                raise RuntimeError("test relay input thread did not terminate")

    def close(self):
        self.stopped.set()
        self.acceptor.join(timeout=5)
        self.listener.close()
        for worker in self.connections:
            worker.join(timeout=10)
            if worker.is_alive():
                raise RuntimeError("test relay connection did not terminate")
        self.temp.cleanup()
