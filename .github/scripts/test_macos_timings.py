#!/usr/bin/env python3
"""Exercise the CI comparison with real benchmark clients and fake servers."""

from __future__ import annotations

import contextlib
import io
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

import macos_timings

TOOLS = Path(__file__).resolve().parents[2] / "tools"
sys.path.insert(0, str(TOOLS))
from test_benchmark_server import FAKE_SERVER


class MacOsTimingTests(unittest.TestCase):
    def prepare(self, root: Path, wrong_voice: bool = False) -> tuple[Path, Path]:
        sources = root / "baseline checkout", root / "candidate checkout"
        for index, source in enumerate(sources):
            binary = source / "target/release/omnivox"
            binary.parent.mkdir(parents=True)
            server = FAKE_SERVER
            if wrong_voice and index == 1:
                server = server.replace(
                    '"voice_id": selected_voice,', '"voice_id": "wrong-voice",'
                )
            binary.write_text(
                f"#!{sys.executable}\n"
                "import sys\n"
                f"print('build-{index} server evidence', file=sys.stderr, flush=True)\n"
                "if '--list-voices' in sys.argv:\n"
                f"    print('Samantha [{macos_timings.VOICES['macos']}]')\n"
                "    raise SystemExit(0)\n" + server,
                encoding="utf-8",
            )
            binary.chmod(0o755)
        candidate_tools = sources[1] / "tools"
        candidate_tools.mkdir()
        shutil.copyfile(TOOLS / "benchmark_server.py", candidate_tools / "benchmark_server.py")
        return sources

    def compare(self, sources: tuple[Path, Path], output: Path) -> int:
        with patch.object(macos_timings.subprocess, "check_output", return_value="a" * 40):
            with contextlib.redirect_stdout(io.StringIO()):
                return macos_timings.compare(*sources, output)

    def test_freezes_a_common_samantha_identity_after_first_use(self) -> None:
        compact, super_compact = macos_timings.SAMANTHA_IDS
        self.assertEqual(
            macos_timings.select_samantha([f"[{super_compact}]", f"[{super_compact}]"]),
            super_compact,
        )
        self.assertEqual(
            macos_timings.select_samantha([f"[{compact}]", f"[{compact}] [{super_compact}]"]),
            compact,
        )
        with self.assertRaisesRegex(RuntimeError, "common supported Samantha"):
            macos_timings.select_samantha([f"[{compact}]", f"[{super_compact}]"])
        with self.assertRaisesRegex(RuntimeError, "common supported Samantha"):
            macos_timings.select_samantha(["Eddy", "Eddy"])

    def test_both_binaries_are_measured_with_exact_voices_and_logs_retained(self) -> None:
        with tempfile.TemporaryDirectory(prefix="macos timings ") as temporary:
            root = Path(temporary)
            output = root / "reports with spaces"
            self.assertEqual(self.compare(self.prepare(root), output), 0)
            runs = json.loads((output / "runs.json").read_text())
            self.assertEqual(len(runs), 8)
            for engine in macos_timings.VOICES:
                self.assertEqual(
                    [run["build"] for run in runs if run["engine"] == engine],
                    ["baseline", "candidate", "candidate", "baseline"],
                )
            for run in runs:
                directory = output / run["id"]
                report = json.loads((directory / "report.json").read_text())
                self.assertFalse(report["measurement"]["acoustic_onset_measured"])
                self.assertEqual(report["configuration"]["audio_output"], "null")
                for cases in report["results"].values():
                    for result in cases.values():
                        self.assertEqual(len(result["samples"]), 5)
                        for sample in result["samples"]:
                            self.assertEqual(sample["engine_id"], run["engine"])
                            self.assertEqual(
                                sample["actual_voice"]["voice_id"],
                                macos_timings.VOICES[run["engine"]],
                            )
                logs = list(directory.glob("server-*.log"))
                self.assertEqual(len(logs), 16)  # 15 cold processes and one warm.
                build_index = 0 if run["build"] == "baseline" else 1
                self.assertTrue(all(f"build-{build_index} server evidence" in log.read_text() for log in logs))
            summary = (output / "summary.md").read_text()
            self.assertIn("| macos | warm | character | 10 / 10 |", summary)
            self.assertNotIn("INCOMPLETE", summary)

    def test_wrong_voice_fails_comparison_and_preserves_partial_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "reports"
            self.assertEqual(self.compare(self.prepare(root, wrong_voice=True), output), 1)
            runs = json.loads((output / "runs.json").read_text())
            self.assertEqual(len(runs), 8)
            self.assertTrue(all(run["exit_code"] != 0 for run in runs if run["build"] == "candidate"))
            self.assertTrue(all(run["exit_code"] == 0 for run in runs if run["build"] == "baseline"))
            self.assertTrue((output / "2-macos-baseline/report.json").exists())
            self.assertTrue(list((output / "1-macos-candidate").glob("server-*.log")))
            summary = (output / "summary.md").read_text()
            self.assertIn("INCOMPLETE", summary)
            self.assertNotIn("| Engine |", summary)


if __name__ == "__main__":
    unittest.main()
