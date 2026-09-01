#!/usr/bin/env python3
"""Unit and fake-helper tests for helper soak resource reporting."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import process_metrics


FAKE_HELPER = r'''
import base64
import json
import sys

pending = None

def emit(record):
    print(json.dumps({"protocol_version": 4, **record}, separators=(",", ":")), flush=True)

for line in sys.stdin:
    request = json.loads(line)
    identifier = request["request_id"]
    kind = request["type"]
    if kind == "hello":
        emit({"request_id": identifier, "type": "hello", "selected_protocol_version": 4})
    elif kind == "describe":
        emit({
            "request_id": identifier,
            "type": "descriptor",
            "descriptor": {
                "id": "fake",
                "version": "test",
                "default_voice_id": "voice",
                "capabilities": {"acss": {}, "markers": {}},
            },
        })
    elif kind == "synthesize":
        emit({
            "request_id": identifier,
            "type": "synthesis_started",
            "actual_voice_id": request["settings"]["voice_id"],
            "format": {"sample_rate": 16000, "channels": 1, "sample_format": "pcm_s16_le"},
        })
        if "Cancellation" in request["text"] or "отмены" in request["text"]:
            pending = identifier
        else:
            emit({
                "request_id": identifier,
                "type": "audio_chunk",
                "chunk": {"sequence": 0, "data_base64": base64.b64encode(b"\0\0").decode()},
            })
            emit({"request_id": identifier, "type": "synthesis_completed", "frame_count": 1})
    elif kind == "cancel":
        emit({
            "request_id": identifier,
            "type": "cancel_accepted",
            "target_request_id": request["target_request_id"],
        })
        emit({"request_id": pending, "type": "synthesis_cancelled"})
        pending = None
    elif kind == "ping":
        emit({"request_id": identifier, "type": "pong"})
    elif kind == "shutdown":
        emit({"request_id": identifier, "type": "shutting_down"})
        break
'''


class StressHelperTests(unittest.TestCase):
    def test_proc_metrics_observe_current_process(self) -> None:
        metrics = process_metrics.proc_metrics(os.getpid())
        self.assertIsNotNone(metrics)
        assert metrics is not None
        self.assertGreater(metrics["working_set_bytes"], 0)
        self.assertGreaterEqual(metrics["handle_count"], 1)
        self.assertGreaterEqual(metrics["thread_count"], 1)

    def test_resource_summary_records_growth_and_peak(self) -> None:
        summary = process_metrics.summarize_samples(
            [
                {"working_set_bytes": 100, "handle_count": 4},
                {"working_set_bytes": 130, "handle_count": 5},
                {"working_set_bytes": 120, "handle_count": 4},
            ]
        )
        self.assertEqual(
            summary["metrics"]["working_set_bytes"],
            {
                "first": 100,
                "last": 120,
                "minimum": 100,
                "maximum": 130,
                "growth": 20,
            },
        )

    def test_windows_style_executable_name_is_path_independent(self) -> None:
        self.assertEqual(
            process_metrics.executable_name(r"C:\Tools\omnivox-helper.exe"),
            "omnivox-helper.exe",
        )

    def test_process_tree_aggregation_excludes_unrelated_processes(self) -> None:
        snapshot = {
            10: {
                "parent": 1,
                "name": "omnivox.exe",
                "working_set_bytes": 100,
                "handle_count": 4,
            },
            11: {
                "parent": 10,
                "name": "helper.exe",
                "working_set_bytes": 50,
                "handle_count": 2,
            },
            12: {
                "parent": 11,
                "name": "worker.exe",
                "working_set_bytes": 25,
                "handle_count": 1,
            },
            20: {
                "parent": 1,
                "name": "helper.exe",
                "working_set_bytes": 999,
                "handle_count": 99,
            },
        }
        aggregate = process_metrics.aggregate_tree(10, snapshot)
        self.assertEqual(
            process_metrics.tree_process_ids(10, snapshot), {10, 11, 12}
        )
        self.assertIsNotNone(aggregate)
        assert aggregate is not None
        self.assertEqual(aggregate["process_count"], 3)
        self.assertEqual(aggregate["working_set_bytes"], 175)
        self.assertEqual(aggregate["handle_count"], 7)
        self.assertEqual(aggregate["by_name"]["helper.exe"]["process_count"], 1)

    def test_fake_helper_soak_writes_metrics_and_periodic_cancellations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report_path = Path(directory) / "soak.json"
            completed = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("stress_helper.py")),
                    sys.executable,
                    "--engine-id",
                    "fake",
                    "--voice-id",
                    "voice",
                    "--iterations",
                    "2",
                    "--cancel-every",
                    "1",
                    "--health-every",
                    "1",
                    "--resource-sample-every",
                    "1",
                    "--helper-arg=-c",
                    f"--helper-arg={FAKE_HELPER}",
                    "--json-output",
                    str(report_path),
                ],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=20,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(report["result"]["status"], "completed")
        self.assertEqual(report["result"]["syntheses"], 2)
        self.assertEqual(report["result"]["cancellation_probes"], 2)
        self.assertEqual(report["configuration"]["helper_name"], Path(sys.executable).name)
        self.assertNotIn("helper", report["configuration"])
        self.assertEqual(report["resources"]["provider"], "procfs")
        self.assertGreaterEqual(report["resources"]["summary"]["sample_count"], 3)
        self.assertGreaterEqual(
            report["resources"]["steady_state_summary"]["sample_count"], 2
        )


if __name__ == "__main__":
    unittest.main()
