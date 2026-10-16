//! Rich-tone timeline decoding and lossless execution preparation.
//!
//! Keep sound shapes separate from older action types so an older executor
//! cannot accidentally discard them. Execution returns both speech and shapes.

use crate::logical_voices::LogicalVoiceRegistry;
use crate::timeline_protocol::*;
use crate::timeline_v5::{NativeSpeechSpan, PresentationTimelineV5};
use base64::{engine::general_purpose::STANDARD, Engine};
use omnivox_core::tone::{ToneParameters, ToneShape};
use serde::{Deserialize, Serialize};

pub const PRESENTATION_TIMELINE_PROTOCOL_V6: u32 = 6;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RichTimelineAction {
    #[serde(flatten)]
    pub action: PresentationTimelineAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<ToneShape>,
}

impl<'de> Deserialize<'de> for RichTimelineAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let shape = value
            .as_object_mut()
            .and_then(|object| object.remove("shape"))
            .map(serde_json::from_value::<ToneShape>)
            .transpose()
            .map_err(serde::de::Error::custom)?;
        let action = crate::timeline_v5::strict_action(value).map_err(serde::de::Error::custom)?;
        if shape.is_some() && !matches!(action.action, PresentationAction::Tone { .. }) {
            return Err(serde::de::Error::custom(
                "shape applies only to tone actions",
            ));
        }
        Ok(Self { action, shape })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationTimelineV6 {
    pub protocol_version: u32,
    pub generation: u64,
    pub dispatch_id: u64,
    pub registry_generation: u64,
    pub delivery_policy: PresentationDeliveryPolicy,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_key"
    )]
    pub replacement_key: Option<String>,
    pub spans: Vec<NativeSpeechSpan>,
    pub actions: Vec<RichTimelineAction>,
}

fn present_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

impl PresentationTimelineV6 {
    /// Split into the existing native speech model and mandatory per-action
    /// shapes. The native model's version is internal; admission and replacement
    /// retain the original version-six document and domain.
    pub fn into_execution_parts(
        self,
    ) -> (
        PresentationTimelineV5,
        std::collections::HashMap<String, ToneShape>,
    ) {
        let native = self.validation_projection();
        let shapes = self
            .actions
            .into_iter()
            .filter_map(|action| action.shape.map(|shape| (action.action.id, shape)))
            .collect();
        (native, shapes)
    }

    // Validation only: never publish this projection for execution or replay.
    fn validation_projection(&self) -> PresentationTimelineV5 {
        PresentationTimelineV5 {
            protocol_version: 5,
            generation: self.generation,
            dispatch_id: self.dispatch_id,
            registry_generation: self.registry_generation,
            delivery_policy: self.delivery_policy,
            replacement_key: self.replacement_key.clone(),
            spans: self.spans.clone(),
            actions: self
                .actions
                .iter()
                .map(|action| action.action.clone())
                .collect(),
        }
    }

    pub fn validate(&self, aggregate: bool) -> Result<(), PresentationTimelineError> {
        if !matches!(self.protocol_version, 6 | 7) {
            return Err(PresentationTimelineError::InvalidTimeline(
                "expected timeline version 6 or 7".into(),
            ));
        }
        self.validation_projection().validate(aggregate)?;
        for action in &self.actions {
            if self.protocol_version == 6
                && action
                    .shape
                    .is_some_and(|shape| shape.requires_extended_contract())
            {
                return Err(PresentationTimelineError::InvalidTimeline(
                    "extended tone shape requires timeline version 7".into(),
                ));
            }
            match &action.action.action {
                PresentationAction::Tone {
                    frequency_hz,
                    duration_ms,
                    volume,
                    ..
                } => {
                    ToneParameters {
                        frequency_hz: *frequency_hz,
                        duration_ms: *duration_ms,
                        volume: *volume,
                        shape: action.shape.unwrap_or_default(),
                    }
                    .validate()
                    .map_err(|error| {
                        PresentationTimelineError::InvalidTimeline(format!(
                            "tone action {}: {error}",
                            action.action.id
                        ))
                    })?;
                }
                _ if action.shape.is_some() => {
                    return Err(PresentationTimelineError::InvalidTimeline(
                        "shape applies only to tone actions".into(),
                    ))
                }
                _ => (),
            }
        }
        Ok(())
    }

    pub fn validate_registry(
        &self,
        registry: &LogicalVoiceRegistry,
    ) -> Result<(), PresentationTimelineError> {
        self.validation_projection().validate_registry(registry)
    }

    pub fn tracking_identity(&self) -> Option<PresentationTimelineIdentity> {
        PresentationTimelineIdentity::new(self.generation, self.dispatch_id)
    }

    pub fn shares_replacement_domain(&self, other: &Self) -> bool {
        self.protocol_version == other.protocol_version
            && self.delivery_policy == PresentationDeliveryPolicy::Replaceable
            && other.delivery_policy == PresentationDeliveryPolicy::Replaceable
            && self.replacement_key == other.replacement_key
    }
}

fn limit(aggregate: bool) -> usize {
    if aggregate {
        MAX_TIMELINE_AGGREGATE_BYTES
    } else {
        MAX_TIMELINE_PAYLOAD_BYTES
    }
}

fn size_error(aggregate: bool) -> PresentationTimelineError {
    if aggregate {
        PresentationTimelineError::AggregatePayloadTooLarge
    } else {
        PresentationTimelineError::PayloadTooLarge
    }
}

pub fn encode_timeline_v6(
    timeline: &PresentationTimelineV6,
    aggregate: bool,
) -> Result<String, PresentationTimelineError> {
    timeline.validate(aggregate)?;
    let bytes = serde_json::to_vec(timeline).map_err(PresentationTimelineError::InvalidJson)?;
    if bytes.len() > limit(aggregate) {
        return Err(size_error(aggregate));
    }
    Ok(STANDARD.encode(bytes))
}

pub fn decode_timeline_v6(
    payload: &str,
    declared: Option<usize>,
) -> Result<PresentationTimelineV6, PresentationTimelineDecodeError> {
    let aggregate = declared.is_some();
    let decode = || {
        if declared.is_some_and(|size| size == 0 || size > limit(aggregate)) {
            return Err(size_error(aggregate));
        }
        let payload = payload.trim();
        let payload = payload
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .unwrap_or(payload);
        if payload.len() > limit(aggregate).div_ceil(3) * 4 {
            return Err(size_error(aggregate));
        }
        let bytes = STANDARD
            .decode(payload)
            .map_err(PresentationTimelineError::InvalidBase64)?;
        if bytes.len() > limit(aggregate) {
            return Err(size_error(aggregate));
        }
        if declared.is_some_and(|size| size != bytes.len()) {
            return Err(PresentationTimelineError::InvalidTimeline(
                "multipart decoded size differs from declaration".into(),
            ));
        }
        serde_json::from_slice::<crate::control::DuplicateFreeJson>(&bytes)
            .map_err(PresentationTimelineError::InvalidJson)?;
        serde_json::from_slice::<PresentationTimelineV6>(&bytes)
            .map_err(PresentationTimelineError::InvalidJson)
    };
    let timeline = decode().map_err(|error| PresentationTimelineDecodeError::new(None, error))?;
    timeline.validate(aggregate).map_err(|error| {
        PresentationTimelineDecodeError::new(timeline.tracking_identity(), error)
    })?;
    Ok(timeline)
}

#[cfg(test)]
mod tests;
