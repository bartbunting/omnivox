use super::*;
use omnivox_tts::contracts::{Availability, EngineDescriptor, VoiceDescriptor};
use omnivox_tts::control::ControlRequest;
use omnivox_tts::{SynthesisRequest, SynthesisResult, TtsError, VoiceInfo};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::time::Instant;
fn fixture(name: &str) -> Value {
    serde_json::from_str::<Value>(include_str!(
        "../../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap()["messages"][name]
        .clone()
}
struct Fake {
    epoch: AtomicU64,
    calls: AtomicUsize,
    sources: Mutex<Vec<HelperSource>>,
    wait: Mutex<Option<mpsc::Receiver<()>>>,
    answer: Mutex<parameters::ExplanationResult>,
}
impl TtsEngine for Fake {
    fn descriptor(&self) -> EngineDescriptor {
        let mut d = EngineDescriptor::unavailable("dectalk", "test");
        d.availability = Availability::Available;
        d.health = omnivox_tts::contracts::EngineHealth::Healthy;
        d.capabilities.acss = omnivox_tts::contracts::AcssCapabilities {
            richness: true,
            ..Default::default()
        };
        d.voices.push(VoiceDescriptor::from_voice_info(
            "dectalk",
            VoiceInfo {
                identifier: "paul".into(),
                name: "Paul".into(),
                language: "en-US".into(),
                quality: omnivox_tts::VoiceQuality::Compact,
            },
        ));
        d.default_voice_id = Some("paul".into());
        d
    }
    fn parameter_cache_epoch(&self) -> Option<u64> {
        Some(self.epoch.load(Ordering::Acquire))
    }
    fn engine_parameters(&self, _: CatalogueQuery) -> Result<CatalogueResult, CatalogueError> {
        Ok(serde_json::from_value(fixture("catalogue_response")["result"].clone()).unwrap())
    }
    fn explain_voice_parameters(
        &self,
        source: HelperSource,
    ) -> Result<parameters::ExplanationResult, CatalogueError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.sources.lock().unwrap().push(source);
        if let Some(wait) = self.wait.lock().unwrap().take() {
            wait.recv().unwrap();
        }
        Ok(self.answer.lock().unwrap().clone())
    }
    fn synthesize(&self, _: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        panic!("explanation must not synthesize")
    }
    fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
        panic!("explanation must not recover")
    }
    fn stop(&self) {
        panic!("explanation must not stop speech")
    }
    fn is_speaking(&self) -> bool {
        false
    }
    fn voice_info(&self, _: &str) -> Option<VoiceInfo> {
        panic!("explanation must not load voices")
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        panic!("explanation must not load voices")
    }
}
fn setup() -> (
    ParameterQueries,
    mpsc::Receiver<ControlResponseEnvelope>,
    Arc<Fake>,
    EngineRegistry,
) {
    let mut value = fixture("explain_response")["result"].clone();
    value.as_object_mut().unwrap().remove("choice_id");
    let engine = Arc::new(Fake {
        epoch: AtomicU64::new(1),
        calls: AtomicUsize::new(0),
        sources: Mutex::new(vec![]),
        wait: Mutex::new(None),
        answer: Mutex::new(serde_json::from_value(value).unwrap()),
    });
    let mut registry = EngineRegistry::new();
    registry.register(engine.clone()).unwrap();
    let (tx, rx) = mpsc::channel();
    let mut service = ParameterQueries::new();
    service.deadline = Duration::from_millis(80);
    service.report = Arc::new(move |r| {
        let _ = tx.send(r.clone());
    });
    (service, rx, engine, registry)
}
fn draft() -> ExplanationSource {
    let r: omnivox_tts::control::ControlRequestEnvelope =
        serde_json::from_value(fixture("explain_request")).unwrap();
    let ControlRequest::ExplainVoiceParametersV1(r) = r.request else {
        panic!()
    };
    r.source
}
fn idle(service: &ParameterQueries) {
    let end = Instant::now() + Duration::from_secs(2);
    while service.active.load(Ordering::Acquire) {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}
fn receive(rx: &mpsc::Receiver<ControlResponseEnvelope>) -> ControlResponse {
    rx.recv_timeout(Duration::from_secs(2)).unwrap().response
}
fn cache(
    service: &ParameterQueries,
    rx: &mpsc::Receiver<ControlResponseEnvelope>,
    engines: &EngineRegistry,
) {
    let query: omnivox_tts::control::ControlRequestEnvelope =
        serde_json::from_value(fixture("catalogue_request")).unwrap();
    assert!(service.try_handle(
        &omnivox_tts::control::encode_request(&query).unwrap(),
        engines,
        &[]
    ));
    assert!(matches!(
        receive(rx),
        ControlResponse::EngineParametersV1 { .. }
    ));
    idle(service);
}
fn ask(service: &ParameterQueries, source: ExplanationSource, engines: &EngineRegistry) {
    service.explain(
        804,
        source,
        0.65,
        engines,
        &RoutingPolicyRegistry::new("dectalk"),
    );
}
fn expect_unavailable(result: ControlResponse, expected: Unavailable) {
    let ControlResponse::VoiceParametersExplainedV1(ExplanationResponse {
        result: ExplanationResult::Unavailable { reason, .. },
    }) = result
    else {
        panic!("{result:?}")
    };
    assert_eq!(reason, expected);
}
fn publish(service: &ParameterQueries, engine: &Arc<Fake>) -> String {
    let owned: Arc<dyn TtsEngine> = engine.clone();
    let identity = match engine.answer.lock().unwrap().clone() {
        parameters::ExplanationResult::Ready { identity, .. } => identity,
        _ => panic!(),
    };
    service
        .plans
        .publish(
            &Some((Arc::downgrade(&owned), 1)),
            &PhysicalVoiceId::new("dectalk", "paul"),
            &parameters::NativeApplication {
                status: parameters::ApplicationStatus::Applied,
                plan_id: Some("helper-7".into()),
                identity: Some(identity),
                masked_parameters: vec![],
                reason: None,
            },
            Some("paul-soft"),
        )
        .plan_id
        .unwrap()
}
#[test]
fn drafts_are_exact_read_only_and_preserve_context_and_native_values() {
    let (service, rx, engine, registry) = setup();
    ask(&service, draft(), &registry);
    expect_unavailable(receive(&rx), Unavailable::NativeUnavailable);
    assert_eq!(engine.calls.load(Ordering::Acquire), 0);
    cache(&service, &rx, &registry);
    let mut source = draft();
    if let ExplanationSource::Draft {
        selection, context, ..
    } = &mut source
    {
        *selection = omnivox_tts::voice_preview_v2::VoicePreviewSelection::Choice {
            choice_id: "paul-soft".into(),
        };
        *context = serde_json::from_value(serde_json::json!({"richness":{"op":"set","value":0.5}}))
            .unwrap();
    }
    ask(&service, source, &registry);
    let ControlResponse::VoiceParametersExplainedV1(ExplanationResponse {
        result:
            ExplanationResult::Ready {
                choice_id,
                evidence,
                plan_id,
                ..
            },
    }) = receive(&rx)
    else {
        panic!()
    };
    assert_eq!(choice_id, "paul-soft");
    assert_eq!(evidence, parameters::Evidence::Planned);
    assert!(plan_id.is_none());
    let sources = engine.sources.lock().unwrap();
    let HelperSource::Draft {
        settings,
        voice_parameters,
    } = &sources[0]
    else {
        panic!()
    };
    assert_eq!(settings.voice_id.as_deref(), Some("paul"));
    assert_eq!(settings.richness, Some(0.5));
    let p = voice_parameters.as_ref().unwrap();
    assert_eq!(
        serde_json::to_value(&p.native.parameters["sm"]).unwrap(),
        serde_json::json!({"op":"set","value":80})
    );
    assert_eq!(
        p.context_dimensions,
        vec![omnivox_tts::native_parameters::CommonInput::Richness]
    );
    assert_eq!(p.unavailable_policy, parameters::UnavailablePolicy::Require);
}
#[test]
fn applied_lookup_keeps_frozen_choice_and_public_identity_and_expires() {
    let (service, rx, engine, registry) = setup();
    let id = publish(&service, &engine);
    if let parameters::ExplanationResult::Ready {
        evidence,
        plan_id,
        parameters,
        ..
    } = &mut *engine.answer.lock().unwrap()
    {
        *evidence = parameters::Evidence::AdapterApplied;
        *plan_id = Some("helper-7".into());
        parameters[0].read_back = true;
    }
    ask(
        &service,
        ExplanationSource::Applied {
            plan_id: id.clone(),
        },
        &registry,
    );
    let ControlResponse::VoiceParametersExplainedV1(ExplanationResponse {
        result:
            ExplanationResult::Ready {
                choice_id,
                plan_id,
                evidence,
                ..
            },
    }) = receive(&rx)
    else {
        panic!()
    };
    assert_eq!(choice_id, "paul-soft");
    assert_eq!(plan_id.as_deref(), Some(id.as_str()));
    assert_eq!(evidence, parameters::Evidence::AdapterApplied);
    idle(&service);
    let (other, other_rx, _, _) = setup();
    ask(
        &other,
        ExplanationSource::Applied {
            plan_id: id.clone(),
        },
        &registry,
    );
    expect_unavailable(receive(&other_rx), Unavailable::PlanExpired);
    for _ in 0..65 {
        publish(&service, &engine);
    }
    ask(
        &service,
        ExplanationSource::Applied { plan_id: id },
        &registry,
    );
    expect_unavailable(receive(&rx), Unavailable::PlanExpired);
    let id = publish(&service, &engine);
    engine.epoch.store(2, Ordering::Release);
    ask(
        &service,
        ExplanationSource::Applied { plan_id: id },
        &registry,
    );
    expect_unavailable(receive(&rx), Unavailable::PlanExpired);
    assert_eq!(engine.calls.load(Ordering::Acquire), 1);
}
#[test]
fn explanation_timeout_keeps_shared_query_admission_until_adapter_retires() {
    let (service, rx, engine, registry) = setup();
    cache(&service, &rx, &registry);
    let (release, wait) = mpsc::channel();
    *engine.wait.lock().unwrap() = Some(wait);
    ask(&service, draft(), &registry);
    expect_unavailable(receive(&rx), Unavailable::NativeUnavailable);
    ask(&service, draft(), &registry);
    assert!(matches!(
        receive(&rx),
        ControlResponse::VoiceParametersExplainedV1(ExplanationResponse {
            result: ExplanationResult::Busy { .. }
        })
    ));
    let q: omnivox_tts::control::ControlRequestEnvelope =
        serde_json::from_value(fixture("catalogue_request")).unwrap();
    service.try_handle(
        &omnivox_tts::control::encode_request(&q).unwrap(),
        &registry,
        &[],
    );
    assert!(matches!(
        receive(&rx),
        ControlResponse::EngineParametersV1 {
            result: CatalogueResult::Busy { .. },
            ..
        }
    ));
    release.send(()).unwrap();
    idle(&service);
    assert!(rx.try_recv().is_err());
    ask(&service, draft(), &registry);
    assert!(matches!(
        receive(&rx),
        ControlResponse::VoiceParametersExplainedV1(ExplanationResponse {
            result: ExplanationResult::Ready { .. }
        })
    ));
}
#[test]
fn late_runtime_change_and_mismatched_evidence_never_become_ready() {
    let (service, rx, engine, registry) = setup();
    cache(&service, &rx, &registry);
    let prepared = service
        .prepare_explanation(
            draft(),
            0.65,
            &registry,
            &RoutingPolicyRegistry::new("dectalk"),
        )
        .unwrap();
    let good = engine.answer.lock().unwrap().clone();
    for kind in 0..3 {
        let mut bad = good.clone();
        if let parameters::ExplanationResult::Ready {
            realized,
            identity,
            parameters,
            ..
        } = &mut bad
        {
            match kind {
                0 => realized.voice_id = "wrong".into(),
                1 => identity.runtime_generation += 1,
                _ => parameters[0].read_back = true,
            }
        }
        expect_unavailable(
            checked_explanation(&prepared, Ok(bad)),
            Unavailable::NativeUnavailable,
        );
    }
    engine.epoch.store(2, Ordering::Release);
    expect_unavailable(
        checked_explanation(&prepared, Ok(good)),
        Unavailable::NativeUnavailable,
    );
}
#[test]
fn invalid_or_disabled_drafts_do_not_contact_adapters() {
    let (service, rx, engine, registry) = setup();
    let mut source = draft();
    if let ExplanationSource::Draft {
        expected_base_rate, ..
    } = &mut source
    {
        *expected_base_rate = Some(0.7);
    }
    ask(&service, source, &registry);
    assert!(matches!(
        receive(&rx),
        ControlResponse::Error {
            code: ControlErrorCode::InvalidConfiguration,
            ..
        }
    ));
    let mut source = draft();
    if let ExplanationSource::Draft {
        disabled_engine_ids,
        ..
    } = &mut source
    {
        disabled_engine_ids.push("dectalk".into());
    }
    ask(&service, source, &registry);
    expect_unavailable(receive(&rx), Unavailable::VoiceUnavailable);
    service.explain(
        0,
        draft(),
        0.65,
        &registry,
        &RoutingPolicyRegistry::new("dectalk"),
    );
    assert!(matches!(receive(&rx), ControlResponse::Error { .. }));
    assert_eq!(engine.calls.load(Ordering::Acquire), 0);
}

#[test]
fn common_only_draft_can_be_explained_without_a_native_block() {
    let (service, rx, engine, registry) = setup();
    let mut source = draft();
    if let ExplanationSource::Draft { voice, .. } = &mut source {
        for choice in &mut voice.choices {
            choice.native = None;
        }
    }
    ask(&service, source, &registry);
    assert!(matches!(
        receive(&rx),
        ControlResponse::VoiceParametersExplainedV1(ExplanationResponse {
            result: ExplanationResult::Ready {
                evidence: parameters::Evidence::Planned,
                ..
            }
        })
    ));
    assert!(matches!(
        &engine.sources.lock().unwrap()[0],
        HelperSource::Draft {
            voice_parameters: None,
            ..
        }
    ));
}

#[test]
fn closing_connection_suppresses_a_late_explanation_reply() {
    let (service, rx, engine, registry) = setup();
    cache(&service, &rx, &registry);
    let (release, wait) = mpsc::channel();
    *engine.wait.lock().unwrap() = Some(wait);
    ask(&service, draft(), &registry);
    let active = service.active.clone();
    drop(service);
    release.send(()).unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while active.load(Ordering::Acquire) {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn applied_queries_reject_changed_voice_plan_or_runtime_evidence() {
    let (service, _, engine, registry) = setup();
    let id = publish(&service, &engine);
    let prepared = service
        .prepare_explanation(
            ExplanationSource::Applied { plan_id: id },
            0.65,
            &registry,
            &RoutingPolicyRegistry::new("dectalk"),
        )
        .unwrap();
    let mut good = engine.answer.lock().unwrap().clone();
    if let parameters::ExplanationResult::Ready {
        evidence, plan_id, ..
    } = &mut good
    {
        *evidence = parameters::Evidence::AdapterApplied;
        *plan_id = Some("helper-7".into());
    }
    for kind in 0..4 {
        let mut bad = good.clone();
        if let parameters::ExplanationResult::Ready {
            evidence,
            plan_id,
            realized,
            identity,
            ..
        } = &mut bad
        {
            match kind {
                0 => *evidence = parameters::Evidence::Planned,
                1 => *plan_id = Some("other-helper-plan".into()),
                2 => realized.voice_id = "betty".into(),
                _ => identity.runtime_generation += 1,
            }
        }
        expect_unavailable(
            checked_explanation(&prepared, Ok(bad)),
            Unavailable::PlanExpired,
        );
    }
}

#[test]
fn explanation_crosses_isolation_wrapper_without_recovery_or_synthesis() {
    use crate::engine_execution::{IsolatedTtsEngine, IsolationBudget};
    let (service, rx, engine, _) = setup();
    let isolated = Arc::new(IsolatedTtsEngine::new(
        engine.clone(),
        Arc::new(AtomicU64::new(0)),
        Arc::new(IsolationBudget::new()),
    ));
    let mut registry = EngineRegistry::new();
    registry.register(isolated.clone()).unwrap();
    cache(&service, &rx, &registry);
    ask(&service, draft(), &registry);
    assert!(matches!(
        receive(&rx),
        ControlResponse::VoiceParametersExplainedV1(ExplanationResponse {
            result: ExplanationResult::Ready { .. }
        })
    ));
    idle(&service);
    let source = engine.sources.lock().unwrap()[0].clone();
    isolated.prepare_recovery_probe().unwrap();
    assert!(matches!(
        isolated.explain_voice_parameters(source).unwrap(),
        parameters::ExplanationResult::Busy { .. }
    ));
    assert_eq!(engine.calls.load(Ordering::Acquire), 1);
}
