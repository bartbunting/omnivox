//! Atomic native registration over the existing generation domain.
use super::*;
use crate::engine_voice_choices::{
    EngineRegisteredVoiceDefinition, NativeCatalogueSnapshot, NativeChoiceError,
    NativeChoiceStatus, VoiceRegistrationV3,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeVoiceRegistration {
    pub registry_generation: u64,
    pub definition_count: usize,
    pub unresolved_logical_voice_ids: Vec<String>,
    pub native_status: Vec<NativeChoiceStatus>,
}

impl LogicalVoiceRegistry {
    /// Pure admission against a connection owner's immutable metadata snapshot.
    /// The public control handler must still bound its complete reply envelope
    /// before publication. Runtime identities are not part of saved definitions.
    pub fn register_v3(
        &mut self,
        request: VoiceRegistrationV3,
        snapshot: &NativeCatalogueSnapshot<'_>,
    ) -> Result<NativeVoiceRegistration, NativeChoiceError> {
        if request.registry_generation != 0 && request.registry_generation < self.generation {
            return Err(
                ChoiceTuningError::from(LogicalVoiceRegistryError::StaleGeneration {
                    current: self.generation,
                    received: request.registry_generation,
                })
                .into(),
            );
        }
        request.validate()?;
        let mut native_status = Vec::new();
        for definition in &request.definitions {
            if let EngineRegisteredVoiceDefinition::EngineLayered(native) = definition {
                native_status.extend(native.statuses(snapshot)?);
            }
        }
        let mut candidate = self.clone();
        let registration = candidate.replace_definitions(
            request.registry_generation,
            request.definitions.into_iter().map(Into::into).collect(),
            request.fallback_policy.into(),
            snapshot.inventory(),
        )?;
        let result = NativeVoiceRegistration {
            registry_generation: candidate.generation,
            definition_count: candidate.registered_definitions.len(),
            unresolved_logical_voice_ids: registration
                .bindings
                .iter()
                .filter(|b| matches!(b, LogicalVoiceBinding::Unresolved { .. }))
                .map(|b| b.logical_voice_id().to_owned())
                .collect(),
            native_status,
        };
        if serde_json::to_vec(&result)
            .map_err(|_| ChoiceTuningError::Invalid("invalid native acknowledgement"))?
            .len()
            > crate::control::MAX_CONTROL_PAYLOAD_BYTES
        {
            return Err(
                ChoiceTuningError::Invalid("native acknowledgement exceeds control bound").into(),
            );
        }
        *self = candidate;
        Ok(result)
    }
}
