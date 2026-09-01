#!/usr/bin/env python3
"""Unit tests for stress_server.py protocol and process safety checks."""

from __future__ import annotations

import base64
import json
import queue
import time
import unittest

import benchmark_server
import stress_server


def marker(identifier: int, sequence: int, event_type: str) -> str:
    record = {
        "protocol_version": 2,
        "dispatch_id": identifier,
        "sequence": sequence,
        "type": event_type,
    }
    if event_type == "utterance_started":
        record["engine_id"] = "fake"
    return benchmark_server.MARKER_PREFIX + base64.b64encode(
        json.dumps(record).encode()
    ).decode()


class FakeSession:
    def __init__(self, lines: list[str]) -> None:
        self.timeout = 1.0
        self.output: queue.Queue[tuple[int, str | None]] = queue.Queue()
        for line in lines:
            self.output.put((time.perf_counter_ns(), line))

    def receive_line(self, deadline: float) -> tuple[int, str]:
        del deadline
        return self.output.get_nowait()

    def failure_context(self) -> str:
        return "fake session"


class StressServerTests(unittest.TestCase):
    def test_collects_ordered_markers_and_one_terminal(self) -> None:
        session = FakeSession(
            [
                marker(11, 1, "semantic_event_reached"),
                marker(11, 2, "utterance_started"),
                f"{benchmark_server.TRACKED_PREFIX}11 completed",
            ]
        )
        histories = stress_server.collect_histories(session, {11}, 0)
        stress_server.validate_histories(histories, {11: "completed"}, "fake")
        self.assertEqual(histories[11]["marker_sequences"], [1, 2])
        self.assertEqual(histories[11]["terminal_count"], 1)

    def test_rejects_a_marker_after_terminal(self) -> None:
        session = FakeSession(
            [
                f"{benchmark_server.TRACKED_PREFIX}12 cancelled",
                marker(12, 1, "semantic_event_reached"),
            ]
        )
        with self.assertRaisesRegex(RuntimeError, "marker after terminal"):
            stress_server.collect_histories(session, {12}, 0.1)

    def test_selects_only_a_new_descendant_helper(self) -> None:
        before = {
            10: {"parent": 1, "name": "omnivox-flite-helper.exe", "path": None}
        }
        after = {
            **before,
            20: {"parent": 1, "name": "omnivox.exe", "path": None},
            21: {"parent": 20, "name": "omnivox-flite-helper.exe", "path": None},
            22: {"parent": 1, "name": "omnivox-flite-helper.exe", "path": None},
        }
        self.assertEqual(
            stress_server.select_fault_target(
                before, after, "omnivox-flite-helper.exe", {20}
            ),
            21,
        )

    def test_refuses_ambiguous_helper_targets(self) -> None:
        after = {
            20: {"parent": 1, "name": "omnivox.exe", "path": None},
            21: {"parent": 20, "name": "helper", "path": None},
            22: {"parent": 20, "name": "helper", "path": None},
        }
        with self.assertRaisesRegex(RuntimeError, "expected one new child helper"):
            stress_server.select_fault_target({}, after, "helper", {20})

    def test_semantic_timeline_keeps_domains_explicit(self) -> None:
        timeline = stress_server.semantic_timeline(
            7, 13, "replacement", "replaceable", "navigation"
        )
        self.assertEqual(timeline["generation"], 7)
        self.assertEqual(timeline["replacement_key"], "navigation")
        self.assertEqual(timeline["actions"][0]["type"], "semantic_event")


if __name__ == "__main__":
    unittest.main()
