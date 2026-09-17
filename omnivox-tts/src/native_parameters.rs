//! Typed native voice parameters and pure, actual-choice planning.
//!
//! These types do not advertise capabilities or extend existing wire operations.
//! Adapters supply their already mapped/clamped common values; this module owns
//! validation, native overlays, provenance and dependency ordering, not calibration.

mod planning;
mod types;
mod validation;

pub use planning::{compose, NativePlan, PlannedParameter, ValueOrigin};
pub use types::*;

use serde::de::DeserializeOwned;
use thiserror::Error;

pub const MAX_PARAMETERS: usize = 512;
pub const MAX_NATIVE_OPERATIONS: usize = 64;
pub const MAX_ENUM_CHOICES: usize = 64;
pub const MAX_IDENTIFIER_BYTES: usize = 128;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParameterError {
    #[error("invalid native parameter definition: {0}")]
    Invalid(&'static str),
    #[error("native parameter {id}: {reason}")]
    Parameter { id: String, reason: &'static str },
    #[error("native parameter engine or schema does not match the selected choice")]
    IdentityMismatch,
    #[error("native parameter evidence belongs to a different runtime or profile")]
    StaleIdentity,
    #[error("native parameter dependencies require an adapter-specific plan")]
    DependencyCycle,
}

fn decode<T: DeserializeOwned>(json: &[u8]) -> Result<T, ParameterError> {
    if json.len() > crate::control::MAX_CONTROL_PAYLOAD_BYTES {
        return Err(ParameterError::Invalid("payload exceeds control bound"));
    }
    serde_json::from_slice::<crate::control::DuplicateFreeJson>(json)
        .map_err(|_| ParameterError::Invalid("malformed or duplicate-key JSON"))?;
    serde_json::from_slice(json).map_err(|_| ParameterError::Invalid("invalid typed JSON"))
}

#[cfg(test)]
mod tests;
