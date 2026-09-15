use std::io::{self, Read};

use serde_json::{json, Value};

use super::*;

fn examples() -> Value {
    serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/voice-library-v1.json"
    ))
    .unwrap()
}

fn runtime() -> Value {
    examples()["runtime_after_successful_validation"].clone()
}

fn index() -> Value {
    examples()["index_before_validation"].clone()
}

fn parse_runtime(value: &Value) -> Result<RuntimeLibrary, LibraryError> {
    RuntimeLibrary::parse(&serde_json::to_vec(value).unwrap(), HostPlatform::Windows)
}

fn parse_index(value: &Value) -> Result<LibraryIndex, LibraryError> {
    LibraryIndex::parse(&serde_json::to_vec(value).unwrap(), HostPlatform::Windows)
}

#[test]
fn independent_examples_decode_without_opening_fictitious_assets() {
    let runtime = parse_runtime(&runtime()).unwrap();
    let index = parse_index(&index()).unwrap();
    let pointer =
        ActivePointer::parse(&serde_json::to_vec(&examples()["active_pointer"]).unwrap()).unwrap();
    assert_eq!(runtime.document().generation_id, pointer.generation_id);
    assert_eq!(runtime.document().profile_id, index.document().profile_id);
    assert_eq!(runtime.document().target_id, index.document().target_id);
    assert_eq!(index.document().voices.len(), 2);
    assert!(index.document().packages[0].validation.is_none());
    assert!(runtime.permits(&PhysicalVoiceId::new(
        "piper",
        "piper:v1/c/example-multispeaker/0"
    )));
    assert!(!runtime.permits(&PhysicalVoiceId::new(
        "piper",
        "piper:v1/c/example-multispeaker/1"
    )));
    assert!(!runtime.permits(&PhysicalVoiceId::new("piper", "piper:unlisted")));
    assert!(runtime.permits(&PhysicalVoiceId::new("flite", "cmu_us_slt")));
}

#[test]
fn original_generation_bytes_survive_parsing_for_exact_digest() {
    let input = format!(" \n{}\n", serde_json::to_string_pretty(&runtime()).unwrap());
    let parsed = RuntimeLibrary::read(input.as_bytes(), HostPlatform::Windows).unwrap();
    assert_eq!(parsed.source_bytes(), input.as_bytes());
    let input = serde_json::to_vec_pretty(&index()).unwrap();
    assert_eq!(
        LibraryIndex::read(input.as_slice(), HostPlatform::Windows)
            .unwrap()
            .source_bytes(),
        input
    );
}

#[test]
fn null_provider_and_explicit_empty_load_set_have_different_meanings() {
    let mut document = runtime();
    document["piper"] = Value::Null;
    document["flite"] = Value::Null;
    let legacy = parse_runtime(&document).unwrap();
    assert!(legacy.permits(&PhysicalVoiceId::new("piper", "piper:legacy")));
    assert!(!legacy.permits(&PhysicalVoiceId::new(
        "piper",
        "piper:v1/c/example-multispeaker/1"
    )));
    document["piper"] = json!({"models": []});
    document["flite"] = json!({"builtin_slt": false, "files": []});
    let empty = parse_runtime(&document).unwrap();
    assert!(!empty.permits(&PhysicalVoiceId::new("piper", "piper:legacy")));
    assert!(!empty.permits(&PhysicalVoiceId::new("flite", "cmu_us_slt")));
    assert!(empty.permits(&PhysicalVoiceId::new("winrt", "os-voice")));
}

#[test]
fn disabled_unknown_native_voices_stay_disabled_without_an_index_row() {
    let mut document = runtime();
    document["disabled_physical_ids"]
        .as_array_mut()
        .unwrap()
        .push(json!({"engine_id": "winrt", "voice_id": "future-voice"}));
    assert!(!parse_runtime(&document)
        .unwrap()
        .permits(&PhysicalVoiceId::new("winrt", "future-voice")));
    let mut desired = index();
    desired["disabled_physical_ids"] = document["disabled_physical_ids"].clone();
    parse_index(&desired).unwrap();
}

#[test]
fn canonical_identity_rules_include_digit_prefixes_and_native_speaker_bound() {
    for key in ["a", "0-model", "en_us-example_2"] {
        let identity = ModelIdentity::Catalogue {
            catalogue_key: key.into(),
        };
        assert_eq!(
            identity.piper_voice_id(2).unwrap(),
            format!("piper:v1/c/{key}/2")
        );
    }
    for key in ["", "Example", "_model", "-model", "a/b", "a%2fb", "café"] {
        assert!(ModelIdentity::Catalogue {
            catalogue_key: key.into()
        }
        .piper_voice_id(0)
        .is_err());
    }
    let identity = ModelIdentity::Catalogue {
        catalogue_key: "example".into(),
    };
    assert!(identity.piper_voice_id(i32::MAX as u32).is_ok());
    assert!(identity.piper_voice_id(i32::MAX as u32 + 1).is_err());
    for id in [
        "piper:v1/c/example/01",
        "piper:v1/c/example/+1",
        "piper:v1/c/Example/1",
    ] {
        assert!(identity.validate_binding(id, 1).is_err());
    }
}

#[test]
fn separate_imports_with_equal_filenames_have_distinct_stable_ids() {
    let mut document = runtime();
    document["piper"]["models"] = json!([]);
    let prototype = runtime()["piper"]["models"][0].clone();
    for uuid in [
        "77777777-7777-4777-8777-777777777777",
        "88888888-8888-4888-8888-888888888888",
    ] {
        let mut model = prototype.clone();
        model["identity"] = json!({"import_id": uuid});
        model["model"]["path"] = json!(format!("C:\\imports\\{uuid}\\voice.onnx"));
        model["config"]["path"] = json!(format!("C:\\imports\\{uuid}\\voice.onnx.json"));
        model["voices"][0]["physical_id"] = json!(format!("piper:v1/i/{uuid}/0"));
        // The filename is not used to derive either physical identity.
        document["piper"]["models"]
            .as_array_mut()
            .unwrap()
            .push(model);
    }
    let before = parse_runtime(&document).unwrap();
    document["piper"]["models"][0]["voices"][0]["display_name"] = json!("Renamed speaker");
    document["piper"]["models"][0]["model"]["sha256"] = json!("2".repeat(64));
    let after = parse_runtime(&document).unwrap();
    let models = &before.document().piper.as_ref().unwrap().models;
    assert_ne!(
        models[0].voices[0].physical_id,
        models[1].voices[0].physical_id
    );
    assert_eq!(
        models[0].voices[0].physical_id,
        after.document().piper.as_ref().unwrap().models[0].voices[0].physical_id
    );
}

#[test]
fn adoption_preserves_legacy_id_only_for_explicit_imported_default() {
    let mut desired = index();
    desired["packages"][0]["identity"] =
        json!({"import_id": "77777777-7777-4777-8777-777777777777"});
    desired["packages"][0]["ownership"] = json!("imported");
    desired["packages"][0]["catalogue"] = Value::Null;
    desired["voices"].as_array_mut().unwrap().truncate(1);
    desired["voices"][0]["physical_id"] = json!("piper:old-café");
    assert!(parse_index(&desired).is_err());
    desired["voices"][0]["legacy_physical_id"] = json!("piper:old-café");
    let parsed = parse_index(&desired).unwrap();
    assert_eq!(parsed.document().voices[0].physical_id, "piper:old-café");
    desired["voices"][0]["speaker_index"] = json!(1);
    assert!(parse_index(&desired).is_err());
    desired["voices"][0]["speaker_index"] = json!(0);
    desired["packages"][0]["identity"] = json!({"catalogue_key": "example"});
    assert!(parse_index(&desired).is_err());
}

#[test]
fn legacy_and_new_ids_cannot_alias_one_imported_speaker() {
    let mut document = runtime();
    let model = &mut document["piper"]["models"][0];
    model["identity"] = json!({"import_id": "77777777-7777-4777-8777-777777777777"});
    model["voices"][0]["physical_id"] = json!("piper:legacy");
    let mut alias = model["voices"][0].clone();
    alias["physical_id"] = json!("piper:v1/i/77777777-7777-4777-8777-777777777777/0");
    model["voices"].as_array_mut().unwrap().push(alias);
    assert!(parse_runtime(&document).is_err());
}

#[test]
fn duplicate_keys_unknown_fields_and_missing_nullable_members_are_rejected() {
    let compact = serde_json::to_string(&runtime()).unwrap();
    for bytes in [
        compact.replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
            1,
        ),
        compact.replacen(
            "\"speaker_index\":0",
            "\"speaker_index\":0,\"speaker_index\":0",
            1,
        ),
        compact.replacen(
            "\"catalogue_key\":\"example-multispeaker\"",
            "\"catalogue_key\":\"example-multispeaker\",\"catalogue_key\":\"example-multispeaker\"",
            1,
        ),
    ] {
        assert_ne!(bytes, compact);
        assert!(RuntimeLibrary::parse(bytes.as_bytes(), HostPlatform::Windows).is_err());
    }
    for pointer in [
        "",
        "/piper",
        "/piper/models/0",
        "/piper/models/0/model",
        "/piper/models/0/voices/0",
        "/disabled_physical_ids/0",
    ] {
        let mut document = runtime();
        document
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), json!(true));
        assert!(parse_runtime(&document).is_err(), "{pointer}");
    }
    for field in ["piper", "flite"] {
        let mut document = runtime();
        document.as_object_mut().unwrap().remove(field);
        assert!(parse_runtime(&document).is_err());
    }
    let mut document = runtime();
    document["piper"]["models"][0]["voices"][0]
        .as_object_mut()
        .unwrap()
        .remove("language");
    assert!(parse_runtime(&document).is_err());
    for field in ["validation", "catalogue"] {
        let mut document = index();
        document["packages"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(parse_index(&document).is_err());
    }
}

#[test]
fn malformed_versions_identities_and_asset_records_are_rejected() {
    for (pointer, value) in [
        ("/schema_version", json!(2)),
        ("/target_id", json!("11111111-1111-4111-8111-11111111111Z")),
        (
            "/piper/models/0/identity",
            json!({"catalogue_key": "example", "import_id": "77777777-7777-4777-8777-777777777777"}),
        ),
        ("/piper/models/0/voices/0/speaker_index", json!(-1)),
        (
            "/piper/models/0/voices/0/display_name",
            json!("hidden\nline"),
        ),
        ("/piper/models/0/voices/0/language", json!("x".repeat(65))),
        ("/piper/models/0/model/sha256", json!("A".repeat(64))),
        ("/piper/models/0/model/bytes", json!(0)),
        ("/piper/models/0/voices", json!([])),
    ] {
        let mut document = runtime();
        *document.pointer_mut(pointer).unwrap() = value;
        assert!(parse_runtime(&document).is_err(), "{pointer}");
    }
}

#[test]
fn disabled_list_must_be_unique_sorted_and_consistent_with_load_set() {
    let mut document = runtime();
    let disabled = document["disabled_physical_ids"][0].clone();
    document["disabled_physical_ids"]
        .as_array_mut()
        .unwrap()
        .push(disabled);
    assert!(parse_runtime(&document).is_err());
    document["disabled_physical_ids"] = json!([{ "engine_id": "flite", "voice_id": "cmu_us_slt" }]);
    assert!(parse_runtime(&document).is_err());
    document["disabled_physical_ids"] =
        json!([{ "engine_id": "piper", "voice_id": "piper:v1/c/example-multispeaker/0" }]);
    assert!(parse_runtime(&document).is_err());
    let mut desired = index();
    desired["voices"][1]["enabled"] = json!(true);
    assert!(parse_index(&desired).is_err());
    desired = index();
    desired["voices"][0]["enabled"] = json!(false);
    assert!(parse_index(&desired).is_err());
}

#[test]
fn target_native_paths_are_validated_without_host_filesystem_access() {
    for path in [
        "C:\\models\\voice.onnx",
        "C:/models/voice.onnx",
        "\\\\host\\share\\voice.onnx",
    ] {
        assert!(native_path(path, HostPlatform::Windows).is_ok());
        assert!(native_path(path, HostPlatform::Posix).is_err());
    }
    for path in [
        "relative.onnx",
        "C:voice.onnx",
        "\\voice.onnx",
        "~/voice.onnx",
        "\\\\host",
        "\\\\?\\C:\\voice.onnx",
        "C:\\bad\nname",
    ] {
        assert!(
            native_path(path, HostPlatform::Windows).is_err(),
            "{path:?}"
        );
    }
    assert!(native_path("/home/example/voice.onnx", HostPlatform::Posix).is_ok());
    assert!(native_path("/home/example/voice.onnx", HostPlatform::Windows).is_err());
}

#[test]
fn flite_ids_come_from_native_names_and_cannot_collide() {
    let mut document = runtime();
    let voice = json!({"physical_id": "flitevox:cmu_us_slt", "file": document["piper"]["models"][0]["model"].clone(), "display_name": "External SLT", "language": null});
    document["flite"]["files"] = json!([voice.clone()]);
    assert!(parse_runtime(&document)
        .unwrap()
        .permits(&PhysicalVoiceId::new("flite", "flitevox:cmu_us_slt")));
    document["flite"]["files"] = json!([voice.clone(), voice]);
    assert!(parse_runtime(&document).is_err());
    document["flite"]["files"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    document["flite"]["files"][0]["physical_id"] = json!("cmu_us_slt");
    assert!(parse_runtime(&document).is_err());
}

#[test]
fn package_references_roles_provenance_and_provider_must_resolve() {
    for (pointer, value) in [
        ("/packages/0/provider", json!("unknown")),
        ("/packages/0/ownership", json!("borrowed")),
        ("/packages/0/catalogue", Value::Null),
        ("/packages/0/files/1/role", json!("model")),
        ("/voices/0/package_id", Value::Null),
        (
            "/voices/0/revision_id",
            json!("99999999-9999-4999-8999-999999999999"),
        ),
        ("/voices/0/engine_id", json!("flite")),
        ("/voices/0/speaker_index", Value::Null),
    ] {
        let mut document = index();
        *document.pointer_mut(pointer).unwrap() = value;
        assert!(parse_index(&document).is_err(), "{pointer}");
    }
    let mut document = index();
    let package = document["packages"][0].clone();
    document["packages"].as_array_mut().unwrap().push(package);
    assert!(parse_index(&document).is_err());
}

#[test]
fn compatible_package_revisions_keep_identity_and_voice_references() {
    let mut document = index();
    let mut update = document["packages"][0].clone();
    update["revision_id"] = json!("99999999-9999-4999-8999-999999999999");
    document["packages"].as_array_mut().unwrap().push(update);
    document["voices"][0]["revision_id"] = json!("99999999-9999-4999-8999-999999999999");
    parse_index(&document).unwrap();
    document["packages"][1]["identity"] = json!({"catalogue_key": "different-person"});
    assert!(parse_index(&document).is_err());
}

#[test]
fn file_set_hash_input_has_fixed_field_and_role_order() {
    let parsed = parse_index(&index()).unwrap();
    let package = &parsed.document().packages[0];
    let bytes = package.file_set_bytes().unwrap();
    assert!(bytes.starts_with(b"[{\"role\":\"config\",\"path\":"));
    let string = std::str::from_utf8(&bytes).unwrap();
    assert!(string.contains("\",\"bytes\":100,\"sha256\":\""));
    let mut reversed = package.clone();
    reversed.files.reverse();
    assert_eq!(bytes, reversed.file_set_bytes().unwrap());
}

#[test]
fn validation_evidence_is_structural_not_proof_of_native_readiness() {
    let mut document = index();
    document["packages"][0]["validation"] = json!({
        "validator_version": "example-1", "target_id": document["target_id"].clone(),
        "validated_at": "2024-02-29T12:34:56.123Z", "file_set_sha256": "0".repeat(64)
    });
    parse_index(&document).unwrap();
    for value in [
        "2023-02-29T12:34:56Z",
        "2024-02-29T24:00:00Z",
        "2024-02-29T12:34:56+10:00",
        "2024-02-29T12:34:56.Z",
        "tomorrow",
    ] {
        document["packages"][0]["validation"]["validated_at"] = json!(value);
        assert!(parse_index(&document).is_err(), "{value}");
    }
}

struct Endless(usize);

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(b' ');
        self.0 += buf.len();
        Ok(buf.len())
    }
}

#[test]
fn oversized_readers_stop_after_one_overflow_byte() {
    let mut input = Endless(0);
    assert!(RuntimeLibrary::read(&mut input, HostPlatform::Windows).is_err());
    assert_eq!(input.0, MAX_RUNTIME_BYTES + 1);
    let mut input = Endless(0);
    assert!(LibraryIndex::read(&mut input, HostPlatform::Windows).is_err());
    assert_eq!(input.0, MAX_INDEX_BYTES + 1);
}

#[test]
fn runtime_limits_reject_excess_without_truncating_valid_boundary() {
    let mut document = runtime();
    document["disabled_physical_ids"] = json!([]);
    document["flite"] = Value::Null;
    let mut voices = Vec::new();
    for speaker in 0..=MAX_PROJECTED_VOICES {
        voices.push(json!({"physical_id": format!("piper:v1/c/example-multispeaker/{speaker}"), "speaker_index": speaker, "display_name": "Speaker", "language": null}));
    }
    document["piper"]["models"][0]["voices"] = json!(voices);
    assert!(matches!(
        parse_runtime(&document),
        Err(LibraryError::Invalid("too many projected voices"))
    ));
    document["piper"]["models"][0]["voices"]
        .as_array_mut()
        .unwrap()
        .pop();
    parse_runtime(&document).unwrap();
    document["flite"] = json!({"builtin_slt": true, "files": []});
    assert!(parse_runtime(&document).is_err());
}

#[test]
fn model_file_and_index_record_count_limits_are_enforced() {
    let mut document = runtime();
    let model = document["piper"]["models"][0].clone();
    document["piper"]["models"] = json!(vec![model; MAX_PIPER_MODELS + 1]);
    assert!(matches!(
        parse_runtime(&document),
        Err(LibraryError::Invalid("too many Piper models"))
    ));
    document = runtime();
    document["flite"]["files"] = json!(vec![
        json!({"physical_id":"flitevox:a","file":document["piper"]["models"][0]["model"].clone(),"display_name":"a","language":null});
        MAX_FLITE_FILES + 1
    ]);
    assert!(matches!(
        parse_runtime(&document),
        Err(LibraryError::Invalid("too many Flite files"))
    ));
    let mut desired = index();
    desired["packages"] = json!(vec![
        desired["packages"][0].clone();
        MAX_PACKAGE_REVISIONS + 1
    ]);
    assert!(matches!(
        parse_index(&desired),
        Err(LibraryError::Invalid("too many package revisions"))
    ));
    desired = index();
    desired["voices"] = json!(vec![desired["voices"][0].clone(); MAX_INDEX_VOICES + 1]);
    assert!(matches!(
        parse_index(&desired),
        Err(LibraryError::Invalid("too many indexed voices"))
    ));
}

#[test]
fn validation_units_load_one_model_or_voice_without_mutating_the_generation() {
    let mut value = runtime();
    value["disabled_physical_ids"] = json!([]);
    value["flite"]["files"] = json!([{
        "physical_id": "flitevox:example", "display_name": "Example", "language": null,
        "file": {"path": "C:\\Voices\\example.flitevox", "bytes": 10, "sha256": "0".repeat(64)}
    }]);
    let mut speaker = value["piper"]["models"][0]["voices"][0].clone();
    speaker["physical_id"] = json!("piper:v1/c/example-multispeaker/1");
    speaker["speaker_index"] = json!(1);
    value["piper"]["models"][0]["voices"]
        .as_array_mut()
        .unwrap()
        .push(speaker);
    let library = parse_runtime(&value).unwrap();
    let original = library.source_bytes().to_vec();
    let targets = library.validation_targets();
    assert_eq!(targets.len(), 3); // One Piper model, SLT, one external Flite voice.
    for target in targets {
        let unit = library.validation_unit(&target).unwrap();
        let reread = RuntimeLibrary::parse(unit.source_bytes(), HostPlatform::Windows).unwrap();
        assert_eq!(reread.validation_targets().len(), 1);
        assert_ne!(unit.sha256(), library.sha256());
        if target.engine_id == "piper" {
            assert_eq!(
                unit.document().piper.as_ref().unwrap().models[0]
                    .voices
                    .len(),
                2
            );
            assert!(!unit.document().flite.as_ref().unwrap().builtin_slt);
            assert!(unit.document().flite.as_ref().unwrap().files.is_empty());
        } else {
            assert!(unit.document().piper.as_ref().unwrap().models.is_empty());
            let flite = unit.document().flite.as_ref().unwrap();
            assert_eq!(flite.builtin_slt, target.voice_id == "cmu_us_slt");
            assert_eq!(flite.files.len(), usize::from(!flite.builtin_slt));
        }
    }
    assert_eq!(library.source_bytes(), original);
    assert!(library
        .validation_unit(&PhysicalVoiceId::new("piper", "piper:missing"))
        .is_err());
    assert!(library
        .validation_unit(&PhysicalVoiceId::new("espeak", "en"))
        .is_err());
}
