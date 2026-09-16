"""Exercise native-validated imports, desired edits and activation candidates."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import uuid


def verify_installation(server, root, source, helpers, environment, prepare, start, finish):
    directory = root / "installation"
    profile_id = source["profile_id"]
    (directory / "operations").mkdir(parents=True)
    profile_path = directory / "profiles" / profile_id
    profile_path.mkdir(parents=True)

    def command(name, *arguments, success=True):
        result = subprocess.run([str(server), name, str(directory), *map(str, arguments)],
                                env=environment, stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=30)
        assert (result.returncode == 0) == success, (name, result.stdout, result.stderr)
        return result.stdout

    def index():
        raw = command("--inspect-voice-library", profile_id).encode()
        assert raw == (profile_path / "index.json").read_bytes()
        return json.loads(raw), hashlib.sha256(raw).hexdigest()

    command("--prepare-voice-admission", source["target_id"], profile_id)
    command("--initialize-voice-library", profile_id, uuid.uuid4())
    command("--initialize-voice-library", profile_id, uuid.uuid4(), success=False)
    original_assets = {file["path"]: file for file in
                       [source["piper"]["models"][0]["model"], source["piper"]["models"][0]["config"]]}
    imported = copy.deepcopy(source)
    imported["generation_id"] = str(uuid.uuid4())
    imported["flite"] = None
    import_id = str(uuid.uuid4())
    imported["piper"]["models"][0]["identity"] = {"import_id": import_id}
    for voice in imported["piper"]["models"][0]["voices"]:
        voice["physical_id"] = f"piper:v1/i/{import_id}/{voice['speaker_index']}"
    operation = prepare(directory, imported, {"piper": helpers["piper"]})
    _, before = index()
    package, revision = str(uuid.uuid4()), str(uuid.uuid4())

    def install(success):
        command("--import-validated-voice", profile_id, operation.name,
                package, revision, uuid.uuid4(), before, success=success)

    install(False)  # A prepared request is not installation evidence.
    finish(start(directory, operation), True)
    for name in ["validation-evidence.json", "workers/0000-cleaned.json"]:
        path = operation / name
        original = path.read_bytes()
        path.write_bytes(original + b" ")
        try:
            install(False)
            assert index()[1] == before
        finally:
            path.write_bytes(original)
    model_path = Path(imported["piper"]["models"][0]["model"]["path"])
    original = model_path.read_bytes()
    model_path.write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
    try:
        install(False)
        assert index()[1] == before
    finally:
        model_path.write_bytes(original)
    install(True)
    installed, installed_hash = index()
    assert len(installed["voices"]) == 2 and all(not voice["enabled"] for voice in installed["voices"])
    stored = installed["packages"][0]
    assert stored["ownership"] == "imported" and stored["catalogue"] is None
    assert stored["validation"]["validator_version"].startswith("sha256:")
    assert {file["path"] for file in stored["files"]} == set(original_assets)
    install(False)  # Reusing a stale plan or package cannot overwrite the import.
    assert index()[1] == installed_hash

    speaker = installed["voices"][1]["physical_id"]
    command("--set-library-voice-enabled", profile_id, "piper", speaker,
            "true", uuid.uuid4(), installed_hash)
    enabled, enabled_hash = index()
    assert [voice["enabled"] for voice in enabled["voices"]] == [False, True]
    command("--set-library-voice-enabled", profile_id, "piper", speaker,
            "false", uuid.uuid4(), installed_hash, success=False)
    assert index()[1] == enabled_hash
    generation = str(uuid.uuid4())
    candidate = json.loads(command("--stage-voice-library-activation", profile_id,
                                   generation, "piper", enabled_hash))
    assert candidate["previous_active_json"] is None
    assert candidate["index_sha256"] == enabled_hash
    path = profile_path / "generations" / f"{generation}.json"
    projected = json.loads(path.read_bytes())
    assert hashlib.sha256(path.read_bytes()).hexdigest() == candidate["configuration"]["sha256"]
    assert projected["flite"] is None
    assert [voice["physical_id"] for voice in projected["piper"]["models"][0]["voices"]] == [speaker]
    command("--inspect-voice-library-activation", profile_id, generation)
    finish(start(directory, prepare(directory, projected, {"piper": helpers["piper"]})), True)
    command("--set-library-voice-enabled", profile_id, "piper", speaker,
            "false", uuid.uuid4(), enabled_hash)
    command("--inspect-voice-library-activation", profile_id, generation, success=False)
    empty_id = str(uuid.uuid4())
    command("--stage-voice-library-activation", profile_id, empty_id, "piper", index()[1])
    empty = json.loads((profile_path / "generations" / f"{empty_id}.json").read_bytes())
    assert empty["piper"]["models"] == []
    assert not (profile_path / "active.json").exists()

    if source["flite"]["files"]:
        imported_flite = copy.deepcopy(source)
        imported_flite["generation_id"] = str(uuid.uuid4())
        imported_flite["piper"] = None
        imported_flite["flite"]["builtin_slt"] = False
        operation = prepare(directory, imported_flite, {"flite": helpers["flite"]})
        finish(start(directory, operation), True)
        command("--import-validated-voice", profile_id, operation.name,
                uuid.uuid4(), uuid.uuid4(), uuid.uuid4(), index()[1])
        installed, before = index()
        voice = installed["voices"][-1]
        assert voice["physical_id"] == "flitevox:cmu_us_slt" and not voice["enabled"]
        command("--set-library-voice-enabled", profile_id, "flite", voice["physical_id"],
                "true", uuid.uuid4(), before)
        generation = str(uuid.uuid4())
        command("--stage-voice-library-activation", profile_id, generation, "flite", index()[1])
        projected = json.loads((profile_path / "generations" / f"{generation}.json").read_bytes())
        assert not projected["flite"]["builtin_slt"] and len(projected["flite"]["files"]) == 1
        finish(start(directory, prepare(directory, projected, {"flite": helpers["flite"]})), True)
    assert not (profile_path / "active.json").exists()
    for file in original_assets.values():
        assert hashlib.sha256(Path(file["path"]).read_bytes()).hexdigest() == file["sha256"]
    print("Validated Piper/Flite imports start disabled; desired enablement projects exact native candidates without changing active speech", flush=True)
