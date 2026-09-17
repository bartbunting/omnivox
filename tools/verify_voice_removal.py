#!/usr/bin/env python3
"""Acquire, pin, uninstall and reinstall reviewed voices in a private native root."""
import argparse
import base64
import contextlib
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import uuid

from verify_local_voice_owner import Peer
from verify_release import read_wav
from verify_voice_library_startup import native


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    parser.add_argument("--catalogue", type=Path, required=True)
    parser.add_argument("--entry", action="append")
    parser.add_argument("--mbrola-helper", type=Path)
    parser.add_argument("--rhvoice-library", type=Path)
    parser.add_argument("--rhvoice-data", type=Path)
    parser.add_argument("--scratch-dir", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    server = args.server.resolve()
    windows = server.suffix.lower() == ".exe"
    root = Path(tempfile.mkdtemp(prefix="omnivox uninstall ", dir=args.scratch_dir))
    print(f"Private native removal root: {root}", flush=True)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("OMNIVOX_", "LD_", "DYLD_")) and key != "ESPEAK_NG_DATA"}
    environment.update(OMNIVOX_VOICE_ROOT=native(root, windows), OMNIVOX_ENGINE="espeak", OMNIVOX_AUDIO_OUTPUT="null")
    if args.mbrola_helper:
        environment["OMNIVOX_MBROLA_HELPER"] = native(args.mbrola_helper.resolve(), windows)
    if args.rhvoice_library:
        environment["OMNIVOX_RHVOICE_LIBRARY"] = native(args.rhvoice_library.resolve(), windows)
    if args.rhvoice_data:
        environment["OMNIVOX_RHVOICE_DATA"] = native(args.rhvoice_data.resolve(), windows)
    forwarded = ["OMNIVOX_RHVOICE_LIBRARY", "OMNIVOX_RHVOICE_DATA", "OMNIVOX_VOICE_ROOT", "OMNIVOX_ENGINE", "OMNIVOX_AUDIO_OUTPUT", "OMNIVOX_MBROLA_HELPER", "OMNIVOX_VOICE_LIBRARY"]
    environment["WSLENV"] = ":".join([item for item in environment.get("WSLENV", "").split(":")
                                       if item and item.split("/")[0] not in forwarded] + forwarded)
    catalogue = json.loads(args.catalogue.read_text())
    entries = [entry for entry in catalogue["entries"] if not args.entry or entry["id"] in args.entry]
    assert entries

    def service(command, **fields):
        result = subprocess.run([str(server), "--voice-library-service"], env=environment,
                                input=json.dumps(dict(request_id=1, command=command, **fields)) + "\n",
                                capture_output=True, text=True, timeout=300, check=True)
        reply = json.loads(result.stdout.removeprefix("OMNIVOX-LOCAL "))
        assert reply["type"] != "error", reply
        return reply

    @contextlib.contextmanager
    def locked_file(path):
        """Use a real Windows handle that permits reads but refuses deletion."""
        script = ("$ErrorActionPreference='Stop'; "
                  "$file=[IO.File]::Open($env:OMNIVOX_TEST_LOCK_FILE,'Open','Read','Read'); "
                  "try { [Console]::WriteLine('locked'); [Console]::ReadLine() | Out-Null } "
                  "finally { $file.Dispose() }")
        encoded = base64.b64encode(script.encode("utf-16le")).decode()
        lock_environment = dict(environment, OMNIVOX_TEST_LOCK_FILE=path,
                                WSLENV=environment["WSLENV"] + ":OMNIVOX_TEST_LOCK_FILE")
        with subprocess.Popen(["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded],
                              env=lock_environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True) as process:
            messages = queue.Queue()
            reader = threading.Thread(target=lambda: messages.put(process.stdout.readline()), daemon=True)
            reader.start()
            try:
                assert messages.get(timeout=30).strip() == "locked", "Windows file lock was not established"
                yield
            finally:
                process.stdin.close()
                process.wait(timeout=15)
                reader.join(timeout=5)
            assert process.returncode == 0, process.stderr.read()

    def acquire(entry):
        messages = queue.Queue()
        with tempfile.TemporaryFile() as diagnostics, subprocess.Popen(
                [str(server), "--voice-library-acquire"], env=environment,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=diagnostics, text=True) as process:
            def read():
                for line in process.stdout:
                    messages.put(json.loads(line.removeprefix("OMNIVOX-LOCAL ")))
                messages.put(None)
            reader = threading.Thread(target=read, daemon=True)
            reader.start()
            process.stdin.write(json.dumps(dict(request_id=1, command="acquire", voice=entry["id"], plan_json=json.dumps(catalogue))) + "\n")
            process.stdin.flush()
            last = None
            try:
                while (message := messages.get(timeout=180)) is not None:
                    last = message
                assert process.wait(timeout=15) == 0
                assert last["progress"]["state"] == "installed-disabled", last
            finally:
                process.stdin.close()
                process.wait(timeout=150)
                reader.join(timeout=5)

    host = service("host")
    assert host["removal_version"] == 1
    external_rhvoice = None
    if all(entry["provider"] == "rhvoice" for entry in entries):
        owner = Peer(server, "--voice-library-owner", environment)
        try:
            status = owner.request("voice_library_status_v1", control=True)
            external_rhvoice = {voice["voice_id"] for voice in status["eligible_voices"]
                                if voice["engine_id"] == "rhvoice"}
        finally:
            owner.close()
    results = []
    for entry in entries:
        acquire(entry)
        engine = entry["provider"]
        voices = entry["voices"]
        first = voices[0]["physical_id"]

        def enable(enabled):
            for voice in voices:
                current = service("inspect")
                service("enable", engine=engine, voice=voice["physical_id"], enabled=enabled,
                        expected_sha256=current["sha256"])

        def review():
            return service("uninstall-preview", engine=engine, voice=first,
                           expected_sha256=service("inspect")["sha256"])["review"]

        enable(True)
        assert review()["blockers"], "enabled shared speakers must block removal"
        generation = str(uuid.uuid4())
        candidate = service("stage", generation=generation, expected_sha256=service("inspect")["sha256"], **{engine: True})
        owned_environment = dict(environment, OMNIVOX_VOICE_LIBRARY=candidate["path"])
        owners = []
        identities = []
        try:
            for _ in range(2):
                owners.append(Peer(server, "--voice-library-owner", owned_environment))
                # Initial admission briefly owns storage; acknowledge it before
                # starting the next lane, as the coordinated Apply flow does.
                identities.append(owners[-1].request("describe"))
            assert all(not owner["retired"] and not owner["startup_error"] for owner in identities), identities
            enable(False)
            pinned = review()
            assert len(pinned["voices"]) == len(voices)
            assert pinned["blockers"], "live native sessions must retain assets"
            blocked = service("uninstall", operation=pinned["operation_id"], expected_sha256=pinned["plan_sha256"])["result"]
            assert blocked["status"] == "blocked" and blocked["removed_bytes"] == 0
            owners[0].request("retire", worker=identities[0]["worker"])
            assert review()["blockers"], "notification owner must independently retain assets"
            owners[1].request("retire", worker=identities[1]["worker"])
        finally:
            for owner in owners:
                owner.close()
        ready = review()
        assert not ready["blockers"], ready
        if windows:
            package = next(package for package in service("inspect")["index"]["packages"]
                           if package["package_id"] == ready["package_id"])
            with locked_file(package["files"][0]["path"]):
                partial = service("uninstall", operation=ready["operation_id"],
                                  expected_sha256=ready["plan_sha256"])["result"]
                assert partial["status"] == "partial" and partial["remaining_bytes"] > 0, partial
                assert not any(row["physical_id"] == first for row in service("inspect")["index"]["voices"])
                assert any(item["operation_id"] == ready["operation_id"]
                           for item in service("uninstall-pending")["reviews"])
        result = service("uninstall", operation=ready["operation_id"], expected_sha256=ready["plan_sha256"])["result"]
        assert result["status"] == "complete" and result["removed_bytes"] == ready["package_bytes"], result
        assert result["remaining_bytes"] == result["unconfirmed_bytes"] == 0
        assert not (root / "packages" / ready["package_id"] / ready["revision_id"]).exists()
        after = service("inspect")
        assert not any(row["physical_id"] == first for row in after["index"]["voices"])
        if engine == "mbrola":
            assert any(row["physical_id"] == "mbrola:v1/mb-en1/en1" and row["enabled"] for row in after["index"]["voices"])
        acquire(entry)
        reinstalled = service("inspect")
        assert any(row["physical_id"] == first and not row["enabled"] for row in reinstalled["index"]["voices"])
        results.append(dict(entry=entry["id"], removed_bytes=result["removed_bytes"], shared_speakers=len(voices),
                            two_native_owners=True, reinstalled_disabled=True, native_file_lock_recovery=windows))
        print(f"PASS {entry['id']}: two owners, removal and disabled reinstallation", flush=True)
        args.report.write_text(json.dumps(dict(binary_sha256=hashlib.sha256(server.read_bytes()).hexdigest(),
                                              catalogue_sha256=hashlib.sha256(args.catalogue.read_bytes()).hexdigest(),
                                              native_windows=windows, results=results), indent=2) + "\n")

    if external_rhvoice is not None:
        managed = {voice["physical_id"] for entry in entries for voice in entry["voices"]}
        for physical in sorted(managed):
            service("enable", engine="rhvoice", voice=physical, enabled=True,
                    expected_sha256=service("inspect")["sha256"])

        def check_rhvoice_set(expected):
            candidate = service("stage", generation=str(uuid.uuid4()), rhvoice=True,
                                expected_sha256=service("inspect")["sha256"])
            owned = dict(environment, OMNIVOX_VOICE_LIBRARY=candidate["path"])
            owner = Peer(server, "--voice-library-owner", owned)
            try:
                identity = owner.request("describe")
                assert not identity["retired"] and not identity["startup_error"], identity
                status = owner.request("voice_library_status_v1", control=True)
                actual = {voice["voice_id"] for voice in status["eligible_voices"]
                          if voice["engine_id"] == "rhvoice"}
                assert actual == expected | external_rhvoice, (actual, expected, external_rhvoice)
            finally:
                owner.close()
            for physical in sorted(expected | external_rhvoice):
                wav = root / (physical.split(":")[-1] + ".wav")
                result = subprocess.run([str(server), "--engine", "rhvoice", "--dump-wav",
                                         physical, native(wav, windows), "Managed RHVoice acceptance."],
                                        env=owned, capture_output=True, text=True, timeout=120)
                assert result.returncode == 0, result.stderr
                read_wav(wav, canonical=True)

        check_rhvoice_set(managed)
        removed = sorted(managed)[0]
        service("enable", engine="rhvoice", voice=removed, enabled=False,
                expected_sha256=service("inspect")["sha256"])
        ready = service("uninstall-preview", engine="rhvoice", voice=removed,
                        expected_sha256=service("inspect")["sha256"])["review"]
        assert not ready["blockers"], ready
        result = service("uninstall", operation=ready["operation_id"],
                         expected_sha256=ready["plan_sha256"])["result"]
        assert result["status"] == "complete", result
        check_rhvoice_set(managed - {removed})
        report = json.loads(args.report.read_text())
        report["rhvoice_combined"] = dict(external_voices=sorted(external_rhvoice),
                                          managed_voices=sorted(managed), removed_voice=removed,
                                          remaining_voices_synthesize=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
        print("PASS RHVoice: combined synthesis, external voices and independent removal", flush=True)


if __name__ == "__main__":
    main()
