#!/usr/bin/env python3
"""Exercise nonactivating operation preparation and inspection with a staged CLI."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    args = parser.parse_args()
    server = args.server.resolve()
    platform = {"linux": "linux", "darwin": "macos", "win32": "windows"}[sys.platform]
    operation_id = "44444444-4444-4444-8444-444444444444"
    with tempfile.TemporaryDirectory(prefix="omnivox-operation-probe-") as name:
        root = Path(name).resolve()
        operations = root / "operations"
        operations.mkdir()
        # Neither preparation nor inspection may load native code, verify this
        # deliberately absent asset, or execute the deliberately absent helper.
        generation = {"schema_version": 1,
                      "target_id": "11111111-1111-4111-8111-111111111111",
                      "profile_id": "22222222-2222-4222-8222-222222222222",
                      "generation_id": "33333333-3333-4333-8333-333333333333",
                      "piper": None, "disabled_physical_ids": [],
                      "flite": {"builtin_slt": False, "files": [{
                          "physical_id": "flitevox:fixture", "display_name": "Fixture", "language": None,
                          "file": {"path": str(root / "absent.flitevox"), "bytes": 1, "sha256": "a" * 64}}]}}
        plan = {"schema_version": 1, "operation_kind": "native_validation", "operation_id": operation_id,
                "platform": platform, "generation_json": json.dumps(generation),
                "validator_path": str(server), "helpers": {"flite": str(root / "absent-helper")},
                "timeout_seconds": 60, "memory_bytes": 4096 * 1024 * 1024,
                "runtime_policy": "bundled-companions-v1"}
        plan_path = root / "request.json"
        plan_path.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")

        def run(*arguments, success=True):
            result = subprocess.run([str(server), *map(str, arguments)], capture_output=True,
                                    text=True, timeout=20, stdin=subprocess.DEVNULL)
            assert (result.returncode == 0) == success, (result.returncode, result.stdout, result.stderr)
            return result.stdout

        output = run("--prepare-voice-validation", plan_path, operations)
        assert "no native work started" in output
        directory = operations / operation_id
        assert (directory / "plan.json").read_bytes() == plan_path.read_bytes()
        journal = directory / "journal.frames"
        original = journal.read_bytes()
        payload, checksum, end = original.split(b"\n")
        assert end == b"" and checksum.decode() == hashlib.sha256(payload).hexdigest()
        assert json.loads(payload)["transition"]["state"] == "prepared"
        assert "Prepared" in run("--inspect-voice-operation", directory)
        for start in [b"", b"WRONG\n"]:
            result = subprocess.run([str(server), "--internal-voice-validation-supervisor", str(root),
                                     generation["profile_id"], operation_id], input=start,
                                    capture_output=True, timeout=20)
            assert result.returncode != 0
            assert b"supervisor startup" in result.stderr
            assert journal.read_bytes() == original
            assert not (root / "profiles").exists()
        run("--prepare-voice-validation", plan_path, operations, success=False)
        assert journal.read_bytes() == original
        with journal.open("ab") as stream:
            stream.write(b'{"incomplete"')
        damaged = journal.read_bytes()
        assert "reuse is blocked" in run("--inspect-voice-operation", directory)
        assert journal.read_bytes() == damaged
        run("--inspect-voice-operation", root / "absent-operation", success=False)
        assert not (root / "absent-operation").exists()
        assert not (root / "absent.flitevox").exists()
    print("Operation preparation and inspection preserve exact inputs and block damaged journals without native work")


if __name__ == "__main__":
    main()
