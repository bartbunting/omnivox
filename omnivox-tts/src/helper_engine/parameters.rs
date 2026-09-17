//! Parent-owned native request correlation; metadata never waits behind speech.
use super::*;
use crate::native_parameters::CatalogueIdentity;
use parameters::{
    ApplicationStatus, CatalogueQuery, CatalogueResult, CatalogueUnavailable, ExplanationResult,
    ExplanationSource, ExplanationUnavailable, NativeApplication, RequestBody, ResponseBody,
    UnavailablePolicy, VoiceParameters,
};

const QUERY_TIMEOUT: Duration = Duration::from_millis(200);

// Cover both a blocked stdin write and the response wait. Join each watchdog
// before releasing lifecycle ownership, so fast queries cannot accumulate timers.
fn query_with_deadline<T>(
    connection: &Arc<dyn HelperConnection>,
    timeout: Duration,
    query: impl FnOnce(Instant) -> Result<T, HelperEngineError>,
) -> Result<T, HelperEngineError> {
    const ACTIVE: u8 = 0;
    const COMPLETED: u8 = 1;
    const EXPIRED: u8 = 2;
    let deadline = Instant::now() + timeout;
    let state = Arc::new(AtomicU8::new(ACTIVE));
    let (finished, waiting) = mpsc::sync_channel(1);
    let watched = Arc::clone(connection);
    let watched_state = Arc::clone(&state);
    let watchdog = thread::Builder::new()
        .name("omnivox-helper-query-deadline".into())
        .spawn(move || {
            if matches!(
                waiting.recv_timeout(deadline.saturating_duration_since(Instant::now())),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) && watched_state
                .compare_exchange(ACTIVE, EXPIRED, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // terminate kills before taking the writer lock; a full pipe wakes.
                if let Err(error) = watched.terminate() {
                    warn!(%error, "Parameter query deadline could not confirm helper cleanup");
                }
            }
        })
        .map_err(|e| {
            HelperEngineError::Transport(format!("could not start query deadline: {e}"))
        })?;
    let result = query(deadline);
    if Instant::now() >= deadline {
        if state
            .compare_exchange(ACTIVE, EXPIRED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = connection.terminate();
        }
    } else {
        let _ = state.compare_exchange(ACTIVE, COMPLETED, Ordering::AcqRel, Ordering::Acquire);
    }
    let _ = finished.send(());
    watchdog
        .join()
        .map_err(|_| HelperEngineError::Transport("query deadline worker panicked".into()))?;
    if state.load(Ordering::Acquire) == EXPIRED {
        return Err(HelperEngineError::Timeout("parameter query"));
    }
    result
}

pub(super) struct AppliedPlan {
    id: String,
    identity: CatalogueIdentity,
    voice_id: String,
}

pub(super) struct ParameterExchange {
    protocol_version: u16,
    common: HelperRequest,
    native: Option<parameters::Request>,
    degraded: Option<NativeApplication>,
    started: bool,
}

impl ParameterExchange {
    pub(super) fn new(
        version: u16,
        common: &HelperRequest,
        parameters: Option<&VoiceParameters>,
    ) -> Result<Self, HelperEngineError> {
        common.validate()?;
        if let Some(p) = parameters {
            p.validate()?;
        }
        let mut exchange = Self {
            protocol_version: version,
            common: common.clone(),
            native: None,
            degraded: None,
            started: false,
        };
        if version == parameters::PROTOCOL_VERSION {
            let HelperRequestBody::Synthesize {
                text,
                settings,
                anchors,
            } = &common.body
            else {
                return Err(HelperEngineError::UnexpectedResponse(
                    "expected synthesis request",
                ));
            };
            let request = parameters::Request {
                protocol_version: version,
                request_id: common.request_id,
                body: RequestBody::Synthesize {
                    text: text.clone(),
                    settings: settings.clone(),
                    anchors: anchors.clone().unwrap_or_default(),
                    voice_parameters: parameters.cloned(),
                },
            };
            request.validate()?;
            exchange.native = Some(request);
        } else if let Some(p) = parameters {
            if p.unavailable_policy == UnavailablePolicy::Require {
                return Err(HelperEngineError::Remote {
                    code: HelperErrorCode::InvalidParameter,
                    message: "Native parameters require helper protocol 6".into(),
                    retryable: false,
                });
            }
            exchange.degraded = Some(NativeApplication {
                status: ApplicationStatus::CommonOnly,
                plan_id: None,
                identity: None,
                masked_parameters: vec![],
                reason: Some("This helper does not support native parameters".into()),
            });
        }
        Ok(exchange)
    }

    pub(super) fn send(
        &self,
        connection: &Arc<dyn HelperConnection>,
    ) -> Result<(), HelperEngineError> {
        match &self.native {
            Some(r) => connection.send_parameters(r),
            None => connection.send(&self.common),
        }
    }

    fn accept(
        &mut self,
        message: session::Response,
        engine: &HelperTtsEngine,
        application: &mut dyn FnMut(&NativeApplication),
    ) -> Result<HelperResponse, HelperEngineError> {
        let mut response = match message {
            session::Response::Parameters(r) => {
                let request = self
                    .native
                    .as_ref()
                    .ok_or(HelperEngineError::UnexpectedResponse(
                        "unexpected native response",
                    ))?;
                r.validate_for(request, &engine.config.engine_id)?;
                let ResponseBody::SynthesisStarted {
                    format,
                    actual_voice_id,
                    native_application,
                } = r.body
                else {
                    return Err(HelperEngineError::UnexpectedResponse(
                        "expected synthesis start",
                    ));
                };
                if self.started {
                    return Err(HelperEngineError::UnexpectedResponse(
                        "duplicate synthesis start",
                    ));
                }
                self.started = true;
                if let Some(a) = native_application {
                    engine.retain_application(&a, &actual_voice_id);
                    application(&a);
                }
                HelperResponse::for_request_version(
                    HELPER_PROTOCOL_V5,
                    r.request_id,
                    HelperResponseBody::SynthesisStarted {
                        format,
                        actual_voice_id,
                    },
                )
            }
            session::Response::Common(r) => {
                if let HelperResponseBody::SynthesisStarted {
                    actual_voice_id, ..
                } = &r.body
                {
                    if self.started {
                        return Err(HelperEngineError::UnexpectedResponse(
                            "duplicate synthesis start",
                        ));
                    }
                    // Match the voice before publishing even a degradation receipt.
                    if let HelperRequestBody::Synthesize { settings, .. } = &self.common.body {
                        if settings
                            .voice_id
                            .as_ref()
                            .is_some_and(|v| v != actual_voice_id)
                        {
                            return Err(HelperEngineError::UnexpectedResponse(
                                "unexpected realized voice",
                            ));
                        }
                    }
                    self.started = true;
                    if let Some(a) = &self.degraded {
                        application(a);
                    }
                }
                r
            }
        };
        // Only shared v5 PCM semantics reach the existing collectors. Version,
        // owner and required native receipt have already been checked.
        response.protocol_version = response.protocol_version.min(HELPER_PROTOCOL_V5);
        Ok(response)
    }
}

impl HelperTtsEngine {
    /// Synthesize with a native block and return its acknowledged application.
    /// The receipt describes adapter state, not audio consumed by playback.
    pub fn synthesize_with_parameters(
        &self,
        request: &SynthesisRequest,
        parameters: &VoiceParameters,
    ) -> Result<(SynthesisResult, NativeApplication), TtsError> {
        parameters
            .validate()
            .map_err(|e| Self::map_error(e.into()))?;
        let mut application = None;
        let result = self.synthesize_inner(request, Some(parameters), &mut |a| {
            application = Some(a.clone())
        })?;
        let application = application.ok_or_else(|| {
            TtsError::SynthesisFailed("Missing native application receipt".into())
        })?;
        Ok((result, application))
    }

    /// Stream a native request. The callback precedes start/PCM and is tentative:
    /// callers must retain existing PCM commitment and playback-start boundaries.
    pub fn synthesize_stream_with_parameters(
        &self,
        request: &SynthesisRequest,
        parameters: &VoiceParameters,
        sink: &mut dyn SynthesisStreamSink,
        application: &mut dyn FnMut(&NativeApplication),
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        parameters
            .validate()
            .map_err(|e| Self::map_error(e.into()))?;
        self.synthesize_stream_inner(request, sink, Some(parameters), application)
    }

    pub(super) fn receive_synthesis_response(
        &self,
        connection: &Arc<dyn HelperConnection>,
        exchange: &mut ParameterExchange,
        application: &mut dyn FnMut(&NativeApplication),
    ) -> Result<HelperResponse, HelperEngineError> {
        // Unrelated cancel acknowledgements cannot extend the current frame deadline.
        let deadline = Instant::now() + self.config.synthesis_idle_timeout;
        loop {
            let response =
                connection.receive_message(deadline.saturating_duration_since(Instant::now()))?;
            response.validate()?;
            if response.version() != exchange.protocol_version {
                return Err(HelperEngineError::UnexpectedResponse(
                    "response uses a different negotiated protocol version",
                ));
            }
            if response.request_id() != Some(exchange.common.request_id) {
                if let session::Response::Common(r) = &response {
                    if self.consume_cancel_response(r)? {
                        continue;
                    }
                }
                return Err(HelperEngineError::RequestMismatch {
                    expected: exchange.common.request_id,
                    received: response.request_id(),
                });
            }
            return exchange.accept(response, self, application);
        }
    }

    fn retain_application(&self, application: &NativeApplication, voice_id: &str) {
        if let (ApplicationStatus::Applied, Some(id), Some(identity)) = (
            application.status,
            &application.plan_id,
            &application.identity,
        ) {
            let mut plans = self.applied_plans.lock().unwrap();
            plans.retain(|plan| plan.id != *id);
            if plans.len() == 64 {
                plans.pop_front();
            }
            plans.push_back(AppliedPlan {
                id: id.clone(),
                identity: identity.clone(),
                voice_id: voice_id.into(),
            });
        }
    }

    /// Query a single catalogue page on an existing connection. No native load,
    /// reconnect or wait for active synthesis is performed by this method.
    pub fn query_parameters(
        &self,
        query: CatalogueQuery,
    ) -> Result<CatalogueResult, HelperEngineError> {
        query.validate()?;
        if query.engine_id != self.config.engine_id {
            return Err(
                crate::helper_protocol::HelperProtocolError::InvalidField("engine_id").into(),
            );
        }
        let _lifecycle = match self.lifecycle.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                return Ok(CatalogueResult::Busy { retry_after_ms: 50 })
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(HelperEngineError::Transport(
                    "helper lifecycle poisoned".into(),
                ))
            }
        };
        let Ok(connection) = self.current_connection() else {
            return Ok(CatalogueResult::Unavailable {
                reason: CatalogueUnavailable::EngineUnavailable,
                message: "Helper connection is not ready".into(),
            });
        };
        if self.protocol_version.load(Ordering::Acquire) != u64::from(parameters::PROTOCOL_VERSION)
        {
            return Ok(CatalogueResult::Unavailable {
                reason: CatalogueUnavailable::UnsupportedHelper,
                message: "Helper does not support native parameter queries".into(),
            });
        }
        let response =
            self.parameter_query(&connection, RequestBody::GetEngineParametersV1(query))?;
        let ResponseBody::EngineParametersV1 { result, .. } = response else {
            unreachable!("correlated query response")
        };
        Ok(result)
    }

    /// Explain a resolved draft or a retained application without synthesizing.
    pub fn explain_parameters(
        &self,
        source: ExplanationSource,
    ) -> Result<ExplanationResult, HelperEngineError> {
        // Validate before Busy/Unavailable so neither can hide malformed inputs.
        let body = RequestBody::ExplainVoiceParametersV1 {
            source: source.clone(),
        };
        parameters::Request {
            protocol_version: parameters::PROTOCOL_VERSION,
            request_id: 1,
            body: body.clone(),
        }
        .validate()?;
        let _lifecycle = match self.lifecycle.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                return Ok(ExplanationResult::Busy { retry_after_ms: 50 })
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(HelperEngineError::Transport(
                    "helper lifecycle poisoned".into(),
                ))
            }
        };
        let expected_plan = if let ExplanationSource::Applied { plan_id } = &source {
            let plans = self.applied_plans.lock().unwrap();
            let Some(plan) = plans.iter().find(|p| p.id == *plan_id) else {
                return Ok(expired_plan());
            };
            Some((plan.identity.clone(), plan.voice_id.clone()))
        } else {
            None
        };
        let Ok(connection) = self.current_connection() else {
            return Ok(explanation_unavailable(&source));
        };
        if self.protocol_version.load(Ordering::Acquire) != u64::from(parameters::PROTOCOL_VERSION)
        {
            return Ok(explanation_unavailable(&source));
        }
        let response = self.parameter_query(&connection, body)?;
        let ResponseBody::VoiceParametersExplainedV1 { result } = response else {
            unreachable!("correlated query response")
        };
        if let (
            Some((expected_identity, expected_voice)),
            ExplanationResult::Ready {
                identity, realized, ..
            },
        ) = (expected_plan, &result)
        {
            if *identity != expected_identity || realized.voice_id != expected_voice {
                self.invalidate_connection(&connection);
                return Err(HelperEngineError::UnexpectedResponse(
                    "applied explanation changed runtime or voice",
                ));
            }
        }
        Ok(result)
    }

    fn parameter_query(
        &self,
        connection: &Arc<dyn HelperConnection>,
        body: RequestBody,
    ) -> Result<ResponseBody, HelperEngineError> {
        let request = parameters::Request {
            protocol_version: parameters::PROTOCOL_VERSION,
            request_id: self.allocate_request_id(),
            body,
        };
        request.validate()?;
        let result = query_with_deadline(
            connection,
            self.config.request_timeout.min(QUERY_TIMEOUT),
            |deadline| {
                connection.send_parameters(&request)?;
                loop {
                    let response = connection
                        .receive_message(deadline.saturating_duration_since(Instant::now()))?;
                    response.validate()?;
                    if response.version() != request.protocol_version {
                        return Err(HelperEngineError::UnexpectedResponse(
                            "query response changed protocol version",
                        ));
                    }
                    if response.request_id() != Some(request.request_id) {
                        if let session::Response::Common(r) = &response {
                            if self.consume_cancel_response(r)? {
                                continue;
                            }
                        }
                        return Err(HelperEngineError::RequestMismatch {
                            expected: request.request_id,
                            received: response.request_id(),
                        });
                    }
                    match response {
                        session::Response::Parameters(r) => {
                            r.validate_for(&request, &self.config.engine_id)?;
                            return Ok(r.body);
                        }
                        session::Response::Common(HelperResponse {
                            body:
                                HelperResponseBody::Error {
                                    code,
                                    message,
                                    retryable,
                                },
                            ..
                        }) => {
                            return Err(HelperEngineError::Remote {
                                code,
                                message,
                                retryable,
                            });
                        }
                        _ => {
                            return Err(HelperEngineError::UnexpectedResponse(
                                "expected parameter query response",
                            ))
                        }
                    }
                }
            },
        );
        if result
            .as_ref()
            .is_err_and(|e| !matches!(e, HelperEngineError::Remote { .. }))
        {
            // A late reply cannot be consumed as the next speech request's result.
            self.invalidate_connection(connection);
        }
        result
    }
}

fn expired_plan() -> ExplanationResult {
    ExplanationResult::Unavailable {
        reason: ExplanationUnavailable::PlanExpired,
        message: "Applied plan is no longer retained by this worker".into(),
    }
}
fn explanation_unavailable(source: &ExplanationSource) -> ExplanationResult {
    if matches!(source, ExplanationSource::Applied { .. }) {
        return expired_plan();
    }
    ExplanationResult::Unavailable {
        reason: ExplanationUnavailable::NativeUnavailable,
        message: "Helper native parameters are unavailable".into(),
    }
}
