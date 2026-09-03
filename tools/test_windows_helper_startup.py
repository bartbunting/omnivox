#!/usr/bin/env python3
"""Exercise helper protocol startup without proprietary speech runtimes."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
import uuid


REPOSITORY = Path(__file__).resolve().parent.parent
HELPERS = REPOSITORY / "windows-helpers" / "bin"


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--helpers",
        type=Path,
        default=HELPERS,
        help="directory containing the two built helper executables",
    )
    return parser.parse_args()


def request(
    protocol_version: int, request_id: int, kind: str, **fields: object
) -> dict[str, object]:
    value: dict[str, object] = {
        "protocol_version": protocol_version,
        "request_id": request_id,
        "type": kind,
    }
    value.update(fields)
    return value


def check_helper(
    executable: Path, engine: str, dll_name: str, protocol_version: int
) -> None:
    if not executable.is_file():
        raise AssertionError(f"helper executable was not built: {executable}")

    missing_path = (
        rf"C:\omnivox-helper-startup-{uuid.uuid4().hex}\{dll_name}"
    )
    supported_versions = list(range(protocol_version, 0, -1))
    requests = [
        request(
            protocol_version,
            1,
            "hello",
            supported_protocol_versions=supported_versions,
        ),
        request(protocol_version, 2, "describe"),
        request(
            protocol_version,
            3,
            "synthesize",
            text="unavailable runtime",
            settings={"rate": 0.5, "pitch": 1.0, "volume": 1.0},
            anchors=[],
        ),
        request(protocol_version, 4, "ping"),
        request(protocol_version, 5, "shutdown"),
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
    ) != protocol_version:
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
    arguments = parse_arguments()
    for protocol_version in (5, 4):
        check_helper(
            arguments.helpers / "OmnivoxEloquenceHelper32.exe",
            "eloquence",
            "ECI.DLL",
            protocol_version,
        )
        check_helper(
            arguments.helpers / "OmnivoxDectalkHelper32.exe",
            "dectalk",
            "DECtalk.dll",
            protocol_version,
        )
    print("Windows helpers report missing runtimes through the protocol")
    return 0


if __name__ == "__main__":
    sys.exit(main())
