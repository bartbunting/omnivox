#!/usr/bin/env python3
"""Exercise frozen local-owner startup against a staged Unix binary (make dev)."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import uuid

from verify_local_voice_owner import Peer


ROOT = Path(__file__).resolve().parents[1]
SHIPPED = ["espeak", "winrt", "macos", "piper", "rhvoice", "flite", "rutts",
           "tgspeechbox", "eloquence", "dectalk", "mbrola"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    args = parser.parse_args()
    if os.name != "posix" or args.server.suffix.lower() == ".exe":
        parser.error("this fixture requires a native Unix worker and Python interpreter")
    server = args.server.resolve()
    root = Path(tempfile.mkdtemp(prefix="omnivox frozen launch "))
    peers = []
    try:
        configuration = root / "configuration"
        manifests = configuration / "helpers.d"
        manifests.mkdir(parents=True)
        (configuration / "config.json").write_text(json.dumps(dict(
            schema=1, engine_overrides={name: dict(enabled=False) for name in SHIPPED},
            routing=dict(preferred_engine_ids=["org.fixture"]))))
        helper = root / "helper with spaces"
        helper.write_text("#!" + sys.executable + "\n" +
                          (ROOT / "test-fixtures/framework-helper.py").read_text())
        helper.chmod(0o700)
        descriptor = json.loads((ROOT / "docs/protocol-fixtures/control-inventory-response.json").read_text())["engines"][0]
        descriptor.update(id="org.fixture", display_name="Frozen fixture", default_voice_id="voice")
        descriptor["voices"][0]["id"] = dict(engine_id="org.fixture", voice_id="voice")
        descriptor["capabilities"]["cancellation"] = "synthesis_and_playback"
        descriptor_path = root / "descriptor.json"
        descriptor_path.write_text(json.dumps(descriptor))
        arguments = ["--descriptor", str(descriptor_path), "--record", str(root / "argv.jsonl"),
                     "--record-environment", str(root / "environment.jsonl"), "--tag", "$(private literal)"]
        (manifests / "fixture.json").write_text(json.dumps(dict(
            schema=1, engine_id="org.fixture", program=str(helper), arguments=arguments)))
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith("OMNIVOX_") and key != "ESPEAK_NG_DATA"}
        environment.update(OMNIVOX_VOICE_ROOT=str(root / "voices"),
                           OMNIVOX_CONFIG_DIR=str(configuration), OMNIVOX_AUDIO_OUTPUT="null",
                           OMNIVOX_FIXTURE_PRIVATE="retained private value")

        def owner(settings):
            peer = Peer(server, "--voice-library-owner", settings)
            peers.append(peer)
            description = peer.request("describe")
            return peer, description

        first, initial = owner(environment)
        assert not initial["retired"] and initial["startup_error"] is None, initial
        initial_inventory = first.request("inventory", control=True)
        assert initial_inventory["preferred_engine_id"] == "org.fixture"
        retained = json.loads(Path(initial["startup"]).read_text())
        activation = retained["engines"]["activation_id"]

        # Mutate every discovery input before starting the other worker. The
        # retained executable/argv/environment must still reach the real helper.
        shutil.rmtree(manifests)
        (configuration / "config.json").write_text('{"schema":1,"invalid":true}')
        changed = dict(environment, OMNIVOX_FIXTURE_PRIVATE="changed private value",
                       OMNIVOX_ENGINE="espeak", OMNIVOX_OWNED_STARTUP=initial["startup"],
                       OMNIVOX_OWNED_STARTUP_SHA256=initial["startup_sha256"])
        second, recovered = owner(changed)
        assert not recovered["retired"] and recovered["startup_error"] is None, recovered
        recovered_inventory = second.request("inventory", control=True)
        assert recovered_inventory["preferred_engine_id"] == "org.fixture"
        copied = json.loads(Path(recovered["startup"]).read_text())
        assert copied["engines"]["activation_id"] == activation
        launches = [json.loads(line) for line in (root / "argv.jsonl").read_text().splitlines()]
        assert launches == [arguments, arguments]
        environments = [json.loads(line) for line in (root / "environment.jsonl").read_text().splitlines()]
        assert {item["value"] for item in environments} == {"retained private value"}
        assert len({item["pid"] for item in environments}) == 2
        assert "private" not in json.dumps([initial_inventory, recovered_inventory])

        # Retiring one lane leaves the other's independently owned helper alive.
        assert first.request("retire", worker=initial["worker"])["type"] == "retired"
        assert second.request("inventory", control=True)["preferred_engine_id"] == "org.fixture"
        fresh, rejected = owner(environment)
        assert rejected["retired"] and rejected["startup_error"], rejected
        assert fresh.request("retire", worker=rejected["worker"])["type"] == "retired"
        assert second.request("retire", worker=recovered["worker"])["type"] == "retired"

        # A child that never reads a frame must not strand the owner in write().
        stalled_program = root / "worker that never opens startup"
        stalled_program.write_text("#!" + sys.executable + "\nimport time\ntime.sleep(300)\n")
        stalled_program.chmod(0o700)
        program_bytes = stalled_program.read_bytes()
        stalled_record = copy.deepcopy(retained)
        stalled_record["executable"] = dict(path=str(stalled_program), bytes=len(program_bytes),
                                            sha256=hashlib.sha256(program_bytes).hexdigest())
        for index in range(40):
            stalled_record["engines"]["environment"].append([
                {"Unix": list(f"LARGE_{index}".encode())}, {"Unix": [120] * 32000}])
        stalled_path = Path(initial["startup"]).parent / (str(uuid.uuid4()) + ".json")
        stalled_bytes = json.dumps(stalled_record).encode()
        stalled_path.write_bytes(stalled_bytes)
        stalled_path.chmod(0o600)
        stalled, rejected = owner(dict(environment, OMNIVOX_OWNED_STARTUP=str(stalled_path),
                                       OMNIVOX_OWNED_STARTUP_SHA256=hashlib.sha256(stalled_bytes).hexdigest()))
        assert rejected["retired"] and "transmission did not complete" in rejected["startup_error"], rejected
        assert stalled.request("retire", worker=rejected["worker"])["type"] == "retired"
        print("Frozen owner startup, two independent helpers, file/environment mutation, fresh-activation rejection, blocked startup write and retirement passed")
    finally:
        errors = []
        for peer in reversed(peers):
            try:
                peer.close()
            except Exception as error:
                errors.append(error)
        if errors:
            raise RuntimeError(f"cleanup failed; retained {root}") from errors[0]
        shutil.rmtree(root)


if __name__ == "__main__":
    main()
