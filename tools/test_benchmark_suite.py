#!/usr/bin/env python3
"""Unit tests for seeded benchmark suite planning."""

from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest

import benchmark_suite
from test_benchmark_server import FAKE_SERVER


class BenchmarkSuiteTests(unittest.TestCase):
    def write_plan(self, directory: str, **updates: object) -> Path:
        plan = {
            "plan_version": 1,
            "server": "../server",
            "seed": 42,
            "repeats": 2,
            "benchmark": {"iterations": 3, "warmups": 1, "null_audio": True},
            "runs": [
                {
                    "id": "rutts-male",
                    "engine": "rutts",
                    "expected_engine_id": "rutts",
                    "voice_id": "male",
                    "text_profile": "rutts-ru",
                },
                {
                    "id": "winrt",
                    "engine": "native",
                    "expected_engine_id": "winrt",
                },
                {
                    "id": "dectalk",
                    "engine": "native",
                    "preferred_engine_id": "dectalk",
                    "expected_engine_id": "dectalk",
                },
            ],
        }
        plan.update(updates)
        path = Path(directory) / "plan.json"
        path.write_text(json.dumps(plan), encoding="utf-8")
        return path

    def test_plan_paths_are_relative_to_the_plan(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write_plan(directory, provenance="../PROVENANCE")
            plan = benchmark_suite.load_plan(path)
            parent = Path(directory).resolve().parent
            self.assertEqual(plan["server"], str(parent / "server"))
            self.assertEqual(plan["provenance"], str(parent / "PROVENANCE"))

    def test_seeded_schedule_is_reproducible_and_complete(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            plan = benchmark_suite.load_plan(self.write_plan(directory))
        first = benchmark_suite.execution_schedule(
            plan["runs"], plan["repeats"], plan["seed"]
        )
        second = benchmark_suite.execution_schedule(
            plan["runs"], plan["repeats"], plan["seed"]
        )
        self.assertEqual(
            [(repeat, order, run["id"]) for repeat, order, run in first],
            [(repeat, order, run["id"]) for repeat, order, run in second],
        )
        for repeat in (1, 2):
            self.assertCountEqual(
                [run["id"] for value, _, run in first if value == repeat],
                ["rutts-male", "winrt", "dectalk"],
            )

    def test_command_carries_strict_voice_and_common_configuration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            plan = benchmark_suite.load_plan(self.write_plan(directory))
            output = Path(directory) / "report.json"
            command = benchmark_suite.benchmark_command(
                plan, plan["runs"][0], output
            )
        self.assertIn("--voice-id", command)
        self.assertIn("male", command)
        self.assertIn("--text-profile", command)
        self.assertIn("rutts-ru", command)
        self.assertIn("--iterations", command)
        self.assertIn("3", command)
        self.assertIn("--null-audio", command)
        self.assertEqual(command[-1], "replacement")

    def test_rejects_unsafe_duplicate_and_unroutable_runs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write_plan(directory, runs=[{"id": "../escape"}])
            with self.assertRaisesRegex(ValueError, "safe portable identifier"):
                benchmark_suite.load_plan(path)

            path = self.write_plan(
                directory,
                runs=[{"id": "same"}, {"id": "same"}],
            )
            with self.assertRaisesRegex(ValueError, "duplicate run ID"):
                benchmark_suite.load_plan(path)

            path = self.write_plan(
                directory,
                runs=[{"id": "voice", "voice_id": "male"}],
            )
            with self.assertRaisesRegex(ValueError, "requires expected_engine_id"):
                benchmark_suite.load_plan(path)

    def test_rejects_unbounded_plan_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write_plan(directory, repeats=101)
            with self.assertRaisesRegex(ValueError, "repeats"):
                benchmark_suite.load_plan(path)
            path = self.write_plan(
                directory,
                benchmark={"timeout_seconds": 3601},
            )
            with self.assertRaisesRegex(ValueError, "timeout_seconds"):
                benchmark_suite.load_plan(path)
            path = self.write_plan(
                directory,
                benchmark={"null_audio": "yes"},
            )
            with self.assertRaisesRegex(ValueError, "null_audio"):
                benchmark_suite.load_plan(path)

    def test_runs_repeated_fake_server_reports_and_writes_complete_index(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            plan_path = self.write_plan(
                directory,
                server=sys.executable,
                server_args=["-c", FAKE_SERVER],
                repeats=2,
                benchmark={
                    "mode": "warm",
                    "iterations": 1,
                    "warmups": 0,
                },
                runs=[
                    {
                        "id": "fake",
                        "expected_engine_id": "fake",
                        "cases": ["character"],
                    }
                ],
            )
            output = Path(directory) / "evidence"
            benchmark_suite.run_suite(plan_path, output)
            index = json.loads((output / "suite.json").read_text(encoding="utf-8"))
            self.assertEqual(index["status"], "complete")
            self.assertEqual(len(index["runs"]), 2)
            self.assertEqual(
                [entry["repeat"] for entry in index["runs"]], [1, 2]
            )
            for entry in index["runs"]:
                report = output / entry["report"]
                self.assertTrue(report.is_file())
                self.assertEqual(len(entry["sha256"]), 64)
                self.assertEqual(entry["benchmark_report_version"], 2)


if __name__ == "__main__":
    unittest.main()
