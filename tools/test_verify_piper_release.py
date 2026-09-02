#!/usr/bin/env python3
"""Regression tests for Piper release model-failure verification."""

from __future__ import annotations

from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import verify_piper_release


class PiperModelFailureVerificationTests(unittest.TestCase):
    def test_fallback_probe_selects_espeak_explicitly(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            working = Path(directory)
            candidate = working / "missing model.onnx"
            environment = {"PATH": "test-path"}
            exact_result = subprocess.CompletedProcess(
                args=[], returncode=1, stdout="", stderr="unavailable"
            )

            with (
                mock.patch.object(
                    verify_piper_release.common,
                    "clean_environment",
                    return_value=environment,
                ),
                mock.patch.object(
                    verify_piper_release.subprocess,
                    "run",
                    return_value=exact_result,
                ) as run,
                mock.patch.object(
                    verify_piper_release.common,
                    "run",
                    return_value="Found 1 voices\nEnglish [espeak:en]",
                ) as common_run,
            ):
                verify_piper_release.verify_unavailable_model_behavior(
                    "omnivox", working, candidate, "missing"
                )

            self.assertEqual(
                run.call_args.args[0],
                ["omnivox", "--engine", "piper", "--list-voices"],
            )
            common_run.assert_called_once_with(
                ["omnivox", "--engine", "espeak", "--list-voices"],
                working,
                environment,
            )
            self.assertEqual(
                environment["OMNIVOX_PIPER_MODEL"], str(candidate.resolve())
            )

    def test_fallback_probe_requires_an_espeak_voice(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            working = Path(directory)
            exact_result = subprocess.CompletedProcess(
                args=[], returncode=1, stdout="", stderr="unavailable"
            )
            with (
                mock.patch.object(
                    verify_piper_release.subprocess,
                    "run",
                    return_value=exact_result,
                ),
                mock.patch.object(
                    verify_piper_release.common,
                    "run",
                    return_value="Found 1 voices\nNative [macos:voice]",
                ),
                self.assertRaisesRegex(
                    verify_piper_release.PiperVerificationError,
                    "eSpeak fallback usable",
                ),
            ):
                verify_piper_release.verify_unavailable_model_behavior(
                    "omnivox", working, working / "missing.onnx", "missing"
                )


if __name__ == "__main__":
    unittest.main()
