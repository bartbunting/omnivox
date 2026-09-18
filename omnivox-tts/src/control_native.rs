//! Atomic public native registration using already observed metadata.
use super::*;
use crate::engine_voice_choices::{
    NativeCatalogueSnapshot, NativeChoiceError, ParameterKnowledge, VoiceRegistrationV3,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn register(
    request_id: u64,
    inventory_generation: u64,
    request: VoiceRegistrationV3,
    engines: &[EngineDescriptor],
    knowledge: &[ParameterKnowledge<'_>],
    logical_voices: &mut LogicalVoiceRegistry,
    routing_policy: &RoutingPolicyRegistry,
) -> ControlResponseEnvelope {
    if request_id == 0 {
        return error_response(
            Some(request_id),
            ControlErrorCode::InvalidConfiguration,
            "Native registration needs a positive request ID".into(),
        );
    }
    let projected = routing_policy.project_inventory(engines.to_vec());
    let mut candidate = logical_voices.clone();
    let result = NativeCatalogueSnapshot::new(&projected, knowledge)
        .and_then(|metadata| candidate.register_v3(request, &metadata));
    match result {
        Ok(registration) => {
            let effective = routing_policy.effective_fallback_policy(candidate.fallback_policy());
            let resolved = candidate.resolve_and_store_with_policy(&projected, &effective);
            let response = ControlResponseEnvelope {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                request_id: Some(request_id),
                response: ControlResponse::LogicalVoicesRegisteredV3 {
                    registry_generation: registration.registry_generation,
                    inventory_generation: routing_policy.inventory_generation(inventory_generation),
                    definition_count: registration.definition_count,
                    unresolved_logical_voice_ids: resolved
                        .bindings
                        .iter()
                        .filter(|binding| matches!(binding, LogicalVoiceBinding::Unresolved { .. }))
                        .map(|binding| binding.logical_voice_id().to_owned())
                        .collect(),
                    native_status: registration.native_status,
                },
            };
            // Acknowledgement size includes the complete public envelope. Nothing
            // becomes visible until clients can receive that exact acknowledgement.
            match encode_response(&response) {
                Ok(_) => {
                    *logical_voices = candidate;
                    response
                }
                Err(error) => error_response(Some(request_id), error.code(), error.to_string()),
            }
        }
        Err(error) => {
            let code = match &error {
                NativeChoiceError::Common(ChoiceTuningError::Registry(error)) => {
                    registry_error_code(error)
                }
                _ => ControlErrorCode::InvalidConfiguration,
            };
            error_response(Some(request_id), code, bounded_message(error.to_string()))
        }
    }
}

#[cfg(test)]
#[path = "control_native/tests.rs"]
mod tests;
