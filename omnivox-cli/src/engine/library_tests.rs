use super::*;
use omnivox_tts::contracts::{Availability, EngineHealth, VoiceDescriptor};
use omnivox_tts::{VoiceInfo, VoiceQuality};
use serde_json::json;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(provider: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "omnivox-fallback-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let asset = root.join("voice.data");
        std::fs::write(&asset, b"abc").unwrap();
        let config = root.join("voice.config");
        std::fs::write(&config, b"abc").unwrap();
        let file = json!({"path": asset, "bytes": 3,
            "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"});
        let mut config_file = file.clone();
        config_file["path"] = json!(config);
        let mut document = json!({
            "schema_version": 1,
            "target_id": "11111111-1111-4111-8111-111111111111",
            "profile_id": "22222222-2222-4222-8222-222222222222",
            "generation_id": "33333333-3333-4333-8333-333333333333",
            "disabled_physical_ids": [{"engine_id":"espeak", "voice_id":"excluded"}],
            "piper": null, "flite": null
        });
        document[provider] = match provider {
            "piper" => json!({"models": [{"identity": {"catalogue_key": "test"},
                "model": file, "config": config_file,
                "voices": [{"physical_id": "piper:v1/c/test/0", "speaker_index": 0,
                    "display_name": "Test", "language": "en"}]}]}),
            "flite" => json!({"builtin_slt": true, "files": [{"physical_id": "flitevox:test",
                "file": file, "display_name": "Test", "language": "en"}]}),
            _ => unreachable!(),
        };
        let path = root.join("generation.json");
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        Self { root, path }
    }

    fn library(&self) -> StartupLibrary {
        StartupLibrary::from_environment(Some(self.path.to_str().unwrap()), None)
            .unwrap()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn unavailable_reason(registry: &EngineRegistry, id: &str) -> String {
    let descriptor = registry
        .inventory()
        .into_iter()
        .find(|d| d.id == id)
        .unwrap();
    assert!(registry.engine(id).is_none());
    match descriptor.availability {
        Availability::Unavailable { reason } => reason,
        _ => panic!("failed provider must remain unavailable"),
    }
}

#[test]
fn absent_managed_helpers_are_reported_without_aborting_startup() {
    let fixture = Fixture::new("flite");
    let library = fixture.library();
    let mut registry = library.registry().unwrap();
    register_missing_managed_helpers(&mut registry, &[], Some(&library)).unwrap();
    assert!(unavailable_reason(&registry, "flite").contains("not found"));
    crate::voice_library::preflight_responses(&registry, "espeak").unwrap();
}

#[cfg(not(feature = "piper"))]
#[test]
fn build_without_piper_retains_the_library_and_other_engine_choices() {
    let fixture = Fixture::new("piper");
    let library = fixture.library();
    let configs = configured_helper_configs("piper", None, Some(&library));
    assert!(configs.iter().all(|config| config.engine_id != "piper"));
    let mut registry = library.registry().unwrap();
    register_missing_managed_helpers(&mut registry, &configs, Some(&library)).unwrap();
    assert!(unavailable_reason(&registry, "piper").contains("--features piper"));
    let status = registry.voice_library_status(0, &registry.inventory(), &[]);
    assert_eq!(
        status.configuration.unwrap(),
        library.library.configuration()
    );
    assert!(status.eligible_voices.is_empty());
    assert!(create_engine("piper", None, Some(fixture.path.to_str().unwrap())).is_err());
}

#[test]
fn managed_helper_failure_keeps_an_unavailable_inventory_entry() {
    let fixture = Fixture::new("flite");
    let library = fixture.library();
    let pending = start_helper_initializations_with(
        vec![HelperEngineConfig::new("flite", "unused-helper")],
        |_| -> HelperInitializationResult {
            Err(omnivox_tts::helper_engine::HelperEngineError::Transport(
                "runtime missing".into(),
            ))
        },
    );
    let mut registry = library.registry().unwrap();
    register_initialized_helpers(
        &mut registry,
        pending,
        Arc::new(AtomicU64::new(0)),
        Arc::new(IsolationBudget::new()),
        Some(&library),
    )
    .unwrap();
    assert!(unavailable_reason(&registry, "flite").contains("runtime missing"));
}

#[test]
fn bad_assets_disable_only_their_provider_before_native_loading() {
    let fixture = Fixture::new("flite");
    std::fs::write(fixture.root.join("voice.data"), b"bad").unwrap();
    let library = fixture.library();
    library.verify_assets("espeak").unwrap();
    assert!(library
        .verify_assets("flite")
        .unwrap_err()
        .to_string()
        .contains("SHA-256"));
    let pending = start_helper_initializations(
        vec![HelperEngineConfig::new("flite", "must-not-be-started")],
        Some(&library),
    );
    let mut registry = library.registry().unwrap();
    register_initialized_helpers(
        &mut registry,
        pending,
        Arc::new(AtomicU64::new(0)),
        Arc::new(IsolationBudget::new()),
        Some(&library),
    )
    .unwrap();
    assert!(unavailable_reason(&registry, "flite").contains("SHA-256"));
    assert!(create_engine("flite", None, Some(fixture.path.to_str().unwrap())).is_err());
}

#[test]
fn incomplete_managed_inventory_is_unavailable_instead_of_fatal() {
    let fixture = Fixture::new("flite");
    let library = fixture.library();
    let mut descriptor = EngineDescriptor::unavailable("flite", "unused");
    descriptor.availability = Availability::Available;
    descriptor.health = EngineHealth::Healthy;
    descriptor.capabilities.cancellation =
        omnivox_tts::contracts::CancellationSupport::SynthesisAndPlayback;
    descriptor.voices = vec![VoiceDescriptor::from_voice_info(
        "flite",
        VoiceInfo {
            identifier: "cmu_us_slt".into(),
            name: "SLT".into(),
            language: "en".into(),
            quality: VoiceQuality::Compact,
        },
    )];
    descriptor.default_voice_id = Some("cmu_us_slt".into());
    let pending = start_helper_initializations_with(
        vec![HelperEngineConfig::new("flite", "unused-helper")],
        move |config| HelperTtsEngine::new_deferred(config, descriptor.clone()).map(Arc::new),
    );
    let mut registry = library.registry().unwrap();
    register_initialized_helpers(
        &mut registry,
        pending,
        Arc::new(AtomicU64::new(0)),
        Arc::new(IsolationBudget::new()),
        Some(&library),
    )
    .unwrap();
    assert!(unavailable_reason(&registry, "flite").contains("complete configured voice set"));
}
