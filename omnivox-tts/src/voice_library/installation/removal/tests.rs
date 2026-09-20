use super::*;
use crate::voice_library::local::{Host, Startup};
use std::collections::BTreeMap;

fn write_catalogue_file(directory: &Path, file: &catalogue::DownloadFile) -> AssetFile {
    let relative: PathBuf = file.filename().unwrap().split('/').collect();
    let path = directory.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"abc").unwrap();
    // Like acquisition, create the file before metadata_path verifies that
    // its ordinary and verbatim Windows paths resolve to the same object.
    file.asset(directory).unwrap()
}

struct Fixture {
    host: Host,
    voice: PhysicalVoiceId,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self::with_catalogue(catalogue::tests::fixture())
    }
    fn rhvoice() -> Self {
        Self::with_catalogue(crate::voice_library::rhvoice::tests::catalogue_fixture())
    }
    fn with_catalogue(value: serde_json::Value) -> Self {
        let root =
            std::env::temp_dir().join(format!("omnivox-removal-{}", local::new_uuid().unwrap()));
        let host = Host::open(&root).unwrap();
        let catalogue = Catalogue::parse(value.to_string().as_bytes()).unwrap();
        let package = local::new_uuid().unwrap();
        let revision = local::new_uuid().unwrap();
        let directory = package_directory(
            &PathBuf::from(catalogue::metadata_path(&host.root).unwrap()),
            &package,
            &revision,
        )
        .unwrap();
        fs::create_dir_all(&directory).unwrap();
        // Acquisition publishes from a canonical root; removal reconstructs
        // its ordinary Windows path from the reviewed ownership record.
        let directory = directory.canonicalize().unwrap();
        let entry = &catalogue.document().entries[0];
        let mut files = Vec::new();
        for file in &entry.files {
            let asset = write_catalogue_file(&directory, file);
            if file.role == "voice" || file.role.starts_with("rhvoice/") {
                files.push(imports::file(
                    if entry.provider == Provider::Rhvoice {
                        FileRole::RhvoiceData
                    } else {
                        FileRole::Voice
                    },
                    &asset,
                ));
            }
        }
        fs::write(directory.join("catalogue.json"), catalogue.source_bytes()).unwrap();
        let mut profile = host.profile().unwrap();
        let mut doc = profile.index.document().clone();
        doc.revision_id = local::new_uuid().unwrap();
        doc.schema_version = catalogue.document().schema_version;
        doc.packages.push(PackageRevision {
            package_id: package.clone(),
            revision_id: revision.clone(),
            provider: entry.provider,
            ownership: Ownership::Managed,
            identity: entry.identity(),
            files,
            validation: None,
            catalogue: Some(CatalogueReference {
                revision: catalogue.document().revision.clone(),
                entry_id: entry.id.clone(),
            }),
        });
        let voice = PhysicalVoiceId::new(entry.engine_id(), &entry.voices[0].physical_id);
        doc.voices.push(imports::row(
            entry.engine_id(),
            &voice.voice_id,
            "Test",
            &None,
            None,
            &package,
            &revision,
        ));
        doc.disabled_physical_ids.push(voice.clone());
        profile.replace_index(doc, &profile.index_sha256()).unwrap();
        drop(profile);
        Self {
            host,
            voice,
            directory,
        }
    }
    fn review(&self) -> RemovalReview {
        let profile = self.host.profile().unwrap();
        profile
            .prepare_removal(&self.voice, &profile.index_sha256())
            .unwrap()
    }
    fn execute(&self, review: &RemovalReview) -> RemovalResult {
        self.host
            .profile()
            .unwrap()
            .execute_removal(&review.operation_id, &review.plan_sha256)
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.host.root);
    }
}

#[test]
fn reviewed_package_removal_preserves_exclusions_and_permits_reinstallation() {
    let fixture = Fixture::new();
    let review = fixture.review();
    assert!(review.blockers.is_empty(), "{:?}", review.blockers);
    assert!(fixture.directory.exists()); // Review cannot delete or detach.
    assert_eq!(
        fixture
            .host
            .profile()
            .unwrap()
            .index
            .document()
            .voices
            .len(),
        1
    );
    let result = fixture.execute(&review);
    assert_eq!(result.status, "complete");
    assert_eq!(result.removed_bytes, review.package_bytes);
    assert_eq!(result.remaining_bytes, 0);
    assert_eq!(result.unconfirmed_bytes, 0);
    assert!(!fixture.directory.exists());
    let profile = fixture.host.profile().unwrap();
    assert!(profile.index.document().packages.is_empty());
    assert!(profile.index.document().voices.is_empty());
    assert_eq!(
        profile.index.document().disabled_physical_ids,
        vec![fixture.voice.clone()]
    );
    drop(profile);
    // Acquisition's real duplicate admission now accepts the same catalogue key.
    let catalogue = Catalogue::parse(catalogue::tests::fixture().to_string().as_bytes()).unwrap();
    assert!(acquisition::Acquisition::prepare(&fixture.host, &catalogue, "flite-test").is_ok());
}

#[test]
fn enabled_shared_speakers_and_stale_confirmation_cannot_delete_files() {
    let fixture = Fixture::new();
    let review = fixture.review();
    let mut profile = fixture.host.profile().unwrap();
    profile
        .set_enabled(
            &fixture.voice,
            true,
            &local::new_uuid().unwrap(),
            &profile.index_sha256(),
        )
        .unwrap();
    drop(profile);
    assert_eq!(fixture.execute(&review).status, "blocked");
    assert!(!fixture.review().blockers.is_empty());
    assert!(fixture.directory.join("voice.flitevox").exists());
}

#[test]
fn piper_package_review_includes_every_speaker_and_keeps_shared_model_until_all_disabled() {
    let mut fixture = Fixture::new();
    let mut value = catalogue::tests::fixture();
    let entry = &mut value["entries"][0];
    entry["id"] = "piper-test".into();
    entry["provider"] = "piper".into();
    entry["files"] = serde_json::json!(["model", "config", "model_card"].iter().map(|role|
        serde_json::json!({"role":role,"url":"https://example.org/file","bytes":3,"sha256":digest(b"abc")})).collect::<Vec<_>>());
    entry["voices"] = serde_json::json!([
        {"physical_id":"piper:v1/c/piper-test/0","name":"Zero","speaker_index":0},
        {"physical_id":"piper:v1/c/piper-test/1","name":"One","speaker_index":1}]);
    let catalogue = Catalogue::parse(value.to_string().as_bytes()).unwrap();
    let entry = catalogue.entry("piper-test").unwrap();
    fs::remove_file(fixture.directory.join("voice.flitevox")).unwrap();
    fs::write(
        fixture.directory.join("catalogue.json"),
        catalogue.source_bytes(),
    )
    .unwrap();
    for file in &entry.files {
        write_catalogue_file(&fixture.directory, file);
    }
    let mut profile = fixture.host.profile().unwrap();
    let mut doc = profile.index.document().clone();
    doc.revision_id = local::new_uuid().unwrap();
    let package = &mut doc.packages[0];
    package.provider = Provider::Piper;
    package.identity = entry.identity();
    package.catalogue.as_mut().unwrap().entry_id = entry.id.clone();
    package.files = entry.files[..2]
        .iter()
        .enumerate()
        .map(|(i, file)| {
            imports::file(
                if i == 0 {
                    FileRole::Model
                } else {
                    FileRole::Config
                },
                &file.asset(&fixture.directory).unwrap(),
            )
        })
        .collect();
    doc.voices = entry
        .voices
        .iter()
        .map(|voice| {
            imports::row(
                "piper",
                &voice.physical_id,
                &voice.name,
                &None,
                voice.speaker_index,
                &package.package_id,
                &package.revision_id,
            )
        })
        .collect();
    doc.disabled_physical_ids = doc
        .voices
        .iter()
        .map(|voice| PhysicalVoiceId::new("piper", &voice.physical_id))
        .collect();
    profile.replace_index(doc, &profile.index_sha256()).unwrap();
    fixture.voice = PhysicalVoiceId::new("piper", &entry.voices[0].physical_id);
    let other = PhysicalVoiceId::new("piper", &entry.voices[1].physical_id);
    profile
        .set_enabled(
            &other,
            true,
            &local::new_uuid().unwrap(),
            &profile.index_sha256(),
        )
        .unwrap();
    drop(profile);
    let review = fixture.review();
    assert_eq!(review.voices.len(), 2);
    assert!(!review.blockers.is_empty());
    assert_eq!(fixture.execute(&review).status, "blocked");
    let mut profile = fixture.host.profile().unwrap();
    profile
        .set_enabled(
            &other,
            false,
            &local::new_uuid().unwrap(),
            &profile.index_sha256(),
        )
        .unwrap();
    drop(profile);
    assert_eq!(fixture.execute(&fixture.review()).status, "complete");
}

#[test]
fn builtin_mbrola_en1_and_its_runtime_are_preserved() {
    let fixture = Fixture::new();
    let mut profile = fixture.host.profile().unwrap();
    let mut doc = profile.index.document().clone();
    doc.schema_version = 2;
    doc.revision_id = local::new_uuid().unwrap();
    doc.voices.push(IndexedVoice {
        physical_id: MBROLA_EN1.into(),
        engine_id: "mbrola".into(),
        display_name: "Included en1".into(),
        language: Some("en-GB".into()),
        enabled: true,
        package_id: None,
        revision_id: None,
        speaker_index: None,
        legacy_physical_id: None,
    });
    profile.replace_index(doc, &profile.index_sha256()).unwrap();
    assert!(profile
        .prepare_removal(
            &PhysicalVoiceId::new("mbrola", MBROLA_EN1),
            &profile.index_sha256()
        )
        .is_err());
    drop(profile);
    let runtime = fixture.host.root.join("mbrola-runtime");
    fs::write(&runtime, b"separate runtime").unwrap();
    assert_eq!(fixture.execute(&fixture.review()).status, "complete");
    assert_eq!(fs::read(runtime).unwrap(), b"separate runtime");
    assert_eq!(
        fixture.host.profile().unwrap().index.document().voices[0].physical_id,
        MBROLA_EN1
    );
}

#[test]
fn active_and_unresolved_rollback_generations_retain_packages() {
    let fixture = Fixture::new();
    let mut profile = fixture.host.profile().unwrap();
    let mut doc = profile.index.document().clone();
    doc.revision_id = local::new_uuid().unwrap();
    doc.packages[0].validation = Some(NativeValidation {
        validator_version: "test".into(),
        target_id: fixture.host.target_id.clone(),
        validated_at: "2026-09-17T00:00:00Z".into(),
        file_set_sha256: doc.packages[0].file_set_sha256().unwrap(),
    });
    profile.replace_index(doc, &profile.index_sha256()).unwrap();
    profile
        .set_enabled(
            &fixture.voice,
            true,
            &local::new_uuid().unwrap(),
            &profile.index_sha256(),
        )
        .unwrap();
    let generation = local::new_uuid().unwrap();
    let candidate = profile
        .stage_activation(
            &generation,
            false,
            true,
            false,
            false,
            &profile.index_sha256(),
        )
        .unwrap();
    let cfg = candidate.configuration;
    let active = serde_json::json!({"schema_version":1,"target_id":cfg.target_id,"profile_id":cfg.profile_id,
        "generation_id":cfg.generation_id,"sha256":cfg.sha256});
    fs::write(profile.path.join("active.json"), active.to_string()).unwrap();
    profile
        .set_enabled(
            &fixture.voice,
            false,
            &local::new_uuid().unwrap(),
            &profile.index_sha256(),
        )
        .unwrap();
    drop(profile);
    assert!(fixture
        .review()
        .blockers
        .iter()
        .any(|reason| reason.contains("Active generation")));
    let profile = fixture.host.profile().unwrap();
    fs::remove_file(profile.path.join("active.json")).unwrap();
    fs::create_dir_all(
        profile
            .path
            .join("activations")
            .join(local::new_uuid().unwrap()),
    )
    .unwrap();
    drop(profile);
    assert_eq!(fixture.execute(&fixture.review()).status, "blocked");
    assert!(fixture.directory.exists());
}

#[test]
fn another_profiles_shared_files_and_busy_profile_block_cleanup() {
    let fixture = Fixture::new();
    let original = fixture.host.profile().unwrap().index.document().packages[0].clone();
    let id = local::new_uuid().unwrap();
    fs::create_dir(fixture.host.root.join("profiles").join(&id)).unwrap();
    drop(Admission::create(&fixture.host.root, &fixture.host.target_id, &id).unwrap());
    let mut other =
        Profile::initialize(&fixture.host.root, &id, &local::new_uuid().unwrap()).unwrap();
    assert!(!fixture.review().blockers.is_empty());
    let mut doc = other.index.document().clone();
    doc.revision_id = local::new_uuid().unwrap();
    let mut shared = original.clone();
    shared.ownership = Ownership::Imported;
    shared.identity = ModelIdentity::Import {
        import_id: local::new_uuid().unwrap(),
    };
    shared.package_id = local::new_uuid().unwrap();
    shared.catalogue = None;
    doc.packages.push(shared);
    other.replace_index(doc, &other.index_sha256()).unwrap();
    drop(other);
    assert!(fixture
        .review()
        .blockers
        .iter()
        .any(|reason| reason.contains("Shared files")));
    assert_eq!(fixture.execute(&fixture.review()).status, "blocked");
}

#[test]
fn interrupted_index_publication_can_resume_the_same_reviewed_plan() {
    let fixture = Fixture::new();
    let review = fixture.review();
    let profile = fixture.host.profile().unwrap();
    let removal = profile.read_removal(&review.operation_id).unwrap();
    save_new(
        &profile
            .path
            .join("index-revisions")
            .join(format!("{}.json", removal.plan.next_revision)),
        removal.next.source_bytes(),
    )
    .unwrap();
    assert_eq!(profile.pending_removals().unwrap().len(), 1);
    drop(profile);
    assert_eq!(fixture.execute(&review).status, "complete");
}

#[test]
fn imported_and_builtin_voices_are_never_removal_targets() {
    let fixture = Fixture::new();
    let mut profile = fixture.host.profile().unwrap();
    profile
        .include_flite_slt(&local::new_uuid().unwrap(), &profile.index_sha256())
        .unwrap();
    assert!(profile
        .prepare_removal(
            &PhysicalVoiceId::new("flite", "cmu_us_slt"),
            &profile.index_sha256()
        )
        .is_err());
    let mut doc = profile.index.document().clone();
    doc.revision_id = local::new_uuid().unwrap();
    doc.packages[0].ownership = Ownership::Imported;
    doc.packages[0].identity = ModelIdentity::Import {
        import_id: local::new_uuid().unwrap(),
    };
    doc.packages[0].catalogue = None;
    profile.replace_index(doc, &profile.index_sha256()).unwrap();
    assert!(profile
        .prepare_removal(&fixture.voice, &profile.index_sha256())
        .is_err());
    assert!(fixture.directory.exists());
}

#[test]
fn interrupted_unlink_resumes_without_claiming_unconfirmed_byte_savings() {
    let fixture = Fixture::new();
    let review = fixture.review();
    let mut profile = fixture.host.profile().unwrap();
    let removal = profile.read_removal(&review.operation_id).unwrap();
    profile
        .replace_index(removal.next.document().clone(), &profile.index_sha256())
        .unwrap();
    // Crash after an unlink but before its receipt. Missing is not proof of
    // reclaimed bytes, and remaining package files can still be removed.
    fs::remove_file(&removal.files[0].path).unwrap();
    assert_eq!(profile.pending_removals().unwrap().len(), 1);
    drop(profile);
    let result = fixture.execute(&review);
    assert_eq!(result.status, "complete");
    assert_eq!(result.unconfirmed_bytes, 3);
    assert_eq!(result.removed_bytes, review.package_bytes - 3);
    assert!(fixture
        .host
        .profile()
        .unwrap()
        .pending_removals()
        .unwrap()
        .is_empty());
}

#[test]
fn damaged_deletion_evidence_blocks_further_cleanup() {
    let fixture = Fixture::new();
    let review = fixture.review();
    let mut profile = fixture.host.profile().unwrap();
    let removal = profile.read_removal(&review.operation_id).unwrap();
    profile
        .replace_index(removal.next.document().clone(), &profile.index_sha256())
        .unwrap();
    fs::remove_file(&removal.files[0].path).unwrap();
    fs::write(removal.receipt(0), b"partial receipt").unwrap();
    assert!(profile
        .execute_removal(&review.operation_id, &review.plan_sha256)
        .is_err());
    assert!(fixture.directory.join("catalogue.json").exists());
}

#[test]
fn unexpected_or_changed_files_retain_the_complete_installed_package() {
    for extra in [true, false] {
        let fixture = Fixture::new();
        let review = fixture.review();
        if extra {
            fs::write(fixture.directory.join("user-notes.txt"), "retain").unwrap();
        } else {
            fs::write(fixture.directory.join("voice.flitevox"), b"bad").unwrap();
        }
        assert_eq!(fixture.execute(&review).status, "blocked");
        assert_eq!(
            fixture
                .host
                .profile()
                .unwrap()
                .index
                .document()
                .packages
                .len(),
            1
        );
        assert!(fixture.directory.exists());
    }
}

#[test]
fn live_and_legacy_snapshots_pin_files_until_confirmed_native_retirement() {
    let fixture = Fixture::new();
    let review = fixture.review();
    let startup = Startup {
        executable: AssetFile {
            path: "/unused".into(),
            bytes: 1,
            sha256: "0".repeat(64),
        },
        arguments: Vec::new(),
        working_directory: fixture.host.root.clone(),
        configuration: None,
        environment: BTreeMap::from([(
            "OMNIVOX_FLITE_VOICES".into(),
            fixture
                .directory
                .join("voice.flitevox")
                .to_string_lossy()
                .into(),
        )]),
    };
    let (path, hash) = startup
        .save(&fixture.host, &local::new_uuid().unwrap())
        .unwrap();
    assert_eq!(fixture.execute(&review).status, "blocked");
    retention::retired(&fixture.host, &path, &hash).unwrap();
    assert_eq!(fixture.execute(&review).status, "complete");
}

#[test]
fn prepared_snapshots_and_storage_gate_do_not_invent_live_owners() {
    let fixture = Fixture::new();
    let review = fixture.review();
    let startup = Startup {
        executable: AssetFile {
            path: "/unused".into(),
            bytes: 1,
            sha256: "0".repeat(64),
        },
        arguments: Vec::new(),
        working_directory: fixture.host.root.clone(),
        configuration: None,
        environment: BTreeMap::from([(
            "OMNIVOX_PIPER_MODEL".into(),
            fixture
                .directory
                .join("voice.flitevox")
                .to_string_lossy()
                .into(),
        )]),
    };
    startup
        .save_prepared(&fixture.host, &local::new_uuid().unwrap())
        .unwrap();
    let gate = retention::Gate::acquire(&fixture.host.root).unwrap();
    assert!(fixture
        .host
        .profile()
        .unwrap()
        .execute_removal(&review.operation_id, &review.plan_sha256)
        .is_err());
    assert!(startup
        .save(&fixture.host, &local::new_uuid().unwrap())
        .is_err());
    drop(gate);
    assert_eq!(fixture.execute(&review).status, "complete");
}

#[cfg(unix)]
#[test]
fn symlinks_and_shared_hardlinks_never_become_cleanup_authority() {
    use std::os::unix::fs::symlink;
    for hard in [false, true] {
        let fixture = Fixture::new();
        let review = fixture.review();
        let original = fixture.directory.join("voice.flitevox");
        let external = fixture.host.root.join("user-owned.flitevox");
        if hard {
            fs::hard_link(&original, &external).unwrap();
        } else {
            fs::rename(&original, &external).unwrap();
            symlink(&external, &original).unwrap();
        }
        assert_eq!(fixture.execute(&review).status, "blocked");
        assert_eq!(fs::read(external).unwrap(), b"abc");
    }
}

#[test]
fn rhvoice_removal_rejects_unowned_nested_files_and_removes_only_its_package() {
    let fixture = Fixture::rhvoice();
    let outside = fixture.host.root.join("external-rhvoice");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("SLT"), b"external voice").unwrap();
    let extra = fixture.directory.join("rhvoice/voice/personal-note");
    fs::write(&extra, b"preserve").unwrap();
    let profile = fixture.host.profile().unwrap();
    assert!(!profile
        .prepare_removal(&fixture.voice, &profile.index_sha256())
        .unwrap()
        .blockers
        .is_empty());
    drop(profile);
    assert!(extra.exists());
    fs::remove_file(extra).unwrap();
    let review = fixture.review();
    assert!(review.blockers.is_empty());
    let removed = fixture.execute(&review);
    assert_eq!(removed.status, "complete");
    assert_eq!(removed.remaining_bytes, 0);
    assert_eq!(removed.unconfirmed_bytes, 0);
    assert!(!fixture.directory.exists());
    assert_eq!(fs::read(outside.join("SLT")).unwrap(), b"external voice");
    assert_eq!(fixture.execute(&review).status, "complete");
}
