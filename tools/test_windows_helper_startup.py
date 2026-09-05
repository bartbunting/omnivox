#!/usr/bin/env python3
"""Exercise helper protocol startup without proprietary speech runtimes."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
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


def check_dectalk_discovery(executable: Path) -> None:
    def windows_path(path: Path) -> str:
        if os.name == "nt":
            return str(path)
        return subprocess.check_output(
            ["wslpath", "-w", str(path)], text=True
        ).strip()

    def wrong_architecture(path: Path, machine: int) -> None:
        contents = bytearray(128)
        struct.pack_into("<H", contents, 0, 0x5A4D)
        struct.pack_into("<I", contents, 0x3C, 64)
        struct.pack_into("<IH", contents, 64, 0x4550, machine)
        path.write_bytes(contents)

    with tempfile.TemporaryDirectory(dir=executable.resolve().parent) as temporary:
        root = Path(temporary)
        binary_directory = root / "bin"
        runtime_directory = root / "runtime"
        binary_directory.mkdir()
        runtime_directory.mkdir()
        helper = binary_directory / executable.name
        shutil.copy2(executable, helper)
        # The working directory is never an implicit runtime installation.
        (root / "DECtalk.dll").write_bytes(b"untrusted working-directory DLL")
        environment = os.environ.copy()
        for variable in ("OMNIVOX_DECTALK_DLL", "EMACSVOX_DECTALK_DLL"):
            environment[variable] = ""
            entries = environment.get("WSLENV", "").split(":")
            entries = [entry for entry in entries if entry.split("/")[0] != variable]
            environment["WSLENV"] = ":".join(filter(None, [*entries, variable]))

        def describe(*arguments: str) -> dict[str, object]:
            requests = [
                request(5, 1, "hello", supported_protocol_versions=[5]),
                request(5, 2, "describe"),
                request(5, 3, "shutdown"),
            ]
            completed = subprocess.run(
                [str(helper), *arguments],
                input="".join(json.dumps(value) + "\n" for value in requests),
                capture_output=True,
                text=True,
                encoding="utf-8",
                env=environment,
                cwd=root,
                timeout=10,
                check=True,
            )
            frames = [json.loads(line) for line in completed.stdout.splitlines()]
            if len(frames) != 3 or frames[-1].get("type") != "shutting_down":
                raise AssertionError(f"DECtalk discovery broke the protocol: {frames}")
            return frames[1]

        def unavailable_containing(message: str, *arguments: str) -> None:
            response = describe(*arguments)
            if response.get("code") != "not_available" or message not in str(
                response.get("message")
            ):
                raise AssertionError(f"Expected {message!r}: {response}")

        response = describe()
        if response.get("type") == "descriptor":
            descriptor = response["descriptor"]
            if descriptor["id"] != "dectalk" or descriptor["availability"] != {
                "status": "available"
            }:
                raise AssertionError(f"Invalid standard-location runtime: {response}")
        elif response.get("code") != "not_available" or not all(
            expected in str(response.get("message"))
            for expected in (
                r"Omnivox\runtimes\dectalk\x86\DECtalk.dll",
                "dtalk_us.dic",
                "OMNIVOX_DECTALK_DLL",
            )
        ):
            raise AssertionError(f"Missing standard-location guidance: {response}")

        wrong_architecture(runtime_directory / "DECtalk.dll", 0x8664)
        unavailable_containing("machine type 0x8664")
        wrong_architecture(binary_directory / "DECtalk.dll", 0xAA64)
        unavailable_containing("machine type 0xaa64")

        # Explicit selections must fail on their own path, even when another
        # runtime is installed. Also verify argument and environment precedence.
        legacy = windows_path(root / "legacy" / "DECtalk.dll")
        override = windows_path(root / "override" / "DECtalk.dll")
        argument = windows_path(root / "argument" / "DECtalk.dll")
        environment["EMACSVOX_DECTALK_DLL"] = legacy
        unavailable_containing(legacy)
        environment["OMNIVOX_DECTALK_DLL"] = override
        unavailable_containing(override)
        unavailable_containing(argument, argument)


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
    check_dectalk_discovery(arguments.helpers / "OmnivoxDectalkHelper32.exe")
    print("Windows helper startup and DECtalk discovery checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
