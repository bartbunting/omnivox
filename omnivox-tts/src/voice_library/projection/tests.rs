use super::*;

const GENERATION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

fn index() -> IndexDocument {
    let examples: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../docs/protocol-fixtures/voice-library-v1.json"
    ))
    .unwrap();
    let parsed = LibraryIndex::parse(
        &serde_json::to_vec(&examples["index_before_validation"]).unwrap(),
        HostPlatform::Windows,
    )
    .unwrap();
    let mut document = parsed.document().clone();
    for package in &mut document.packages {
        package.validation = Some(NativeValidation {
            validator_version: "fixture".into(),
            target_id: document.target_id.clone(),
            validated_at: "2026-09-16T00:00:00Z".into(),
            file_set_sha256: package.file_set_sha256().unwrap(),
        });
    }
    document
}
fn parse(document: &IndexDocument) -> LibraryIndex {
    LibraryIndex::parse(
        &serde_json::to_vec(document).unwrap(),
        HostPlatform::Windows,
    )
    .unwrap()
}

#[test]
fn enabled_speakers_share_one_model_and_unselected_providers_keep_legacy_mode() {
    let mut document = index();
    for voice in &mut document.voices {
        voice.enabled = true;
    }
    document.disabled_physical_ids.clear();
    document.voices.reverse();
    let library = parse(&document)
        .project(GENERATION, true, false, HostPlatform::Windows)
        .unwrap();
    let models = &library.document().piper.as_ref().unwrap().models;
    assert_eq!(models.len(), 1);
    assert_eq!(
        models[0]
            .voices
            .iter()
            .map(|voice| voice.speaker_index)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(library.document().flite.is_none());
    assert!(parse(&document)
        .project(GENERATION, false, false, HostPlatform::Windows)
        .unwrap()
        .document()
        .piper
        .is_none());
}

#[test]
fn disabled_assets_need_no_validation_or_loading_and_empty_is_not_legacy() {
    let mut document = index();
    for voice in &mut document.voices {
        voice.enabled = false;
        let id = PhysicalVoiceId::new(&voice.engine_id, &voice.physical_id);
        if !document.disabled_physical_ids.contains(&id) {
            document.disabled_physical_ids.push(id);
        }
    }
    document
        .disabled_physical_ids
        .sort_by(|a, b| (&a.engine_id, &a.voice_id).cmp(&(&b.engine_id, &b.voice_id)));
    document.packages[0].validation = None;
    let library = parse(&document)
        .project(GENERATION, true, true, HostPlatform::Windows)
        .unwrap();
    assert!(library.validation_targets().is_empty());
    assert!(library.document().piper.as_ref().unwrap().models.is_empty());
    assert!(!library.document().flite.as_ref().unwrap().builtin_slt);
    assert_eq!(
        library.document().disabled_physical_ids,
        document.disabled_physical_ids
    );
    library.verify_assets(ProviderOverrides::default()).unwrap();
}

#[test]
fn enabled_inputs_require_matching_validation_and_one_revision_per_model() {
    for mode in [
        "unvalidated",
        "wrong-target",
        "changed-files",
        "mixed-revisions",
    ] {
        let mut document = index();
        match mode {
            "unvalidated" => document.packages[0].validation = None,
            "wrong-target" => {
                document.packages[0].validation.as_mut().unwrap().target_id = GENERATION.into()
            }
            "changed-files" => document.packages[0].files[0].bytes += 1,
            "mixed-revisions" => {
                let mut other = document.packages[0].clone();
                other.revision_id = GENERATION.into();
                document.voices[1].revision_id = Some(GENERATION.into());
                document.voices[1].enabled = true;
                document.disabled_physical_ids.clear();
                document.packages.push(other);
            }
            _ => unreachable!(),
        }
        assert!(
            parse(&document)
                .project(GENERATION, true, false, HostPlatform::Windows)
                .is_err(),
            "{mode}"
        );
    }
}

#[test]
fn flite_builtin_selection_and_global_exclusions_survive_projection() {
    let mut document = index();
    document.voices.push(IndexedVoice {
        engine_id: "flite".into(),
        physical_id: "cmu_us_slt".into(),
        display_name: "SLT".into(),
        language: None,
        enabled: true,
        package_id: None,
        revision_id: None,
        speaker_index: None,
        legacy_physical_id: None,
    });
    let library = parse(&document)
        .project(GENERATION, false, true, HostPlatform::Windows)
        .unwrap();
    assert!(library.document().piper.is_none());
    assert!(library.document().flite.as_ref().unwrap().builtin_slt);
    assert_eq!(
        library.document().disabled_physical_ids,
        document.disabled_physical_ids
    );
}
