#!/usr/bin/env python3
"""Stress Omnivox cancellation, marker ordering, and helper recovery.

The default scenarios interleave two replacement domains with ordered and
urgent work, periodically issue a hard stop, and require exactly one terminal
record per dispatch with no later marker or semantic event.  An explicit
--fault-helper-process option kills only one validated child helper, then
verifies fallback and a control-requested helper recovery.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import queue
import shutil
import signal
import subprocess
import sys
import time
from typing import Any

import benchmark_server
import process_metrics


STRESS_REPORT_VERSION = 2
DEFAULT_TEXT_PROFILE = "english"
STRESS_TEXTS = {
    "english": {
        "stale_segment": "Stale navigation segment {number} must be cancelled promptly.",
        "ordered_survivor": "ordered survivor",
        "navigation_winner": "navigation winner",
        "urgent_survivor": "urgent survivor",
        "completion_winner": "completion winner",
        "hard_stop_segment": "Hard stop segment {number} must never survive cancellation.",
        "hard_stop_recovery": "speech recovered after hard stop",
        "helper_fallback": "helper failure should use fallback",
        "helper_inflight": "Helper fault segment {number} keeps native synthesis busy until failure.",
        "helper_recovery": "helper restarted after recovery probe",
    },
    "rutts-ru": {
        "stale_segment": "Устаревший фрагмент навигации {number} должен быть быстро отменён.",
        "ordered_survivor": "упорядоченная речь продолжается",
        "navigation_winner": "новая навигация продолжается",
        "urgent_survivor": "срочная речь продолжается",
        "completion_winner": "новое завершение продолжается",
        "hard_stop_segment": "Фрагмент остановки {number} не должен пережить отмену.",
        "hard_stop_recovery": "речь восстановилась после остановки",
        "helper_fallback": "после сбоя помощника нужен резервный голос",
        "helper_inflight": "Фрагмент сбоя {number} удерживает синтез до отказа помощника.",
        "helper_recovery": "помощник перезапущен после проверки",
    },
}


def profile_text(profile: str, key: str, **fields: int) -> str:
    texts = STRESS_TEXTS.get(profile)
    if texts is None:
        raise ValueError(f"unknown stress text profile: {profile}")
    return texts[key].format(**fields)


def semantic_timeline(
    generation: int,
    dispatch_id: int,
    text: str,
    policy: str,
    replacement_key: str | None = None,
    logical_voice_id: str | None = None,
) -> dict[str, Any]:
    span: dict[str, Any] = {"id": 1, "text": text}
    if logical_voice_id:
        span["logical_voice_id"] = logical_voice_id
    timeline: dict[str, Any] = {
        "protocol_version": 3,
        "generation": generation,
        "dispatch_id": dispatch_id,
        "delivery_policy": policy,
        "spans": [span],
        "actions": [
            {
                "id": f"semantic-{dispatch_id}",
                "position": {
                    "position": "span_boundary",
                    "span_id": 1,
                    "affinity": "before",
                },
                "lifecycle_anchor": "run",
                "type": "semantic_event",
            }
        ],
    }
    if replacement_key is not None:
        timeline["replacement_key"] = replacement_key
    return timeline


def empty_history() -> dict[str, Any]:
    return {
        "marker_sequences": [],
        "marker_types": [],
        "source_at_monotonic_ns": None,
        "engine_id": None,
        "actual_voice": None,
        "terminal_at_monotonic_ns": None,
        "status": None,
        "terminal_count": 0,
    }


def record_output_line(
    line: str,
    observed_at: int,
    histories: dict[int, dict[str, Any]],
) -> None:
    if line.startswith(benchmark_server.MARKER_PREFIX):
        event = benchmark_server.decode_record(
            line[len(benchmark_server.MARKER_PREFIX) :]
        )
        identifier = event.get("dispatch_id")
        if identifier not in histories:
            return
        history = histories[identifier]
        if history["terminal_at_monotonic_ns"] is not None:
            raise RuntimeError(f"dispatch {identifier} emitted a marker after terminal")
        sequence = event.get("sequence")
        expected = len(history["marker_sequences"]) + 1
        if sequence != expected:
            raise RuntimeError(
                f"dispatch {identifier} marker sequence {sequence!r} != {expected}"
            )
        history["marker_sequences"].append(sequence)
        history["marker_types"].append(event.get("type"))
        if (
            event.get("type") == "utterance_started"
            and history["source_at_monotonic_ns"] is None
        ):
            history["source_at_monotonic_ns"] = observed_at
            history["engine_id"] = event.get("engine_id")
            history["actual_voice"] = event.get("actual_voice")
        return

    if not line.startswith(benchmark_server.TRACKED_PREFIX):
        return
    fields = line.split()
    if len(fields) != 3:
        raise RuntimeError(f"malformed tracked terminal record: {line!r}")
    try:
        identifier = int(fields[1])
    except ValueError as error:
        raise RuntimeError(f"malformed tracked dispatch ID: {line!r}") from error
    if identifier not in histories:
        return
    history = histories[identifier]
    history["terminal_count"] += 1
    if history["terminal_count"] != 1:
        raise RuntimeError(f"dispatch {identifier} emitted duplicate terminal records")
    history["terminal_at_monotonic_ns"] = observed_at
    history["status"] = fields[2]


def collect_histories(
    session: benchmark_server.ServerSession,
    identifiers: set[int],
    quiet_seconds: float,
) -> dict[int, dict[str, Any]]:
    histories = {identifier: empty_history() for identifier in identifiers}
    deadline = time.monotonic() + session.timeout
    while any(history["terminal_count"] == 0 for history in histories.values()):
        observed_at, line = session.receive_line(deadline)
        record_output_line(line, observed_at, histories)

    quiet_deadline = time.monotonic() + quiet_seconds
    while True:
        remaining = quiet_deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            observed_at, line = session.output.get(timeout=remaining)
        except queue.Empty:
            break
        if line is None:
            raise RuntimeError(f"server exited after terminal output; {session.failure_context()}")
        record_output_line(line, observed_at, histories)
    return histories


def validate_histories(
    histories: dict[int, dict[str, Any]],
    expected_statuses: dict[int, str],
    expected_engine_id: str | None,
    expected_voice_id: str | None = None,
) -> None:
    for identifier, expected_status in expected_statuses.items():
        history = histories[identifier]
        if history["terminal_count"] != 1 or history["status"] != expected_status:
            raise RuntimeError(
                f"dispatch {identifier} expected {expected_status}, got {history['status']}"
            )
        if expected_status == "completed":
            if history["source_at_monotonic_ns"] is None:
                raise RuntimeError(f"completed dispatch {identifier} has no source marker")
            if "semantic_event_reached" not in history["marker_types"]:
                raise RuntimeError(
                    f"completed dispatch {identifier} omitted its semantic callback"
                )
            if expected_engine_id and history["engine_id"] != expected_engine_id:
                raise RuntimeError(
                    f"dispatch {identifier} expected engine {expected_engine_id!r}, "
                    f"realized {history['engine_id']!r}"
                )
            if expected_voice_id:
                expected_voice = {
                    "engine_id": expected_engine_id,
                    "voice_id": expected_voice_id,
                }
                if history["actual_voice"] != expected_voice:
                    raise RuntimeError(
                        f"dispatch {identifier} expected voice {expected_voice!r}, "
                        f"realized {history['actual_voice']!r}"
                    )


def send_timeline(
    session: benchmark_server.ServerSession,
    identities: benchmark_server.IdentitySequence,
    text: str,
    policy: str,
    replacement_key: str | None = None,
    logical_voice_id: str | None = None,
) -> tuple[int, int]:
    generation, identifier = identities.next()
    sent_at = session.send_timeline(
        semantic_timeline(
            generation,
            identifier,
            text,
            policy,
            replacement_key,
            logical_voice_id,
        )
    )
    return identifier, sent_at


def realized_voices(
    histories: dict[int, dict[str, Any]],
    identifiers: set[int],
) -> list[dict[str, str]]:
    voices = {
        (voice["engine_id"], voice["voice_id"])
        for identifier in identifiers
        if isinstance((voice := histories[identifier].get("actual_voice")), dict)
        and isinstance(voice.get("engine_id"), str)
        and isinstance(voice.get("voice_id"), str)
    }
    return [
        {"engine_id": engine_id, "voice_id": voice_id}
        for engine_id, voice_id in sorted(voices)
    ]


def run_replacement_iteration(
    session: benchmark_server.ServerSession,
    identities: benchmark_server.IdentitySequence,
    expected_engine_id: str | None,
    expected_voice_id: str | None,
    text_profile: str,
    quiet_seconds: float,
) -> dict[str, Any]:
    logical_voice_id = (
        benchmark_server.BENCHMARK_LOGICAL_VOICE_ID if expected_voice_id else None
    )
    stale_text = " ".join(
        profile_text(text_profile, "stale_segment", number=number)
        for number in range(1, 13)
    )
    sends = []
    sends.append(
        (
            *send_timeline(
                session,
                identities,
                stale_text,
                "replaceable",
                "navigation",
                logical_voice_id,
            ),
            "cancelled",
        )
    )
    sends.append(
        (
            *send_timeline(
                session,
                identities,
                stale_text,
                "replaceable",
                "completion",
                logical_voice_id,
            ),
            "cancelled",
        )
    )
    sends.append(
        (
            *send_timeline(
                session,
                identities,
                profile_text(text_profile, "ordered_survivor"),
                "ordered",
                logical_voice_id=logical_voice_id,
            ),
            "completed",
        )
    )
    sends.append(
        (
            *send_timeline(
                session,
                identities,
                profile_text(text_profile, "navigation_winner"),
                "replaceable",
                "navigation",
                logical_voice_id,
            ),
            "completed",
        )
    )
    sends.append(
        (
            *send_timeline(
                session,
                identities,
                profile_text(text_profile, "urgent_survivor"),
                "urgent",
                logical_voice_id=logical_voice_id,
            ),
            "completed",
        )
    )
    sends.append(
        (
            *send_timeline(
                session,
                identities,
                profile_text(text_profile, "completion_winner"),
                "replaceable",
                "completion",
                logical_voice_id,
            ),
            "completed",
        )
    )
    expected = {identifier: status for identifier, _, status in sends}
    histories = collect_histories(session, set(expected), quiet_seconds)
    validate_histories(
        histories, expected, expected_engine_id, expected_voice_id
    )
    terminal_ms = {
        str(identifier): benchmark_server.milliseconds(
            histories[identifier]["terminal_at_monotonic_ns"], sent_at
        )
        for identifier, sent_at, _ in sends
    }
    return {
        "dispatches": len(sends),
        "cancelled": sum(status == "cancelled" for status in expected.values()),
        "completed": sum(status == "completed" for status in expected.values()),
        "actual_voices": realized_voices(
            histories,
            {
                identifier
                for identifier, status in expected.items()
                if status == "completed"
            },
        ),
        "cancelled_with_reached_markers": sum(
            bool(histories[identifier]["marker_sequences"])
            for identifier, status in expected.items()
            if status == "cancelled"
        ),
        "terminal_ms": terminal_ms,
    }


def run_hard_stop(
    session: benchmark_server.ServerSession,
    identities: benchmark_server.IdentitySequence,
    expected_engine_id: str | None,
    expected_voice_id: str | None,
    text_profile: str,
    quiet_seconds: float,
) -> dict[str, Any]:
    logical_voice_id = (
        benchmark_server.BENCHMARK_LOGICAL_VOICE_ID if expected_voice_id else None
    )
    long_text = " ".join(
        profile_text(text_profile, "hard_stop_segment", number=number)
        for number in range(1, 17)
    )
    stopped = [
        send_timeline(
            session,
            identities,
            long_text,
            "ordered",
            logical_voice_id=logical_voice_id,
        ),
        send_timeline(
            session,
            identities,
            long_text,
            "replaceable",
            "hard-stop",
            logical_voice_id,
        ),
    ]
    stop_sent_at = session.send_line("s")
    expected = {identifier: "cancelled" for identifier, _ in stopped}
    histories = collect_histories(session, set(expected), quiet_seconds)
    validate_histories(histories, expected, None)

    survivor, survivor_sent = send_timeline(
        session,
        identities,
        profile_text(text_profile, "hard_stop_recovery"),
        "ordered",
        logical_voice_id=logical_voice_id,
    )
    survivor_history = collect_histories(session, {survivor}, quiet_seconds)
    validate_histories(
        survivor_history,
        {survivor: "completed"},
        expected_engine_id,
        expected_voice_id,
    )
    return {
        "cancelled": len(stopped),
        "stop_to_last_terminal_ms": max(
            benchmark_server.milliseconds(
                histories[identifier]["terminal_at_monotonic_ns"], stop_sent_at
            )
            for identifier, _ in stopped
        ),
        "recovery_dispatch_to_source_ms": benchmark_server.milliseconds(
            survivor_history[survivor]["source_at_monotonic_ns"], survivor_sent
        ),
        "actual_voice": survivor_history[survivor]["actual_voice"],
    }


def windows_process_snapshot(powershell: str) -> dict[int, dict[str, Any]]:
    command = (
        "Get-CimInstance Win32_Process | "
        "Select-Object ProcessId,ParentProcessId,Name,ExecutablePath | "
        "ConvertTo-Json -Compress"
    )
    completed = subprocess.run(
        [powershell, "-NoProfile", "-NonInteractive", "-Command", command],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
    )
    decoded = json.loads(completed.stdout or "[]")
    rows = decoded if isinstance(decoded, list) else [decoded]
    return {
        int(row["ProcessId"]): {
            "parent": int(row["ParentProcessId"]),
            "name": row.get("Name") or "",
            "path": row.get("ExecutablePath"),
        }
        for row in rows
        if isinstance(row, dict) and row.get("ProcessId") is not None
    }


def proc_process_snapshot(proc_root: Path = Path("/proc")) -> dict[int, dict[str, Any]]:
    snapshot = {}
    for status_path in proc_root.glob("[0-9]*/status"):
        try:
            values = {}
            for line in status_path.read_text(encoding="utf-8").splitlines():
                key, separator, value = line.partition(":")
                if separator and key in ("Name", "PPid", "State"):
                    values[key] = value.strip()
            # A killed helper can remain unreaped until the next host request.
            # It has already exited and cannot execute native code.
            if values.get("State", "").startswith(("Z", "X")):
                continue
            pid = int(status_path.parent.name)
            executable = (status_path.parent / "exe").resolve().name
            snapshot[pid] = {
                "parent": int(values["PPid"]),
                "name": executable or values["Name"],
                "path": None,
            }
        except (FileNotFoundError, KeyError, OSError, ValueError):
            continue
    return snapshot


def is_descendant(
    process_id: int,
    ancestor_ids: set[int],
    snapshot: dict[int, dict[str, Any]],
) -> bool:
    seen = set()
    current = process_id
    while current not in seen and current in snapshot:
        seen.add(current)
        parent = snapshot[current]["parent"]
        if parent in ancestor_ids:
            return True
        current = parent
    return False


def select_fault_target(
    before: dict[int, dict[str, Any]],
    after: dict[int, dict[str, Any]],
    helper_name: str,
    server_ids: set[int],
) -> int:
    candidates = [
        process_id
        for process_id, process in after.items()
        if process_id not in before
        and process["name"].casefold() == helper_name.casefold()
        and is_descendant(process_id, server_ids, after)
    ]
    if len(candidates) != 1:
        raise RuntimeError(
            f"expected one new child helper named {helper_name!r}, found {candidates}"
        )
    return candidates[0]


class HelperFaultInjector:
    def __init__(self, helper_name: str) -> None:
        self.helper_name = helper_name
        self.powershell = shutil.which("powershell.exe") or (
            shutil.which("powershell") if platform.system() == "Windows" else None
        )
        self.provider = "windows" if self.powershell and helper_name.lower().endswith(".exe") else "proc"
        self.taskkill = shutil.which("taskkill.exe") if self.provider == "windows" else None
        self.before = self.snapshot()

    def snapshot(self) -> dict[int, dict[str, Any]]:
        if self.provider == "windows":
            assert self.powershell is not None
            return windows_process_snapshot(self.powershell)
        if not Path("/proc").is_dir():
            raise RuntimeError("helper fault injection requires Windows process data or /proc")
        return proc_process_snapshot()

    def resolve(self, server_process_id: int) -> int:
        after = self.snapshot()
        if self.provider == "windows":
            server_ids = {
                process_id
                for process_id, process in after.items()
                if process_id not in self.before
                and process["name"].casefold() == "omnivox.exe"
            }
        else:
            server_ids = {server_process_id}
        if not server_ids:
            raise RuntimeError("could not identify the dedicated Omnivox server process")
        return select_fault_target(
            self.before, after, self.helper_name, server_ids
        )

    def terminate(self, target: int) -> int:
        if self.provider == "windows":
            if self.taskkill is None:
                raise RuntimeError("Windows helper fault injection requires taskkill.exe")
            subprocess.run(
                [self.taskkill, "/PID", str(target), "/F"],
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=30,
            )
        else:
            os.kill(target, signal.SIGKILL)
        deadline = time.monotonic() + 5
        while target in self.snapshot() and time.monotonic() < deadline:
            time.sleep(0.05)
        if target in self.snapshot():
            raise RuntimeError(f"helper process {target} remained alive after fault injection")
        return target

    def kill(self, server_process_id: int) -> int:
        return self.terminate(self.resolve(server_process_id))


def run_helper_recovery(
    session: benchmark_server.ServerSession,
    identities: benchmark_server.IdentitySequence,
    injector: HelperFaultInjector,
    fault_engine_id: str,
    fallback_engine_id: str | None,
    recovered_voice_id: str | None,
    text_profile: str,
    quiet_seconds: float,
    control_request_id: int,
    fault_mode: str,
    fault_delay_ms: float,
) -> dict[str, Any]:
    logical_voice_id = (
        benchmark_server.BENCHMARK_LOGICAL_VOICE_ID if recovered_voice_id else None
    )
    fault_dispatch = None
    if fault_mode == "dispatch":
        target = injector.resolve(session.process.pid)
        failure_text = " ".join(
            profile_text(text_profile, "helper_inflight", number=number)
            for number in range(1, 129)
        )
        fault_id, fault_sent = send_timeline(
            session,
            identities,
            failure_text,
            "ordered",
            logical_voice_id=logical_voice_id,
        )
        if fault_delay_ms:
            time.sleep(fault_delay_ms / 1000.0)
        killed_pid = injector.terminate(target)
        stop_sent = session.send_line("s")
        fault_history = collect_histories(session, {fault_id}, quiet_seconds)
        validate_histories(fault_history, {fault_id: "cancelled"}, None)
        fault_dispatch = {
            "dispatch_to_stop_ms": benchmark_server.milliseconds(
                stop_sent, fault_sent
            ),
            "stop_to_terminal_ms": benchmark_server.milliseconds(
                fault_history[fault_id]["terminal_at_monotonic_ns"], stop_sent
            ),
            "marker_count": len(fault_history[fault_id]["marker_sequences"]),
            "actual_voice": fault_history[fault_id]["actual_voice"],
        }
        failed_id, failed_sent = send_timeline(
            session,
            identities,
            profile_text(text_profile, "helper_fallback"),
            "ordered",
            logical_voice_id=logical_voice_id,
        )
    elif fault_mode == "idle":
        killed_pid = injector.kill(session.process.pid)
        failed_id, failed_sent = send_timeline(
            session,
            identities,
            profile_text(text_profile, "helper_fallback"),
            "ordered",
            logical_voice_id=logical_voice_id,
        )
    else:
        raise ValueError(f"unknown helper fault mode: {fault_mode}")
    failed_history = collect_histories(session, {failed_id}, quiet_seconds)
    validate_histories(failed_history, {failed_id: "completed"}, fallback_engine_id)
    realized_fallback = failed_history[failed_id]["engine_id"]
    if realized_fallback == fault_engine_id:
        raise RuntimeError("killed helper unexpectedly realized the fallback dispatch")

    response, _ = session.request_control(
        {
            "protocol_version": 1,
            "request_id": control_request_id,
            "type": "request_engine_recovery_probe",
            "engine_id": fault_engine_id,
        }
    )
    if response.get("type") != "engine_recovery_probe_requested":
        raise RuntimeError(f"server rejected helper recovery probe: {response}")

    recovered_id, recovered_sent = send_timeline(
        session,
        identities,
        profile_text(text_profile, "helper_recovery"),
        "ordered",
        logical_voice_id=logical_voice_id,
    )
    recovered_history = collect_histories(session, {recovered_id}, quiet_seconds)
    validate_histories(
        recovered_history,
        {recovered_id: "completed"},
        fault_engine_id,
        recovered_voice_id,
    )
    return {
        "fault_mode": fault_mode,
        "fault_delay_ms": fault_delay_ms,
        "fault_dispatch": fault_dispatch,
        "provider": injector.provider,
        "killed_process_id": killed_pid,
        "fallback_engine_id": realized_fallback,
        "fallback_actual_voice": failed_history[failed_id]["actual_voice"],
        "fallback_dispatch_to_source_ms": benchmark_server.milliseconds(
            failed_history[failed_id]["source_at_monotonic_ns"], failed_sent
        ),
        "recovered_engine_id": recovered_history[recovered_id]["engine_id"],
        "recovered_actual_voice": recovered_history[recovered_id]["actual_voice"],
        "recovery_dispatch_to_source_ms": benchmark_server.milliseconds(
            recovered_history[recovered_id]["source_at_monotonic_ns"], recovered_sent
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", help="Omnivox executable or launcher")
    parser.add_argument("--server-arg", action="append", default=[])
    parser.add_argument("--engine", help="set OMNIVOX_ENGINE for the server")
    parser.add_argument("--expected-engine-id")
    parser.add_argument(
        "--preferred-engine-id",
        help="set one strict runtime engine preference before stress dispatches",
    )
    parser.add_argument(
        "--voice-id",
        help="register and require an exact voice; requires --expected-engine-id",
    )
    parser.add_argument(
        "--text-profile",
        choices=tuple(STRESS_TEXTS),
        default=DEFAULT_TEXT_PROFILE,
        help="language-specific stress text (default: english)",
    )
    parser.add_argument("--iterations", type=int, default=10)
    parser.add_argument("--stop-every", type=int, default=5)
    parser.add_argument("--quiet-seconds", type=float, default=0.1)
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument(
        "--resource-sample-every",
        type=int,
        default=0,
        help="sample the server process tree every N iterations (default: disabled)",
    )
    parser.add_argument(
        "--resource-process-name",
        help="exact native Windows server name to resolve from a WSL launcher",
    )
    parser.add_argument(
        "--fault-helper-process",
        help="exact child executable name to kill, such as omnivox-flite-helper.exe",
    )
    parser.add_argument(
        "--fault-engine-id",
        help="engine whose helper is killed and explicitly recovered",
    )
    parser.add_argument(
        "--fallback-engine-id",
        help="required realized fallback after the optional helper fault",
    )
    parser.add_argument(
        "--fault-mode",
        choices=("idle", "dispatch"),
        default="idle",
        help="kill before a probe or while a long dispatch is outstanding",
    )
    parser.add_argument(
        "--fault-count",
        type=int,
        default=1,
        help="number of bounded fault/fallback/recovery cycles (default: 1)",
    )
    parser.add_argument(
        "--fault-delay-ms",
        type=float,
        default=0.0,
        help="delay after dispatch before killing its helper (default: 0)",
    )
    parser.add_argument("--provenance")
    parser.add_argument("--json-output")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.iterations <= 0:
        raise SystemExit("--iterations must be positive")
    if args.stop_every < 0:
        raise SystemExit("--stop-every cannot be negative")
    if args.quiet_seconds < 0:
        raise SystemExit("--quiet-seconds cannot be negative")
    if args.timeout <= 0:
        raise SystemExit("--timeout must be positive")
    if args.resource_sample_every < 0:
        raise SystemExit("--resource-sample-every cannot be negative")
    if args.resource_process_name and not args.resource_sample_every:
        raise SystemExit(
            "--resource-process-name requires --resource-sample-every"
        )
    if bool(args.fault_helper_process) != bool(args.fault_engine_id):
        raise SystemExit(
            "--fault-helper-process and --fault-engine-id must be supplied together"
        )
    if args.fault_helper_process and not args.fallback_engine_id:
        raise SystemExit("--fallback-engine-id is required for helper fault recovery")
    if not 1 <= args.fault_count <= 100:
        raise SystemExit("--fault-count must be from 1 through 100")
    if args.fault_count != 1 and not args.fault_helper_process:
        raise SystemExit("--fault-count requires --fault-helper-process")
    if not 0 <= args.fault_delay_ms <= 5000:
        raise SystemExit("--fault-delay-ms must be from 0 through 5000")
    if args.fault_delay_ms and args.fault_mode != "dispatch":
        raise SystemExit("--fault-delay-ms requires --fault-mode dispatch")
    if args.voice_id and not args.expected_engine_id:
        raise SystemExit("--voice-id requires --expected-engine-id")
    if (
        args.fault_engine_id
        and args.preferred_engine_id
        and args.fault_engine_id != args.preferred_engine_id
    ):
        raise SystemExit(
            "--preferred-engine-id must equal --fault-engine-id during fault recovery"
        )

    injector = (
        HelperFaultInjector(args.fault_helper_process)
        if args.fault_helper_process
        else None
    )
    resource_observer = (
        process_metrics.ProcessTreeObserver(
            args.server, args.resource_process_name
        )
        if args.resource_sample_every
        else None
    )
    session = benchmark_server.ServerSession(
        [args.server, *args.server_arg], args.engine, args.timeout
    )
    if resource_observer is not None:
        resource_observer.bind(session.process.pid)
    resource_samples: list[dict[str, Any]] = []

    def capture_resources(iteration: int, phase: str) -> None:
        if resource_observer is None:
            return
        sample = resource_observer.sample()
        if sample is None:
            return
        resource_samples.append(
            {
                "iteration": iteration,
                "phase": phase,
                "elapsed_ms": benchmark_server.milliseconds(
                    time.perf_counter_ns(), session.started_at_ns
                ),
                **sample,
            }
        )

    capture_resources(0, "started")
    identities = benchmark_server.IdentitySequence()
    report: dict[str, Any] = {
        "report_version": STRESS_REPORT_VERSION,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "host": {"platform": platform.platform(), "python": platform.python_version()},
        "configuration": {
            "server": [args.server, *args.server_arg],
            "engine": args.engine,
            "preferred_engine_id": args.preferred_engine_id,
            "expected_engine_id": args.expected_engine_id,
            "voice_id": args.voice_id,
            "text_profile": args.text_profile,
            "iterations": args.iterations,
            "stop_every": args.stop_every,
            "quiet_seconds": args.quiet_seconds,
            "timeout_seconds": args.timeout,
            "resource_sample_every": args.resource_sample_every,
            "resource_process_name": args.resource_process_name,
            "fault_mode": args.fault_mode,
            "fault_count": args.fault_count if injector is not None else 0,
            "fault_delay_ms": args.fault_delay_ms,
        },
        "provenance": (
            benchmark_server.read_provenance(args.provenance)
            if args.provenance
            else None
        ),
        "replacement_iterations": [],
        "hard_stops": [],
        "helper_recovery": None,
        "helper_recoveries": [],
        "resources": None,
    }
    progress_stream = sys.stderr if args.json_output == "-" else sys.stdout
    try:
        capabilities, ready_at = session.negotiate(30_000_000)
        report["server"] = {
            "version": capabilities.get("server_version"),
            "process_start_to_ready_ms": benchmark_server.milliseconds(
                ready_at, session.started_at_ns
            ),
        }
        capture_resources(0, "ready")
        if injector is not None:
            routing_response, _ = session.request_control(
                {
                    "protocol_version": 1,
                    "request_id": 30_000_001,
                    "type": "set_routing_policy",
                    "routing_policy_generation": 1,
                    "preferred_engine_ids": [args.fault_engine_id],
                    "fallback_engine_ids": [args.fallback_engine_id],
                    "disabled_engine_ids": [],
                }
            )
            if routing_response.get("type") != "routing_policy_applied":
                raise RuntimeError(
                    f"server rejected fault-test routing policy: {routing_response}"
                )
        elif args.preferred_engine_id:
            benchmark_server.configure_preferred_engine(
                session,
                capabilities,
                args.preferred_engine_id,
                30_000_001,
            )
        if args.voice_id:
            benchmark_server.configure_exact_voice(
                session,
                capabilities,
                args.expected_engine_id,
                args.voice_id,
                30_000_002,
            )
        for iteration in range(1, args.iterations + 1):
            report["replacement_iterations"].append(
                run_replacement_iteration(
                    session,
                    identities,
                    args.expected_engine_id,
                    args.voice_id,
                    args.text_profile,
                    args.quiet_seconds,
                )
            )
            if args.stop_every and iteration % args.stop_every == 0:
                report["hard_stops"].append(
                    run_hard_stop(
                        session,
                        identities,
                        args.expected_engine_id,
                        args.voice_id,
                        args.text_profile,
                        args.quiet_seconds,
                    )
                )
            if (
                args.resource_sample_every
                and iteration % args.resource_sample_every == 0
            ):
                capture_resources(iteration, "interval")
            print(
                f"completed stress iteration {iteration}/{args.iterations}",
                file=progress_stream,
                flush=True,
            )
        if injector is not None:
            for fault_index in range(1, args.fault_count + 1):
                recovery = run_helper_recovery(
                    session,
                    identities,
                    injector,
                    args.fault_engine_id,
                    args.fallback_engine_id,
                    args.voice_id,
                    args.text_profile,
                    args.quiet_seconds,
                    30_000_002 + fault_index,
                    args.fault_mode,
                    args.fault_delay_ms,
                )
                recovery["fault_index"] = fault_index
                report["helper_recoveries"].append(recovery)
                report["helper_recovery"] = recovery
                print(
                    f"completed validated helper fault and recovery "
                    f"{fault_index}/{args.fault_count}",
                    file=progress_stream,
                    flush=True,
                )
        capture_resources(args.iterations, "before_shutdown")
    finally:
        session.close()

    if resource_observer is not None:
        report["resources"] = {
            **resource_observer.description(),
            "samples": resource_samples,
            "summary": process_metrics.summarize_tree_samples(resource_samples),
            "steady_state_summary": process_metrics.summarize_tree_samples(
                [
                    sample
                    for sample in resource_samples
                    if sample["phase"] != "started"
                ]
            ),
        }

    serialized = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json_output == "-":
        print(serialized, end="")
    else:
        print(
            f"PASS: {args.iterations} replacement iterations, "
            f"{len(report['hard_stops'])} hard stops, "
            f"helper recovery={'yes' if report['helper_recovery'] else 'not requested'}"
        )
        if args.json_output:
            Path(args.json_output).write_text(serialized, encoding="utf-8")
            print(f"wrote raw stress report to {args.json_output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(f"stress failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
