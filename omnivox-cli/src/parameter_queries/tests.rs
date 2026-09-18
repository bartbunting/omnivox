use super::*;
use omnivox_tts::contracts::EngineDescriptor;
use omnivox_tts::control::{encode_request, ControlRequestEnvelope};
use omnivox_tts::{SynthesisRequest, SynthesisResult, TtsError, VoiceInfo};
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::time::Instant;

fn fixture(name: &str) -> Value {
    let value: Value = serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap();
    value["messages"][name].clone()
}
fn payload(value: &Value) -> String {
    encode_request(&serde_json::from_value(value.clone()).unwrap()).unwrap()
}
fn query() -> CatalogueQuery {
    let request: ControlRequestEnvelope =
        serde_json::from_value(fixture("catalogue_request")).unwrap();
    let ControlRequest::GetEngineParametersV1(query) = request.request else {
        panic!()
    };
    query
}
fn ready() -> CatalogueResult {
    serde_json::from_value(fixture("catalogue_response")["result"].clone()).unwrap()
}
struct FakeEngine {
    calls: AtomicUsize,
    block: Mutex<Option<mpsc::Receiver<()>>>,
}
impl TtsEngine for FakeEngine {
    fn descriptor(&self) -> EngineDescriptor {
        let mut d = EngineDescriptor::unavailable("dectalk", "test");
        d.availability = omnivox_tts::contracts::Availability::Available;
        d
    }
    fn engine_parameters(&self, _query: CatalogueQuery) -> Result<CatalogueResult, CatalogueError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if let Some(wait) = self.block.lock().unwrap().take() {
            wait.recv().unwrap();
        }
        Ok(ready())
    }
    fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
        panic!("metadata must not recover")
    }
    fn synthesize(&self, _: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        panic!("metadata must not synthesize")
    }
    fn stop(&self) {
        panic!("metadata must not stop speech")
    }
    fn is_speaking(&self) -> bool {
        false
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        panic!("metadata must not enumerate voices")
    }
    fn voice_info(&self, _: &str) -> Option<VoiceInfo> {
        panic!("metadata must not load voices")
    }
}
fn service() -> (ParameterQueries, mpsc::Receiver<ControlResponseEnvelope>) {
    let (tx, rx) = mpsc::channel();
    let mut service = ParameterQueries::new();
    service.deadline = Duration::from_millis(100);
    service.report = Arc::new(move |response| {
        let _ = tx.send(response.clone());
    });
    (service, rx)
}
fn engine(block: Option<mpsc::Receiver<()>>) -> Arc<FakeEngine> {
    Arc::new(FakeEngine {
        calls: AtomicUsize::new(0),
        block: Mutex::new(block),
    })
}
fn await_idle(service: &ParameterQueries) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.active.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn catalogue_current_engine_reply_is_correlated_and_read_only() {
    let (service, rx) = service();
    let engine = engine(None);
    let mut registry = EngineRegistry::new();
    registry.register(engine.clone()).unwrap();
    assert!(service.try_handle(&payload(&fixture("catalogue_request")), &registry, &[]));
    let result = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        fixture("catalogue_response")
    );
    assert_eq!(engine.calls.load(Ordering::Acquire), 1);
}
#[test]
fn catalogue_timeout_keeps_admission_bounded_and_discards_late_reply() {
    let (service, rx) = service();
    let (release, wait) = mpsc::channel();
    let engine = engine(Some(wait));
    let started = Instant::now();
    service.submit(1, query(), engine.clone());
    assert!(started.elapsed() < Duration::from_millis(50));
    let timed = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(
        timed.response,
        ControlResponse::EngineParametersV1 {
            result: CatalogueResult::Unavailable { .. },
            ..
        }
    ));
    for id in 2..12 {
        service.submit(id, query(), engine.clone());
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap().response,
            ControlResponse::EngineParametersV1 {
                result: CatalogueResult::Busy { .. },
                ..
            }
        ));
    }
    assert_eq!(engine.calls.load(Ordering::Acquire), 1);
    release.send(()).unwrap();
    await_idle(&service);
    assert!(rx.try_recv().is_err());
    service.submit(12, query(), engine);
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap().response,
        ControlResponse::EngineParametersV1 {
            result: CatalogueResult::Ready { .. },
            ..
        }
    ));
}
#[test]
fn catalogue_disconnect_suppresses_reply_and_connections_have_separate_admission() {
    let (first, first_rx) = service();
    let (second, second_rx) = service();
    let (release, wait) = mpsc::channel();
    first.submit(1, query(), engine(Some(wait)));
    drop(first);
    second.submit(2, query(), engine(None));
    assert_eq!(
        second_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .request_id,
        Some(2)
    );
    release.send(()).unwrap();
    assert!(first_rx.recv_timeout(Duration::from_secs(1)).is_err());
}
#[test]
fn catalogue_absent_disabled_and_invalid_queries_never_call_engine() {
    let (service, rx) = service();
    let engine = engine(None);
    let mut registry = EngineRegistry::new();
    registry.register(engine.clone()).unwrap();
    assert!(service.try_handle(
        &payload(&fixture("catalogue_request")),
        &registry,
        &["dectalk".into()]
    ));
    assert!(matches!(
        rx.recv().unwrap().response,
        ControlResponse::EngineParametersV1 {
            result: CatalogueResult::Unavailable {
                reason: CatalogueUnavailable::EngineUnavailable,
                ..
            },
            ..
        }
    ));
    for (field, value) in [
        ("engine_id", json!("unknown")),
        ("cursor", json!("bad")),
        ("engine_id", json!("/bad")),
        ("request_id", json!(0)),
    ] {
        let mut request = fixture("catalogue_request");
        request[field] = value;
        assert!(service.try_handle(&payload(&request), &registry, &[]));
        rx.recv().unwrap();
    }
    assert_eq!(engine.calls.load(Ordering::Acquire), 0);
}
#[test]
fn catalogue_invalid_engine_reply_is_not_published_and_errors_keep_codes() {
    let mut result = ready();
    if let CatalogueResult::Ready { voice_id, .. } = &mut result {
        *voice_id = Some("wrong".into());
    }
    assert!(matches!(
        checked_result(&query(), Ok(result)),
        ControlResponse::EngineParametersV1 {
            result: CatalogueResult::Unavailable { .. },
            ..
        }
    ));
    assert!(matches!(
        checked_result(&query(), Err(CatalogueError::Stale("changed".into()))),
        ControlResponse::Error {
            code: ControlErrorCode::StaleGeneration,
            ..
        }
    ));
    assert!(matches!(
        checked_result(&query(), Err(CatalogueError::Invalid("bad".into()))),
        ControlResponse::Error {
            code: ControlErrorCode::InvalidConfiguration,
            ..
        }
    ));
}

#[test]
fn catalogue_pending_recovery_returns_busy_without_recovering() {
    use crate::engine_execution::{IsolatedTtsEngine, IsolationBudget};
    use std::sync::atomic::AtomicU64;
    let engine = engine(None);
    let isolated = IsolatedTtsEngine::new(
        engine.clone(),
        Arc::new(AtomicU64::new(0)),
        Arc::new(IsolationBudget::new()),
    );
    assert!(matches!(
        isolated.engine_parameters(query()).unwrap(),
        CatalogueResult::Ready { .. }
    ));
    isolated.prepare_recovery_probe().unwrap();
    assert!(matches!(
        isolated.engine_parameters(query()).unwrap(),
        CatalogueResult::Busy { .. }
    ));
    assert_eq!(engine.calls.load(Ordering::Acquire), 1);
}
