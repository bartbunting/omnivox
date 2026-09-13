#!/usr/bin/env python3
"""CI-only baseline/candidate comparison using the existing server benchmark."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


VOICES = {
    # The macOS 26 hosted image includes super-compact Samantha. Parham's
    # compact variant is a separate voice ID and is not installed on this image.
    "macos": "com.apple.voice.super-compact.en-US.Samantha",
    "espeak": "espeak:gmw/en-US",
}
CASES = ("character", "word", "line")


def run_order() -> list[tuple[str, str, str]]:
    # Reverse engine and build order on the second pass to expose host drift.
    return [
        (f"{round_id}-{engine}-{build}", engine, build)
        for round_id, engines, builds in (
            (1, ("macos", "espeak"), ("baseline", "candidate")),
            (2, ("espeak", "macos"), ("candidate", "baseline")),
        )
        for engine in engines
        for build in builds
    ]


def summarize(output: Path, runs: list[dict]) -> None:
    lines = [
        "# macOS speech timing comparison",
        "",
        "Null output: software source-start latency, not audible onset. "
        "Lower is better. Delta is candidate minus baseline; negative means faster.",
        "",
        "Cold means a fresh server process, not a cold macOS voice service. "
        "Warm reuses a server after two warmups per case. "
        "Each pass uses five measured samples per mode and case.",
        "",
        "Order: " + ", ".join(run["id"] for run in runs) + ".",
        "",
    ]
    failed = [run["id"] for run in runs if run["exit_code"] != 0]
    if failed:
        lines.extend([
            "INCOMPLETE: failed runs: " + ", ".join(failed) + ". "
            "No aggregate comparison is shown. Inspect the retained logs.",
            "",
        ])
    else:
        # Pool raw observations, never percentiles from individual runs.
        sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
        from benchmark_server import nearest_rank

        samples: dict[tuple[str, str, str, str], list[dict]] = {}
        for run in runs:
            report = json.loads((output / run["id"] / "report.json").read_text())
            for mode, cases in report["results"].items():
                for case, result in cases.items():
                    key = run["engine"], mode, case, run["build"]
                    samples.setdefault(key, []).extend(result["samples"])
        for metric, title in (
            ("dispatch_to_source_ms", "Dispatch to first source marker"),
            ("process_start_to_source_ms", "Process start to first source marker (cold)"),
        ):
            lines.extend([
                "## " + title,
                "",
                "All values are milliseconds. p50/p95 use nearest rank; "
                "ten samples give only a rough view of the tail.",
                "",
                "| Engine | Mode | Case | n per build | Baseline p50 / p95 | Candidate p50 / p95 | p50 delta |",
                "|---|---|---|---:|---:|---:|---:|",
            ])
            for engine in VOICES:
                for mode in ("cold", "warm"):
                    if metric.startswith("process_start") and mode != "cold":
                        continue
                    for case in CASES:
                        before, after = [
                            [sample[metric] for sample in samples[engine, mode, case, build]]
                            for build in ("baseline", "candidate")
                        ]
                        b50, a50 = nearest_rank(before, 0.5), nearest_rank(after, 0.5)
                        lines.append(
                            f"| {engine} | {mode} | {case} | {len(before)} / {len(after)} | "
                            f"{b50:.1f} / {nearest_rank(before, 0.95):.1f} | "
                            f"{a50:.1f} / {nearest_rank(after, 0.95):.1f} | {a50 - b50:+.1f} |"
                        )
            lines.append("")
    lines.extend([
        "Candidate macOS `server-*.log` files retain `macos_buffer_capture` "
        "records: first/last PCM, completion reason, and first-buffer-to-return time. "
        "These include warmups; match request IDs with raw samples before comparing cases.",
        "",
        "No latency threshold is enforced on shared CI hardware. "
        "The runner's voice inventory and OS service state can differ from the tester's Mac. "
        "Device output, Bluetooth, and acoustic onset still require a listening test.",
        "",
    ])
    (output / "summary.md").write_text("\n".join(lines), encoding="utf-8")


def compare(baseline: Path, candidate: Path, output: Path) -> int:
    output.mkdir(parents=True, exist_ok=True)
    binaries = {
        name: source / "target/release/omnivox"
        for name, source in (("baseline", baseline), ("candidate", candidate))
    }
    for name, source in (("baseline", baseline), ("candidate", candidate)):
        binary = binaries[name]
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=source, text=True
        ).strip()
        (output / f"{name}-provenance.txt").write_text(
            f"SOURCE_COMMIT={commit}\n"
            f"BINARY_SHA256={hashlib.sha256(binary.read_bytes()).hexdigest()}\n"
            "BUILD_FLAGS=--release --package omnivox-cli --features piper\n",
            encoding="utf-8",
        )
        for engine in VOICES:
            with (output / f"{name}-{engine}-voices.txt").open("w") as log:
                subprocess.run(
                    [str(binary), "--engine", engine, "--list-voices"],
                    stdout=log, stderr=subprocess.STDOUT, check=True, timeout=60,
                )

    # benchmark_server consumes stderr internally. Redirect at exec, preserving
    # the protocol on stdout and the server PID for timeout cleanup and log IDs.
    wrapper = output / "server"
    wrapper.write_text(
        '#!/bin/sh\nexec "$BENCHMARK_BINARY" "$@" '
        '2>"$BENCHMARK_LOG_DIR/server-$$.log"\n', encoding="utf-8"
    )
    wrapper.chmod(0o755)
    runs: list[dict] = []
    for identifier, engine, build in run_order():
        directory = output / identifier
        directory.mkdir()  # Do not mix a rerun with earlier evidence.
        environment = os.environ.copy()
        environment.update(
            BENCHMARK_BINARY=str(binaries[build]), BENCHMARK_LOG_DIR=str(directory)
        )
        command = [
            sys.executable, str(candidate / "tools/benchmark_server.py"), str(wrapper),
            "--engine", engine, "--expected-engine-id", engine,
            "--voice-id", VOICES[engine], "--null-audio", "--mode", "both",
            "--iterations", "5", "--warmups", "2", "--timeout", "30",
            "--json-output", str(directory / "report.json"),
            "--provenance", str(output / f"{build}-provenance.txt"),
        ]
        for case in CASES:
            command.extend(("--case", case))
        print(f"Running {identifier}", flush=True)
        with (directory / "benchmark.txt").open("w") as log:
            result = subprocess.run(command, env=environment, stdout=log, stderr=subprocess.STDOUT)
        runs.append({
            "id": identifier, "engine": engine, "build": build,
            "exit_code": result.returncode,
        })
        (output / "runs.json").write_text(json.dumps(runs, indent=2) + "\n")
    summarize(output, runs)
    return int(any(run["exit_code"] for run in runs))


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit("usage: macos_timings.py BASELINE_CHECKOUT CANDIDATE_CHECKOUT OUTPUT")
    raise SystemExit(compare(*(Path(arg).resolve() for arg in sys.argv[1:])))
