use super::*;
use omnivox_tts::contracts::{PhysicalVoiceId, VoiceDescriptor};
use omnivox_tts::control::*;
use omnivox_tts::logical_voices::LogicalVoiceRegistry;
use omnivox_tts::routing_policy::RoutingPolicyRegistry;
use omnivox_tts::{VoiceInfo, VoiceQuality};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omnivox-startup-library-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, builtin: bool) -> PathBuf {
        let path = self.0.join("generation with spaces.json");
        std::fs::write(&path, format!(r#"{{"schema_version":1,"target_id":"11111111-1111-4111-8111-111111111111","profile_id":"22222222-2222-4222-8222-222222222222","generation_id":"33333333-3333-4333-8333-333333333333","disabled_physical_ids":[{{"engine_id":"espeak","voice_id":"excluded"}}],"piper":{{"models":[]}},"flite":{{"builtin_slt":{builtin},"files":[]}}}}"#)).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn voice(engine: &str, id: &str) -> VoiceDescriptor {
    VoiceDescriptor::from_voice_info(
        engine,
        VoiceInfo {
            identifier: id.to_owned(),
            name: id.to_owned(),
            language: "en".to_owned(),
            quality: VoiceQuality::Compact,
        },
    )
}

#[test]
fn empty_path_fails_instead_of_selecting_legacy_mode() {
    assert!(StartupLibrary::read(PathBuf::new(), ProviderOverrides::default()).is_err());
}

#[test]
fn managed_startup_pins_bytes_and_omits_empty_helpers() {
    let fixture = Fixture::new();
    let path = fixture.write(false);
    let library = StartupLibrary::read(path.clone(), ProviderOverrides::default()).unwrap();
    let registry = library.registry().unwrap();
    assert_eq!(registry.len(), 2);
    for engine in ["piper", "flite"] {
        assert!(!library.requires(engine));
        assert!(registry.engine(engine).is_none());
        assert!(registry.request_rescan(engine).is_err());
    }
    preflight_responses(&registry, "espeak").unwrap();
    let mut config = HelperEngineConfig::new("flite", "unused helper");
    library.configure(&mut config);
    assert_eq!(
        config.arguments[1],
        path.canonicalize().unwrap().as_os_str()
    );
    assert_eq!(config.arguments[3], library.library.sha256().as_str());
    let digest = library.library.sha256();
    let mut bytes = library.library.source_bytes().to_vec();
    bytes.push(b'\n');
    let host = if cfg!(windows) {
        HostPlatform::Windows
    } else {
        HostPlatform::Posix
    };
    assert!(RuntimeLibrary::read_expected(bytes.as_slice(), host, Some(&digest)).is_err());
    RuntimeLibrary::read_expected(library.library.source_bytes(), host, Some(&digest)).unwrap();
}

#[test]
fn override_preserves_global_exclusions_and_status_matches_inventory_snapshot() {
    let fixture = Fixture::new();
    let library = StartupLibrary::read(
        fixture.write(false),
        ProviderOverrides {
            piper: true,
            flite: true,
        },
    )
    .unwrap();
    let registry = library.registry().unwrap();
    assert!(registry.is_empty());
    let mut config = HelperEngineConfig::new("piper", "unused");
    config.arguments = vec!["--model".into(), "explicit.onnx".into()];
    library.configure(&mut config);
    assert_eq!(config.arguments[1], "explicit.onnx");
    let mut unavailable =
        EngineDescriptor::unavailable("espeak", "runtime temporarily unavailable");
    unavailable.voices = vec![voice("espeak", "allowed"), voice("espeak", "excluded")];
    let status = registry.voice_library_status(42, &[unavailable.clone()], &[]);
    assert_eq!(status.inventory_generation, 42);
    assert_eq!(
        status.configuration.as_ref().unwrap().sha256,
        library.library.sha256()
    );
    assert_eq!(status.overridden_engines, ["flite", "piper"]);
    assert_eq!(
        status.eligible_voices,
        [PhysicalVoiceId::new("espeak", "allowed")]
    );
    assert!(registry
        .voice_library_status(43, &[unavailable.clone()], &["espeak".into()])
        .eligible_voices
        .is_empty());

    let mut logical = LogicalVoiceRegistry::default();
    let mut policy = RoutingPolicyRegistry::new("espeak");
    let payload = encode_request(&ControlRequestEnvelope {
        protocol_version: 1,
        request_id: 19,
        request: ControlRequest::VoiceLibraryStatusV1,
    })
    .unwrap();
    let response = process_control_request_with_library(
        &payload,
        "test",
        42,
        "espeak",
        &[unavailable],
        &[],
        &mut logical,
        &mut policy,
        Some(&status),
    );
    assert_eq!(response.request_id, Some(19));
    assert_eq!(
        response.response,
        ControlResponse::VoiceLibraryStatusV1(status)
    );
    assert_eq!(
        decode_response(&encode_response(&response).unwrap()).unwrap(),
        response
    );
    let payload = encode_request(&ControlRequestEnvelope {
        protocol_version: 1,
        request_id: 20,
        request: ControlRequest::Capabilities,
    })
    .unwrap();
    let response = process_control_request_with_library(
        &payload,
        "test",
        42,
        "espeak",
        &[],
        &[],
        &mut logical,
        &mut policy,
        None,
    );
    assert!(
        matches!(response.response, ControlResponse::Capabilities{features,..} if !features.contains(&"voice_library_v1".into()))
    );
}

#[test]
fn required_helper_inventory_and_complete_transport_size_are_checked() {
    let fixture = Fixture::new();
    let library = StartupLibrary::read(fixture.write(true), ProviderOverrides::default()).unwrap();
    assert!(library.requires("flite"));
    let mut descriptor = EngineDescriptor::unavailable("flite", "test");
    assert!(library.validate_descriptor(&descriptor).is_err());
    descriptor.availability = omnivox_tts::contracts::Availability::Available;
    descriptor.health = omnivox_tts::contracts::EngineHealth::Healthy;
    descriptor.voices = vec![voice("flite", "wrong")];
    assert!(library.validate_descriptor(&descriptor).is_err());
    descriptor.voices = vec![voice("flite", "cmu_us_slt")];
    library.validate_descriptor(&descriptor).unwrap();

    let mut registry = library.registry().unwrap();
    registry
        .register_unavailable(
            EngineDescriptor::unavailable("unrelated", "x".repeat(MAX_CONTROL_PAYLOAD_BYTES)),
            || Err("unused".into()),
        )
        .unwrap();
    assert!(preflight_responses(&registry, "espeak").is_err());
}
