#!/usr/bin/env python3
"""Local failure-path tests for the opt-in SSH acceptance harness."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

from check_remote_ssh import LoggedProcess, REMOTE_RUNNER


class RemoteHarnessTests(unittest.TestCase):
    def test_early_exit_reports_output_and_closes_pipes(self):
        process = LoggedProcess([sys.executable, "-c", "print('connection refused')"])
        try:
            with self.assertRaisesRegex(RuntimeError, "connection refused"):
                process.expect("ready", timeout=3)
        finally:
            process.close()
        self.assertFalse(process.reader.is_alive())
        self.assertTrue(process.process.stdout.closed)
        self.assertTrue(process.process.stdin.closed)

    def test_unsuccessful_service_shutdown_is_not_a_pass(self):
        process = LoggedProcess([sys.executable, "-c",
                                 "import sys; sys.stdin.readline(); sys.exit(3)"])
        with self.assertRaisesRegex(RuntimeError, "Service exited unsuccessfully: 3"):
            process.close(quit_service=True)
        self.assertFalse(process.reader.is_alive())
        self.assertTrue(process.process.stdout.closed)

    def run_supervisor(self, child_code, close_input):
        with tempfile.TemporaryDirectory(prefix="omnivox-ssh-supervisor-test-") as directory:
            root = Path(directory)
            child = root / "fake-emacs"
            child.write_text(f"#!{sys.executable}\n" + child_code)
            child.chmod(0o700)
            process = subprocess.Popen(
                [sys.executable, "-u", "-c", REMOTE_RUNNER, directory, str(child), "1", "espeak"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                deadline = time.monotonic() + 5
                while not (root / "home/child.pid").exists():
                    if process.poll() is not None or time.monotonic() > deadline:
                        self.fail("Remote supervisor did not start its child")
                    time.sleep(0.01)
                pid = int((root / "home/child.pid").read_text())
                if close_input:
                    process.stdin.close()
                    process.stdin = None
                # Keep stdin open on normal completion: the old buffered
                # daemon reader caused a fatal interpreter-shutdown error.
                process.wait(timeout=5)
                output, errors = process.communicate(timeout=2)
                self.assertNotIn("Fatal Python error", errors)
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
                return process.returncode, output, errors
            finally:
                if process.poll() is None:
                    process.stdin.close()
                    process.stdin = None
                    process.wait(timeout=5)
                for stream in (process.stdin, process.stdout, process.stderr):
                    if stream:
                        stream.close()

    @unittest.skipUnless(hasattr(os, "killpg"), "remote supervisor requires POSIX")
    def test_client_connection_eof_retires_remote_emacs(self):
        code = ("import os, pathlib, time\n"
                "pathlib.Path(os.environ['HOME'], 'child.pid').write_text(str(os.getpid()))\n"
                "time.sleep(60)\n")
        result, _, errors = self.run_supervisor(code, close_input=True)
        self.assertNotEqual(result, 0)
        self.assertEqual(errors, "")

    @unittest.skipUnless(hasattr(os, "killpg"), "remote supervisor requires POSIX")
    def test_completed_emacs_does_not_wait_for_ssh_stdin(self):
        code = ("import os, pathlib\n"
                "pathlib.Path(os.environ['HOME'], 'child.pid').write_text(str(os.getpid()))\n"
                "print('complete')\n")
        result, output, errors = self.run_supervisor(code, close_input=False)
        self.assertEqual(result, 0, errors)
        self.assertEqual(output, "complete\n")


if __name__ == "__main__":
    unittest.main()
