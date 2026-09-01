#!/usr/bin/env python3
"""Conservative standard-library process resource observation."""

from __future__ import annotations

import json
from pathlib import Path
import platform
import shutil
import subprocess
import time
from typing import Any


WINDOWS_PROCESS_FIELDS = (
    "ProcessId,ParentProcessId,Name,ExecutablePath,WorkingSetSize,PrivatePageCount,"
    "HandleCount,ThreadCount,KernelModeTime,UserModeTime"
)


def proc_metrics(process_id: int) -> dict[str, int] | None:
    status_path = Path(f"/proc/{process_id}/status")
    try:
        values = {}
        for line in status_path.read_text(encoding="utf-8").splitlines():
            key, separator, value = line.partition(":")
            if separator:
                values[key] = value.strip()
        file_descriptors = sum(1 for _ in Path(f"/proc/{process_id}/fd").iterdir())
        return {
            "working_set_bytes": int(values["VmRSS"].split()[0]) * 1024,
            "virtual_bytes": int(values["VmSize"].split()[0]) * 1024,
            "thread_count": int(values["Threads"]),
            "handle_count": file_descriptors,
        }
    except (FileNotFoundError, KeyError, OSError, ValueError):
        return None


def windows_process_snapshot(powershell: str) -> dict[int, dict[str, Any]]:
    command = (
        "Get-CimInstance Win32_Process | "
        f"Select-Object {WINDOWS_PROCESS_FIELDS} | ConvertTo-Json -Compress"
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
    snapshot = {}
    for row in rows:
        if not isinstance(row, dict) or row.get("ProcessId") is None:
            continue
        process_id = int(row["ProcessId"])
        snapshot[process_id] = {
            "parent": integer_or_none(row.get("ParentProcessId")),
            "name": row.get("Name") or "",
            "path": row.get("ExecutablePath"),
            "working_set_bytes": integer_or_none(row.get("WorkingSetSize")),
            "private_bytes": integer_or_none(row.get("PrivatePageCount")),
            "handle_count": integer_or_none(row.get("HandleCount")),
            "thread_count": integer_or_none(row.get("ThreadCount")),
            "cpu_100ns": sum_optional_integers(
                row.get("KernelModeTime"), row.get("UserModeTime")
            ),
        }
    return snapshot


def integer_or_none(value: Any) -> int | None:
    try:
        return None if value is None else int(value)
    except (TypeError, ValueError):
        return None


def sum_optional_integers(*values: Any) -> int | None:
    converted = [integer_or_none(value) for value in values]
    if any(value is None for value in converted):
        return None
    return sum(value for value in converted if value is not None)


def executable_name(path: str) -> str:
    return path.replace("\\", "/").rsplit("/", 1)[-1]


class ProcessObserver:
    """Observe one launched process without guessing among ambiguous targets."""

    def __init__(self, executable: str) -> None:
        self.executable = executable
        self.name = executable_name(executable)
        self.powershell = shutil.which("powershell.exe") or (
            shutil.which("powershell") if platform.system() == "Windows" else None
        )
        self.provider = (
            "windows-cim"
            if self.powershell and self.name.casefold().endswith(".exe")
            else "procfs"
        )
        self.process_id: int | None = None
        self.error: str | None = None
        self.before: dict[int, dict[str, Any]] = {}
        if self.provider == "windows-cim":
            try:
                assert self.powershell is not None
                self.before = windows_process_snapshot(self.powershell)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                self.provider = "unavailable"
                self.error = f"Windows process snapshot failed: {error}"

    def bind(self, launcher_process_id: int) -> None:
        if self.provider == "procfs":
            self.process_id = launcher_process_id
            if proc_metrics(launcher_process_id) is None:
                self.provider = "unavailable"
                self.error = "launcher process is not observable through procfs"
            return
        if self.provider != "windows-cim":
            return

        assert self.powershell is not None
        deadline = time.monotonic() + 5
        candidates: list[int] = []
        while time.monotonic() < deadline:
            try:
                snapshot = windows_process_snapshot(self.powershell)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                self.provider = "unavailable"
                self.error = f"Windows process snapshot failed: {error}"
                return
            candidates = [
                process_id
                for process_id, process in snapshot.items()
                if process_id not in self.before
                and process["name"].casefold() == self.name.casefold()
            ]
            if len(candidates) == 1:
                self.process_id = candidates[0]
                return
            if len(candidates) > 1:
                break
            time.sleep(0.05)
        self.provider = "unavailable"
        self.error = (
            f"expected one new Windows process named {self.name!r}, "
            f"found {candidates}"
        )

    def sample(self) -> dict[str, int] | None:
        if self.process_id is None:
            return None
        if self.provider == "procfs":
            return proc_metrics(self.process_id)
        if self.provider == "windows-cim":
            assert self.powershell is not None
            try:
                process = windows_process_snapshot(self.powershell).get(self.process_id)
                if process is None:
                    return None
                return {
                    key: value
                    for key, value in process.items()
                    if key not in ("parent", "name", "path")
                    and isinstance(value, int)
                }
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                self.error = f"Windows process sampling failed: {error}"
        return None

    def description(self) -> dict[str, Any]:
        return {
            "provider": self.provider,
            "process_id": self.process_id,
            "error": self.error,
        }


def summarize_samples(samples: list[dict[str, Any]]) -> dict[str, Any]:
    fields = (
        "process_count",
        "working_set_bytes",
        "private_bytes",
        "virtual_bytes",
        "handle_count",
        "thread_count",
        "cpu_100ns",
    )
    summary = {"sample_count": len(samples), "metrics": {}}
    for field in fields:
        values = [
            sample[field]
            for sample in samples
            if isinstance(sample.get(field), int)
        ]
        if values:
            summary["metrics"][field] = {
                "first": values[0],
                "last": values[-1],
                "minimum": min(values),
                "maximum": max(values),
                "growth": values[-1] - values[0],
            }
    return summary


def summarize_tree_samples(samples: list[dict[str, Any]]) -> dict[str, Any]:
    summary = summarize_samples(samples)
    names = sorted(
        {
            name
            for sample in samples
            for name in sample.get("by_name", {})
        }
    )
    summary["by_name"] = {
        name: summarize_samples(
            [
                sample["by_name"].get(name, {"process_count": 0})
                for sample in samples
            ]
        )
        for name in names
    }
    return summary


def proc_process_snapshot() -> dict[int, dict[str, Any]]:
    snapshot = {}
    for status_path in Path("/proc").glob("[0-9]*/status"):
        try:
            values = {}
            for line in status_path.read_text(encoding="utf-8").splitlines():
                key, separator, value = line.partition(":")
                if separator and key in ("Name", "PPid"):
                    values[key] = value.strip()
            process_id = int(status_path.parent.name)
            metrics = proc_metrics(process_id)
            if metrics is None:
                continue
            snapshot[process_id] = {
                "parent": int(values["PPid"]),
                "name": values["Name"],
                **metrics,
            }
        except (FileNotFoundError, KeyError, OSError, ValueError):
            continue
    return snapshot


def tree_process_ids(
    root_process_id: int,
    snapshot: dict[int, dict[str, Any]],
) -> set[int]:
    selected = {root_process_id} if root_process_id in snapshot else set()
    changed = True
    while changed:
        changed = False
        for process_id, process in snapshot.items():
            if process_id not in selected and process.get("parent") in selected:
                selected.add(process_id)
                changed = True
    return selected


def aggregate_tree(
    root_process_id: int,
    snapshot: dict[int, dict[str, Any]],
) -> dict[str, Any] | None:
    process_ids = tree_process_ids(root_process_id, snapshot)
    if not process_ids:
        return None
    metric_names = (
        "working_set_bytes",
        "private_bytes",
        "virtual_bytes",
        "handle_count",
        "thread_count",
        "cpu_100ns",
    )
    aggregate: dict[str, Any] = {"process_count": len(process_ids), "by_name": {}}
    for process_id in process_ids:
        process = snapshot[process_id]
        name = str(process.get("name") or "unknown")
        named = aggregate["by_name"].setdefault(
            name, {"process_count": 0}
        )
        named["process_count"] += 1
        for metric in metric_names:
            value = process.get(metric)
            if isinstance(value, int):
                aggregate[metric] = aggregate.get(metric, 0) + value
                named[metric] = named.get(metric, 0) + value
    return aggregate


class ProcessTreeObserver:
    """Observe one new process root and all of its current descendants."""

    def __init__(self, executable: str, windows_process_name: str | None = None) -> None:
        self.name = windows_process_name or executable_name(executable)
        self.powershell = shutil.which("powershell.exe") or (
            shutil.which("powershell") if platform.system() == "Windows" else None
        )
        use_windows = self.powershell and (
            windows_process_name is not None or platform.system() == "Windows"
        )
        self.provider = (
            "windows-cim-tree"
            if use_windows
            else "procfs-tree"
        )
        self.process_id: int | None = None
        self.error: str | None = None
        self.before: dict[int, dict[str, Any]] = {}
        if self.provider == "windows-cim-tree":
            try:
                assert self.powershell is not None
                self.before = windows_process_snapshot(self.powershell)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                self.provider = "unavailable"
                self.error = f"Windows process snapshot failed: {error}"

    def bind(self, launcher_process_id: int) -> None:
        if self.provider == "procfs-tree":
            self.process_id = launcher_process_id
            if launcher_process_id not in proc_process_snapshot():
                self.provider = "unavailable"
                self.error = "server process is not observable through procfs"
            return
        if self.provider != "windows-cim-tree":
            return

        assert self.powershell is not None
        deadline = time.monotonic() + 5
        candidates: list[int] = []
        while time.monotonic() < deadline:
            try:
                snapshot = windows_process_snapshot(self.powershell)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                self.provider = "unavailable"
                self.error = f"Windows process snapshot failed: {error}"
                return
            candidates = [
                process_id
                for process_id, process in snapshot.items()
                if process_id not in self.before
                and process["name"].casefold() == self.name.casefold()
            ]
            if len(candidates) == 1:
                self.process_id = candidates[0]
                return
            if len(candidates) > 1:
                break
            time.sleep(0.05)
        self.provider = "unavailable"
        self.error = (
            f"expected one new Windows process named {self.name!r}, "
            f"found {candidates}"
        )

    def sample(self) -> dict[str, Any] | None:
        if self.process_id is None:
            return None
        try:
            snapshot = (
                windows_process_snapshot(self.powershell)
                if self.provider == "windows-cim-tree" and self.powershell
                else proc_process_snapshot()
            )
            return aggregate_tree(self.process_id, snapshot)
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            self.error = f"process-tree sampling failed: {error}"
            return None

    def description(self) -> dict[str, Any]:
        return {
            "provider": self.provider,
            "process_id": self.process_id,
            "process_name": self.name,
            "error": self.error,
        }
