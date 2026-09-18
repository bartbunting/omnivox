//! Silent real-helper acceptance through the Rust parent, not a direct wire client.
//! Usage: helper_parameters_probe ENGINE PROGRAM VOICE PARAMETER INTEGER [HELPER-ARG...]
use omnivox_tts::{
    helper_engine::{HelperEngineConfig, HelperTtsEngine},
    helper_protocol::parameters::*,
    native_parameters::ParameterCatalogue,
    *,
};
use serde_json::json;
use std::{
    cell::Cell,
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};

fn catalogue(engine: &HelperTtsEngine, id: &str) -> Result<ParameterCatalogue, Box<dyn Error>> {
    let mut pages = CatalogueAssembly::new(CatalogueQuery {
        engine_id: id.into(),
        voice_id: None,
        cursor: None,
        expected_catalogue_revision: None,
    })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while let Some(query) = pages.next_query().cloned() {
        let result = engine.query_parameters(query.clone())?;
        match result {
            CatalogueResult::Ready { .. } => pages.push(&query, &result)?,
            CatalogueResult::Busy { retry_after_ms } if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(u64::from(retry_after_ms)))
            }
            other => return Err(format!("Catalogue unavailable: {other:?}").into()),
        }
    }
    Ok(pages.finish()?)
}
struct Sink<'a> {
    engine: &'a HelperTtsEngine,
    receipt: &'a Cell<bool>,
    frames: u64,
    interrupt: bool,
    busy: bool,
}
impl SynthesisStreamSink for Sink<'_> {
    fn start(&mut self, _: SynthesisStreamStart) -> Result<(), TtsError> {
        assert!(self.receipt.get());
        Ok(())
    }
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        assert!(self.receipt.get());
        assert!(audio.samples.iter().all(|v| v.is_finite()));
        self.frames += audio.frame_count() as u64;
        if self.interrupt && !self.busy {
            let query = CatalogueQuery {
                engine_id: self.engine.descriptor().id,
                voice_id: None,
                cursor: None,
                expected_catalogue_revision: None,
            };
            let start = Instant::now();
            assert!(matches!(
                self.engine.query_parameters(query).unwrap(),
                CatalogueResult::Busy { .. }
            ));
            assert!(start.elapsed() < Duration::from_millis(100));
            self.busy = true;
            self.engine.stop();
        }
        Ok(())
    }
    fn markers(&mut self, _: Vec<SynthesisMarker>, _: Vec<ResolvedAnchor>) -> Result<(), TtsError> {
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 6 {
        return Err(
            "Usage: helper_parameters_probe ENGINE PROGRAM VOICE PARAMETER INTEGER [HELPER-ARG...]"
                .into(),
        );
    }
    let mut config = HelperEngineConfig::new(&args[1], &args[2]);
    config.arguments = args[6..].iter().map(Into::into).collect();
    let engine = Arc::new(HelperTtsEngine::new(config)?);
    let execution =
        Arc::new(voice_library::VoiceEligibility::default()).guard_engine(engine.clone());
    let initial = catalogue(&engine, &args[1])?;
    let mut native: VoiceParameters = serde_json::from_value(
        json!({"native":{"engine_id":args[1],"schema_id":initial.identity.schema_id,
        "parameters":{args[4].clone():{"op":"set","value":args[5].parse::<i64>()?}}},"context_dimensions":[],"expected_identity":initial.identity,"unavailable_policy":"require"}),
    )?;
    let request = SynthesisRequest::new(
        "Native parameter parent acceptance.",
        TtsSettings {
            voice: args[3].clone(),
            ..TtsSettings::default()
        },
    );
    let draft = engine.explain_parameters(ExplanationSource::Draft {
        settings: omnivox_tts::helper_protocol::HelperSynthesisSettings {
            voice_id: Some(args[3].clone()),
            rate: request.settings.rate,
            pitch: request.settings.pitch,
            volume: request.settings.volume,
            pitch_range: None,
            stress: None,
            richness: None,
        },
        voice_parameters: Some(Box::new(native.clone())),
    })?;
    assert!(matches!(
        draft,
        ExplanationResult::Ready {
            evidence: Evidence::Planned,
            ..
        }
    ));
    let (buffered, receipt) = execution.synthesize_with_parameters(&request, &native)?;
    assert!(!buffered.audio.is_empty());
    let explanation = engine.explain_parameters(ExplanationSource::Applied {
        plan_id: receipt.plan_id.clone().unwrap(),
    })?;
    assert!(matches!(
        explanation,
        ExplanationResult::Ready {
            evidence: Evidence::AdapterApplied,
            ..
        }
    ));
    let seen = Cell::new(false);
    let mut sink = Sink {
        engine: &engine,
        receipt: &seen,
        frames: 0,
        interrupt: false,
        busy: false,
    };
    let completed =
        execution.synthesize_stream_with_parameters(&request, &native, &mut sink, &mut |_| {
            assert!(!seen.replace(true));
        })?;
    assert_eq!(sink.frames, completed.frame_count);
    assert!(sink.frames > 0);
    let progressive_frames = sink.frames;
    let mut long = request.clone();
    long.text = "A long cancellable native parameter utterance. ".repeat(500);
    seen.set(false);
    sink.frames = 0;
    sink.interrupt = true;
    let cancelled =
        execution.synthesize_stream_with_parameters(&long, &native, &mut sink, &mut |_| {
            seen.set(true);
        });
    eprintln!("Cancellation outcome: {cancelled:?}");
    assert!(cancelled.is_err());
    assert!(sink.busy);
    let native_cancel_error = format!("{:?}", cancelled.unwrap_err());
    let native_restarted = engine.prewarm_connection()?;
    let refreshed = catalogue(&engine, &args[1])?;
    assert_eq!(
        refreshed.identity.catalogue_revision,
        initial.identity.catalogue_revision
    );
    native.expected_identity = refreshed.identity;
    assert!(execution
        .synthesize_with_parameters(&request, &native)
        .is_ok());
    // Compare the same first-PCM cancellation with ordinary speech. Existing
    // watchdog retirement is valid, but stale native identity must never replay.
    seen.set(true);
    sink.frames = 0;
    sink.busy = false;
    let ordinary_cancel = engine.synthesize_stream(&long, &mut sink);
    assert!(ordinary_cancel.is_err() && sink.busy);
    let ordinary_cancel_error = format!("{:?}", ordinary_cancel.unwrap_err());
    let ordinary_restarted = engine.prewarm_connection()?;
    native.expected_identity = catalogue(&engine, &args[1])?.identity;
    assert!(engine.synthesize(&request).is_ok());
    let mut stale = native.clone();
    stale.expected_identity.runtime_generation += 1;
    assert!(execution
        .synthesize_with_parameters(&request, &stale)
        .is_err());
    stale.unavailable_policy = UnavailablePolicy::CommonOnly;
    assert_eq!(
        execution
            .synthesize_with_parameters(&request, &stale)?
            .1
            .status,
        ApplicationStatus::CommonOnly
    );
    engine.prepare_recovery_probe()?;
    let renewed = catalogue(&engine, &args[1])?;
    assert_eq!(
        renewed.identity.catalogue_revision,
        initial.identity.catalogue_revision
    );
    assert_ne!(
        renewed.identity.runtime_generation,
        initial.identity.runtime_generation
    );
    assert!(matches!(
        engine.explain_parameters(ExplanationSource::Applied {
            plan_id: receipt.plan_id.unwrap()
        })?,
        ExplanationResult::Unavailable {
            reason: ExplanationUnavailable::PlanExpired,
            ..
        }
    ));
    assert!(execution
        .synthesize_with_parameters(&request, &native)
        .is_err());
    let mut fresh = native;
    fresh.expected_identity = renewed.identity;
    assert!(execution
        .synthesize_with_parameters(&request, &fresh)
        .is_ok());
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"engine":args[1],"execution_path":"TtsEngine through voice eligibility","parameter_count":initial.parameters.len(),
        "buffered_frames":buffered.audio.frame_count(),"progressive_frames":progressive_frames,
        "draft":draft,"applied":explanation,"query_during_speech":"busy","native_cancellation":{"error":native_cancel_error,"restarted":native_restarted},
        "ordinary_cancellation":{"error":ordinary_cancel_error,"restarted":ordinary_restarted},
        "reconnect":"fresh_generation_old_plan_expired","ordinary_followup":"passed","stale_strict":"rejected",
        "explicit_common_only":"passed"})
        )?
    );
    Ok(())
}
