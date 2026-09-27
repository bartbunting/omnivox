//! Public metadata projected from an actual retained launch record. No argv,
//! helper executable paths, managed generation contents or environment values.
use serde::{Deserialize, Serialize};

use super::{EngineOrigin, LaunchSnapshot};
use crate::contracts::{Availability, EngineDescriptor};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationStatus {
    pub engine_id: String,
    pub origin: EngineOrigin,
    #[serde(deserialize_with = "required_nullable")]
    pub source: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub override_source: Option<String>,
    pub enabled: bool,
    pub availability: Availability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineConfigurationStatus {
    pub activation_id: String,
    pub inventory_generation: u64,
    #[serde(deserialize_with = "required_nullable")]
    pub configuration_root: Option<String>,
    pub registrations: Vec<RegistrationStatus>,
}

impl EngineConfigurationStatus {
    pub(crate) fn from_snapshot(snapshot: &LaunchSnapshot) -> Self {
        let resolved = snapshot.resolved();
        let source = |path: &std::path::Path| {
            resolved
                .root
                .as_ref()
                .and_then(|root| path.strip_prefix(root).ok())
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        };
        Self {
            activation_id: snapshot.activation_id().into(),
            inventory_generation: 0,
            configuration_root: resolved
                .root
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            registrations: resolved
                .registrations()
                .map(|entry| RegistrationStatus {
                    engine_id: entry.engine_id.clone(),
                    origin: entry.origin,
                    source: entry.source.as_deref().map(source),
                    override_source: entry.override_source.as_deref().map(source),
                    enabled: entry.enabled,
                    availability: bounded_availability(Availability::Unavailable {
                        reason: entry
                            .unavailable
                            .clone()
                            .unwrap_or_else(|| "not registered for this startup".into()),
                    }),
                })
                .collect(),
        }
    }

    pub(crate) fn with_inventory(&self, generation: u64, engines: &[EngineDescriptor]) -> Self {
        let mut status = self.clone();
        status.inventory_generation = generation;
        for entry in &mut status.registrations {
            if let Some(engine) = engines.iter().find(|engine| engine.id == entry.engine_id) {
                entry.availability = bounded_availability(engine.availability.clone());
            }
        }
        status
    }
}

// Optional runtime diagnostics must not crowd the actual activation identity
// out of a bounded status reply. Keep at most 512 bytes, including the ellipsis.
fn bounded_availability(availability: Availability) -> Availability {
    match availability {
        Availability::Unavailable { mut reason } if reason.len() > 512 => {
            let mut end = 509;
            while !reason.is_char_boundary(end) {
                end -= 1;
            }
            reason.truncate(end);
            reason.push('…');
            Availability::Unavailable { reason }
        }
        availability => availability,
    }
}

fn required_nullable<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::deserialize(deserializer)
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
