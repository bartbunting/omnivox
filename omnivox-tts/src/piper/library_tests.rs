use super::*;
use crate::voice_library::{HostPlatform, RuntimeLibrary};
use crate::{ResolvedAnchor, SynthesisCancellationToken, SynthesisMarker, TtsSettings};
use serde_json::{json, Value};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("omnivox-piper-library-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("alpha.onnx"),
            include_bytes!("../../../test-fixtures/piper-speakers/alpha.onnx"),
        )
        .unwrap();
        std::fs::write(
            root.join("beta.onnx"),
            include_bytes!("../../../test-fixtures/piper-speakers/beta.onnx"),
        )
        .unwrap();
        std::fs::write(root.join("z-bad.onnx"), vec![b'x'; 828]).unwrap();
        std::fs::write(
            root.join("config.json"),
            include_bytes!("../../../test-fixtures/piper-speakers/config.json"),
        )
        .unwrap();
        Self(root)
    }

    fn document(&self) -> Value {
        let asset = |name: &str| {
            let path = self.0.join(name);
            json!({"path": path.to_str().unwrap(), "bytes": std::fs::metadata(path).unwrap().len(), "sha256": "0".repeat(64)})
        };
        let models: Vec<_> = ["alpha", "beta", "z-bad"].iter().map(|name| json!({
            "identity": {"catalogue_key": name}, "model": asset(&format!("{name}.onnx")), "config": asset("config.json"),
            "voices": ([0, 1].iter().map(|speaker| json!({
                "physical_id": format!("piper:v1/c/{name}/{speaker}"), "speaker_index": speaker,
                "display_name": format!("{name} {speaker}"), "language": null
            })).collect::<Vec<_>>())
        })).collect();
        json!({"schema_version":1, "target_id":"11111111-1111-4111-8111-111111111111",
            "profile_id":"22222222-2222-4222-8222-222222222222", "generation_id":"33333333-3333-4333-8333-333333333333",
            "disabled_physical_ids": [], "piper":{"models": models}, "flite":null})
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn engine_from_document(document: &Value) -> PiperTtsEngine {
    let host = if cfg!(windows) {
        HostPlatform::Windows
    } else {
        HostPlatform::Posix
    };
    let library = RuntimeLibrary::parse(&serde_json::to_vec(document).unwrap(), host).unwrap();
    PiperTtsEngine::from_library(&library).unwrap()
}

fn request(model: &str, speaker: u32) -> SynthesisRequest {
    SynthesisRequest::new(
        "a",
        TtsSettings {
            voice: format!("piper:v1/c/{model}/{speaker}"),
            ..TtsSettings::default()
        },
    )
}

#[derive(Default)]
struct Sink {
    start: Option<SynthesisStreamStart>,
    samples: Vec<f32>,
    cancel_on_start: Option<SynthesisCancellationToken>,
}

impl SynthesisStreamSink for Sink {
    fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
        self.start = Some(start);
        if let Some(token) = self.cancel_on_start.take() {
            token.cancel();
        }
        Ok(())
    }
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        self.samples.extend(audio.samples);
        Ok(())
    }
    fn markers(&mut self, _: Vec<SynthesisMarker>, _: Vec<ResolvedAnchor>) -> Result<(), TtsError> {
        Ok(())
    }
}

fn constant_samples(samples: &[f32], expected: f32) {
    assert!(!samples.is_empty());
    assert!(
        samples
            .iter()
            .all(|sample| (*sample - expected).abs() < 0.00001),
        "expected {expected}; got {:?}",
        &samples[..samples.len().min(8)]
    );
}

#[test]
fn native_library_selects_models_and_speakers_without_overlapping_residency() {
    let fixture = Fixture::new();
    let document = fixture.document();
    let engine = engine_from_document(&document);
    assert!(engine.state.lock().unwrap().is_none());
    {
        let _native_lock = engine.state.lock().unwrap();
        // Metadata discovery is independent of the potentially blocked native lock.
        assert_eq!(engine.descriptor().voices.len(), 6);
    }
    assert!(matches!(
        engine.synthesize(&request("unlisted", 0)),
        Err(TtsError::VoiceNotFound(_))
    ));
    let cancelled = SynthesisCancellationToken::new();
    cancelled.cancel();
    assert!(engine
        .synthesize(&request("alpha", 0).with_cancellation(cancelled.clone()))
        .is_err());
    assert!(engine.state.lock().unwrap().is_none());
    assert!(engine.failed_models.lock().unwrap().is_empty());

    let first = engine.synthesize(&request("alpha", 0)).unwrap();
    assert_eq!(first.actual_voice.unwrap().voice_id, "piper:v1/c/alpha/0");
    constant_samples(&first.audio.samples, 0.125);
    let pointer = engine.state.lock().unwrap().as_ref().unwrap().native.0;
    let second = engine.synthesize(&request("alpha", 1)).unwrap();
    constant_samples(&second.audio.samples, 0.25);
    assert_eq!(
        engine.state.lock().unwrap().as_ref().unwrap().native.0,
        pointer
    );

    let mut sink = Sink::default();
    let completion = engine
        .synthesize_stream(&request("beta", 1), &mut sink)
        .unwrap();
    assert_eq!(completion.frame_count as usize * 2, sink.samples.len());
    assert_eq!(
        sink.start.unwrap().actual_voice.unwrap().voice_id,
        "piper:v1/c/beta/1"
    );
    constant_samples(&sink.samples, 0.5);
    assert_eq!(
        engine.state.lock().unwrap().as_ref().unwrap().model_index,
        1
    );

    assert!(matches!(
        engine.synthesize(&request("z-bad", 0)),
        Err(TtsError::VoiceNotFound(_))
    ));
    assert!(engine.state.lock().unwrap().is_none());
    let descriptor = engine.descriptor();
    assert!(descriptor.can_synthesize());
    assert!(descriptor
        .voices
        .iter()
        .filter(|voice| voice.id.voice_id.contains("z-bad"))
        .all(|voice| matches!(voice.availability, Availability::Unavailable { .. })));
    // Repairing the file does not quietly retry a quarantined model, including
    // another speaker of it. A fresh generation/helper is required.
    std::fs::copy(fixture.0.join("alpha.onnx"), fixture.0.join("z-bad.onnx")).unwrap();
    assert!(matches!(
        engine.synthesize(&request("z-bad", 1)),
        Err(TtsError::VoiceNotFound(_))
    ));
    let good = engine.synthesize(&request("beta", 0)).unwrap();
    constant_samples(&good.audio.samples, 0.375);
    assert!(engine
        .synthesize_stream(
            &request("alpha", 0).with_cancellation(cancelled),
            &mut Sink::default()
        )
        .is_err());
    assert_eq!(
        engine.state.lock().unwrap().as_ref().unwrap().model_index,
        1
    );

    // Cancel after the entry check, before native selection. Starting the
    // worker must not clear this request's cancellation and load alpha.
    let late_cancel = SynthesisCancellationToken::new();
    let mut sink = Sink {
        cancel_on_start: Some(late_cancel.clone()),
        ..Sink::default()
    };
    assert!(engine
        .synthesize_stream(
            &request("alpha", 0).with_cancellation(late_cancel),
            &mut sink
        )
        .is_err());
    assert!(sink.samples.is_empty());
    assert_eq!(
        engine.state.lock().unwrap().as_ref().unwrap().model_index,
        1
    );
    drop(engine);

    let fresh = engine_from_document(&document);
    constant_samples(
        &fresh
            .synthesize(&request("z-bad", 1))
            .unwrap()
            .audio
            .samples,
        0.25,
    );
    drop(fresh);

    let mut disabled = document.clone();
    disabled["piper"]["models"][0]["voices"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    disabled["disabled_physical_ids"] =
        json!([{"engine_id":"piper", "voice_id":"piper:v1/c/alpha/1"}]);
    let restricted = engine_from_document(&disabled);
    assert!(matches!(
        restricted.synthesize(&request("alpha", 1)),
        Err(TtsError::VoiceNotFound(_))
    ));
    assert!(restricted.state.lock().unwrap().is_none());
    drop(restricted);

    let mut empty = document.clone();
    empty["piper"]["models"] = json!([]);
    let empty = engine_from_document(&empty);
    assert!(empty.available_voices().is_empty());
    assert!(empty.descriptor().default_voice_id.is_none());
    assert!(!empty.descriptor().can_synthesize());
    assert!(matches!(
        empty.synthesize(&request("alpha", 0)),
        Err(TtsError::VoiceNotFound(_))
    ));
    assert!(empty.state.lock().unwrap().is_none());

    let mut changed_asset = document.clone();
    changed_asset["piper"]["models"][0]["model"]["bytes"] = json!(1);
    let changed = engine_from_document(&changed_asset);
    assert_eq!(changed.available_voices().len(), 6);
    assert!(matches!(
        changed.synthesize(&request("alpha", 0)),
        Err(TtsError::VoiceNotFound(_))
    ));
    assert!(changed.state.lock().unwrap().is_none());
    assert_eq!(changed.available_voices().len(), 4);

    let mut invalid_speaker = document.clone();
    invalid_speaker["piper"]["models"][0]["voices"][1]["speaker_index"] = json!(2);
    invalid_speaker["piper"]["models"][0]["voices"][1]["physical_id"] = json!("piper:v1/c/alpha/2");
    let invalid = engine_from_document(&invalid_speaker);
    assert!(matches!(
        invalid.synthesize(&request("alpha", 2)),
        Err(TtsError::VoiceNotFound(_))
    ));
    assert!(invalid.state.lock().unwrap().is_none());
    constant_samples(
        &invalid
            .synthesize(&request("beta", 0))
            .unwrap()
            .audio
            .samples,
        0.375,
    );
    drop(invalid);

    let legacy_model = fixture.0.join("legacy.onnx");
    std::fs::copy(fixture.0.join("alpha.onnx"), &legacy_model).unwrap();
    std::fs::copy(
        fixture.0.join("config.json"),
        fixture.0.join("legacy.onnx.json"),
    )
    .unwrap();
    let legacy = PiperTtsEngine::new(&legacy_model).unwrap();
    assert_eq!(legacy.available_voices()[0].identifier, "piper:legacy");
    let request = SynthesisRequest::new(
        "a",
        TtsSettings {
            voice: "piper:legacy".to_owned(),
            ..TtsSettings::default()
        },
    );
    constant_samples(&legacy.synthesize(&request).unwrap().audio.samples, 0.125);
}
