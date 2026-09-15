use super::*;
use crate::contracts::{
    EngineHealth, FallbackPolicy, LogicalVoiceDefinition, VoiceDescriptor, VoiceSelector,
};
use crate::engine_registry::EngineRegistry;
use crate::resolver::resolve_voice;
use crate::voice_library::HostPlatform;
use crate::{AudioBuffer, TtsSettings, VoiceQuality};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const ZERO: &str = "piper:v1/c/example/0";
const TWO: &str = "piper:v1/c/example/2";

fn document() -> Value {
    let asset = |path| json!({"path":path,"bytes":1,"sha256":"0".repeat(64)});
    json!({"schema_version":1,"target_id":"11111111-1111-4111-8111-111111111111",
        "profile_id":"22222222-2222-4222-8222-222222222222","generation_id":"33333333-3333-4333-8333-333333333333",
        "disabled_physical_ids":[{"engine_id":"espeak","voice_id":"v0"},{"engine_id":"piper","voice_id":"piper:excluded"}],
        "piper":{"models":[{"identity":{"catalogue_key":"example"},"model":asset("/test/model.onnx"),"config":asset("/test/model.json"),
            "voices":[{"physical_id":ZERO,"speaker_index":0,"display_name":"Zero","language":null},
                {"physical_id":TWO,"speaker_index":2,"display_name":"Two","language":null}]}]},
        "flite":{"builtin_slt":false,"files":[]}})
}

fn make_policy(document: &Value, overrides: ProviderOverrides) -> Arc<VoiceEligibility> {
    Arc::new(VoiceEligibility::from_library(
        &RuntimeLibrary::parse(&serde_json::to_vec(document).unwrap(), HostPlatform::Posix)
            .unwrap(),
        overrides,
    ))
}

fn descriptor(id: &str, voices: &[&str]) -> EngineDescriptor {
    let mut descriptor = EngineDescriptor::unavailable(id, "unused");
    descriptor.availability = Availability::Available;
    descriptor.health = EngineHealth::Healthy;
    descriptor.voices = voices
        .iter()
        .map(|voice| {
            VoiceDescriptor::from_voice_info(
                id,
                VoiceInfo {
                    identifier: (*voice).to_owned(),
                    name: format!("Label {voice}"),
                    language: "en-US".to_owned(),
                    quality: VoiceQuality::Compact,
                },
            )
        })
        .collect();
    descriptor.default_voice_id = voices.first().map(|voice| (*voice).to_owned());
    descriptor
}

struct Native {
    descriptor: Mutex<EngineDescriptor>,
    calls: AtomicUsize,
    probes: AtomicUsize,
}
impl Native {
    fn new(id: &str, voices: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            descriptor: Mutex::new(descriptor(id, voices)),
            calls: AtomicUsize::new(0),
            probes: AtomicUsize::new(0),
        })
    }
}
impl TtsEngine for Native {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.lock().unwrap().clone()
    }
    fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let id = self.descriptor().id;
        Ok(SynthesisResult::audio(
            &id,
            Some(PhysicalVoiceId::new(&id, request.voice_id_for_engine(&id)?)),
            AudioBuffer::empty(),
        ))
    }
    fn stop(&self) {}
    fn is_speaking(&self) -> bool {
        false
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        panic!("eligibility uses descriptor metadata")
    }
    fn voice_info(&self, _: &str) -> Option<VoiceInfo> {
        panic!("native aliases must not bypass eligibility")
    }
}

#[derive(Default)]
struct Sink(usize);
impl SynthesisStreamSink for Sink {
    fn start(&mut self, _: crate::SynthesisStreamStart) -> Result<(), TtsError> {
        self.0 += 1;
        Ok(())
    }
    fn audio(&mut self, _: AudioBuffer) -> Result<(), TtsError> {
        Ok(())
    }
    fn markers(
        &mut self,
        _: Vec<crate::SynthesisMarker>,
        _: Vec<crate::ResolvedAnchor>,
    ) -> Result<(), TtsError> {
        Ok(())
    }
}

fn request(id: &str) -> SynthesisRequest {
    SynthesisRequest::new(
        "test",
        TtsSettings {
            voice: id.to_owned(),
            ..TtsSettings::default()
        },
    )
}

#[test]
fn explicit_overrides_preserve_global_exclusions_and_native_defaults() {
    let policy = make_policy(
        &document(),
        ProviderOverrides {
            piper: true,
            flite: true,
        },
    );
    assert_eq!(policy.overridden_engines(), ["flite", "piper"]);
    assert!(policy.permits(&PhysicalVoiceId::new("piper", "piper:legacy")));
    assert!(!policy.permits(&PhysicalVoiceId::new("piper", "piper:excluded")));
    assert!(policy.permits(&PhysicalVoiceId::new("flite", "cmu_us_slt")));
    assert!(!policy.permits(&PhysicalVoiceId::new("espeak", "v0")));
    assert!(policy.permits(&PhysicalVoiceId::new("other", "v0")));
    let projected = policy.project_descriptor(descriptor("piper", &["piper:legacy", ZERO]));
    assert_eq!(projected.default_voice_id.as_deref(), Some("piper:legacy"));

    let mut source = document();
    source["piper"] = Value::Null;
    source["flite"] = Value::Null;
    let legacy = make_policy(
        &source,
        ProviderOverrides {
            piper: true,
            flite: true,
        },
    );
    assert!(legacy.overridden_engines().is_empty());
    assert_eq!(
        legacy
            .project_descriptor(descriptor("espeak", &["v0", "v1"]))
            .default_voice_id
            .as_deref(),
        Some("v1")
    );
}

#[test]
fn managed_defaults_follow_projection_order_and_health_remains_separate() {
    let policy = make_policy(&document(), ProviderOverrides::default());
    let mut native = descriptor("piper", &[TWO, ZERO, "piper:unlisted"]);
    let projected = policy.project_descriptor(native.clone());
    assert_eq!(projected.default_voice_id.as_deref(), Some(ZERO));
    assert!(!projected.voices[2].availability.is_available());
    native.voices[1].availability = Availability::Unavailable {
        reason: "model failed".to_owned(),
    };
    native.health = EngineHealth::Failed {
        reason: "helper exited".to_owned(),
    };
    let projected = policy.project_descriptor(native);
    assert_eq!(projected.default_voice_id.as_deref(), Some(TWO));
    assert!(matches!(projected.health, EngineHealth::Failed { .. }));
    assert_eq!(
        policy.eligible_voices(std::slice::from_ref(&projected), &[]),
        [
            PhysicalVoiceId::new("piper", ZERO),
            PhysicalVoiceId::new("piper", TWO)
        ]
    );
    assert!(policy
        .eligible_voices(&[projected], &["piper".to_owned()])
        .is_empty());
    assert!(policy.excludes_provider("flite"));
    let empty = policy.project_descriptor(descriptor("flite", &["cmu_us_slt"]));
    assert!(!empty.can_synthesize());
    assert!(empty.default_voice_id.is_none());
    let unavailable = EngineDescriptor::unavailable("rhvoice", "runtime missing");
    assert_eq!(policy.project_descriptor(unavailable.clone()), unavailable);
}

#[test]
fn registry_guards_both_native_paths_and_preserves_excluded_inventory_rows() {
    let policy = make_policy(&document(), ProviderOverrides::default());
    let native = Native::new("piper", &["piper:excluded", TWO, ZERO]);
    let mut registry = EngineRegistry::with_voice_eligibility(policy);
    registry.register(native.clone()).unwrap();
    let guarded = registry.engine("piper").unwrap();
    assert_eq!(registry.inventory()[0].voices.len(), 3);
    assert_eq!(
        registry.inventory()[0].default_voice_id.as_deref(),
        Some(ZERO)
    );
    assert_eq!(guarded.available_voices().len(), 2);
    for id in [
        "piper:excluded",
        "piper:unlisted",
        "Label piper:excluded",
        "",
    ] {
        let mut sink = Sink::default();
        assert!(matches!(
            guarded.synthesize(&request(id)),
            Err(TtsError::VoiceNotFound(_))
        ));
        assert!(matches!(
            guarded.synthesize_stream(&request(id), &mut sink),
            Err(TtsError::VoiceNotFound(_))
        ));
        assert_eq!(sink.0, 0);
    }
    let mut typed = request(ZERO);
    typed.requested_voice = Some(PhysicalVoiceId::new("piper", "piper:excluded"));
    assert!(guarded.synthesize(&typed).is_err());
    assert_eq!(native.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        guarded
            .synthesize(&request(ZERO))
            .unwrap()
            .actual_voice
            .unwrap()
            .voice_id,
        ZERO
    );
    guarded
        .synthesize_stream(&request(TWO), &mut Sink::default())
        .unwrap();
    assert_eq!(native.calls.load(Ordering::SeqCst), 2);

    // A descriptor refresh cannot re-enable an excluded voice.
    native.descriptor.lock().unwrap().voices[0].display_name = "Renamed".to_owned();
    let generation = registry.generation();
    assert!(registry.refresh_descriptor("piper").unwrap());
    assert_eq!(registry.generation(), generation + 1);
    assert!(!registry.inventory()[0].voices[0]
        .availability
        .is_available());
}

#[test]
fn exact_default_properties_and_fallback_selectors_share_eligibility() {
    let policy = make_policy(&document(), ProviderOverrides::default());
    let inventory = vec![policy.project_descriptor(descriptor("espeak", &["v0", "v1"]))];
    let mut definition = LogicalVoiceDefinition {
        id: "saved".to_owned(),
        language: Some("en-US".to_owned()),
        preferences: vec![],
        acss: Default::default(),
        effects: Default::default(),
    };
    for selector in [
        VoiceSelector::EngineDefault {
            engine_id: "espeak".to_owned(),
        },
        VoiceSelector::Properties {
            engine_id: None,
            language: Some("en-US".to_owned()),
            gender: None,
        },
        VoiceSelector::Exact(PhysicalVoiceId::new("espeak", "v1")),
    ] {
        definition.preferences = vec![selector];
        assert_eq!(
            resolve_voice(&inventory, &definition, &FallbackPolicy::default())
                .unwrap()
                .realized
                .voice_id,
            "v1"
        );
    }
    definition.preferences = vec![VoiceSelector::Exact(PhysicalVoiceId::new("espeak", "v0"))];
    let saved = definition.clone();
    assert!(resolve_voice(&inventory, &definition, &FallbackPolicy::default()).is_err());
    let fallback = FallbackPolicy {
        fallback_engines: vec!["espeak".to_owned()],
        ..FallbackPolicy::default()
    };
    assert_eq!(
        resolve_voice(&inventory, &definition, &fallback)
            .unwrap()
            .realized
            .voice_id,
        "v1"
    );
    assert_eq!(definition, saved);
}

#[test]
fn empty_load_sets_cannot_rescan_and_late_discovery_retains_exclusions() {
    let policy = make_policy(&document(), ProviderOverrides::default());
    let mut registry = EngineRegistry::with_voice_eligibility(policy.clone());
    registry
        .register_unavailable(EngineDescriptor::unavailable("flite", "unused"), || {
            panic!("empty provider must never spawn")
        })
        .unwrap();
    assert!(registry.request_rescan("flite").is_err());
    let native = Native::new("flite", &["cmu_us_slt"]);
    assert!(policy
        .guard_engine(native.clone())
        .prepare_recovery_probe()
        .is_err());
    assert_eq!(native.probes.load(Ordering::SeqCst), 0);
    registry
        .register_unavailable(
            EngineDescriptor::unavailable("espeak", "startup failure"),
            || Ok(Native::new("espeak", &["v0", "v1"])),
        )
        .unwrap();
    let before = registry.generation();
    registry.request_rescan("espeak").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while registry.generation() < before + 2 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let engine = registry.engine("espeak").unwrap();
    assert!(engine.synthesize(&request("v0")).is_err());
    assert!(engine.synthesize(&request("v1")).is_ok());
    assert_eq!(
        registry
            .descriptor("espeak")
            .unwrap()
            .default_voice_id
            .as_deref(),
        Some("v1")
    );
}
