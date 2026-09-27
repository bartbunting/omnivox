#!/usr/bin/env python3
"""Input failure checks against a staged payload (run make dev first).

Set OMNIVOX_INPUT_TEST_PROGRAM to another staged executable. When running a
Windows executable from WSL, also set OMNIVOX_INPUT_TEST_WINDOWS=1.
All configuration, audio and processes belong to this isolated test.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PROGRAM = os.environ.get("OMNIVOX_INPUT_TEST_PROGRAM", str(ROOT / "target/debug/omnivox"))
WINDOWS = os.name == "nt" or os.environ.get("OMNIVOX_INPUT_TEST_WINDOWS") == "1"


def native_path(path: Path) -> str:
    if WINDOWS and os.name != "nt":
        return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()
    return str(path)


class InputValidationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="omnivox-input-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        disabled = ["winrt", "macos", "piper", "rhvoice", "flite", "rutts",
                    "tgspeechbox", "mbrola", "eloquence", "dectalk"]
        (self.root / "config.json").write_text(json.dumps({
            "schema": 1,
            "engine_overrides": {engine: {"enabled": False} for engine in disabled},
        }))
        self.environment = {
            key: value for key, value in os.environ.items()
            if not key.startswith(("OMNIVOX_", "EMACSVOX_LOCAL_"))
        }

    def run_omnivox(self, *arguments, input=None, **options):
        return subprocess.run(
            [PROGRAM, "--config-dir", native_path(self.root), "--engine", "espeak",
             "--audio-output", "null", *arguments],
            input=input, text=True, capture_output=True, timeout=15,
            env=self.environment, **options)

    def test_oversized_silence_is_rejected_and_later_speech_completes(self):
        result = self.run_omnivox(
            input="sh 15001\nq Speech after rejection.\nemacsvox_tracked_dispatch 41\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Invalid silence:", result.stderr)
        self.assertIn("__EMACSVOX_TRACKED__ 41 completed", result.stdout)

    @unittest.skipUnless(os.name == "posix" and not WINDOWS, "POSIX memory limit")
    def test_huge_silence_cannot_exhaust_process_memory(self):
        import resource

        def limit_memory():
            resource.setrlimit(resource.RLIMIT_AS, (600 * 1024 * 1024,) * 2)
            resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

        result = self.run_omnivox(
            input="sh 3600000\nsh 4294967295\nq Still speaking.\nemacsvox_tracked_dispatch 42\n",
            preexec_fn=limit_memory)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr.count("Invalid silence:"), 2)
        self.assertIn("__EMACSVOX_TRACKED__ 42 completed", result.stdout)


if __name__ == "__main__":
    unittest.main()
