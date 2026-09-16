#!/usr/bin/env python3
"""Silent native voice validation and Linux/macOS ownership fault probes."""
import argparse
import copy
import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time


def asset(path):
    data = path.read_bytes()
    return {"path": str(path), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    parser.add_argument("--flite-helper", type=Path, required=True)
    parser.add_argument("--piper-helper", type=Path, required=True)
    parser.add_argument("--flite-tests", type=Path, help="native test binary that can export bundled SLT")
    args = parser.parse_args()
    assert sys.platform in {"linux", "darwin"}, "Linux or macOS fault-probe runner required"
    if sys.platform == "linux":
        # Adopt this test's descendants if a killed supervisor exits. macOS
        # instead relies on launchd and observes the recorded identities vanish.
        libc = ctypes.CDLL(None, use_errno=True)
        assert libc.prctl(36, 1, 0, 0, 0) == 0
    else:
        class MacUsage(ctypes.Structure):
            _fields_ = [("uuid", ctypes.c_ubyte * 16)] + [
                (field, ctypes.c_uint64) for field in (
                    "user_time", "system_time", "package_idle_wakeups", "interrupt_wakeups",
                    "pageins", "wired_size", "resident_size", "physical_footprint",
                    "process_start", "process_exit")]
        libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.POINTER(MacUsage)]
        libproc.proc_pid_rusage.restype = ctypes.c_int

    def identity(pid):
        if sys.platform == "linux":
            try:
                return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
            except FileNotFoundError:
                return None
        record = MacUsage()
        if libproc.proc_pid_rusage(pid, 0, ctypes.byref(record)) == -1:
            error = ctypes.get_errno()
            if error == errno.ESRCH:
                return None
            raise OSError(error, os.strerror(error))
        assert record.process_start, "missing native process identity"
        return record.process_start
    server = args.server.resolve()
    source = Path(__file__).resolve().parent.parent / "test-fixtures/piper-speakers"
    with tempfile.TemporaryDirectory(prefix="omnivox-validation-probe-") as name:
        root = Path(name)
        environment = {key: value for key, value in os.environ.items() if not key.startswith("OMNIVOX_")}
        environment["TMPDIR"] = str(root)
        model = root / "alpha.onnx"
        config = root / "alpha.onnx.json"
        model.write_bytes((source / "alpha.onnx").read_bytes())
        config.write_bytes((source / "config.json").read_bytes())
        document = {"schema_version": 1, "target_id": "11111111-1111-4111-8111-111111111111",
                    "profile_id": "22222222-2222-4222-8222-222222222222",
                    "generation_id": "33333333-3333-4333-8333-333333333333",
                    "disabled_physical_ids": [], "flite": {"builtin_slt": True, "files": []},
                    "piper": {"models": [{"identity": {"catalogue_key": "alpha"}, "model": asset(model),
                       "config": asset(config), "voices": [
                         {"physical_id": f"piper:v1/c/alpha/{speaker}", "speaker_index": speaker,
                          "display_name": f"Alpha {speaker}", "language": None} for speaker in [0, 1]]}]}}
        if args.flite_tests:
            external = root / "exported.flitevox"
            subprocess.run([str(args.flite_tests.resolve()), "--exact", "library::tests::export_bundled_fixture"],
                           env={**environment, "OMNIVOX_FLITE_TEST_EXPORT": str(external)},
                           check=True, timeout=20, stdout=subprocess.DEVNULL)
            document["flite"]["files"] = [{"physical_id": "flitevox:cmu_us_slt", "file": asset(external),
                                          "display_name": "External SLT", "language": None}]
        path = root / "generation.json"
        native = ["--piper-helper", str(args.piper_helper.resolve()), "--flite-helper", str(args.flite_helper.resolve())]
        def start(doc, flags=native, timeout=60):
            path.write_text(json.dumps(doc))
            output = tempfile.TemporaryFile(mode="w+")
            errors = tempfile.TemporaryFile(mode="w+")
            process = subprocess.Popen([str(server), "--validate-voice-library", str(path), *flags,
                                        "--validation-timeout-seconds", str(timeout)],
                                       env=environment, stdin=subprocess.PIPE, stdout=output, stderr=errors)
            return process, output, errors
        def finish(run, success):
            process, output, errors = run
            try:
                result = process.wait(timeout=180)
                output.seek(0); errors.seek(0)
                text, diagnostics = output.read(), errors.read()
                assert (result == 0) == success, (result, text, diagnostics)
                return text, diagnostics
            finally:
                process.stdin.close()
                if process.poll() is None:
                    process.kill(); process.wait(timeout=10)
                output.close(); errors.close()
        text, _ = finish(start(document), True)
        assert "cleanup confirmed" in text and "Validated piper" in text and "Validated flite" in text
        assert not list(root.glob("omnivox-voice-validation-*")), "successful scratch was retained"
        print("Piper speakers and Flite native probes passed without playback", flush=True)
        report = root / "validation-report.json"
        report_flags = [*native, "--validation-report", str(report)]
        text, _ = finish(start(document, report_flags), True)
        saved = report.read_bytes()
        evidence = json.loads(saved)
        assert evidence["cleanup_confirmed"] and evidence["snapshot"]["generation_sha256"] == asset(path)["sha256"]
        assert len(evidence["snapshot"]["loads"][0]["voices"]) == 2
        assert "Saved validation evidence" in text
        compare_flags = [*native, "--check-validation-report", str(report)]
        text, _ = finish(start(document, compare_flags), True)
        assert "observations match" in text and "Validated generation" not in text
        finish(start(document, report_flags), False)
        assert report.read_bytes() == saved, "existing report was overwritten"
        finish(start(document, compare_flags, timeout=59), False)
        changed = copy.deepcopy(document)
        changed["piper"]["models"][0]["voices"].pop()
        finish(start(changed, compare_flags), False)
        rejected_report = root / "rejected-report.json"
        rejected_flags = [*native, "--validation-report", str(rejected_report)]
        environment["OMNIVOX_PIPER_ESPEAK_DATA"] = str(root)
        try:
            _, diagnostics = finish(start(document, rejected_flags), False)
            assert "runtime override" in diagnostics
        finally:
            del environment["OMNIVOX_PIPER_ESPEAK_DATA"]
        assert not rejected_report.exists()
        print("Saved native evidence matches exact inputs; changed policy, speakers and overrides are rejected", flush=True)
        bad = copy.deepcopy(document)
        bad["piper"]["models"][0]["model"]["sha256"] = "0" * 64
        text, _ = finish(start(bad, rejected_flags), False)
        assert not rejected_report.exists()
        assert "Validated flite" not in text and "Validated generation" not in text
        # Valid bytes and digest, invalid native ONNX: must fail and stop before Flite.
        model.write_bytes(b"invalid ONNX")
        bad["piper"]["models"][0]["model"] = asset(model)
        text, _ = finish(start(bad, rejected_flags), False)
        assert not rejected_report.exists()
        assert "Validated flite" not in text and "Validated generation" not in text
        model.write_bytes((source / "alpha.onnx").read_bytes())
        print("Hash and native model failures block the next load", flush=True)
        fake = root / "hanging helper"
        pidfile = root / "owned-pids"
        # Direct helper and its child inherit the worker's private group and pipes.
        fake.write_text("#!/bin/sh\nsleep 60 &\nprintf '%s %s %s\\n' \"$PPID\" \"$$\" \"$!\" > \"$VALIDATION_PID_FILE\"\nwait\n")
        fake.chmod(0o700)
        environment["VALIDATION_PID_FILE"] = str(pidfile)
        flags = ["--piper-helper", str(fake), "--flite-helper", str(args.flite_helper.resolve())]
        def await_pids(timeout=10):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if pidfile.exists():
                    fields = pidfile.read_text().split()
                    if len(fields) == 3:
                        return list(map(int, fields))
                time.sleep(.01)
            raise AssertionError("owned helper never started")
        def gone(pids, identities):
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                for pid in pids:
                    try:
                        os.waitpid(pid, os.WNOHANG)
                    except ChildProcessError:
                        pass
                if all(identity(pid) != identities[pid] for pid in pids):
                    return
                time.sleep(.01)
            raise AssertionError(f"owned processes survived: {pids}")
        for action in ["deadline", "cancel", "parent-death", "unconfirmed-pipe"]:
            if action == "unconfirmed-pipe":
                # Deliberately escape the group while retaining stderr. Group
                # ownership is not a sandbox; the retained pipe must prevent
                # a false cleanup acknowledgement or admission of Flite.
                fake.write_text(f"#!{sys.executable}\n" +
                    "import os, time\nchild = os.fork()\n" +
                    "if child == 0:\n os.setsid()\n time.sleep(60)\n" +
                    "else:\n" +
                    " with open(os.environ['VALIDATION_PID_FILE'], 'w') as f: f.write(f'{os.getppid()} {os.getpid()} {child}\\n')\n" +
                    " time.sleep(60)\n")
            pidfile.unlink(missing_ok=True)
            run = start(document, flags, timeout=1 if action in ["deadline", "unconfirmed-pipe"] else 60)
            pids = []
            identities = {}
            try:
                pids = await_pids()
                identities = {pid: identity(pid) for pid in pids}
                assert all(value is not None for value in identities.values()), "fixture exited before observation"
                if sys.platform == "linux":
                    limits = Path(f"/proc/{pids[1]}/limits").read_text()
                    address_limit = next(line for line in limits.splitlines() if line.startswith("Max address space"))
                    assert address_limit.split()[3:5] == [str(4096 * 1024 * 1024)] * 2, address_limit
                if action == "cancel":
                    run[0].stdin.close()
                elif action == "parent-death":
                    run[0].kill()
                text, diagnostics = finish(run, False)
                assert "Validated" not in text
                if action == "unconfirmed-pipe":
                    assert "pipe cleanup unconfirmed" in diagnostics, diagnostics
                else:
                    gone(pids, identities)
                    pids = []  # Never signal a numeric PID after confirming its reaping.
            finally:
                if run[0].poll() is None:
                    run[0].kill()
                    run[0].wait(timeout=10)
                run[0].stdin.close()
                run[1].close()
                run[2].close()
                for pid in pids:
                    # Exact process identity, including native start time; never
                    # signal a PID that has been reused after a failed check.
                    try:
                        recorded = identities.get(pid)
                        if recorded is not None and identity(pid) == recorded:
                            os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError: pass
                    except FileNotFoundError: pass
                gone(pids, identities)
            if action == "unconfirmed-pipe":
                print("unconfirmed-pipe: next load blocked; test owner retired escaped process", flush=True)
            else:
                print(f"{action}: worker, helper and descendant reaped", flush=True)
        # A deliberately synthetic companion exercises report cancellation after
        # checksum observation, while the owned native load is still pending.
        # Its editable provenance is identity metadata, not publisher trust.
        staged = root / "fault-companion"
        staged.mkdir()
        staged_helper = staged / "omnivox-flite-helper"
        staged_helper.write_text("#!/bin/sh\nsleep 60 &\nprintf '%s %s %s\\n' \"$PPID\" \"$$\" \"$!\" > \"$VALIDATION_PID_FILE\"\nwait\n")
        staged_helper.chmod(0o700)
        provenance = args.flite_helper.resolve().parent / "SOURCE-PROVENANCE.json"
        (staged / provenance.name).write_bytes(provenance.read_bytes())
        (staged / "SHA256SUMS").write_text("".join(
            f"{asset(item)['sha256']}  {item.name}\n" for item in sorted(staged.iterdir())))
        stalled = copy.deepcopy(document)
        stalled["piper"] = None
        stalled["flite"]["files"] = []
        for action in ["cancel", "parent-death"]:
            pidfile.unlink(missing_ok=True)
            pending_report = root / f"{action}-report.json"
            run = start(stalled, ["--flite-helper", str(staged_helper), "--validation-report", str(pending_report)])
            pids = []
            identities = {}
            try:
                pids = await_pids(timeout=90)
                identities = {pid: identity(pid) for pid in pids}
                if action == "cancel":
                    run[0].stdin.close()
                else:
                    run[0].kill()
                finish(run, False)
                gone(pids, identities)
                pids = []
                assert not pending_report.exists(), "interrupted validation published success"
            finally:
                if run[0].poll() is None:
                    run[0].kill(); run[0].wait(timeout=10)
                run[0].stdin.close(); run[1].close(); run[2].close()
                for pid in pids:
                    if identities.get(pid) is not None and identity(pid) == identities[pid]:
                        try: os.kill(pid, signal.SIGKILL)
                        except ProcessLookupError: pass
                gone(pids, identities)
        print("Cancellation and supervisor death cannot publish pending evidence", flush=True)
        text, _ = finish(start(document), True)
        assert "cleanup confirmed" in text
        print("Fresh validation succeeds after failure cleanup", flush=True)


if __name__ == "__main__":
    main()
