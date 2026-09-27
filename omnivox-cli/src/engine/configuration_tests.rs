use super::*;
#[cfg(unix)]
use omnivox_tts::contracts::{Availability, CancellationSupport, EngineHealth, VoiceDescriptor};
use omnivox_tts::engine_configuration::{shipped, Configuration, LoadedConfiguration};
#[cfg(unix)]
use omnivox_tts::{SynthesisRequest, TtsSettings, VoiceInfo, VoiceQuality};
use serde_json::json;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "omnivox framework {} {}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("helpers.d")).unwrap();
        Self(root)
    }
    fn config(&self, preferred: &[&str]) {
        let disabled: serde_json::Map<String, serde_json::Value> = shipped::ENGINES
            .iter()
            .map(|engine| (engine.id.into(), json!({"enabled":false})))
            .collect();
        std::fs::write(self.0.join("config.json"), serde_json::to_vec(&json!({"schema":1, "routing":{"preferred_engine_ids":preferred}, "engine_overrides": disabled})).unwrap()).unwrap();
    }
    #[cfg(unix)]
    fn descriptor(&self, id: &str, file: &str) -> PathBuf {
        let mut descriptor = EngineDescriptor::unavailable(id, "fixture");
        descriptor.availability = Availability::Available;
        descriptor.health = EngineHealth::Healthy;
        descriptor.capabilities.cancellation = CancellationSupport::SynthesisAndPlayback;
        descriptor.voices = vec![VoiceDescriptor::from_voice_info(
            id,
            VoiceInfo {
                identifier: "voice".into(),
                name: "Fixture voice".into(),
                language: "en-AU".into(),
                quality: VoiceQuality::Compact,
            },
        )];
        descriptor.default_voice_id = Some("voice".into());
        let path = self.0.join(file);
        std::fs::write(&path, serde_json::to_vec(&descriptor).unwrap()).unwrap();
        path
    }
    #[cfg(unix)]
    fn helper(&self) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let python = std::process::Command::new("python3")
            .args(["-c", "import sys; print(sys.executable)"])
            .output()
            .expect("Python 3 is required for the framework process fixture");
        assert!(python.status.success());
        let interpreter = String::from_utf8(python.stdout).unwrap();
        let path = self.0.join("unknown helper with spaces");
        std::fs::write(
            &path,
            format!(
                "#!{}\n{}",
                interpreter.trim(),
                include_str!("../../../test-fixtures/framework-helper.py")
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    #[cfg(unix)]
    fn manifest(&self, id: &str, program: &Path, descriptor: &Path, tag: &str) {
        std::fs::write(self.0.join("helpers.d").join(format!("{id}.json")), serde_json::to_vec(&json!({"schema":1, "engine_id":id, "program":program, "arguments":["--descriptor", descriptor, "--record",self.0.join("launches.jsonl"), "--tag", tag]})).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn disabled_native_engines_are_rejected_before_construction_and_exact_diagnostics_do_not_fallback()
{
    let fixture = Fixture::new();
    fixture.config(&[]);
    let root = Some(fixture.0.to_str().unwrap());
    let startup = EngineStartup::read("espeak", None, None, root).unwrap();
    assert!(!startup.permits_construction("espeak"));
    assert!(startup.helper_configs(false).is_empty());
    assert!(startup.helper_configs(true).is_empty());
    assert!(create_engine("espeak", None, None, root)
        .err()
        .unwrap()
        .to_string()
        .contains("disabled"));
    assert!(
        create_engines("espeak", None, None, root, Arc::new(AtomicU64::new(0)))
            .err()
            .unwrap()
            .to_string()
            .contains("No eligible")
    );
    std::fs::write(
        fixture.0.join("config.json"),
        b"{\"schema\":1,\"unrecognized\":true}",
    )
    .unwrap();
    assert!(
        create_engines("espeak", None, None, root, Arc::new(AtomicU64::new(0)))
            .err()
            .unwrap()
            .to_string()
            .contains("unknown field")
    );
}

#[test]
fn captured_environment_and_cli_preferences_feed_the_same_launch_resolution() {
    let fixture = Fixture::new();
    let loaded = LoadedConfiguration {
        configuration: Configuration::parse(br#"{"schema":1,"routing":{"preferred_engine_ids":["rhvoice"]},"engine_overrides":{"flite":{"program":"/local/flite"}}}"#, Platform::Unix).unwrap(),
        ..LoadedConfiguration::default()
    };
    let environment = LaunchEnvironment::from_variables([
        ("OMNIVOX_ENGINE".into(), "flite".into()),
        ("OMNIVOX_FLITE_HELPER".into(), "/captured/flite".into()),
    ]);
    let startup = EngineStartup::from_inputs(
        "espeak",
        None,
        None,
        loaded,
        environment,
        &fixture.0.join("omnivox"),
    )
    .unwrap();
    assert_eq!(&startup.order()[..2], &["espeak", "rhvoice"]);
    let config = startup
        .helper_configs(false)
        .into_iter()
        .find(|config| config.engine_id == "flite")
        .unwrap();
    assert_eq!(config.program, Path::new("/captured/flite"));
    assert_eq!(config.environment, startup.snapshot.resolved().environment);
}

#[test]
fn local_references_keep_unavailable_and_disabled_shipped_identities_visible() {
    let fixture = Fixture::new();
    let loaded = LoadedConfiguration {
        configuration: Configuration::parse(br#"{"schema":1,"routing":{"preferred_engine_ids":["rhvoice"],"disabled_engine_ids":["piper"]}}"#, Platform::native()).unwrap(),
        ..LoadedConfiguration::default()
    };
    let startup = EngineStartup::from_inputs(
        "",
        None,
        None,
        loaded,
        LaunchEnvironment::from_variables([]),
        &fixture.0.join("omnivox"),
    )
    .unwrap();
    let registry = startup.registry().unwrap();
    assert!(registry
        .inventory()
        .iter()
        .any(|entry| entry.id == "rhvoice" && !entry.can_synthesize()));
    assert!(registry
        .inventory()
        .iter()
        .any(|entry| entry.id == "piper" && !entry.can_synthesize()));
    assert!(registry.selection_permissions().disabled("piper"));
    assert!(startup.helper_configs(false).is_empty());
}

#[cfg(unix)]
#[test]
fn unknown_helper_works_in_server_and_exact_diagnostics_with_literal_arguments() {
    let fixture = Fixture::new();
    let id = "org.example.framework";
    fixture.config(&[id]);
    let helper = fixture.helper();
    let descriptor = fixture.descriptor(id, "descriptor.json");
    fixture.manifest(id, &helper, &descriptor, "literal $HOME $(no shell) spaces");
    let root = Some(fixture.0.to_str().unwrap());
    let created = create_engines("", None, None, root, Arc::new(AtomicU64::new(0))).unwrap();
    assert_eq!(created.preferred.descriptor().id, id);
    assert!(!created
        .registry
        .selection_permissions()
        .permits_automatic(id));
    let speech = created
        .preferred
        .synthesize(&SynthesisRequest::new(
            "fixture",
            TtsSettings {
                voice: "voice".into(),
                ..TtsSettings::default()
            },
        ))
        .unwrap();
    assert!(!speech.audio.is_empty());
    let exact = create_engine(id, None, None, root).unwrap();
    assert_eq!(exact.descriptor().voices[0].id.engine_id, id);
    assert!(create_engine("org.missing", None, None, root).is_err());
    let launches = std::fs::read_to_string(fixture.0.join("launches.jsonl")).unwrap();
    for launch in launches.lines() {
        let args: Vec<String> = serde_json::from_str(launch).unwrap();
        assert_eq!(args.last().unwrap(), "literal $HOME $(no shell) spaces");
    }
}

#[cfg(unix)]
#[test]
fn external_and_shipped_definition_paths_preserve_adapter_identity_and_pcm() {
    let fixture = Fixture::new();
    let id = "org.example.distribution";
    fixture.config(&[id]);
    let helper = fixture.helper();
    let descriptor = fixture.descriptor(id, "descriptor.json");
    fixture.manifest(id, &helper, &descriptor, "same adapter");
    let root = Some(fixture.0.to_str().unwrap());
    let startup = EngineStartup::read("", None, None, root).unwrap();
    let definitions = startup.helper_configs(true);
    let external = create_engines("", None, None, root, Arc::new(AtomicU64::new(0))).unwrap();

    // Model promotion with a test-only compiled definition of the same adapter.
    // It enters the actual shipped initializer/registry path with the same ID;
    // no manifest is read and no production reserved ID is repurposed.
    std::fs::remove_dir_all(fixture.0.join("helpers.d")).unwrap();
    let mut shipped = EngineRegistry::new();
    register_initialized_helpers(
        &mut shipped,
        start_helper_initializations(definitions, None),
        Arc::new(AtomicU64::new(0)),
        Arc::new(IsolationBudget::new()),
        None,
    )
    .unwrap();
    let promoted = shipped.engine(id).unwrap();
    assert_eq!(external.preferred.descriptor(), promoted.descriptor());
    let request = SynthesisRequest::new(
        "distribution fixture",
        TtsSettings {
            voice: "voice".into(),
            ..TtsSettings::default()
        },
    );
    let original = external.preferred.synthesize(&request).unwrap();
    let promoted = promoted.synthesize(&request).unwrap();
    assert!(!original.audio.is_empty());
    assert_eq!(original.audio.samples, promoted.audio.samples);
}

#[cfg(unix)]
#[test]
fn rescan_reuses_captured_registration_after_manifest_replacement() {
    let fixture = Fixture::new();
    fixture.config(&["org.recover", "org.fallback"]);
    let helper = fixture.helper();
    let wrong = fixture.descriptor("wrong.identity", "recover.json");
    let fallback = fixture.descriptor("org.fallback", "fallback.json");
    fixture.manifest("org.recover", &helper, &wrong, "original arguments");
    fixture.manifest("org.fallback", &helper, &fallback, "fallback");
    let created = create_engines(
        "",
        None,
        None,
        Some(fixture.0.to_str().unwrap()),
        Arc::new(AtomicU64::new(0)),
    )
    .unwrap();
    assert_eq!(created.preferred.descriptor().id, "org.fallback");
    assert!(created.registry.engine("org.recover").is_none());
    fixture.manifest(
        "org.recover",
        &fixture.0.join("must not launch"),
        &wrong,
        "changed arguments",
    );
    fixture.descriptor("org.recover", "recover.json");
    created.registry.request_rescan("org.recover").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while created.registry.engine("org.recover").is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(created.registry.engine("org.recover").is_some());
    let launches = std::fs::read_to_string(fixture.0.join("launches.jsonl")).unwrap();
    assert_eq!(
        launches
            .lines()
            .filter(|line| line.contains("original arguments"))
            .count(),
        2
    );
    assert!(!launches.contains("changed arguments"));
}
