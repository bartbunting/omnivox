//! One bounded read-only explanation shares catalogue-query admission.
use super::*;
use crate::routing::choice::AttemptStyle;
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::engine_voice_choices::{NativeChoiceExecution, ParameterKnowledge};
use omnivox_tts::helper_protocol::HelperSynthesisSettings;
use omnivox_tts::native_parameters::CatalogueIdentity;
use omnivox_tts::routing_policy::RoutingPolicyRegistry;
use omnivox_tts::voice_explanation::{ExplanationResponse, ExplanationResult, ExplanationSource};
use omnivox_tts::voice_preview_v2::PRIVATE_PREVIEW_VOICE_ID;
use parameters::{ExplanationSource as HelperSource, ExplanationUnavailable as Unavailable};

struct PreparedExplanation {
    engine: Arc<dyn TtsEngine>,
    epoch: Option<u64>,
    source: HelperSource,
    choice_id: String,
    realized: PhysicalVoiceId,
    identity: Option<CatalogueIdentity>,
    public_plan: Option<String>,
}
fn explained(result: ExplanationResult) -> ControlResponse {
    ControlResponse::VoiceParametersExplainedV1(ExplanationResponse { result })
}
fn unavailable_result(reason: Unavailable, message: impl Into<String>) -> ControlResponse {
    let mut message = message.into();
    // Bound diagnostics even when a resolver mentions several saved choices.
    if message.len() > 1024 {
        let mut end = 1024;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    explained(ExplanationResult::Unavailable { reason, message })
}
impl ParameterQueries {
    pub(crate) fn explain(
        &self,
        id: u64,
        source: ExplanationSource,
        rate: f32,
        engines: &EngineRegistry,
        policy: &RoutingPolicyRegistry,
    ) {
        if id == 0 {
            (self.report)(&response(
                id,
                error(
                    ControlErrorCode::InvalidConfiguration,
                    "Explanations need a positive request ID",
                ),
            ));
            return;
        }
        if let Err(message) = source.validate(rate) {
            (self.report)(&response(
                id,
                error(ControlErrorCode::InvalidConfiguration, message),
            ));
            return;
        }
        match self.prepare_explanation(source, rate, engines, policy) {
            Ok(prepared) => self.submit_explanation(id, prepared),
            Err(result) => (self.report)(&response(id, *result)),
        }
    }
    fn prepare_explanation(
        &self,
        source: ExplanationSource,
        rate: f32,
        engines: &EngineRegistry,
        policy: &RoutingPolicyRegistry,
    ) -> Result<PreparedExplanation, Box<ControlResponse>> {
        if let ExplanationSource::Applied { plan_id } = &source {
            let reference = self.plans.lookup(plan_id, engines).ok_or_else(|| {
                unavailable_result(
                    Unavailable::PlanExpired,
                    "Applied plan is no longer retained by this connection and worker",
                )
            })?;
            if policy
                .policy()
                .disabled_engine_ids
                .contains(&reference.voice.engine_id)
            {
                return Err(Box::new(unavailable_result(
                    Unavailable::VoiceUnavailable,
                    "Engine is disabled",
                )));
            }
            let engine = reference.owner.upgrade().ok_or_else(|| {
                unavailable_result(Unavailable::PlanExpired, "Applied worker is unavailable")
            })?;
            let choice_id = reference.choice_id.ok_or_else(|| {
                unavailable_result(
                    Unavailable::PlanExpired,
                    "Applied choice identity is unavailable",
                )
            })?;
            return Ok(PreparedExplanation {
                epoch: reference.epoch,
                engine,
                source: HelperSource::Applied {
                    plan_id: reference.helper_id,
                },
                choice_id,
                realized: reference.voice,
                identity: Some(reference.identity),
                public_plan: Some(reference.public_id),
            });
        }
        let input = source.preview_inputs().expect("validated draft");
        let prepared = crate::server::prepare_voice_preview_v3(
            input,
            rate,
            engines,
            policy,
            self.cached_catalogues(engines, &policy.policy().disabled_engine_ids),
        )
        .map_err(|e| error(ControlErrorCode::InvalidConfiguration, e))?;
        let crate::server::PreviewTarget::Native {
            context, placement, ..
        } = prepared.target
        else {
            unreachable!()
        };
        let routing = prepared.routing;
        let route = routing
            .initial_native_route(PRIVATE_PREVIEW_VOICE_ID, engines)
            .map_err(|e| unavailable_result(Unavailable::VoiceUnavailable, e))?;
        let knowledge = routing
            .parameter_catalogues
            .iter()
            .map(|c| ParameterKnowledge::Ready(c.as_ref()))
            .collect::<Vec<_>>();
        let attempt = AttemptStyle::EngineLayered {
            context: &context,
            base_rate: rate,
            placement_pan: placement.pan,
            knowledge: &knowledge,
            policy: parameters::UnavailablePolicy::Require,
        }
        .prepare(&routing, &route, &route.engine.descriptor())
        .map_err(|e| unavailable_result(Unavailable::NativeUnavailable, e))?;
        let native = match attempt.native {
            NativeChoiceExecution::Parameters(p) => Some(p),
            NativeChoiceExecution::NotRequested => None,
            NativeChoiceExecution::CommonOnly { .. } => {
                unreachable!("strict planning cannot degrade")
            }
        };
        let identity = native.as_ref().map(|p| p.expected_identity.clone());
        let source = HelperSource::Draft {
            settings: HelperSynthesisSettings {
                voice_id: Some(attempt.resolution.realized.voice_id.clone()),
                rate: attempt.settings.rate,
                pitch: attempt.settings.pitch,
                volume: attempt.settings.volume,
                pitch_range: attempt.acss.style.pitch_range,
                stress: attempt.acss.style.stress,
                richness: attempt.acss.style.richness,
            },
            voice_parameters: native,
        };
        omnivox_tts::voice_explanation::validate_helper_source(&source)
            .map_err(|e| error(ControlErrorCode::InvalidConfiguration, e))?;
        Ok(PreparedExplanation {
            epoch: route.engine.parameter_cache_epoch(),
            engine: route.engine,
            source,
            choice_id: attempt.choice_id.expect("selected draft has an identity"),
            realized: attempt.resolution.realized,
            identity,
            public_plan: None,
        })
    }
    fn submit_explanation(&self, id: u64, prepared: PreparedExplanation) {
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            (self.report)(&response(
                id,
                explained(ExplanationResult::Busy { retry_after_ms: 50 }),
            ));
            return;
        }
        let lease = Arc::new(QueryLease(self.active.clone()));
        let report = self.report.clone();
        let closed = self.closed.clone();
        let deadline = self.deadline;
        let spawn = thread::Builder::new()
            .name("omnivox-explanation-query".into())
            .spawn(move || {
                let _lease = lease;
                let adapter_lease = _lease.clone();
                let engine = prepared.engine.clone();
                let source = prepared.source.clone();
                let (send, receive) = mpsc::sync_channel(1);
                let started = thread::Builder::new()
                    .name("omnivox-explanation-adapter".into())
                    .spawn(move || {
                        let _lease = adapter_lease;
                        let result = engine.explain_voice_parameters(source);
                        let _ = send.send(result);
                    });
                let result = match started {
                    Ok(_) => match receive.recv_timeout(deadline) {
                        Ok(result) => checked_explanation(&prepared, result),
                        Err(_) => unavailable_result(
                            Unavailable::NativeUnavailable,
                            "Explanation did not finish within its deadline",
                        ),
                    },
                    Err(_) => unavailable_result(
                        Unavailable::NativeUnavailable,
                        "Explanation worker could not start",
                    ),
                };
                if !closed.load(Ordering::Acquire) {
                    report(&response(id, result));
                }
            });
        if spawn.is_err() {
            (self.report)(&response(
                id,
                unavailable_result(
                    Unavailable::NativeUnavailable,
                    "Explanation worker could not start",
                ),
            ));
        }
    }
}
fn checked_explanation(
    p: &PreparedExplanation,
    result: Result<parameters::ExplanationResult, CatalogueError>,
) -> ControlResponse {
    let unavailable = || {
        unavailable_result(
            if p.public_plan.is_some() {
                Unavailable::PlanExpired
            } else {
                Unavailable::NativeUnavailable
            },
            "Engine returned invalid, stale or mismatched explanation evidence",
        )
    };
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
        body: parameters::RequestBody::ExplainVoiceParametersV1 {
            source: p.source.clone(),
        },
    };
    let reply = parameters::Response {
        protocol_version: parameters::PROTOCOL_VERSION,
        request_id: 1,
        body: parameters::ResponseBody::VoiceParametersExplainedV1 {
            result: result.clone(),
        },
    };
    if reply.validate_for(&request, &p.realized.engine_id).is_err() {
        return unavailable();
    }
    if let parameters::ExplanationResult::Ready {
        identity, realized, ..
    } = &result
    {
        if p.epoch.is_none()
            || p.epoch != p.engine.parameter_cache_epoch()
            || realized.voice_id != p.realized.voice_id
            || p.identity
                .as_ref()
                .is_some_and(|expected| expected != identity)
        {
            return unavailable();
        }
    }
    let result = ExplanationResult::from_helper(result, p.choice_id.clone(), p.public_plan.clone());
    if result.validate().is_err() {
        return unavailable();
    }
    explained(result)
}
#[cfg(test)]
mod tests;
