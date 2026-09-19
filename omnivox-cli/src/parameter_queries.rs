//! Bounded read-only catalogue queries, independent of the speech command loop.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use omnivox_tts::control::{
    decode_request, encode_response, ControlErrorCode, ControlRequest, ControlResponse,
    ControlResponseEnvelope, CONTROL_PROTOCOL_VERSION,
};
use omnivox_tts::engine_parameters::{
    unavailable, validate_query, CatalogueError, CatalogueQuery, CatalogueResult,
    CatalogueUnavailable,
};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::helper_protocol::parameters;
use omnivox_tts::TtsEngine;

mod cache;
mod explanations;
use cache::CatalogueCache;

const QUERY_DEADLINE: Duration = Duration::from_secs(1);
type Reporter = Arc<dyn Fn(&ControlResponseEnvelope) + Send + Sync>;

pub(crate) struct ParameterQueries {
    active: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    report: Reporter,
    deadline: Duration,
    cache: Arc<Mutex<CatalogueCache>>,
    plans: Arc<crate::native_plans::NativePlanReferences>,
}

// Admission remains occupied until BOTH the query and its deadline reporter exit.
// A timed-out or panicking adapter cannot accumulate detached work on retries.
struct QueryLease(Arc<AtomicBool>);
impl Drop for QueryLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Drop for ParameterQueries {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl ParameterQueries {
    pub(crate) fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            closed: Arc::new(AtomicBool::new(false)),
            report: Arc::new(crate::server::write_control_response),
            deadline: QUERY_DEADLINE,
            cache: Arc::new(Mutex::new(CatalogueCache::default())),
            plans: Arc::new(crate::native_plans::NativePlanReferences::default()),
        }
    }

    pub(crate) fn with_native_plans(
        mut self,
        plans: Arc<crate::native_plans::NativePlanReferences>,
    ) -> Self {
        self.plans = plans;
        self
    }

    /// Complete runtime-qualified catalogues for native admission. Contention and
    /// unavailable/replaced runtimes yield missing metadata, never a wait.
    pub(crate) fn cached_catalogues(
        &self,
        registry: &EngineRegistry,
        disabled: &[String],
    ) -> Vec<Arc<omnivox_tts::native_parameters::ParameterCatalogue>> {
        let candidates = match self.cache.try_lock() {
            Ok(cache) => cache.candidates(),
            Err(_) => return vec![],
        };
        candidates
            .into_iter()
            .filter_map(|entry| {
                if disabled.contains(&entry.catalogue.engine_id) {
                    return None;
                }
                let engine = registry.engine(&entry.catalogue.engine_id)?;
                entry.current(&engine).then_some(entry.catalogue)
            })
            .collect()
    }

    /// Return false for other operations or malformed envelopes so the existing
    /// dispatcher retains its version/error/correlation behavior.
    pub(crate) fn try_handle(
        &self,
        payload: &str,
        registry: &EngineRegistry,
        disabled: &[String],
    ) -> bool {
        let Ok(envelope) = decode_request(payload) else {
            return false;
        };
        if envelope.protocol_version != CONTROL_PROTOCOL_VERSION {
            return false;
        }
        let ControlRequest::GetEngineParametersV1(query) = envelope.request else {
            return false;
        };
        let id = envelope.request_id;
        if id == 0 {
            (self.report)(&response(
                id,
                error(
                    ControlErrorCode::InvalidConfiguration,
                    "Parameter queries need a positive request ID",
                ),
            ));
            return true;
        }
        if let Err(e) = validate_query(&query) {
            (self.report)(&response(
                id,
                error(ControlErrorCode::InvalidConfiguration, e.to_string()),
            ));
            return true;
        }
        let engine = if disabled.contains(&query.engine_id) {
            None
        } else {
            registry.engine(&query.engine_id)
        };
        match engine {
            Some(engine) => self.submit(id, query, engine),
            None => (self.report)(&response(
                id,
                catalogue(
                    &query,
                    unavailable(
                        CatalogueUnavailable::EngineUnavailable,
                        "Engine is absent, disabled or not initialized",
                    ),
                ),
            )),
        }
        true
    }

    fn submit(&self, id: u64, query: CatalogueQuery, engine: Arc<dyn TtsEngine>) {
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            (self.report)(&response(
                id,
                catalogue(&query, CatalogueResult::Busy { retry_after_ms: 50 }),
            ));
            return;
        }
        let lease = Arc::new(QueryLease(Arc::clone(&self.active)));
        let report = Arc::clone(&self.report);
        let closed = Arc::clone(&self.closed);
        let deadline = self.deadline;
        let worker_query = query.clone();
        let cache = Arc::clone(&self.cache);
        let epoch = engine.parameter_cache_epoch().filter(|epoch| *epoch > 0);
        let observed_engine = Arc::clone(&engine);
        let spawn = thread::Builder::new()
            .name("omnivox-parameter-query".into())
            .spawn(move || {
                let _lease = lease;
                let native_lease = Arc::clone(&_lease);
                let submitted = worker_query.clone();
                let (send, receive) = mpsc::sync_channel(1);
                let started = thread::Builder::new()
                    .name("omnivox-parameter-adapter".into())
                    .spawn(move || {
                        let _lease = native_lease;
                        let result = engine.engine_parameters(submitted);
                        let _ = send.send(result);
                    });
                let result = match started {
                    Ok(_) => match receive.recv_timeout(deadline) {
                        Ok(result) => checked_result(&worker_query, result),
                        Err(_) => catalogue(
                            &worker_query,
                            unavailable(
                                CatalogueUnavailable::EngineUnavailable,
                                "Parameter query did not finish within its deadline",
                            ),
                        ),
                    },
                    Err(_) => catalogue(
                        &worker_query,
                        unavailable(
                            CatalogueUnavailable::EngineUnavailable,
                            "Parameter query worker could not start",
                        ),
                    ),
                };
                let response = response(id, result);
                if !closed.load(Ordering::Acquire) {
                    // The reporter owns publication. Adapter threads cannot cache
                    // timed-out replies, and a changed runtime invalidates assembly.
                    if let Ok(mut cache) = cache.try_lock() {
                        cache.observe(&worker_query, &response.response, &observed_engine, epoch);
                    }
                    report(&response);
                }
            });
        if spawn.is_err() {
            (self.report)(&response(
                id,
                catalogue(
                    &query,
                    unavailable(
                        CatalogueUnavailable::EngineUnavailable,
                        "Parameter query worker could not start",
                    ),
                ),
            ));
        }
    }
}

fn catalogue(query: &CatalogueQuery, result: CatalogueResult) -> ControlResponse {
    ControlResponse::EngineParametersV1 {
        engine_id: query.engine_id.clone(),
        result,
    }
}
fn error(code: ControlErrorCode, message: impl Into<String>) -> ControlResponse {
    ControlResponse::Error {
        code,
        message: message.into(),
    }
}
fn response(id: u64, result: ControlResponse) -> ControlResponseEnvelope {
    let mut response = ControlResponseEnvelope {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        request_id: Some(id),
        response: result,
    };
    if encode_response(&response).is_err() {
        response.response = error(
            ControlErrorCode::PayloadTooLarge,
            "Parameter catalogue page exceeds the control response bound",
        );
    }
    response
}
fn checked_result(
    query: &CatalogueQuery,
    result: Result<CatalogueResult, CatalogueError>,
) -> ControlResponse {
    let result = match result {
        Ok(result) => result,
        Err(CatalogueError::Invalid(message)) => {
            return error(ControlErrorCode::InvalidConfiguration, message)
        }
        Err(CatalogueError::Stale(message)) => {
            return error(ControlErrorCode::StaleGeneration, message)
        }
    };
    let request = parameters::Request {
        protocol_version: parameters::PROTOCOL_VERSION,
        request_id: 1,
        body: parameters::RequestBody::GetEngineParametersV1(query.clone()),
    };
    let reply = parameters::Response {
        protocol_version: parameters::PROTOCOL_VERSION,
        request_id: 1,
        body: parameters::ResponseBody::EngineParametersV1 {
            engine_id: query.engine_id.clone(),
            result: result.clone(),
        },
    };
    if reply.validate_for(&request, &query.engine_id).is_err() {
        return catalogue(
            query,
            unavailable(
                CatalogueUnavailable::EngineUnavailable,
                "Engine returned an invalid or mismatched parameter catalogue",
            ),
        );
    }
    catalogue(query, result)
}

#[cfg(test)]
mod tests;
