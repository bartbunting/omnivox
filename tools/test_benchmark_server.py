#!/usr/bin/env python3
"""Unit and fake-server protocol tests for benchmark_server.py."""

from __future__ import annotations

import base64
import json
from pathlib import Path
import sys
import tempfile
import unittest

import benchmark_server


FAKE_SERVER = r'''
import base64
import json
import sys

parts = {}
selected_engine = "fake"
selected_voice = "default"

def encoded(record):
    payload = json.dumps(record, separators=(",", ":")).encode()
    return base64.b64encode(payload).decode()

def finish(timeline):
    identifier = timeline["dispatch_id"]
    marker = {
        "protocol_version": 2,
        "dispatch_id": identifier,
        "sequence": 1,
        "type": "utterance_started",
        "utterance_id": 1,
        "engine_id": selected_engine,
        "actual_voice": {
            "engine_id": selected_engine,
            "voice_id": selected_voice,
        },
    }
    print("__EMACSVOX_MARKER__ " + encoded(marker), flush=True)
    print(f"__EMACSVOX_TRACKED__ {identifier} completed", flush=True)

for raw_line in sys.stdin:
    line = raw_line.rstrip("\r\n")
    if line.startswith("omnivox_control "):
        request = json.loads(base64.b64decode(line.split(" ", 1)[1]))
        if request["type"] == "capabilities":
            response = {
                "protocol_version": 1,
                "request_id": request["request_id"],
                "type": "capabilities",
                "server_version": "test",
                "features": [
                    "control_v1",
                    "playback_marker_events_v2",
                    "presentation_timeline_v3",
                    "tracked_playback_completion",
                    "runtime_routing_policy",
                    "logical_voice_registration",
                ],
            }
        elif request["type"] == "set_routing_policy":
            selected_engine = request["preferred_engine_ids"][0]
            response = {
                "protocol_version": 1,
                "request_id": request["request_id"],
                "type": "routing_policy_applied",
                "routing_policy": {
                    "routing_policy_generation": request["routing_policy_generation"],
                    "policy": {
                        "preferred_engine_ids": request["preferred_engine_ids"],
                        "fallback_engine_ids": request["fallback_engine_ids"],
                        "disabled_engine_ids": request["disabled_engine_ids"],
                    },
                },
                "logical_voices": {"registry_generation": 0, "bindings": []},
                "inventory_generation": 1,
            }
        else:
            definition = request["definitions"][0]
            selector = definition["preferences"][0]
            selected_engine = selector["engine_id"]
            selected_voice = selector["voice_id"]
            response = {
                "protocol_version": 1,
                "request_id": request["request_id"],
                "type": "logical_voices_registered",
                "inventory_generation": 1,
                "registration": {
                    "registry_generation": request["registry_generation"],
                    "bindings": [
                        {
                            "status": "resolved",
                            "resolution": {
                                "logical_voice_id": definition["id"],
                                "realized": {
                                    "engine_id": selected_engine,
                                    "voice_id": selected_voice,
                                },
                                "requested_engine_id": selected_engine,
                                "degraded": False,
                            },
                        }
                    ],
                },
            }
        print("__OMNIVOX_CONTROL__ " + encoded(response), flush=True)
    elif line.startswith("emacsvox_timeline "):
        finish(json.loads(base64.b64decode(line.split(" ", 1)[1])))
    elif line.startswith("emacsvox_timeline_part "):
        fields = line.split(" ", 7)
        generation = int(fields[2])
        identifier = int(fields[3])
        index = int(fields[4])
        count = int(fields[5])
        key = (generation, identifier)
        fragments = parts.setdefault(key, [None] * count)
        fragments[index] = fields[7]
        if all(fragment is not None for fragment in fragments):
            finish(json.loads(base64.b64decode("".join(fragments))))
            del parts[key]
'''


class BenchmarkServerTests(unittest.TestCase):
    def test_current_report_schema_records_physical_voices(self) -> None:
        self.assertEqual(benchmark_server.REPORT_VERSION, 2)

    def test_nearest_rank_percentiles_are_reproducible(self) -> None:
        values = list(range(1, 101))
        self.assertEqual(benchmark_server.nearest_rank(values, 0.50), 50)
        self.assertEqual(benchmark_server.nearest_rank(values, 0.95), 95)
        self.assertEqual(benchmark_server.nearest_rank(values, 0.99), 99)

    def test_multipart_lines_reassemble_the_original_timeline(self) -> None:
        timeline = benchmark_server.timeline_for_case("multipart", 7, 19)
        lines = benchmark_server.multipart_timeline_lines(timeline, 3)
        fragments = [line.split(" ", 7)[7] for line in lines]
        rebuilt = json.loads(base64.b64decode("".join(fragments)))
        self.assertEqual(rebuilt, timeline)
        self.assertEqual([int(line.split()[4]) for line in lines], [0, 1, 2])

    def test_dense_workload_uses_bounded_source_offsets(self) -> None:
        timeline = benchmark_server.timeline_for_case("dense", 3, 8)
        text_size = len(timeline["spans"][0]["text"].encode("utf-8"))
        offsets = [action["position"]["utf8_offset"] for action in timeline["actions"]]
        self.assertGreater(len(offsets), 10)
        self.assertEqual(offsets, sorted(set(offsets)))
        self.assertTrue(all(0 <= offset <= text_size for offset in offsets))

    def test_rutts_profile_is_koi8_r_and_keeps_utf8_offsets(self) -> None:
        timeline = benchmark_server.timeline_for_case(
            "dense", 3, 8, text_profile="rutts-ru"
        )
        text = timeline["spans"][0]["text"]
        text.encode("koi8_r")
        text_size = len(text.encode("utf-8"))
        offsets = [action["position"]["utf8_offset"] for action in timeline["actions"]]
        self.assertGreater(len(offsets), 8)
        self.assertEqual(offsets, sorted(set(offsets)))
        self.assertTrue(all(0 <= offset <= text_size for offset in offsets))

    def test_provenance_parser_keeps_explicit_build_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            provenance = Path(directory) / "PROVENANCE"
            provenance.write_text(
                "build_id=abc123\nrustc=rustc 1.97.1\n", encoding="utf-8"
            )
            self.assertEqual(
                benchmark_server.read_provenance(str(provenance)),
                {"build_id": "abc123", "rustc": "rustc 1.97.1"},
            )

    def test_session_handles_direct_and_multipart_dispatches(self) -> None:
        session = benchmark_server.ServerSession(
            [sys.executable, "-c", FAKE_SERVER], None, 5.0
        )
        identities = benchmark_server.IdentitySequence()
        try:
            capabilities, _ = session.negotiate(99)
            self.assertEqual(capabilities["server_version"], "test")
            for case in ("word", "multipart"):
                sample = benchmark_server.execute_case(
                    session, case, identities, "fake", 3
                )
                self.assertEqual(sample["status"], "completed")
                self.assertEqual(sample["engine_id"], "fake")
                self.assertEqual(
                    sample["actual_voice"],
                    {"engine_id": "fake", "voice_id": "default"},
                )
                self.assertGreaterEqual(sample["dispatch_to_source_ms"], 0)
        finally:
            session.close()
        self.assertEqual(session.process.returncode, 0)

    def test_session_registers_and_enforces_exact_voice(self) -> None:
        session = benchmark_server.ServerSession(
            [sys.executable, "-c", FAKE_SERVER], None, 5.0
        )
        identities = benchmark_server.IdentitySequence()
        try:
            capabilities, _ = session.negotiate(103)
            response = benchmark_server.configure_exact_voice(
                session, capabilities, "rutts", "female", 104
            )
            self.assertEqual(response["type"], "logical_voices_registered")
            sample = benchmark_server.execute_case(
                session,
                "word",
                identities,
                "rutts",
                3,
                "female",
                "rutts-ru",
            )
            self.assertEqual(
                sample["actual_voice"],
                {"engine_id": "rutts", "voice_id": "female"},
            )
        finally:
            session.close()
        self.assertEqual(session.process.returncode, 0)

    def test_summary_counts_physical_voices(self) -> None:
        summary = benchmark_server.summarize_samples(
            [
                {
                    "engine_id": "rutts",
                    "actual_voice": {"engine_id": "rutts", "voice_id": "male"},
                },
                {
                    "engine_id": "rutts",
                    "actual_voice": {"engine_id": "rutts", "voice_id": "male"},
                },
            ]
        )
        self.assertEqual(summary["voices"], {"rutts": {"male": 2}})

    def test_session_applies_runtime_engine_preference(self) -> None:
        session = benchmark_server.ServerSession(
            [sys.executable, "-c", FAKE_SERVER], None, 5.0
        )
        identities = benchmark_server.IdentitySequence()
        try:
            capabilities, _ = session.negotiate(101)
            response = benchmark_server.configure_preferred_engine(
                session, capabilities, "registered-helper", 102
            )
            self.assertEqual(response["type"], "routing_policy_applied")
            sample = benchmark_server.execute_case(
                session, "character", identities, "registered-helper", 3
            )
            self.assertEqual(sample["engine_id"], "registered-helper")
        finally:
            session.close()
        self.assertEqual(session.process.returncode, 0)


if __name__ == "__main__":
    unittest.main()
