#!/usr/bin/env python3
"""Benchmark cold and warm Omnivox speech lifecycle latency.

The harness uses the public control, presentation-timeline, marker, and tracked
completion protocols.  It measures client-observed process readiness, first
playback-source consumption, and terminal completion with a monotonic clock.
It does not claim to measure physical acoustic onset.
"""

from __future__ import annotations

import argparse
import base64
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import platform
import queue
import subprocess
import sys
import threading
import time
from typing import Any, Iterable


CONTROL_PREFIX = "__OMNIVOX_CONTROL__ "
MARKER_PREFIX = "__EMACSVOX_MARKER__ "
TRACKED_PREFIX = "__EMACSVOX_TRACKED__ "
REPORT_VERSION = 2
DEFAULT_CASES = ("character", "word", "line", "dense", "multipart", "replacement")
DEFAULT_TEXT_PROFILE = "english"
BENCHMARK_LOGICAL_VOICE_ID = "benchmark-exact-voice"
WORKLOAD_TEXTS = {
    "english": {
        "character": "A",
        "word": "latency",
        "line": "The quick brown fox checks interactive speech latency.",
        "dense": (
            "Dense presentation actions follow every word while one short speech "
            "span keeps the workload useful for interactive latency measurement."
        ),
        "multipart": "Multipart presentation assembly checks one short spoken line.",
        "replacement": (
            "Rapid replacement should retire this deliberately longer navigation "
            "message before stale audio can continue through the mixer."
        ),
    },
    "rutts-ru": {
        "character": "Я",
        "word": "задержка",
        "line": "Быстрая речь проверяет задержку интерактивного синтеза.",
        "dense": (
            "Плотные действия следуют за каждым словом, пока короткая фраза "
            "проверяет задержку интерактивной речи."
        ),
        "multipart": "Составная передача проверяет одну короткую фразу.",
        "replacement": (
            "Быстрая замена должна отменить это длинное сообщение навигации, "
            "прежде чем устаревший звук продолжит воспроизведение."
        ),
    },
}
REQUIRED_FEATURES = {
    "control_v1",
    "playback_marker_events_v2",
    "presentation_timeline_v3",
    "tracked_playback_completion",
}


def encode_record(record: dict[str, Any]) -> str:
    payload = json.dumps(record, ensure_ascii=False, separators=(",", ":")).encode(
        "utf-8"
    )
    return base64.b64encode(payload).decode("ascii")


def decode_record(payload: str) -> dict[str, Any]:
    try:
        decoded = base64.b64decode(payload, validate=True)
        record = json.loads(decoded)
    except (ValueError, TypeError, json.JSONDecodeError) as error:
        raise RuntimeError("server emitted an invalid Base64-JSON record") from error
    if not isinstance(record, dict):
        raise RuntimeError("server emitted a non-object JSON record")
    return record


def milliseconds(later: int, earlier: int) -> float:
    return (later - earlier) / 1_000_000.0


def nearest_rank(values: Iterable[float], percentile: float) -> float:
    ordered = sorted(values)
    if not ordered:
        raise ValueError("cannot calculate a percentile without samples")
    rank = max(1, math.ceil(percentile * len(ordered)))
    return ordered[rank - 1]


def summarize_samples(samples: list[dict[str, Any]]) -> dict[str, Any]:
    metric_names = (
        "process_start_to_ready_ms",
        "process_start_to_source_ms",
        "dispatch_to_source_ms",
        "dispatch_to_terminal_ms",
        "source_to_terminal_ms",
        "cancel_terminal_ms",
    )
    summaries: dict[str, Any] = {}
    for metric in metric_names:
        values = [
            float(sample[metric])
            for sample in samples
            if sample.get(metric) is not None
        ]
        if not values:
            continue
        summaries[metric] = {
            "count": len(values),
            "minimum": min(values),
            "p50": nearest_rank(values, 0.50),
            "p95": nearest_rank(values, 0.95),
            "p99": nearest_rank(values, 0.99),
            "maximum": max(values),
        }
    engines: dict[str, int] = {}
    voices: dict[str, dict[str, int]] = {}
    for sample in samples:
        engine = sample.get("engine_id")
        if engine:
            engines[engine] = engines.get(engine, 0) + 1
        actual_voice = sample.get("actual_voice")
        if isinstance(actual_voice, dict):
            voice_engine = actual_voice.get("engine_id")
            voice_id = actual_voice.get("voice_id")
            if isinstance(voice_engine, str) and isinstance(voice_id, str):
                engine_voices = voices.setdefault(voice_engine, {})
                engine_voices[voice_id] = engine_voices.get(voice_id, 0) + 1
    return {
        "sample_count": len(samples),
        "engines": engines,
        "voices": voices,
        "metrics": summaries,
    }


def read_provenance(path: str) -> dict[str, str]:
    contents = Path(path).read_bytes()
    if len(contents) > 65_536:
        raise ValueError("provenance file exceeds the 64 KiB report limit")
    fields = {}
    for line in contents.decode("utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key:
            raise ValueError(f"invalid provenance line: {line!r}")
        fields[key] = value
    return fields


def configure_preferred_engine(
    session: "ServerSession",
    capabilities: dict[str, Any],
    engine_id: str,
    request_id: int,
) -> dict[str, Any]:
    """Make ENGINE_ID the sole runtime preference for SESSION."""
    if "runtime_routing_policy" not in capabilities.get("features", []):
        raise RuntimeError(
            "server does not advertise runtime_routing_policy required by "
            "--preferred-engine-id"
        )
    response, _ = session.request_control(
        {
            "protocol_version": 1,
            "request_id": request_id,
            "type": "set_routing_policy",
            "routing_policy_generation": 1,
            "preferred_engine_ids": [engine_id],
            "fallback_engine_ids": [],
            "disabled_engine_ids": [],
        }
    )
    applied = response.get("routing_policy", {})
    policy = applied.get("policy", {}) if isinstance(applied, dict) else {}
    if (
        response.get("type") != "routing_policy_applied"
        or applied.get("routing_policy_generation") != 1
        or policy.get("preferred_engine_ids") != [engine_id]
    ):
        raise RuntimeError(
            f"server rejected benchmark routing preference {engine_id!r}: {response}"
        )
    return response


def configure_exact_voice(
    session: "ServerSession",
    capabilities: dict[str, Any],
    engine_id: str,
    voice_id: str,
    request_id: int,
) -> dict[str, Any]:
    """Register one strict exact physical voice for benchmark timelines."""
    if "logical_voice_registration" not in capabilities.get("features", []):
        raise RuntimeError(
            "server does not advertise logical_voice_registration required by "
            "--voice-id"
        )
    response, _ = session.request_control(
        {
            "protocol_version": 1,
            "request_id": request_id,
            "type": "register_logical_voices",
            "registry_generation": 1,
            "definitions": [
                {
                    "id": BENCHMARK_LOGICAL_VOICE_ID,
                    "language": None,
                    "preferences": [
                        {
                            "kind": "exact",
                            "engine_id": engine_id,
                            "voice_id": voice_id,
                        }
                    ],
                    "acss": {},
                    "effects": {},
                }
            ],
            "fallback_policy": {
                "preferred_engines": [],
                "allow_same_language_on_requested_engine": False,
                "global_default": None,
                "fallback_engines": [],
            },
        }
    )
    registration = response.get("registration", {})
    bindings = (
        registration.get("bindings", []) if isinstance(registration, dict) else []
    )
    binding = (
        bindings[0] if len(bindings) == 1 and isinstance(bindings[0], dict) else {}
    )
    resolution = binding.get("resolution", {}) if isinstance(binding, dict) else {}
    realized = resolution.get("realized", {}) if isinstance(resolution, dict) else {}
    if (
        response.get("type") != "logical_voices_registered"
        or registration.get("registry_generation") != 1
        or binding.get("status") != "resolved"
        or realized.get("engine_id") != engine_id
        or realized.get("voice_id") != voice_id
    ):
        raise RuntimeError(
            f"server rejected benchmark exact voice {engine_id}/{voice_id}: {response}"
        )
    return response


def multipart_timeline_lines(timeline: dict[str, Any], part_count: int) -> list[str]:
    if part_count < 2:
        raise ValueError("multipart timelines require at least two parts")
    encoded = encode_record(timeline)
    if len(encoded) < part_count:
        raise ValueError("timeline is too small for the requested part count")
    boundaries = [len(encoded) * index // part_count for index in range(part_count + 1)]
    decoded_bytes = len(
        json.dumps(timeline, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    )
    return [
        "emacsvox_timeline_part "
        f"3 {timeline['generation']} {timeline['dispatch_id']} "
        f"{index} {part_count} {decoded_bytes} "
        f"{encoded[boundaries[index]:boundaries[index + 1]]}"
        for index in range(part_count)
    ]


class ServerSession:
    """One running Omnivox process with timestamped protocol output."""

    def __init__(
        self,
        command: list[str],
        engine: str | None,
        timeout: float,
    ) -> None:
        environment = os.environ.copy()
        if engine:
            environment["OMNIVOX_ENGINE"] = engine
        self.command = command
        self.timeout = timeout
        self.started_at_ns = time.perf_counter_ns()
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            env=environment,
        )
        self.output: queue.Queue[tuple[int, str | None]] = queue.Queue()
        self.stderr_lines: list[str] = []
        self.stdout_thread = threading.Thread(target=self._read_stdout, daemon=True)
        self.stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
        self.stdout_thread.start()
        self.stderr_thread.start()

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.output.put((time.perf_counter_ns(), line.rstrip("\r\n")))
        self.output.put((time.perf_counter_ns(), None))

    def _read_stderr(self) -> None:
        assert self.process.stderr is not None
        for line in self.process.stderr:
            if len(self.stderr_lines) < 500:
                self.stderr_lines.append(line.rstrip())

    def failure_context(self) -> str:
        status = self.process.poll()
        tail = self.stderr_lines[-10:]
        return f"server status={status}; stderr tail={tail}"

    def send_line(self, line: str) -> int:
        if self.process.poll() is not None:
            raise RuntimeError(f"server exited before input: {self.failure_context()}")
        assert self.process.stdin is not None
        sent_at = time.perf_counter_ns()
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()
        return sent_at

    def receive_line(self, deadline: float) -> tuple[int, str]:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError(f"timed out waiting for server output; {self.failure_context()}")
        try:
            observed_at, line = self.output.get(timeout=remaining)
        except queue.Empty as error:
            raise RuntimeError(
                f"timed out waiting for server output; {self.failure_context()}"
            ) from error
        if line is None:
            raise RuntimeError(f"server closed standard output; {self.failure_context()}")
        return observed_at, line

    def negotiate(self, request_id: int) -> tuple[dict[str, Any], int]:
        request = {
            "protocol_version": 1,
            "request_id": request_id,
            "type": "capabilities",
        }
        response, observed_at = self.request_control(request)
        if response.get("type") != "capabilities":
            raise RuntimeError(f"capability request failed: {response}")
        missing = REQUIRED_FEATURES - set(response.get("features", []))
        if missing:
            raise RuntimeError(f"server omits required benchmark features: {sorted(missing)}")
        return response, observed_at

    def request_control(self, request: dict[str, Any]) -> tuple[dict[str, Any], int]:
        request_id = request.get("request_id")
        if not isinstance(request_id, int):
            raise ValueError("control request requires an integer request_id")
        self.send_line(f"omnivox_control {encode_record(request)}")
        deadline = time.monotonic() + self.timeout
        while True:
            observed_at, line = self.receive_line(deadline)
            if not line.startswith(CONTROL_PREFIX):
                continue
            response = decode_record(line[len(CONTROL_PREFIX) :])
            if response.get("request_id") != request_id:
                continue
            return response, observed_at

    def send_timeline(self, timeline: dict[str, Any], multipart_parts: int = 0) -> int:
        if multipart_parts:
            sent_at = 0
            for line in multipart_timeline_lines(timeline, multipart_parts):
                sent_at = self.send_line(line)
            return sent_at
        return self.send_line(f"emacsvox_timeline {encode_record(timeline)}")

    def wait_for_dispatches(self, identifiers: set[int]) -> dict[int, dict[str, Any]]:
        results = {
            identifier: {
                "source_at_ns": None,
                "terminal_at_ns": None,
                "status": None,
                "engine_id": None,
                "actual_voice": None,
            }
            for identifier in identifiers
        }
        deadline = time.monotonic() + self.timeout
        while any(result["terminal_at_ns"] is None for result in results.values()):
            observed_at, line = self.receive_line(deadline)
            if line.startswith(MARKER_PREFIX):
                event = decode_record(line[len(MARKER_PREFIX) :])
                identifier = event.get("dispatch_id")
                if (
                    identifier in results
                    and event.get("type") == "utterance_started"
                    and results[identifier]["source_at_ns"] is None
                ):
                    results[identifier]["source_at_ns"] = observed_at
                    results[identifier]["engine_id"] = event.get("engine_id")
                    results[identifier]["actual_voice"] = event.get("actual_voice")
                continue
            if not line.startswith(TRACKED_PREFIX):
                continue
            fields = line.split()
            if len(fields) != 3:
                raise RuntimeError(f"malformed tracked terminal record: {line!r}")
            try:
                identifier = int(fields[1])
            except ValueError as error:
                raise RuntimeError(f"malformed tracked dispatch ID: {line!r}") from error
            if identifier in results:
                results[identifier]["terminal_at_ns"] = observed_at
                results[identifier]["status"] = fields[2]
        return results

    def close(self) -> None:
        if self.process.poll() is None and self.process.stdin is not None:
            self.process.stdin.close()
        if self.process.poll() is None:
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
        self.stdout_thread.join(timeout=1)
        self.stderr_thread.join(timeout=1)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if stream is not None and not stream.closed:
                stream.close()


class IdentitySequence:
    def __init__(self) -> None:
        self.generation = 1
        self.dispatch_id = 1_000_000

    def next(self) -> tuple[int, int]:
        generation = self.generation
        dispatch_id = self.dispatch_id
        self.generation += 1
        self.dispatch_id += 1
        return generation, dispatch_id


def dense_actions(text: str) -> list[dict[str, Any]]:
    actions = []
    offset = 0
    for index, word in enumerate(text.split()):
        actions.append(
            {
                "id": f"dense-{index + 1}",
                "position": {
                    "position": "text_offset",
                    "span_id": 1,
                    "utf8_offset": offset,
                    "affinity": "before",
                },
                "lifecycle_anchor": "run",
                "type": "semantic_event",
            }
        )
        offset += len(word.encode("utf-8")) + 1
    return actions


def timeline_for_case(
    case: str,
    generation: int,
    dispatch_id: int,
    replacement_key: str | None = None,
    text_profile: str = DEFAULT_TEXT_PROFILE,
    logical_voice_id: str | None = None,
) -> dict[str, Any]:
    texts = WORKLOAD_TEXTS.get(text_profile)
    if texts is None:
        raise ValueError(f"unknown benchmark text profile: {text_profile}")
    if case not in texts:
        raise ValueError(f"unknown benchmark case: {case}")
    text = texts[case]
    span: dict[str, Any] = {"id": 1, "text": text}
    if logical_voice_id:
        span["logical_voice_id"] = logical_voice_id
    timeline = {
        "protocol_version": 3,
        "generation": generation,
        "dispatch_id": dispatch_id,
        "delivery_policy": "replaceable" if replacement_key else "ordered",
        "spans": [span],
        "actions": dense_actions(text) if case == "dense" else [],
    }
    if replacement_key:
        timeline["replacement_key"] = replacement_key
    return timeline


def require_completed(result: dict[str, Any], identifier: int) -> None:
    if result["status"] != "completed":
        raise RuntimeError(f"dispatch {identifier} ended with {result['status']!r}")
    if result["source_at_ns"] is None:
        raise RuntimeError(f"dispatch {identifier} completed without a source marker")


def execute_case(
    session: ServerSession,
    case: str,
    identities: IdentitySequence,
    expected_engine_id: str | None,
    replacement_burst: int,
    expected_voice_id: str | None = None,
    text_profile: str = DEFAULT_TEXT_PROFILE,
) -> dict[str, Any]:
    logical_voice_id = BENCHMARK_LOGICAL_VOICE_ID if expected_voice_id else None
    if case == "replacement":
        sent: dict[int, int] = {}
        for _ in range(replacement_burst):
            generation, identifier = identities.next()
            timeline = timeline_for_case(
                case,
                generation,
                identifier,
                "benchmark-navigation",
                text_profile,
                logical_voice_id,
            )
            sent[identifier] = session.send_timeline(timeline)
        results = session.wait_for_dispatches(set(sent))
        winner = max(sent)
        require_completed(results[winner], winner)
        stale = [identifier for identifier in sent if identifier != winner]
        unexpected = {
            identifier: results[identifier]["status"]
            for identifier in stale
            if results[identifier]["status"] != "cancelled"
        }
        if unexpected:
            raise RuntimeError(f"replacement dispatches did not cancel: {unexpected}")
        cancellation_ms = [
            milliseconds(results[identifier]["terminal_at_ns"], sent[identifier])
            for identifier in stale
        ]
        result = results[winner]
        source_at = result["source_at_ns"]
        terminal_at = result["terminal_at_ns"]
        sample = {
            "dispatch_id": winner,
            "engine_id": result["engine_id"],
            "actual_voice": result["actual_voice"],
            "status": result["status"],
            "cancelled_dispatches": len(stale),
            "sent_at_monotonic_ns": sent[winner],
            "source_observed_at_monotonic_ns": source_at,
            "terminal_observed_at_monotonic_ns": terminal_at,
            "dispatch_to_source_ms": milliseconds(source_at, sent[winner]),
            "dispatch_to_terminal_ms": milliseconds(terminal_at, sent[winner]),
            "source_to_terminal_ms": milliseconds(terminal_at, source_at),
            "cancel_terminal_ms": max(cancellation_ms),
        }
    else:
        generation, identifier = identities.next()
        timeline = timeline_for_case(
            case,
            generation,
            identifier,
            text_profile=text_profile,
            logical_voice_id=logical_voice_id,
        )
        sent_at = session.send_timeline(
            timeline, multipart_parts=3 if case == "multipart" else 0
        )
        result = session.wait_for_dispatches({identifier})[identifier]
        require_completed(result, identifier)
        source_at = result["source_at_ns"]
        terminal_at = result["terminal_at_ns"]
        sample = {
            "dispatch_id": identifier,
            "engine_id": result["engine_id"],
            "actual_voice": result["actual_voice"],
            "status": result["status"],
            "sent_at_monotonic_ns": sent_at,
            "source_observed_at_monotonic_ns": source_at,
            "terminal_observed_at_monotonic_ns": terminal_at,
            "dispatch_to_source_ms": milliseconds(source_at, sent_at),
            "dispatch_to_terminal_ms": milliseconds(terminal_at, sent_at),
            "source_to_terminal_ms": milliseconds(terminal_at, source_at),
        }
    if expected_engine_id and sample["engine_id"] != expected_engine_id:
        raise RuntimeError(
            f"expected engine {expected_engine_id!r}, realized {sample['engine_id']!r}"
        )
    if expected_voice_id:
        expected_voice = {
            "engine_id": expected_engine_id,
            "voice_id": expected_voice_id,
        }
        if sample["actual_voice"] != expected_voice:
            raise RuntimeError(
                f"expected voice {expected_voice!r}, realized "
                f"{sample['actual_voice']!r}"
            )
    return sample


def cold_samples(
    command: list[str],
    engine: str | None,
    preferred_engine_id: str | None,
    case: str,
    iterations: int,
    identities: IdentitySequence,
    expected_engine_id: str | None,
    expected_voice_id: str | None,
    text_profile: str,
    replacement_burst: int,
    timeout: float,
) -> list[dict[str, Any]]:
    samples = []
    for _ in range(iterations):
        session = ServerSession(command, engine, timeout)
        try:
            capabilities, ready_at = session.negotiate(
                identities.dispatch_id + 10_000_000
            )
            if preferred_engine_id:
                configure_preferred_engine(
                    session,
                    capabilities,
                    preferred_engine_id,
                    identities.dispatch_id + 10_000_001,
                )
            if expected_voice_id:
                assert expected_engine_id is not None
                configure_exact_voice(
                    session,
                    capabilities,
                    expected_engine_id,
                    expected_voice_id,
                    identities.dispatch_id + 10_000_002,
                )
            sample = execute_case(
                session,
                case,
                identities,
                expected_engine_id,
                replacement_burst,
                expected_voice_id,
                text_profile,
            )
            sample["server_version"] = capabilities.get("server_version")
            sample["process_start_to_ready_ms"] = milliseconds(
                ready_at, session.started_at_ns
            )
            sample["process_start_to_source_ms"] = milliseconds(
                sample["source_observed_at_monotonic_ns"], session.started_at_ns
            )
            samples.append(sample)
        finally:
            session.close()
    return samples


def warm_samples(
    session: ServerSession,
    case: str,
    iterations: int,
    warmups: int,
    identities: IdentitySequence,
    expected_engine_id: str | None,
    expected_voice_id: str | None,
    text_profile: str,
    replacement_burst: int,
) -> list[dict[str, Any]]:
    for _ in range(warmups):
        execute_case(
            session,
            case,
            identities,
            expected_engine_id,
            replacement_burst,
            expected_voice_id,
            text_profile,
        )
    return [
        execute_case(
            session,
            case,
            identities,
            expected_engine_id,
            replacement_burst,
            expected_voice_id,
            text_profile,
        )
        for _ in range(iterations)
    ]


def print_summary(report: dict[str, Any]) -> None:
    for mode, cases in report["results"].items():
        for case, result in cases.items():
            summary = result["summary"]
            source = summary["metrics"].get("dispatch_to_source_ms")
            if source is None:
                continue
            fields = [
                f"{mode}/{case}",
                f"n={summary['sample_count']}",
                f"source p50={source['p50']:.3f} ms",
                f"p95={source['p95']:.3f} ms",
                f"p99={source['p99']:.3f} ms",
                f"engines={summary['engines']}",
            ]
            cold_source = summary["metrics"].get("process_start_to_source_ms")
            if cold_source:
                fields.insert(2, f"start-to-source p50={cold_source['p50']:.3f} ms")
            cancellation = summary["metrics"].get("cancel_terminal_ms")
            if cancellation:
                fields.append(f"cancel p95={cancellation['p95']:.3f} ms")
            print("; ".join(fields))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", help="Omnivox executable or launcher")
    parser.add_argument(
        "--server-arg",
        action="append",
        default=[],
        help="argument appended to the server command; repeat as needed",
    )
    parser.add_argument(
        "--null-audio",
        action="store_true",
        help="run Omnivox with --audio-output null so samples are not played",
    )
    parser.add_argument(
        "--engine",
        help="set OMNIVOX_ENGINE for every benchmark process",
    )
    parser.add_argument(
        "--expected-engine-id",
        help="fail if the first source marker reports another engine",
    )
    parser.add_argument(
        "--voice-id",
        help=(
            "register and require this exact physical voice; requires "
            "--expected-engine-id"
        ),
    )
    parser.add_argument(
        "--preferred-engine-id",
        help=(
            "set one runtime routing preference after negotiation; use to "
            "exercise live policy replacement independently of startup selection"
        ),
    )
    parser.add_argument(
        "--mode", choices=("cold", "warm", "both"), default="both"
    )
    parser.add_argument(
        "--case",
        action="append",
        dest="cases",
        choices=DEFAULT_CASES,
        help="workload to run; repeat to select several (default: all)",
    )
    parser.add_argument(
        "--text-profile",
        choices=tuple(WORKLOAD_TEXTS),
        default=DEFAULT_TEXT_PROFILE,
        help="language-specific workload text (default: english)",
    )
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--replacement-burst", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=45.0)
    parser.add_argument(
        "--json-output",
        help="write the complete raw-sample report to this file, or '-' for stdout",
    )
    parser.add_argument(
        "--provenance",
        help="bounded KEY=VALUE build-provenance file to include in the report",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.iterations <= 0:
        raise SystemExit("--iterations must be positive")
    if args.warmups < 0:
        raise SystemExit("--warmups cannot be negative")
    if args.replacement_burst < 2:
        raise SystemExit("--replacement-burst must be at least two")
    if args.timeout <= 0:
        raise SystemExit("--timeout must be positive")
    if args.voice_id and not args.expected_engine_id:
        raise SystemExit("--voice-id requires --expected-engine-id")

    command = [args.server, *args.server_arg]
    if args.null_audio:
        command.extend(("--audio-output", "null"))
    cases = args.cases or list(DEFAULT_CASES)
    identities = IdentitySequence()
    report: dict[str, Any] = {
        "report_version": REPORT_VERSION,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "measurement": {
            "clock": "time.perf_counter_ns",
            "source": (
                "client receipt of first utterance_started null-source marker"
                if args.null_audio
                else "client receipt of first utterance_started mixer-source marker"
            ),
            "terminal": "client receipt of tracked terminal record",
            "acoustic_onset_measured": False,
            "real_time_playback": False if args.null_audio else None,
        },
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "configuration": {
            "server_command": command,
            "audio_output": "null" if args.null_audio else None,
            "engine": args.engine,
            "preferred_engine_id": args.preferred_engine_id,
            "expected_engine_id": args.expected_engine_id,
            "voice_id": args.voice_id,
            "text_profile": args.text_profile,
            "mode": args.mode,
            "cases": cases,
            "iterations": args.iterations,
            "warmups": args.warmups,
            "replacement_burst": args.replacement_burst,
            "timeout_seconds": args.timeout,
        },
        "server": {},
        "provenance": read_provenance(args.provenance) if args.provenance else None,
        "results": {},
    }

    if args.mode in ("cold", "both"):
        report["results"]["cold"] = {}
        for case in cases:
            samples = cold_samples(
                command,
                args.engine,
                args.preferred_engine_id,
                case,
                args.iterations,
                identities,
                args.expected_engine_id,
                args.voice_id,
                args.text_profile,
                args.replacement_burst,
                args.timeout,
            )
            report["results"]["cold"][case] = {
                "samples": samples,
                "summary": summarize_samples(samples),
            }
            if not report["server"] and samples:
                report["server"] = {"version": samples[0].get("server_version")}

    if args.mode in ("warm", "both"):
        report["results"]["warm"] = {}
        session = ServerSession(command, args.engine, args.timeout)
        try:
            capabilities, ready_at = session.negotiate(identities.dispatch_id + 20_000_000)
            if args.preferred_engine_id:
                configure_preferred_engine(
                    session,
                    capabilities,
                    args.preferred_engine_id,
                    identities.dispatch_id + 20_000_001,
                )
            if args.voice_id:
                configure_exact_voice(
                    session,
                    capabilities,
                    args.expected_engine_id,
                    args.voice_id,
                    identities.dispatch_id + 20_000_002,
                )
            report["server"] = {
                "version": capabilities.get("server_version"),
                "features": capabilities.get("features", []),
                "warm_process_start_to_ready_ms": milliseconds(
                    ready_at, session.started_at_ns
                ),
            }
            for case in cases:
                samples = warm_samples(
                    session,
                    case,
                    args.iterations,
                    args.warmups,
                    identities,
                    args.expected_engine_id,
                    args.voice_id,
                    args.text_profile,
                    args.replacement_burst,
                )
                report["results"]["warm"][case] = {
                    "samples": samples,
                    "summary": summarize_samples(samples),
                }
        finally:
            session.close()

    serialized = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json_output == "-":
        print(serialized, end="")
    else:
        print_summary(report)
        if args.json_output:
            Path(args.json_output).write_text(serialized, encoding="utf-8")
            print(f"wrote raw benchmark report to {args.json_output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError) as error:
        print(f"benchmark failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
