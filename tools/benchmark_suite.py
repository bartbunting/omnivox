#!/usr/bin/env python3
"""Run a seeded, repeated, cross-engine Omnivox benchmark suite."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import re
import subprocess
import sys
from typing import Any

import benchmark_server


PLAN_VERSION = 1
SUITE_REPORT_VERSION = 1
MAX_PLAN_BYTES = 64 * 1024
MAX_RUNS = 64
MAX_REPEATS = 100
RUN_ID_PATTERN = re.compile(r"[a-z0-9][a-z0-9._-]{0,63}\Z")


def require_integer(
    value: Any,
    field: str,
    minimum: int,
    maximum: int,
) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{field} must be an integer")
    if not minimum <= value <= maximum:
        raise ValueError(f"{field} must be from {minimum} through {maximum}")
    return value


def optional_string(value: Any, field: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string when supplied")
    return value


def load_plan(path: Path) -> dict[str, Any]:
    payload = path.read_bytes()
    if len(payload) > MAX_PLAN_BYTES:
        raise ValueError(f"suite plan exceeds the {MAX_PLAN_BYTES}-byte limit")
    try:
        plan = json.loads(payload)
    except json.JSONDecodeError as error:
        raise ValueError(f"suite plan is not valid JSON: {error}") from error
    if not isinstance(plan, dict):
        raise ValueError("suite plan must be a JSON object")
    if plan.get("plan_version") != PLAN_VERSION:
        raise ValueError(f"suite plan_version must be {PLAN_VERSION}")

    server = optional_string(plan.get("server"), "server")
    if server is None:
        raise ValueError("server is required")
    plan["server"] = str(resolve_plan_path(path, server))
    provenance = optional_string(plan.get("provenance"), "provenance")
    if provenance:
        plan["provenance"] = str(resolve_plan_path(path, provenance))

    server_args = plan.get("server_args", [])
    if not isinstance(server_args, list) or not all(
        isinstance(value, str) for value in server_args
    ):
        raise ValueError("server_args must be an array of strings")

    plan["seed"] = require_integer(plan.get("seed"), "seed", 0, 2**63 - 1)
    plan["repeats"] = require_integer(
        plan.get("repeats", 1), "repeats", 1, MAX_REPEATS
    )
    plan["benchmark"] = validate_benchmark(plan.get("benchmark", {}))
    runs = plan.get("runs")
    if not isinstance(runs, list) or not 1 <= len(runs) <= MAX_RUNS:
        raise ValueError(f"runs must contain from 1 through {MAX_RUNS} entries")
    plan["runs"] = validate_runs(runs)
    return plan


def resolve_plan_path(plan_path: Path, value: str) -> Path:
    candidate = Path(value)
    if candidate.is_absolute():
        return candidate
    return (plan_path.resolve().parent / candidate).resolve()


def validate_benchmark(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError("benchmark must be an object")
    mode = value.get("mode", "both")
    if mode not in ("cold", "warm", "both"):
        raise ValueError("benchmark.mode must be cold, warm, or both")
    timeout = value.get("timeout_seconds", 45.0)
    if isinstance(timeout, bool) or not isinstance(timeout, (int, float)):
        raise ValueError("benchmark.timeout_seconds must be numeric")
    if not 0 < timeout <= 3600:
        raise ValueError("benchmark.timeout_seconds must be greater than 0 and at most 3600")
    null_audio = value.get("null_audio", False)
    if not isinstance(null_audio, bool):
        raise ValueError("benchmark.null_audio must be true or false")
    return {
        "mode": mode,
        "iterations": require_integer(
            value.get("iterations", 20), "benchmark.iterations", 1, 10_000
        ),
        "warmups": require_integer(
            value.get("warmups", 2), "benchmark.warmups", 0, 1_000
        ),
        "replacement_burst": require_integer(
            value.get("replacement_burst", 5),
            "benchmark.replacement_burst",
            2,
            100,
        ),
        "timeout_seconds": float(timeout),
        "null_audio": null_audio,
    }


def validate_runs(values: list[Any]) -> list[dict[str, Any]]:
    runs = []
    identifiers = set()
    for index, value in enumerate(values):
        field = f"runs[{index}]"
        if not isinstance(value, dict):
            raise ValueError(f"{field} must be an object")
        run_id = value.get("id")
        if not isinstance(run_id, str) or not RUN_ID_PATTERN.fullmatch(run_id):
            raise ValueError(f"{field}.id is not a safe portable identifier")
        if run_id in identifiers:
            raise ValueError(f"duplicate run ID: {run_id}")
        identifiers.add(run_id)

        cases = value.get("cases", list(benchmark_server.DEFAULT_CASES))
        if (
            not isinstance(cases, list)
            or not cases
            or not all(case in benchmark_server.DEFAULT_CASES for case in cases)
            or len(set(cases)) != len(cases)
        ):
            raise ValueError(f"{field}.cases must contain unique benchmark cases")
        text_profile = value.get(
            "text_profile", benchmark_server.DEFAULT_TEXT_PROFILE
        )
        if text_profile not in benchmark_server.WORKLOAD_TEXTS:
            raise ValueError(f"{field}.text_profile is unknown")

        run = {
            "id": run_id,
            "engine": optional_string(value.get("engine"), f"{field}.engine"),
            "preferred_engine_id": optional_string(
                value.get("preferred_engine_id"), f"{field}.preferred_engine_id"
            ),
            "expected_engine_id": optional_string(
                value.get("expected_engine_id"), f"{field}.expected_engine_id"
            ),
            "voice_id": optional_string(value.get("voice_id"), f"{field}.voice_id"),
            "text_profile": text_profile,
            "cases": cases,
        }
        if run["voice_id"] and not run["expected_engine_id"]:
            raise ValueError(f"{field}.voice_id requires expected_engine_id")
        runs.append(run)
    return runs


def execution_schedule(
    runs: list[dict[str, Any]], repeats: int, seed: int
) -> list[tuple[int, int, dict[str, Any]]]:
    generator = random.Random(seed)
    schedule = []
    for repeat in range(1, repeats + 1):
        shuffled = list(runs)
        generator.shuffle(shuffled)
        schedule.extend(
            (repeat, ordinal, run)
            for ordinal, run in enumerate(shuffled, start=1)
        )
    return schedule


def benchmark_command(
    plan: dict[str, Any],
    run: dict[str, Any],
    output: Path,
) -> list[str]:
    benchmark = plan["benchmark"]
    command = [
        sys.executable,
        str(Path(__file__).resolve().with_name("benchmark_server.py")),
        plan["server"],
        "--mode",
        benchmark["mode"],
        "--iterations",
        str(benchmark["iterations"]),
        "--warmups",
        str(benchmark["warmups"]),
        "--replacement-burst",
        str(benchmark["replacement_burst"]),
        "--timeout",
        str(benchmark["timeout_seconds"]),
        "--text-profile",
        run["text_profile"],
        "--json-output",
        str(output),
    ]
    for server_arg in plan.get("server_args", []):
        command.append(f"--server-arg={server_arg}")
    if benchmark["null_audio"]:
        command.append("--null-audio")
    for field, option in (
        ("engine", "--engine"),
        ("preferred_engine_id", "--preferred-engine-id"),
        ("expected_engine_id", "--expected-engine-id"),
        ("voice_id", "--voice-id"),
    ):
        if run.get(field):
            command.extend((option, run[field]))
    if plan.get("provenance"):
        command.extend(("--provenance", plan["provenance"]))
    for case in run["cases"]:
        command.extend(("--case", case))
    return command


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_index(path: Path, report: dict[str, Any]) -> None:
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.replace(temporary, path)


def run_suite(plan_path: Path, output: Path) -> None:
    plan = load_plan(plan_path)
    output.mkdir(parents=True, exist_ok=False)
    plan_hash = hashlib.sha256(plan_path.read_bytes()).hexdigest()
    schedule = execution_schedule(plan["runs"], plan["repeats"], plan["seed"])
    index: dict[str, Any] = {
        "report_version": SUITE_REPORT_VERSION,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "configuration": {
            "plan_sha256": plan_hash,
            "seed": plan["seed"],
            "repeats": plan["repeats"],
            "benchmark": plan["benchmark"],
            "run_ids": [run["id"] for run in plan["runs"]],
        },
        "status": "running",
        "runs": [],
    }
    index_path = output / "suite.json"
    write_index(index_path, index)

    for repeat, ordinal, run in schedule:
        filename = f"repeat-{repeat:03d}-order-{ordinal:03d}-{run['id']}.json"
        report_path = output / filename
        print(
            f"running repeat {repeat}/{plan['repeats']} order {ordinal}: {run['id']}",
            flush=True,
        )
        completed = subprocess.run(
            benchmark_command(plan, run, report_path),
            check=False,
        )
        if completed.returncode != 0:
            index["status"] = "failed"
            index["failed_run"] = {
                "repeat": repeat,
                "order": ordinal,
                "run_id": run["id"],
                "return_code": completed.returncode,
            }
            write_index(index_path, index)
            raise RuntimeError(
                f"benchmark run {run['id']!r} failed with status "
                f"{completed.returncode}; partial evidence remains in {output}"
            )
        raw = json.loads(report_path.read_text(encoding="utf-8"))
        index["runs"].append(
            {
                "repeat": repeat,
                "order": ordinal,
                "run_id": run["id"],
                "report": filename,
                "sha256": sha256(report_path),
                "benchmark_report_version": raw.get("report_version"),
                "created_at": raw.get("created_at"),
            }
        )
        write_index(index_path, index)

    index["status"] = "complete"
    index["completed_at"] = datetime.now(timezone.utc).isoformat()
    write_index(index_path, index)
    print(f"PASS: wrote {len(index['runs'])} reports and {index_path}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path, help="bounded JSON suite plan")
    parser.add_argument(
        "output_directory",
        type=Path,
        help="new directory for raw reports and suite.json",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    run_suite(args.plan, args.output_directory)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(f"benchmark suite failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
