//! Generation-safe session routing policy independent of logical voices.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::{Availability, EngineDescriptor, FallbackPolicy};
use crate::engine_configuration::LocalRoutingPolicy;
use crate::logical_voices::MAX_ENGINE_ID_BYTES;

/// Global engine order and administrative disablement for one server session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingPolicy {
    #[serde(default)]
    pub preferred_engine_ids: Vec<String>,
    #[serde(default)]
    pub fallback_engine_ids: Vec<String>,
    #[serde(default)]
    pub disabled_engine_ids: Vec<String>,
}

/// Result of an atomic routing-policy replacement or idempotent retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingPolicyRegistration {
    pub routing_policy_generation: u64,
    pub policy: RoutingPolicy,
}

/// Validation and generation errors that leave the policy untouched.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RoutingPolicyError {
    #[error("routing policy generation {received} is older than current generation {current}")]
    StaleGeneration { current: u64, received: u64 },

    #[error("routing policy generation {generation} was reused with different content")]
    GenerationConflict { generation: u64 },

    #[error("{field} contains an invalid engine ID")]
    InvalidEngineId { field: &'static str },

    #[error("{field} contains duplicate engine {engine_id}")]
    DuplicateEngine {
        field: &'static str,
        engine_id: String,
    },
}

/// Reader-thread-owned routing policy and client generation.
#[derive(Debug, Clone)]
pub struct RoutingPolicyRegistry {
    generation: u64,
    configured: bool,
    policy: RoutingPolicy,
    local_disabled: Vec<String>,
    local_fallback: Option<Vec<String>>,
    session_policy: Option<RoutingPolicy>,
}

impl RoutingPolicyRegistry {
    /// Start with the process-selected engine as the global preference.
    pub fn new(startup_preferred_engine_id: impl Into<String>) -> Self {
        Self::with_local_policy(startup_preferred_engine_id, &LocalRoutingPolicy::default())
    }

    /// Seed ordinary startup without making local preferences an irreversible
    /// session policy. Only local disablement remains a floor after replacement.
    pub fn with_local_policy(
        startup_preferred_engine_id: impl Into<String>,
        local: &LocalRoutingPolicy,
    ) -> Self {
        Self {
            generation: 0,
            configured: false,
            policy: RoutingPolicy {
                preferred_engine_ids: local
                    .preferred_engine_ids
                    .clone()
                    .unwrap_or_else(|| vec![startup_preferred_engine_id.into()]),
                fallback_engine_ids: local.fallback_engine_ids.clone().unwrap_or_default(),
                disabled_engine_ids: local.disabled_engine_ids.clone(),
            },
            local_disabled: local.disabled_engine_ids.clone(),
            local_fallback: local.fallback_engine_ids.clone(),
            session_policy: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn policy(&self) -> &RoutingPolicy {
        &self.policy
    }

    /// Atomically replace the complete policy.
    pub fn register(
        &mut self,
        generation: u64,
        policy: RoutingPolicy,
    ) -> Result<RoutingPolicyRegistration, RoutingPolicyError> {
        if generation < self.generation {
            return Err(RoutingPolicyError::StaleGeneration {
                current: self.generation,
                received: generation,
            });
        }
        validate_policy(&policy)?;
        if generation == self.generation && self.configured {
            if self.session_policy.as_ref() != Some(&policy) {
                return Err(RoutingPolicyError::GenerationConflict { generation });
            }
        } else {
            self.generation = generation;
            self.configured = true;
            self.session_policy = Some(policy.clone());
            self.policy = policy;
            for engine in &self.local_disabled {
                if !self.policy.disabled_engine_ids.contains(engine) {
                    self.policy.disabled_engine_ids.push(engine.clone());
                }
            }
        }
        Ok(self.registration())
    }

    pub fn registration(&self) -> RoutingPolicyRegistration {
        RoutingPolicyRegistration {
            routing_policy_generation: self.generation,
            policy: self.policy.clone(),
        }
    }

    /// Combine independent engine order with registration-owned voice policy.
    pub fn effective_fallback_policy(&self, base: &FallbackPolicy) -> FallbackPolicy {
        FallbackPolicy {
            preferred_engines: self.policy.preferred_engine_ids.clone(),
            fallback_engines: if self.configured {
                self.policy.fallback_engine_ids.clone()
            } else {
                self.local_fallback
                    .clone()
                    .unwrap_or_else(|| base.fallback_engines.clone())
            },
            ..base.clone()
        }
    }

    /// Mark configured disabled engines unavailable without losing their order.
    pub fn project_inventory(&self, mut inventory: Vec<EngineDescriptor>) -> Vec<EngineDescriptor> {
        for descriptor in &mut inventory {
            if self.policy.disabled_engine_ids.contains(&descriptor.id) {
                descriptor.availability = Availability::Unavailable {
                    reason: "disabled by runtime routing policy".to_owned(),
                };
            }
        }
        inventory
    }

    /// Combine base/health inventory generation with policy generation.
    pub fn inventory_generation(&self, base_generation: u64) -> u64 {
        base_generation.saturating_add(self.generation)
    }
}

fn validate_policy(policy: &RoutingPolicy) -> Result<(), RoutingPolicyError> {
    for (field, values) in [
        ("preferred_engine_ids", &policy.preferred_engine_ids),
        ("fallback_engine_ids", &policy.fallback_engine_ids),
        ("disabled_engine_ids", &policy.disabled_engine_ids),
    ] {
        let mut seen = HashSet::with_capacity(values.len());
        for engine_id in values {
            if engine_id.is_empty()
                || engine_id.len() > MAX_ENGINE_ID_BYTES
                || engine_id.chars().any(char::is_whitespace)
            {
                return Err(RoutingPolicyError::InvalidEngineId { field });
            }
            if !seen.insert(engine_id) {
                return Err(RoutingPolicyError::DuplicateEngine {
                    field,
                    engine_id: engine_id.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(preferred: &[&str], fallback: &[&str], disabled: &[&str]) -> RoutingPolicy {
        RoutingPolicy {
            preferred_engine_ids: preferred.iter().map(|value| (*value).to_owned()).collect(),
            fallback_engine_ids: fallback.iter().map(|value| (*value).to_owned()).collect(),
            disabled_engine_ids: disabled.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn policy_registration_is_generation_safe_and_idempotent() {
        let mut registry = RoutingPolicyRegistry::new("winrt");
        let configured = policy(&["eloquence", "dectalk"], &["espeak"], &[]);

        assert!(registry.register(3, configured.clone()).is_ok());
        assert!(registry.register(3, configured).is_ok());
        assert!(matches!(
            registry.register(2, policy(&["winrt"], &[], &[])),
            Err(RoutingPolicyError::StaleGeneration { .. })
        ));
        assert!(matches!(
            registry.register(3, policy(&["winrt"], &[], &[])),
            Err(RoutingPolicyError::GenerationConflict { .. })
        ));
    }

    #[test]
    fn disabled_engines_keep_their_configured_positions() {
        let mut registry = RoutingPolicyRegistry::new("winrt");
        registry
            .register(
                1,
                policy(
                    &["eloquence", "dectalk", "winrt"],
                    &["espeak"],
                    &["dectalk"],
                ),
            )
            .unwrap();

        assert_eq!(
            registry.policy().preferred_engine_ids,
            ["eloquence", "dectalk", "winrt"]
        );
        assert_eq!(registry.policy().disabled_engine_ids, ["dectalk"]);
    }

    #[test]
    fn local_policy_preserves_omission_and_empty_until_session_replacement() {
        let base = FallbackPolicy {
            fallback_engines: vec!["espeak".into()],
            ..FallbackPolicy::default()
        };
        let local = LocalRoutingPolicy {
            preferred_engine_ids: Some(vec![]),
            fallback_engine_ids: Some(vec![]),
            ..LocalRoutingPolicy::default()
        };
        let mut registry = RoutingPolicyRegistry::with_local_policy("espeak", &local);
        assert!(registry.policy().preferred_engine_ids.is_empty());
        assert!(registry
            .effective_fallback_policy(&base)
            .fallback_engines
            .is_empty());
        let defaults =
            RoutingPolicyRegistry::with_local_policy("espeak", &LocalRoutingPolicy::default());
        assert_eq!(defaults.policy().preferred_engine_ids, ["espeak"]);
        assert_eq!(
            defaults.effective_fallback_policy(&base).fallback_engines,
            ["espeak"]
        );
        registry
            .register(1, policy(&["flite"], &["espeak"], &[]))
            .unwrap();
        assert_eq!(
            registry.effective_fallback_policy(&base).preferred_engines,
            ["flite"]
        );
        assert_eq!(
            registry.effective_fallback_policy(&base).fallback_engines,
            ["espeak"]
        );
    }

    #[test]
    fn session_policy_cannot_clear_the_local_floor_and_retries_compare_original_input() {
        let local = LocalRoutingPolicy {
            disabled_engine_ids: vec!["local-disabled".into()],
            ..LocalRoutingPolicy::default()
        };
        let mut registry = RoutingPolicyRegistry::with_local_policy("espeak", &local);
        let first = policy(&[], &[], &["session-disabled"]);
        let accepted = registry.register(2, first.clone()).unwrap();
        assert_eq!(
            accepted.policy.disabled_engine_ids,
            ["session-disabled", "local-disabled"]
        );
        assert_eq!(registry.register(2, first).unwrap(), accepted);
        assert!(matches!(
            registry.register(2, policy(&[], &[], &["session-disabled", "local-disabled"])),
            Err(RoutingPolicyError::GenerationConflict { .. })
        ));
        registry.register(3, policy(&[], &[], &[])).unwrap();
        assert_eq!(registry.policy().disabled_engine_ids, ["local-disabled"]);
        assert!(registry.register(2, policy(&[], &[], &[])).is_err());
        let invalid = policy(&[], &[], &["a", "a"]);
        assert!(registry.register(4, invalid).is_err());
        assert_eq!(registry.generation(), 3);
        assert_eq!(registry.policy().disabled_engine_ids, ["local-disabled"]);
    }
}
