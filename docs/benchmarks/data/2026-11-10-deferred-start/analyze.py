#!/usr/bin/env python3
"""Summarize retained paired observations without claiming tail percentiles."""
import argparse
import json
from pathlib import Path
import re
import statistics

parser = argparse.ArgumentParser()
parser.add_argument("root", type=Path)
args = parser.parse_args()
rows = []
for run in json.loads((args.root / "runs.json").read_text()):
    directory = args.root / f"pair-{run['pair']:02d}-{run['variant']}"
    log = (directory / "stdout.log").read_text()
    timing = re.search(r"Emergency recovery ([\d.]+) ms; concurrent notification submit/source/end ([\d.]+)/([\d.]+)/([\d.]+) ms", log)
    retirement = re.search(r"Retirement request/confirmed ([\d.]+)/([\d.]+) ms; retirement-request to first requested source ([\d.]+) ms", log)
    assert run["exit"] == 0 and timing and retirement, directory
    total, submitted, source, completed = map(float, timing.groups())
    retire_start, retire_end, gap = map(float, retirement.groups())
    row = {key: run[key] for key in ("pair", "variant", "warmup", "profiled")}
    row.update(total_ms=total, before_retirement_ms=retire_start,
               retirement_ms=retire_end-retire_start, after_retirement_ms=total-retire_end,
               source_gap_upper_bound_ms=gap, notification_source_ms=source-submitted,
               notification_complete_ms=completed-submitted)
    profile = json.loads((directory / "profile.json").read_text())
    row["startup_spans"] = profile["operations"][0]["spans"]
    rows.append(row)
summary = {}
for variant in ("legacy", "deferred"):
    measured = [r for r in rows if r["variant"] == variant and not r["warmup"] and not r["profiled"]]
    summary[variant] = {"n": len(measured)}
    for key in ("total_ms", "before_retirement_ms", "retirement_ms", "after_retirement_ms",
                "source_gap_upper_bound_ms", "notification_source_ms", "notification_complete_ms"):
        values = [r[key] for r in measured]
        summary[variant][key] = {"median": statistics.median(values), "min": min(values), "max": max(values)}
result = {"summary": summary, "rows": rows}
(args.root / "analysis.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(summary, indent=2))
