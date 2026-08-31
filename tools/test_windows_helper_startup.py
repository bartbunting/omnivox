#!/usr/bin/env python3
"""Exercise helper protocol startup without proprietary speech runtimes."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
import uuid


REPOSITORY = Path(__file__).resolve().parent.parent
HELPERS = REPOSITORY / "windows-helpers" / "bin"


def request(request_id: int, kind: str, **fields: object) -> dict[str, object]:
    value: dict[str, object] = {
        "protocol_version": 4,
        "request_id": request_id,
        "type": kind,
    }
    value.update(fields)
    return value


def check_helper(executable: Path, engine: str, dll_name: str) -> None:
    if not executable.is_file():
        raise AssertionError(f"helper executable was not built: {executable}")

    missing_path = (
        rf"C:\omnivox-helper-startup-{uuid.uuid4().hex}\{dll_name}"
    )
    requests = [
        request(1, "hello", supported_protocol_versions=[4, 3, 2, 1]),
        request(2, "describe"),
        request(
            3,
            "synthesize",
            text="unavailable runtime",
            settings={"rate": 0.5, "pitch": 1.0, "volume": 1.0},
            anchors=[],
        ),
        request(4, "ping"),
        request(5, "shutdown"),
    ]
    stdin = "".join(json.dumps(value) + "\n" for value in requests)
    completed = subprocess.run(
        [str(executable), missing_path],
        input=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        timeout=10,
        check=False,
    )
    if completed.returncode != 0:
        raise AssertionError(
            f"{engine} helper exited {completed.returncode}: {completed.stderr}"
        )

    frames = [json.loads(line) for line in completed.stdout.splitlines()]
    if len(frames) != 5:
        raise AssertionError(
            f"{engine} helper returned {len(frames)} protocol frames: "
            f"{completed.stdout}"
        )
    if frames[0].get("type") != "hello" or frames[0].get(
        "selected_protocol_version"
    ) != 4:
        raise AssertionError(f"{engine} helper did not negotiate: {frames[0]}")
    for unavailable in frames[1:3]:
        if (
            unavailable.get("type") != "error"
            or unavailable.get("code") != "not_available"
            or unavailable.get("retryable") is not False
            or dll_name not in str(unavailable.get("message"))
        ):
            raise AssertionError(
                f"{engine} helper did not report runtime unavailability: "
                f"{unavailable}"
            )
    if frames[3].get("type") != "pong":
        raise AssertionError(f"{engine} helper did not remain responsive")
    if frames[4].get("type") != "shutting_down":
        raise AssertionError(f"{engine} helper did not shut down cleanly")


def main() -> int:
    check_helper(
        HELPERS / "OmnivoxEloquenceHelper32.exe", "eloquence", "ECI.DLL"
    )
    check_helper(
        HELPERS / "OmnivoxDectalkHelper32.exe", "dectalk", "DECtalk.dll"
    )
    print("Windows helpers report missing runtimes through the protocol")
    return 0


if __name__ == "__main__":
    sys.exit(main())
