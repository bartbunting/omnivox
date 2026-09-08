//! Session registry for Emacsvox-owned logical voice definitions.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::{EngineDescriptor, FallbackPolicy, LogicalVoiceDefinition, VoiceSelector};
use crate::resolver::{resolve_voice, VoiceResolution, VoiceResolutionError};
use crate::voice_choices::{ChoiceTuningError, RegisteredVoiceDefinition};

pub const MAX_LOGICAL_VOICES: usize = 256;
pub const MAX_LOGICAL_VOICE_ID_BYTES: usize = 128;
pub const MAX_VOICE_PREFERENCES: usize = 32;
pub const MAX_ENGINE_ID_BYTES: usize = 128;
pub const MAX_PHYSICAL_VOICE_ID_BYTES: usize = 4096;
pub const MAX_LANGUAGE_TAG_BYTES: usize = 64;

/// Resolution state returned for each registered logical voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LogicalVoiceBinding {
    Resolved { resolution: VoiceResolution },
    Unresolved { error: VoiceResolutionError },
}

impl LogicalVoiceBinding {
    pub fn logical_voice_id(&self) -> &str {
        match self {
            Self::Resolved { resolution } => &resolution.logical_voice_id,
            Self::Unresolved { error } => &error.logical_voice_id,
        }
    }
}

/// Result of an atomic registry replacement or idempotent retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalVoiceRegistration {
    pub registry_generation: u64,
    pub bindings: Vec<LogicalVoiceBinding>,
}

/// Validation and generation errors that leave the registry untouched.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LogicalVoiceRegistryError {
    #[error("logical voice generation {received} is older than current generation {current}")]
    StaleGeneration { current: u64, received: u64 },

    #[error("logical voice generation {generation} was reused with different content")]
    GenerationConflict { generation: u64 },

    #[error("too many logical voices: {count}; limit is {limit}")]
    TooManyDefinitions { count: usize, limit: usize },

    #[error("invalid logical voice ID: {id}")]
    InvalidLogicalVoiceId { id: String },

    #[error("duplicate logical voice ID: {id}")]
    DuplicateLogicalVoiceId { id: String },

    #[error("logical voice {id} has {count} preferences; limit is {limit}")]
    TooManyPreferences {
        id: String,
        count: usize,
        limit: usize,
    },

    #[error("logical voice {id} contains an invalid engine ID")]
    InvalidEngineId { id: String },

    #[error("logical voice {id} contains an invalid physical voice ID")]
    InvalidPhysicalVoiceId { id: String },

    #[error("logical voice {id} contains an invalid language tag")]
    InvalidLanguage { id: String },

    #[error("fallback policy contains an invalid engine ID")]
    InvalidFallbackEngineId,
}

/// Emacsvox session state. Definitions remain client-owned; Omnivox stores the
/// current generation so resolution can use live engine availability.
#[derive(Debug, Clone, Default)]
pub struct LogicalVoiceRegistry {
    generation: u64,
    definitions: Vec<LogicalVoiceDefinition>,
    registered_definitions: Vec<RegisteredVoiceDefinition>,
    fallback_policy: FallbackPolicy,
    bindings: Vec<LogicalVoiceBinding>,
}

impl LogicalVoiceRegistry {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn definitions(&self) -> &[LogicalVoiceDefinition] {
        &self.definitions
    }

    /// Authoritative definitions, including choice IDs and individual patches.
    /// `definitions` is only the compatibility projection used for resolution.
    pub fn registered_definitions(&self) -> &[RegisteredVoiceDefinition] {
        &self.registered_definitions
    }

    pub fn fallback_policy(&self) -> &FallbackPolicy {
        &self.fallback_policy
    }

    /// Return the resolution snapshot produced by the latest registration.
    pub fn bindings(&self) -> &[LogicalVoiceBinding] {
        &self.bindings
    }

    /// Atomically replace all definitions and resolve them against INVENTORY.
    pub fn register(
        &mut self,
        generation: u64,
        mut definitions: Vec<LogicalVoiceDefinition>,
        fallback_policy: FallbackPolicy,
        inventory: &[EngineDescriptor],
    ) -> Result<LogicalVoiceRegistration, LogicalVoiceRegistryError> {
        if generation < self.generation {
            return Err(LogicalVoiceRegistryError::StaleGeneration {
                current: self.generation,
                received: generation,
            });
        }

        validate_registration(&definitions, &fallback_policy)?;
        for definition in &mut definitions {
            definition.acss = definition.acss.clone().clamped();
        }
        let registered = definitions
            .iter()
            .cloned()
            .map(RegisteredVoiceDefinition::Legacy)
            .collect::<Vec<_>>();

        if generation == self.generation {
            if registered != self.registered_definitions || fallback_policy != self.fallback_policy
            {
                return Err(LogicalVoiceRegistryError::GenerationConflict { generation });
            }
        } else {
            self.generation = generation;
            self.definitions = definitions;
            self.registered_definitions = registered;
            self.fallback_policy = fallback_policy;
        }

        let registration = self.resolve_all(inventory);
        self.bindings = registration.bindings.clone();
        Ok(registration)
    }

    /// Validate and replace mixed legacy/layered definitions in the same
    /// generation domain as `register`. Failed validation leaves all state intact.
    pub fn register_v2(
        &mut self,
        generation: u64,
        mut definitions: Vec<RegisteredVoiceDefinition>,
        fallback_policy: FallbackPolicy,
        inventory: &[EngineDescriptor],
    ) -> Result<LogicalVoiceRegistration, ChoiceTuningError> {
        if generation == 0 {
            return Err(ChoiceTuningError::Invalid(
                "registry generation must be positive",
            ));
        }
        if generation < self.generation {
            return Err(LogicalVoiceRegistryError::StaleGeneration {
                current: self.generation,
                received: generation,
            }
            .into());
        }
        if definitions.len() > MAX_LOGICAL_VOICES {
            return Err(LogicalVoiceRegistryError::TooManyDefinitions {
                count: definitions.len(),
                limit: MAX_LOGICAL_VOICES,
            }
            .into());
        }
        let mut projection = Vec::with_capacity(definitions.len());
        for definition in &mut definitions {
            match definition {
                RegisteredVoiceDefinition::Legacy(legacy) => {
                    legacy.acss = legacy.acss.clone().clamped();
                    projection.push(legacy.clone());
                }
                RegisteredVoiceDefinition::Layered(layered) => {
                    layered.validate()?;
                    projection.push(layered.legacy_projection());
                }
            }
        }
        validate_registration(&projection, &fallback_policy)?;
        if generation == self.generation {
            if definitions != self.registered_definitions || fallback_policy != self.fallback_policy
            {
                return Err(LogicalVoiceRegistryError::GenerationConflict { generation }.into());
            }
        } else {
            self.generation = generation;
            self.definitions = projection;
            self.registered_definitions = definitions;
            self.fallback_policy = fallback_policy;
        }
        let registration = self.resolve_all(inventory);
        self.bindings = registration.bindings.clone();
        Ok(registration)
    }

    /// Re-resolve the current definitions after inventory or health changes.
    pub fn resolve_all(&self, inventory: &[EngineDescriptor]) -> LogicalVoiceRegistration {
        self.resolve_with_policy(inventory, &self.fallback_policy)
    }

    /// Re-resolve and retain bindings under an independent runtime policy.
    pub fn resolve_and_store_with_policy(
        &mut self,
        inventory: &[EngineDescriptor],
        fallback_policy: &FallbackPolicy,
    ) -> LogicalVoiceRegistration {
        let registration = self.resolve_with_policy(inventory, fallback_policy);
        self.bindings = registration.bindings.clone();
        registration
    }

    fn resolve_with_policy(
        &self,
        inventory: &[EngineDescriptor],
        fallback_policy: &FallbackPolicy,
    ) -> LogicalVoiceRegistration {
        let bindings = self
            .definitions
            .iter()
            .map(
                |definition| match resolve_voice(inventory, definition, fallback_policy) {
                    Ok(resolution) => LogicalVoiceBinding::Resolved { resolution },
                    Err(error) => LogicalVoiceBinding::Unresolved { error },
                },
            )
            .collect();

        LogicalVoiceRegistration {
            registry_generation: self.generation,
            bindings,
        }
    }
}

pub(crate) fn validate_registration(
    definitions: &[LogicalVoiceDefinition],
    fallback_policy: &FallbackPolicy,
) -> Result<(), LogicalVoiceRegistryError> {
    if definitions.len() > MAX_LOGICAL_VOICES {
        return Err(LogicalVoiceRegistryError::TooManyDefinitions {
            count: definitions.len(),
            limit: MAX_LOGICAL_VOICES,
        });
    }

    let mut ids = HashSet::with_capacity(definitions.len());
    for definition in definitions {
        if !valid_logical_voice_id(&definition.id) {
            return Err(LogicalVoiceRegistryError::InvalidLogicalVoiceId {
                id: definition.id.clone(),
            });
        }
        if !ids.insert(definition.id.as_str()) {
            return Err(LogicalVoiceRegistryError::DuplicateLogicalVoiceId {
                id: definition.id.clone(),
            });
        }
        if definition.preferences.len() > MAX_VOICE_PREFERENCES {
            return Err(LogicalVoiceRegistryError::TooManyPreferences {
                id: definition.id.clone(),
                count: definition.preferences.len(),
                limit: MAX_VOICE_PREFERENCES,
            });
        }
        if definition
            .language
            .as_deref()
            .is_some_and(|language| !valid_language(language))
        {
            return Err(LogicalVoiceRegistryError::InvalidLanguage {
                id: definition.id.clone(),
            });
        }
        for selector in &definition.preferences {
            validate_selector(&definition.id, selector)?;
        }
    }

    if fallback_policy
        .preferred_engines
        .iter()
        .chain(fallback_policy.fallback_engines.iter())
        .any(|engine_id| !valid_engine_id(engine_id))
    {
        return Err(LogicalVoiceRegistryError::InvalidFallbackEngineId);
    }
    if let Some(selector) = &fallback_policy.global_default {
        validate_selector("<global-default>", selector)?;
    }

    Ok(())
}

fn validate_selector(
    logical_voice_id: &str,
    selector: &VoiceSelector,
) -> Result<(), LogicalVoiceRegistryError> {
    if selector
        .engine_id()
        .is_some_and(|engine_id| !valid_engine_id(engine_id))
    {
        return Err(LogicalVoiceRegistryError::InvalidEngineId {
            id: logical_voice_id.to_owned(),
        });
    }

    match selector {
        VoiceSelector::Exact(id) if !valid_physical_voice_id(&id.voice_id) => {
            Err(LogicalVoiceRegistryError::InvalidPhysicalVoiceId {
                id: logical_voice_id.to_owned(),
            })
        }
        VoiceSelector::Properties { language, .. }
            if language
                .as_deref()
                .is_some_and(|language| !valid_language(language)) =>
        {
            Err(LogicalVoiceRegistryError::InvalidLanguage {
                id: logical_voice_id.to_owned(),
            })
        }
        _ => Ok(()),
    }
}

fn valid_logical_voice_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_LOGICAL_VOICE_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_engine_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ENGINE_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_physical_voice_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_PHYSICAL_VOICE_ID_BYTES && !id.chars().any(char::is_control)
}

fn valid_language(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= MAX_LANGUAGE_TAG_BYTES
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{
        AcssCapabilities, AudioOutputMode, Availability, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, EngineHealth, MarkerCapabilities, NormalizedAcss, PhysicalVoiceId,
        VoiceDescriptor,
    };
    use crate::VoiceQuality;

    fn inventory() -> Vec<EngineDescriptor> {
        vec![EngineDescriptor {
            id: "winrt".to_owned(),
            display_name: "WinRT".to_owned(),
            version: None,
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: EngineCapabilities {
                acss: AcssCapabilities::default(),
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::PlaybackOnly,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities::default(),
                language_switching: true,
                text_repertoire: crate::contracts::TextRepertoire::Unicode,
                post_synthesis_dimensions: Vec::new(),
                native_extensions: Vec::new(),
            },
            voices: vec![VoiceDescriptor {
                id: PhysicalVoiceId::new("winrt", "winrt:David"),
                display_name: "David".to_owned(),
                language: Some("en-US".to_owned()),
                gender: None,
                quality: VoiceQuality::Enhanced,
                availability: Availability::Available,
            }],
            default_voice_id: Some("winrt:David".to_owned()),
        }]
    }

    fn definition(id: &str, voice_id: &str) -> LogicalVoiceDefinition {
        LogicalVoiceDefinition {
            id: id.to_owned(),
            language: Some("en-US".to_owned()),
            preferences: vec![VoiceSelector::Exact(PhysicalVoiceId::new(
                "winrt", voice_id,
            ))],
            acss: NormalizedAcss::default(),
            effects: Default::default(),
        }
    }

    #[test]
    fn registration_resolves_and_commits_atomically() {
        let mut registry = LogicalVoiceRegistry::default();

        let result = registry
            .register(
                1,
                vec![definition("source-code", "winrt:David")],
                FallbackPolicy::default(),
                &inventory(),
            )
            .unwrap();

        assert_eq!(registry.generation(), 1);
        assert_eq!(result.bindings.len(), 1);
        assert_eq!(registry.bindings(), result.bindings);
        assert!(matches!(
            result.bindings[0],
            LogicalVoiceBinding::Resolved { .. }
        ));
    }

    #[test]
    fn unresolved_voice_is_retained_with_diagnostics() {
        let mut registry = LogicalVoiceRegistry::default();

        let result = registry
            .register(
                1,
                vec![definition("annotation", "winrt:Missing")],
                FallbackPolicy::default(),
                &inventory(),
            )
            .unwrap();

        assert_eq!(registry.definitions().len(), 1);
        assert!(matches!(
            result.bindings[0],
            LogicalVoiceBinding::Unresolved { .. }
        ));
    }

    #[test]
    fn identical_generation_retry_is_idempotent() {
        let definitions = vec![definition("source-code", "winrt:David")];
        let mut registry = LogicalVoiceRegistry::default();
        registry
            .register(
                4,
                definitions.clone(),
                FallbackPolicy::default(),
                &inventory(),
            )
            .unwrap();

        let retried = registry
            .register(4, definitions, FallbackPolicy::default(), &inventory())
            .unwrap();

        assert_eq!(retried.registry_generation, 4);
    }

    #[test]
    fn stale_and_conflicting_generations_preserve_registry() {
        let original = definition("source-code", "winrt:David");
        let mut registry = LogicalVoiceRegistry::default();
        registry
            .register(
                5,
                vec![original.clone()],
                FallbackPolicy::default(),
                &inventory(),
            )
            .unwrap();

        assert!(matches!(
            registry.register(
                4,
                vec![definition("annotation", "winrt:David")],
                FallbackPolicy::default(),
                &inventory(),
            ),
            Err(LogicalVoiceRegistryError::StaleGeneration { .. })
        ));
        assert!(matches!(
            registry.register(
                5,
                vec![definition("annotation", "winrt:David")],
                FallbackPolicy::default(),
                &inventory(),
            ),
            Err(LogicalVoiceRegistryError::GenerationConflict { .. })
        ));
        assert_eq!(registry.definitions(), &[original]);
    }

    #[test]
    fn invalid_replacement_does_not_mutate_registry() {
        let original = definition("source-code", "winrt:David");
        let mut registry = LogicalVoiceRegistry::default();
        registry
            .register(
                1,
                vec![original.clone()],
                FallbackPolicy::default(),
                &inventory(),
            )
            .unwrap();

        let error = registry
            .register(
                2,
                vec![definition("invalid voice", "winrt:David")],
                FallbackPolicy::default(),
                &inventory(),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            LogicalVoiceRegistryError::InvalidLogicalVoiceId { .. }
        ));
        assert_eq!(registry.generation(), 1);
        assert_eq!(registry.definitions(), &[original]);
    }

    fn layered() -> RegisteredVoiceDefinition {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/voice-choice-tuning.json"
        ))
        .unwrap();
        serde_json::from_value(fixture["messages"]["registration"]["definitions"][0].clone())
            .unwrap()
    }

    #[test]
    fn mixed_registry_has_one_generation_domain_and_retains_record_identity() {
        let mut registry = LogicalVoiceRegistry::default();
        let definitions = vec![
            layered(),
            RegisteredVoiceDefinition::Legacy(definition("plain", "winrt:David")),
        ];
        let policy = FallbackPolicy::default();
        registry
            .register_v2(8, definitions.clone(), policy.clone(), &inventory())
            .unwrap();
        let frozen = registry.clone();
        assert_eq!(registry.registered_definitions(), definitions);
        registry
            .register_v2(8, definitions.clone(), policy.clone(), &[])
            .unwrap();
        assert!(registry
            .bindings()
            .iter()
            .all(|b| matches!(b, LogicalVoiceBinding::Unresolved { .. })));
        assert!(matches!(
            registry.register_v2(7, definitions.clone(), policy.clone(), &[]),
            Err(ChoiceTuningError::Registry(
                LogicalVoiceRegistryError::StaleGeneration { .. }
            ))
        ));
        assert!(matches!(
            registry.register(8, registry.definitions().to_vec(), policy.clone(), &[]),
            Err(LogicalVoiceRegistryError::GenerationConflict { .. })
        ));
        for mutation in 0..3 {
            let mut changed = definitions.clone();
            let RegisteredVoiceDefinition::Layered(voice) = &mut changed[0] else {
                unreachable!()
            };
            match mutation {
                0 => voice.choices[0].id = "new-row".into(),
                1 => voice.choices[0].adjustments = Default::default(),
                _ => voice.choices.swap(0, 1),
            }
            assert!(matches!(
                registry.register_v2(8, changed, policy.clone(), &[]),
                Err(ChoiceTuningError::Registry(
                    LogicalVoiceRegistryError::GenerationConflict { .. }
                ))
            ));
            assert_eq!(registry.registered_definitions(), definitions);
        }
        let projection = registry.definitions().to_vec();
        registry
            .register(9, projection.clone(), policy.clone(), &inventory())
            .unwrap();
        assert!(registry
            .registered_definitions()
            .iter()
            .all(|d| matches!(d, RegisteredVoiceDefinition::Legacy(_))));
        // A snapshot remains independent after replacement of the live registry.
        assert_eq!(frozen.generation(), 8);
        assert_eq!(frozen.registered_definitions(), definitions);
        registry
            .register_v2(
                9,
                projection
                    .into_iter()
                    .map(RegisteredVoiceDefinition::Legacy)
                    .collect(),
                policy,
                &[],
            )
            .unwrap();
    }

    #[test]
    fn v2_rejects_zero_invalid_patches_and_duplicate_definitions_atomically() {
        let mut registry = LogicalVoiceRegistry::default();
        let original = vec![layered()];
        registry
            .register_v2(1, original.clone(), FallbackPolicy::default(), &[])
            .unwrap();
        let mut invalid_patch = original.clone();
        let RegisteredVoiceDefinition::Layered(voice) = &mut invalid_patch[0] else {
            unreachable!()
        };
        voice.choices[0].adjustments.gain =
            Some(crate::voice_choices::Adjustment::Set { value: f32::NAN });
        for definitions in [
            invalid_patch,
            vec![layered(), layered()],
            vec![layered(); MAX_LOGICAL_VOICES + 1],
        ] {
            assert!(registry
                .register_v2(2, definitions, FallbackPolicy::default(), &[])
                .is_err());
            assert_eq!(registry.generation(), 1);
            assert_eq!(registry.registered_definitions(), original);
        }
        assert!(registry
            .register_v2(0, vec![], FallbackPolicy::default(), &[])
            .is_err());
        assert_eq!(registry.registered_definitions(), original);
    }
}
