#!/usr/bin/env python3
"""Summarize retained matched source timings; empirical p95 is nearest rank."""
import argparse
import json
import math
import statistics
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("matrix", type=Path)
args = parser.parse_args()


def summary(values):
    values = sorted(values)
    return dict(n=len(values), median_ms=statistics.median(values),
                p95_ms=values[math.ceil(.95 * len(values)) - 1],
                maximum_ms=max(values), over_100ms=sum(x > 100 for x in values))


cells = []
for workload in ("ordered", "letter"):
    for case in ("release", "receipt", "candidate-idle", "candidate-reload"):
        runs = [json.loads((args.matrix / f"{workload}-{block}-{case}" / "result.json").read_text())
                for block in range(4)]
        assert all(run["success"] and len(run["samples"]) == 256 for run in runs)
        cell = dict(workload=workload, case=case,
                    source=summary(s["source_ms"] for run in runs for s in run["samples"]))
        if case == "candidate-reload":
            cell["overlapping_source"] = summary(
                s["source_ms"] for run in runs for s in run["samples"]
                if any(s["start_ns"] < op["end_ns"] and s["end_ns"] > op["start_ns"]
                       for op in run["operations"]))
            cell["reload"] = summary(op["ms"] for run in runs for op in run["operations"])
        cells.append(cell)
print(json.dumps(cells, indent=2))
