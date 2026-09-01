#!/usr/bin/env python3
"""Unit tests for stress_server.py protocol and process safety checks."""

from __future__ import annotations

import base64
import json
import queue
from types import SimpleNamespace
import time
import unittest

import benchmark_server
import stress_server


def marker(
    identifier: int,
    sequence: int,
    event_type: str,
    engine_id: str = "fake",
    voice_id: str = "voice",
) -> str:
    record = {
        "protocol_version": 2,
        "dispatch_id": identifier,
        "sequence": sequence,
        "type": event_type,
    }
    if event_type == "utterance_started":
        record["engine_id"] = engine_id
        record["actual_voice"] = {"engine_id": engine_id, "voice_id": voice_id}
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


class RecoverySession(FakeSession):
    def __init__(self) -> None:
        super().__init__([])
        self.process = SimpleNamespace(pid=123)
        self.dispatch_count = 0

    def send_timeline(self, timeline: dict[str, object]) -> int:
        self.dispatch_count += 1
        identifier = int(timeline["dispatch_id"])
        if self.dispatch_count == 1:
            self.output.put(
                (
                    time.perf_counter_ns(),
                    f"{benchmark_server.TRACKED_PREFIX}{identifier} cancelled",
                )
            )
            return time.perf_counter_ns()
        if self.dispatch_count == 2:
            engine_id, voice_id = "espeak", "en-us"
        else:
            engine_id, voice_id = "flite", "cmu_us_slt"
        for line in (
            marker(identifier, 1, "semantic_event_reached"),
            marker(identifier, 2, "utterance_started", engine_id, voice_id),
            f"{benchmark_server.TRACKED_PREFIX}{identifier} completed",
        ):
            self.output.put((time.perf_counter_ns(), line))
        return time.perf_counter_ns()

    def send_line(self, line: str) -> int:
        if line != "s":
            raise AssertionError("wrong server command")
        return time.perf_counter_ns()

    def request_control(self, request: dict[str, object]):
        return (
            {
                "protocol_version": 1,
                "request_id": request["request_id"],
                "type": "engine_recovery_probe_requested",
            },
            time.perf_counter_ns(),
        )


class FakeInjector:
    provider = "fake"

    def __init__(self) -> None:
        self.kills = 0

    def resolve(self, server_process_id: int) -> int:
        if server_process_id != 123:
            raise AssertionError("wrong server process")
        return 900 + self.kills + 1

    def terminate(self, target: int) -> int:
        self.kills += 1
        if target != 900 + self.kills:
            raise AssertionError("wrong helper process")
        return target

    def kill(self, server_process_id: int) -> int:
        return self.terminate(self.resolve(server_process_id))


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
        self.assertEqual(
            histories[11]["actual_voice"],
            {"engine_id": "fake", "voice_id": "voice"},
        )

    def test_validates_exact_physical_voice(self) -> None:
        session = FakeSession(
            [
                marker(14, 1, "semantic_event_reached"),
                marker(14, 2, "utterance_started"),
                f"{benchmark_server.TRACKED_PREFIX}14 completed",
            ]
        )
        histories = stress_server.collect_histories(session, {14}, 0)
        stress_server.validate_histories(
            histories, {14: "completed"}, "fake", "voice"
        )
        with self.assertRaisesRegex(RuntimeError, "expected voice"):
            stress_server.validate_histories(
                histories, {14: "completed"}, "fake", "different"
            )

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
            7,
            13,
            "replacement",
            "replaceable",
            "navigation",
            benchmark_server.BENCHMARK_LOGICAL_VOICE_ID,
        )
        self.assertEqual(timeline["generation"], 7)
        self.assertEqual(timeline["replacement_key"], "navigation")
        self.assertEqual(timeline["actions"][0]["type"], "semantic_event")
        self.assertEqual(
            timeline["spans"][0]["logical_voice_id"],
            benchmark_server.BENCHMARK_LOGICAL_VOICE_ID,
        )

    def test_rutts_stress_profile_is_lossless_koi8_r(self) -> None:
        for text in stress_server.STRESS_TEXTS["rutts-ru"].values():
            text.format(number=1).encode("koi8_r")

    def test_dispatch_fault_requires_fallback_then_exact_recovery(self) -> None:
        session = RecoverySession()
        injector = FakeInjector()
        result = stress_server.run_helper_recovery(
            session,
            benchmark_server.IdentitySequence(),
            injector,
            "flite",
            "espeak",
            "cmu_us_slt",
            "english",
            0,
            99,
            "dispatch",
            0,
        )
        self.assertEqual(injector.kills, 1)
        self.assertEqual(result["fault_mode"], "dispatch")
        self.assertEqual(result["fault_dispatch"]["marker_count"], 0)
        self.assertIsNone(result["fault_dispatch"]["actual_voice"])
        self.assertEqual(result["fallback_engine_id"], "espeak")
        self.assertEqual(
            result["fallback_actual_voice"],
            {"engine_id": "espeak", "voice_id": "en-us"},
        )
        self.assertEqual(
            result["recovered_actual_voice"],
            {"engine_id": "flite", "voice_id": "cmu_us_slt"},
        )

    def test_realized_voices_are_unique_and_sorted(self) -> None:
        histories = {
            1: {"actual_voice": {"engine_id": "rutts", "voice_id": "male"}},
            2: {"actual_voice": {"engine_id": "rutts", "voice_id": "female"}},
            3: {"actual_voice": {"engine_id": "rutts", "voice_id": "male"}},
        }
        self.assertEqual(
            stress_server.realized_voices(histories, {1, 2, 3}),
            [
                {"engine_id": "rutts", "voice_id": "female"},
                {"engine_id": "rutts", "voice_id": "male"},
            ],
        )


if __name__ == "__main__":
    unittest.main()
