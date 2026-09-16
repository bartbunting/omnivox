use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const TARGET: &str = "11111111-1111-4111-8111-111111111111";
const PROFILE: &str = "22222222-2222-4222-8222-222222222222";
const FIRST: &str = "33333333-3333-4333-8333-333333333333";
const SECOND: &str = "44444444-4444-4444-8444-444444444444";
const THIRD: &str = "55555555-5555-4555-8555-555555555555";
const GENERATION: &str = "66666666-6666-4666-8666-666666666666";
const OLD: &str = "77777777-7777-4777-8777-777777777777";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omnivox-installation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("operations")).unwrap();
        fs::create_dir_all(path.join("profiles").join(PROFILE)).unwrap();
        drop(Admission::create(&path, TARGET, PROFILE).unwrap());
        drop(Profile::initialize(&path, PROFILE, FIRST).unwrap());
        Self(path)
    }
    fn open(&self) -> Profile {
        Profile::open(&self.0, PROFILE).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn builtin(profile: &mut Profile) {
    let mut document = profile.index.document().clone();
    document.revision_id = SECOND.into();
    document.voices.push(IndexedVoice {
        physical_id: "cmu_us_slt".into(),
        engine_id: "flite".into(),
        display_name: "SLT".into(),
        language: None,
        enabled: true,
        package_id: None,
        revision_id: None,
        speaker_index: None,
        legacy_physical_id: None,
    });
    profile
        .replace_index(document, &profile.index_sha256())
        .unwrap();
}

#[test]
fn explicit_builtin_inclusion_and_disablement_are_pending_until_apply() {
    let fixture = Fixture::new();
    let mut profile = fixture.open();
    profile
        .include_flite_slt(SECOND, &profile.index_sha256())
        .unwrap();
    assert!(profile.index.document().voices[0].enabled);
    assert!(profile.active_json().unwrap().is_none());
    assert!(profile
        .include_flite_slt(THIRD, &profile.index_sha256())
        .is_err());
    profile
        .set_enabled(
            &PhysicalVoiceId::new("flite", "cmu_us_slt"),
            false,
            THIRD,
            &profile.index_sha256(),
        )
        .unwrap();
    let candidate = profile
        .index
        .project(GENERATION, false, true, false, host())
        .unwrap();
    assert!(!candidate.document().flite.as_ref().unwrap().builtin_slt);
    assert!(profile.active_json().unwrap().is_none());
}
fn old_active(profile: &Profile) -> Vec<u8> {
    let library = profile
        .index
        .project(OLD, false, true, false, host())
        .unwrap();
    save_new(&profile.generation_path(OLD), library.source_bytes()).unwrap();
    let configuration = library.configuration();
    let pointer = ActivePointer {
        schema_version: 1,
        target_id: configuration.target_id,
        profile_id: configuration.profile_id,
        generation_id: configuration.generation_id,
        sha256: configuration.sha256,
    };
    let bytes = serde_json::to_vec(&pointer).unwrap();
    save_new(&profile.path.join("active.json"), &bytes).unwrap();
    bytes
}

fn ready_proofs(configuration: &VoiceLibraryConfiguration) -> String {
    serde_json::to_string(
        &["speaker", "notification"]
            .iter()
            .enumerate()
            .map(|(i, role)| {
                serde_json::json!({"role": role, "worker": if i == 0 { FIRST } else { SECOND },
            "ready": true, "negotiated": true, "inventory_generation": i, "request_id": 9,
            "status": {"protocol_version":1, "request_id":9, "type":"voice_library_status_v1",
                "configuration": configuration, "overridden_engines":[], "eligible_voices":[],
                "inventory_generation":i}})
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
fn apply_retains_lease_and_publishes_only_after_both_verified_lanes() {
    let fixture = Fixture::new();
    let profile = fixture.open();
    let previous = old_active(&profile);
    let candidate = profile
        .stage_activation(GENERATION, false, true, false, &profile.index_sha256())
        .unwrap();
    let mut apply = profile.begin_activation(THIRD, GENERATION, "{}").unwrap();
    assert!(Profile::open(&fixture.0, PROFILE).is_err());
    assert!(apply
        .commit(&ready_proofs(&candidate.configuration))
        .is_err());
    apply.activating().unwrap();
    assert!(apply.commit("[]").is_err());
    let mut wrong: serde_json::Value =
        serde_json::from_str(&ready_proofs(&candidate.configuration)).unwrap();
    wrong[1]["status"]["configuration"]["sha256"] = "0".repeat(64).into();
    assert!(apply.commit(&wrong.to_string()).is_err());
    assert_eq!(
        fs::read(fixture.0.join("profiles").join(PROFILE).join("active.json")).unwrap(),
        previous
    );
    apply
        .commit(&ready_proofs(&candidate.configuration))
        .unwrap();
    assert!(apply.rolling_back().is_err());
    apply.finish("succeeded").unwrap();
    drop(apply);
    let active =
        ActivePointer::parse(fixture.open().active_json().unwrap().unwrap().as_bytes()).unwrap();
    assert_eq!(active.generation_id, GENERATION);
}

#[test]
fn interrupted_apply_is_retained_and_blocks_another_activation() {
    let fixture = Fixture::new();
    let profile = fixture.open();
    profile
        .stage_activation(GENERATION, true, false, false, &profile.index_sha256())
        .unwrap();
    let mut apply = profile.begin_activation(THIRD, GENERATION, "{}").unwrap();
    apply.activating().unwrap();
    drop(apply);
    assert!(fixture
        .open()
        .begin_activation(SECOND, GENERATION, "{}")
        .is_err());
    assert!(fixture.open().active_json().unwrap().is_none());
}

#[test]
fn rolled_back_apply_preserves_pointer_and_allows_a_new_reviewed_attempt() {
    let fixture = Fixture::new();
    let profile = fixture.open();
    let previous = old_active(&profile);
    profile
        .stage_activation(GENERATION, true, false, false, &profile.index_sha256())
        .unwrap();
    let mut apply = profile.begin_activation(THIRD, GENERATION, "{}").unwrap();
    apply.activating().unwrap();
    apply.rolling_back().unwrap();
    apply.finish("rolled-back").unwrap();
    drop(apply);
    let profile = fixture.open();
    assert_eq!(profile.active_json().unwrap().unwrap().as_bytes(), previous);
    let mut apply = profile.begin_activation(SECOND, GENERATION, "{}").unwrap();
    apply.finish("cancelled").unwrap();
}

#[test]
fn desired_edits_are_atomic_retained_and_do_not_change_active_configuration() {
    let fixture = Fixture::new();
    let mut profile = fixture.open();
    assert!(Profile::open(&fixture.0, PROFILE).is_err());
    let initial = profile.index.source_bytes().to_vec();
    builtin(&mut profile);
    let enabled = profile.index.source_bytes().to_vec();
    let active = old_active(&profile);
    profile
        .set_enabled(
            &PhysicalVoiceId::new("flite", "cmu_us_slt"),
            false,
            THIRD,
            &profile.index_sha256(),
        )
        .unwrap();
    assert!(!profile.index.document().voices[0].enabled);
    assert_eq!(
        profile.index.document().disabled_physical_ids,
        [PhysicalVoiceId::new("flite", "cmu_us_slt")]
    );
    assert_eq!(fs::read(profile.path.join("active.json")).unwrap(), active);
    assert_eq!(
        fs::read(
            profile
                .path
                .join("index-revisions")
                .join(format!("{FIRST}.json"))
        )
        .unwrap(),
        initial
    );
    assert_eq!(
        fs::read(
            profile
                .path
                .join("index-revisions")
                .join(format!("{SECOND}.json"))
        )
        .unwrap(),
        enabled
    );
    drop(profile);
    assert!(!fixture.open().index.document().voices[0].enabled);
}

#[test]
fn stale_edits_and_reused_revision_ids_preserve_current_desired_state() {
    let fixture = Fixture::new();
    let mut profile = fixture.open();
    let stale = profile.index_sha256();
    builtin(&mut profile);
    let current = profile.index.source_bytes().to_vec();
    let voice = PhysicalVoiceId::new("flite", "cmu_us_slt");
    assert!(profile.set_enabled(&voice, false, THIRD, &stale).is_err());
    assert!(profile
        .set_enabled(&voice, false, FIRST, &profile.index_sha256())
        .is_err());
    assert!(profile
        .set_enabled(
            &PhysicalVoiceId::new("flite", "unknown"),
            false,
            THIRD,
            &profile.index_sha256()
        )
        .is_err());
    assert_eq!(fs::read(profile.path.join("index.json")).unwrap(), current);
    assert!(!profile
        .path
        .join(format!("index-next-{FIRST}.json"))
        .exists());
}

#[test]
fn candidate_preparation_retains_active_pointer_and_detects_stale_plans() {
    for previous in [false, true] {
        let fixture = Fixture::new();
        let mut profile = fixture.open();
        builtin(&mut profile);
        let active = previous.then(|| old_active(&profile));
        let candidate = profile
            .stage_activation(GENERATION, false, true, false, &profile.index_sha256())
            .unwrap();
        assert_eq!(
            candidate
                .previous_active_json
                .as_ref()
                .map(|s| s.as_bytes()),
            active.as_deref()
        );
        let path = profile.generation_path(GENERATION);
        let original = fs::read(&path).unwrap();
        assert_eq!(candidate.configuration.sha256, digest(&original));
        profile.activation_candidate(GENERATION).unwrap();
        assert!(profile
            .stage_activation(GENERATION, false, true, false, &profile.index_sha256())
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        profile
            .set_enabled(
                &PhysicalVoiceId::new("flite", "cmu_us_slt"),
                false,
                THIRD,
                &profile.index_sha256(),
            )
            .unwrap();
        assert!(profile.activation_candidate(GENERATION).is_err());
        assert_eq!(
            profile.active_json().unwrap().map(String::into_bytes),
            active
        );
    }
}

#[test]
fn changed_generation_or_active_pointer_invalidates_prepared_activation() {
    for changed in ["generation", "active", "candidate"] {
        let fixture = Fixture::new();
        let mut profile = fixture.open();
        builtin(&mut profile);
        profile
            .stage_activation(GENERATION, false, true, false, &profile.index_sha256())
            .unwrap();
        match changed {
            "generation" => {
                let path = profile.generation_path(GENERATION);
                let mut bytes = fs::read(&path).unwrap();
                bytes.push(b' ');
                fs::write(path, bytes).unwrap();
            }
            "active" => {
                old_active(&profile);
            }
            "candidate" => {
                let path = profile
                    .path
                    .join("candidates")
                    .join(format!("{GENERATION}.json"));
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                value["unexpected"] = true.into();
                fs::write(path, value.to_string()).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            profile.activation_candidate(GENERATION).is_err(),
            "{changed}"
        );
    }
}

#[test]
fn imported_files_are_rechecked_before_projecting_enabled_assets() {
    let fixture = Fixture::new();
    let mut profile = fixture.open();
    let path = fixture.0.join("voice.flitevox");
    fs::write(&path, b"example").unwrap();
    let mut document = profile.index.document().clone();
    document.revision_id = SECOND.into();
    let mut package = PackageRevision {
        package_id: OLD.into(),
        revision_id: GENERATION.into(),
        provider: Provider::Flite,
        ownership: Ownership::Imported,
        identity: ModelIdentity::Import {
            import_id: OLD.into(),
        },
        files: vec![IndexedFile {
            role: FileRole::Voice,
            path: path.to_str().unwrap().into(),
            bytes: 7,
            sha256: digest(b"example"),
        }],
        catalogue: None,
        validation: None,
    };
    // A metadata fixture, not a claim that these test bytes are a native voice.
    package.validation = Some(NativeValidation {
        validator_version: "fixture".into(),
        target_id: TARGET.into(),
        validated_at: "2026-09-16T00:00:00Z".into(),
        file_set_sha256: package.file_set_sha256().unwrap(),
    });
    document.packages.push(package);
    document.voices.push(IndexedVoice {
        physical_id: "flitevox:example".into(),
        engine_id: "flite".into(),
        display_name: "Example".into(),
        language: None,
        enabled: true,
        package_id: Some(OLD.into()),
        revision_id: Some(GENERATION.into()),
        speaker_index: None,
        legacy_physical_id: None,
    });
    profile
        .replace_index(document, &profile.index_sha256())
        .unwrap();
    fs::write(&path, b"changed").unwrap();
    assert!(profile
        .stage_activation(GENERATION, false, true, false, &profile.index_sha256())
        .is_err());
    assert!(!profile.generation_path(GENERATION).exists());
    profile
        .set_enabled(
            &PhysicalVoiceId::new("flite", "flitevox:example"),
            false,
            THIRD,
            &profile.index_sha256(),
        )
        .unwrap();
    profile
        .stage_activation(GENERATION, false, true, false, &profile.index_sha256())
        .unwrap();
    assert_eq!(fs::read(path).unwrap(), b"changed");
}

#[test]
fn initialization_and_external_index_edits_never_overwrite_retained_work() {
    let fixture = Fixture::new();
    assert!(Profile::initialize(&fixture.0, PROFILE, THIRD).is_err());
    let mut profile = fixture.open();
    builtin(&mut profile);
    let path = profile.path.join("index.json");
    let changed = [profile.index.source_bytes(), b" "].concat();
    fs::write(&path, &changed).unwrap();
    assert!(profile
        .set_enabled(
            &PhysicalVoiceId::new("flite", "cmu_us_slt"),
            false,
            THIRD,
            &profile.index_sha256()
        )
        .is_err());
    drop(profile);
    assert!(Profile::open(&fixture.0, PROFILE).is_err());
    assert_eq!(fs::read(path).unwrap(), changed);
}
