#!/usr/bin/env python3
"""Exercise configuration against make dev or a fully staged Windows payload.

Pass --windows from WSL with the native Windows executable. The native fixture
uses the existing .NET Framework compiler and no private speech runtime.
"""
import argparse
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

from verify_local_voice_owner import Peer, startup_environment
from verify_release import read_wav


ROOT = Path(__file__).resolve().parents[1]
SHIPPED = ["espeak", "winrt", "macos", "piper", "rhvoice", "flite", "rutts",
           "tgspeechbox", "eloquence", "dectalk", "mbrola"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server", type=Path)
    parser.add_argument("--windows", action="store_true", help="run a native Windows payload from WSL")
    args = parser.parse_args()
    if os.name != "posix" or (args.server.suffix.lower() == ".exe") != args.windows:
        parser.error("select a native Unix binary or use --windows for a Windows executable from WSL")
    server = args.server.resolve()

    def native(path):
        if args.windows:
            return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()
        return str(path)

    def local(path):
        if args.windows:
            return Path(subprocess.check_output(["wslpath", "-u", str(path)], text=True).strip())
        return Path(path)

    parent = None
    if args.windows:
        parent = local(subprocess.check_output(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
             "[IO.Path]::GetTempPath()"], text=True).strip())
    root = Path(tempfile.mkdtemp(prefix="omnivox frozen launch ", dir=parent))
    peers = []
    try:
        configuration = root / "configuration"
        manifests = configuration / "helpers.d"
        manifests.mkdir(parents=True)
        (configuration / "config.json").write_text(json.dumps(dict(
            schema=2, speech=dict(max_chunk_words=3, capital_pitch=dict(default=1.4, engines={"org.fixture":1.8}), defaults=dict(
                voice="saved", rate=0.7, pitch=1.1, voice_volume=0.6,
                punctuation="none", split_caps=False, character_scale=1.4)),
            engine_overrides={name: dict(enabled=False) for name in SHIPPED},
            routing=dict(preferred_engine_ids=["org.fixture"]))))
        helper = root / ("helper with spaces.exe" if args.windows else "helper with spaces")
        if args.windows:
            subprocess.run([
                "/mnt/c/Windows/Microsoft.NET/Framework64/v4.0.30319/csc.exe",
                "/nologo", "/target:exe", "/reference:System.Web.Extensions.dll",
                "/out:" + native(helper), native(ROOT / "test-fixtures/framework-helper.cs")],
                check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30, cwd=parent)
        else:
            helper.write_text("#!" + sys.executable + "\n" +
                              (ROOT / "test-fixtures/framework-helper.py").read_text())
            helper.chmod(0o700)
        descriptor = json.loads((ROOT / "docs/protocol-fixtures/control-inventory-response.json").read_text())["engines"][0]
        descriptor.update(id="org.fixture", display_name="Frozen fixture", default_voice_id="voice")
        descriptor["voices"][0]["id"] = dict(engine_id="org.fixture", voice_id="voice")
        saved_voice = copy.deepcopy(descriptor["voices"][0])
        saved_voice["id"]["voice_id"] = "saved"
        descriptor["voices"].append(saved_voice)
        descriptor["capabilities"]["cancellation"] = "synthesis_and_playback"
        descriptor_path = root / "descriptor.json"
        descriptor_path.write_text(json.dumps(descriptor))
        arguments = ["--descriptor", native(descriptor_path), "--record", native(root / "argv.jsonl"),
                     "--record-environment", native(root / "environment.jsonl"),
                     "--record-synthesis", native(root / "synthesis.jsonl"),
                     "--tag", "$(private literal) $HOME `literal` & *", "--empty", ""]
        (manifests / "fixture.json").write_text(json.dumps(dict(
            schema=1, engine_id="org.fixture", program=native(helper), arguments=arguments)))
        environment = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "USER", "TMPDIR")
                       if key in os.environ}
        environment.update(OMNIVOX_VOICE_ROOT=native(root / "voices"),
                           OMNIVOX_CONFIG_DIR=native(configuration), OMNIVOX_AUDIO_OUTPUT="null",
                           OMNIVOX_FIXTURE_PRIVATE="retained private value")
        if args.windows:
            # Values are already native paths; deliberately omit WSLENV /p.
            environment["WSLENV"] = ":".join([
                "OMNIVOX_VOICE_ROOT", "OMNIVOX_CONFIG_DIR", "OMNIVOX_AUDIO_OUTPUT",
                "OMNIVOX_FIXTURE_PRIVATE", "OMNIVOX_AUDIO_TARGET", "OMNIVOX_ENGINE",
                "OMNIVOX_OWNED_ENGINE_STARTUP", "OMNIVOX_OWNED_ENGINE_STARTUP_SHA256",
                "OMNIVOX_OWNED_STARTUP", "OMNIVOX_OWNED_STARTUP_SHA256"])

        def diagnostic(*options, success=True):
            result = subprocess.run([str(server), *options], env=environment, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
            assert (result.returncode == 0) == success, (result.returncode, result.stderr[-2000:])
            return result.stdout

        def syntheses():
            return [json.loads(line) for line in (root / "synthesis.jsonl").read_text().splitlines()]

        def assert_settings(requests, voice, rate, pitch):
            assert requests
            for request in requests:
                settings = request["settings"]
                assert settings["voice_id"] == voice, settings
                assert abs(settings["rate"] - rate) < 0.00001, settings
                assert abs(settings["pitch"] - pitch) < 0.00001, settings
                # Speech gain is applied once by Omnivox, never by the helper.
                assert settings["volume"] == 1, settings

        assert "voice" in diagnostic("--engine", "org.fixture", "--list-voices-alist")
        diagnostic("--engine", "org.missing", "--list-voices-alist", success=False)
        wav_path = root / "fixture.wav"
        diagnostic("--engine", "org.fixture", "--dump-wav", "voice", native(wav_path), "fixture")
        read_wav(wav_path, canonical=True)
        assert_settings(syntheses(), "voice", 0.7, 1.1)
        diagnostic("--engine", "org.fixture", "--dump-wav", "", native(wav_path), "saved defaults")
        assert_settings(syntheses()[-1:], "saved", 0.7, 1.1)
        diagnostic("--engine", "org.fixture", "--rate", "0.8", "--pitch", "1.3",
                   "--voice", "voice", "--dump-wav", "", native(wav_path), "command line")
        assert_settings(syntheses()[-1:], "voice", 0.8, 1.3)
        print("Exact voice listing, missing-engine rejection and canonical WAV synthesis passed", flush=True)

        def owner(settings):
            peer = Peer(server, "--voice-library-owner", settings)
            peers.append(peer)
            description = peer.request("describe")
            return peer, description

        service = Peer(server, "--voice-library-service", environment)
        peers.append(service)
        assert service.request("host")["engine_configuration_version"] == 1
        prepared = service.request("engine-snapshot")
        assert prepared["type"] == "prepared_startup"
        common = dict(environment, OMNIVOX_OWNED_ENGINE_STARTUP=prepared["startup"],
                      OMNIVOX_OWNED_ENGINE_STARTUP_SHA256=prepared["startup_sha256"])
        first, initial = owner(dict(common, OMNIVOX_AUDIO_TARGET="left"))
        assert not initial["retired"] and initial["startup_error"] is None, initial
        initial_inventory = first.request("inventory", control=True)
        assert initial_inventory["preferred_engine_id"] == "org.fixture"
        retained = json.loads(local(initial["startup"]).read_text())
        activation = retained["engines"]["activation_id"]
        assert activation == prepared["activation_id"] == initial["activation_id"]
        assert retained["engines"]["speech"] == dict(max_chunk_words=3)
        assert retained["engines"]["speech_defaults"]["voice"] == "saved"
        assert retained["engines"]["capital_pitch"] == dict(default=1.4, engines={"org.fixture":1.8})
        assert "engine_configuration_v1" in first.request("capabilities", control=True)["features"]
        first_ack = first.request("engine_configuration_status_v1", control=True)
        assert first_ack["activation_id"] == activation
        # Windows canonicalization may add the native verbatim path prefix.
        assert local(first_ack["configuration_root"]).resolve() == configuration.resolve(), first_ack
        registration = next(row for row in first_ack["registrations"] if row["engine_id"] == "org.fixture")
        source = r"helpers.d\fixture.json" if args.windows else "helpers.d/fixture.json"
        assert registration["origin"] == "external_helper" and registration["source"] == source, registration

        preview = json.loads((ROOT / "docs/protocol-fixtures/voice-choice-tuning.json").read_text())["messages"]["preview"]
        for field in ("type", "protocol_version", "request_id", "expected_base_rate"):
            preview.pop(field, None)
        shared = preview["voice"]["shared"]
        shared["acss"] = dict.fromkeys(shared["acss"])
        shared["effects"] = dict.fromkeys(shared["effects"])
        shared["rate_offset"] = None
        preview["voice"] = dict(language=None, shared=shared, choices=[dict(
            id="fixture", selector=dict(kind="exact", engine_id="org.fixture", voice_id="voice"),
            adjustments={})])
        preview["selection"] = dict(mode="choice", choice_id="fixture")
        preview["context"] = {}
        preview["text"] = "one two three four five six seven"

        def synthesis_texts():
            return [request["text"] for request in syntheses()]

        expected_chunks = ["one two three", "four five six", "seven"]
        before = len(synthesis_texts())
        result = first.request("preview_voice_v2", control=True, **preview)
        assert result.get("status") == "completed", result
        assert result["last_started"]["realized"] == dict(engine_id="org.fixture", voice_id="voice"), result
        assert synthesis_texts()[before:] == expected_chunks
        def speak(peer, identifier, text, voice="saved", rate=0.7, pitch=1.1, commands="", letter_pitch=None):
            before = len(syntheses())
            peer.process.stdin.write((commands + f"q {text}\nemacsvox_marker_dispatch {identifier}\n").encode())
            peer.process.stdin.flush()
            started = False
            deadline = time.monotonic() + 30
            while True:
                message = peer.messages.get(timeout=max(0, deadline - time.monotonic()))
                if message.startswith(b"__EMACSVOX_MARKER__ "):
                    event = json.loads(base64.b64decode(message.split()[1]))
                    assert event["dispatch_id"] == identifier, event
                    if event["type"] == "utterance_started":
                        assert event["actual_voice"] == dict(engine_id="org.fixture", voice_id=voice), event
                        started = True
                else:
                    assert message.strip() == f"__EMACSVOX_TRACKED__ {identifier} completed".encode(), message
                    assert started
                    break
            requests = syntheses()[before:]
            if letter_pitch is not None:
                assert requests[0]["text"] == "a", requests
                assert_settings(requests[:1], voice, rate * 1.4, letter_pitch)
                requests = requests[1:]
            assert_settings(requests, voice, rate, pitch)
            return [request["text"] for request in requests]

        assert speak(first, 9901, preview["text"]) == expected_chunks
        assert speak(first, 9902, preview["text"], voice="voice", rate=0.2, pitch=1.6,
                     commands="tts_set_voice voice\ntts_set_speech_rate 20\ntts_set_pitch_multiplier 1.6\n") == expected_chunks
        assert speak(first, 9903, preview["text"], commands="tts_reset\nl A\n", letter_pitch=1.8) == expected_chunks
        assert speak(first, 9904, "CamelCase!") == ["CamelCase!"]
        print("Saved speech defaults, capital pitch, client overrides, reset, text preparation and configured chunks passed", flush=True)

        # Mutate every discovery input before starting the other worker. The
        # retained executable/argv/environment must still reach the real helper.
        shutil.rmtree(manifests)
        (configuration / "config.json").write_text('{"schema":1,"invalid":true}')
        changed = dict(common, OMNIVOX_FIXTURE_PRIVATE="changed private value",
                       OMNIVOX_ENGINE="espeak", OMNIVOX_AUDIO_TARGET="right")
        second, recovered = owner(changed)
        assert not recovered["retired"] and recovered["startup_error"] is None, recovered
        recovered_inventory = second.request("inventory", control=True)
        assert recovered_inventory["preferred_engine_id"] == "org.fixture"
        copied = json.loads(local(recovered["startup"]).read_text())
        assert copied["engines"]["activation_id"] == activation
        second_ack = second.request("engine_configuration_status_v1", control=True)
        assert second_ack["activation_id"] == recovered["activation_id"] == activation
        assert copied["engines"] == retained["engines"]
        assert speak(second, 9905, preview["text"], commands="l A\n", letter_pitch=1.8) == expected_chunks
        assert speak(first, 9906, preview["text"], commands="tts_set_speech_rate 20\ntts_reset\nl A\n", letter_pitch=1.8) == expected_chunks
        before = len(synthesis_texts())
        assert second.request("preview_voice_v2", control=True, **preview)["status"] == "completed"
        assert synthesis_texts()[before:] == expected_chunks
        assert startup_environment(retained)["OMNIVOX_AUDIO_TARGET"] == "left"
        assert startup_environment(copied)["OMNIVOX_AUDIO_TARGET"] == "right"
        launches = [json.loads(line) for line in (root / "argv.jsonl").read_text().splitlines()]
        assert launches == [arguments] * 6  # Four exact diagnostics and two owned workers.
        environments = [json.loads(line) for line in (root / "environment.jsonl").read_text().splitlines()]
        assert {item["value"] for item in environments} == {"retained private value"}
        assert len({item["pid"] for item in environments}) == 6
        public_status = json.dumps([initial_inventory, recovered_inventory, first_ack, second_ack])
        # macOS legitimately reports paths under /private. Check the actual
        # private inputs, rather than a word that can occur in a public path.
        for private_input in ("OMNIVOX_FIXTURE_PRIVATE", "retained private value",
                              "changed private value", arguments[arguments.index("--tag") + 1]):
            assert private_input not in public_status

        # Retiring one lane leaves the other's independently owned helper alive.
        assert first.request("retire", worker=initial["worker"])["type"] == "retired"
        assert second.request("inventory", control=True)["preferred_engine_id"] == "org.fixture"
        restarted, restart = owner(dict(environment, OMNIVOX_OWNED_STARTUP=initial["startup"],
                                        OMNIVOX_OWNED_STARTUP_SHA256=initial["startup_sha256"]))
        assert not restart["retired"] and restart["startup_error"] is None, restart
        assert restart["activation_id"] == activation
        assert speak(restarted, 9907, preview["text"], commands="tts_reset\nl A\n", letter_pitch=1.8) == expected_chunks
        before = len(synthesis_texts())
        assert restarted.request("preview_voice_v2", control=True, **preview)["status"] == "completed"
        assert synthesis_texts()[before:] == expected_chunks
        assert restarted.request("retire", worker=restart["worker"])["type"] == "retired"
        fresh, rejected = owner(environment)
        assert rejected["retired"] and rejected["startup_error"], rejected
        assert fresh.request("retire", worker=rejected["worker"])["type"] == "retired"
        assert second.request("retire", worker=recovered["worker"])["type"] == "retired"

        # A child that never reads a frame must not strand the owner in write().
        stalled_program = root / ("worker that never opens startup.exe" if args.windows else "worker that never opens startup")
        if args.windows:
            shutil.copyfile(helper, stalled_program)
        else:
            stalled_program.write_text("#!" + sys.executable + "\nimport time\ntime.sleep(300)\n")
            stalled_program.chmod(0o700)
        program_bytes = stalled_program.read_bytes()
        stalled_record = copy.deepcopy(retained)
        stalled_record["executable"] = dict(path=native(stalled_program), bytes=len(program_bytes),
                                            sha256=hashlib.sha256(program_bytes).hexdigest())
        encoding = "Windows" if args.windows else "Unix"
        for index in range(40):
            stalled_record["engines"]["environment"].append([
                {encoding: list(f"OMNIVOX_FIXTURE_LARGE_{index}".encode())}, {encoding: [120] * 32000}])
        stalled_path = local(initial["startup"]).parent / (str(uuid.uuid4()) + ".json")
        stalled_bytes = json.dumps(stalled_record).encode()
        stalled_path.write_bytes(stalled_bytes)
        stalled_path.chmod(0o600)
        stalled, rejected = owner(dict(environment, OMNIVOX_OWNED_STARTUP=native(stalled_path),
                                       OMNIVOX_OWNED_STARTUP_SHA256=hashlib.sha256(stalled_bytes).hexdigest()))
        assert rejected["retired"] and "transmission did not complete" in rejected["startup_error"], rejected
        assert stalled.request("retire", worker=rejected["worker"])["type"] == "retired"
        (configuration / "config.json").write_text('{"schema":1}')
        fresh_prepared = service.request("engine-snapshot")
        assert fresh_prepared["type"] == "prepared_startup" and fresh_prepared["activation_id"] != activation
        print("Shared preparation, independent worker acknowledgements/audio targets, frozen file/environment inputs, fresh activation, blocked startup write and retirement passed")
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
