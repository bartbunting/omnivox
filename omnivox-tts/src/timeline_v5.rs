//! Version-five native spans with the existing bounded timeline transport.
use crate::logical_voices::LogicalVoiceRegistry;
use crate::timeline_protocol::*;
use crate::timeline_v4::{LayeredSpeechSpan, MixedSpeechSpan, PresentationTimelineV4};
use crate::voice_choices::RegisteredVoiceDefinition;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

pub const PRESENTATION_TIMELINE_PROTOCOL_V5: u32 = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "span",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NativeSpeechSpan {
    Legacy(#[serde(deserialize_with = "legacy_span")] PresentationSpeechSpan),
    Layered(LayeredSpeechSpan),
    EngineLayered(LayeredSpeechSpan),
}
fn legacy_span<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PresentationSpeechSpan, D::Error> {
    // Preserve the legacy values, but reject extensions in the new versioned form.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Span {
        id: u64,
        text: String,
        #[serde(default)]
        logical_voice_id: Option<String>,
        #[serde(default)]
        acss: crate::contracts::NormalizedAcss,
        #[serde(default)]
        rate_offset: Option<i16>,
        #[serde(default)]
        effects: PresentationEffectDirective,
    }
    let value = serde_json::Value::deserialize(d)?;
    if let Some(acss) = value.get("acss") {
        known_fields(
            acss,
            &[
                "rate",
                "average_pitch",
                "pitch_range",
                "stress",
                "richness",
                "volume",
            ],
        )
        .map_err(serde::de::Error::custom)?;
    }
    if let Some(effects) = value.get("effects") {
        let fields: &[&str] = if effects["mode"] == "replace" {
            &["mode", "state_id", "style"]
        } else {
            &["mode"]
        };
        known_fields(effects, fields).map_err(serde::de::Error::custom)?;
        if let Some(style) = effects.get("style") {
            known_fields(
                style,
                &[
                    "gain",
                    "low_pass",
                    "high_pass",
                    "pan",
                    "chorus",
                    "reverb",
                    "echo",
                ],
            )
            .map_err(serde::de::Error::custom)?;
        }
    }
    let s: Span = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
    Ok(PresentationSpeechSpan {
        id: s.id,
        text: s.text,
        logical_voice_id: s.logical_voice_id,
        acss: s.acss,
        rate_offset: s.rate_offset,
        effects: s.effects,
    })
}
impl NativeSpeechSpan {
    pub fn text(&self) -> &str {
        match self {
            Self::Legacy(s) => &s.text,
            Self::Layered(s) | Self::EngineLayered(s) => &s.text,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationTimelineV5 {
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
    #[serde(deserialize_with = "strict_actions")]
    pub actions: Vec<PresentationTimelineAction>,
}
// These legacy structures deliberately remain permissive in older protocols.
// At the new boundary, reject unknown nested fields before reusing their readers.
fn known_fields(value: &serde_json::Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("expected an object")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unknown field {key}"));
    }
    Ok(())
}
fn strict_actions<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<PresentationTimelineAction>, D::Error> {
    let values = Vec::<serde_json::Value>::deserialize(d)?;
    for value in &values {
        let mut fields = vec!["id", "position", "lifecycle_anchor", "type"];
        match value["type"].as_str() {
            Some("audio") => fields.extend(["path", "mode", "volume", "pan", "effect_bus"]),
            Some("tone") => fields.extend([
                "frequency_hz",
                "duration_ms",
                "mode",
                "volume",
                "pan",
                "effect_bus",
            ]),
            Some("silence") => fields.push("duration_ms"),
            _ => (),
        }
        known_fields(value, &fields).map_err(serde::de::Error::custom)?;
        let position = &value["position"];
        let mut fields = vec!["position", "span_id", "affinity"];
        if position["position"] == "text_offset" {
            fields.push("utf8_offset");
        }
        known_fields(position, &fields).map_err(serde::de::Error::custom)?;
    }
    values
        .into_iter()
        .map(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))
        .collect()
}
fn present_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
impl PresentationTimelineV5 {
    /// Preserve all modes and contexts in the common mixed-span executor.
    pub fn into_execution(self) -> PresentationTimelineV4 {
        PresentationTimelineV4 {
            protocol_version: self.protocol_version,
            generation: self.generation,
            dispatch_id: self.dispatch_id,
            registry_generation: self.registry_generation,
            delivery_policy: self.delivery_policy,
            replacement_key: self.replacement_key,
            spans: self
                .spans
                .into_iter()
                .map(|s| match s {
                    NativeSpeechSpan::Legacy(s) => MixedSpeechSpan::Legacy(s),
                    NativeSpeechSpan::Layered(s) => MixedSpeechSpan::Layered(s),
                    NativeSpeechSpan::EngineLayered(s) => MixedSpeechSpan::EngineLayered(s),
                })
                .collect(),
            actions: self.actions,
        }
    }
    pub fn validate(&self, aggregate: bool) -> Result<(), PresentationTimelineError> {
        self.clone()
            .into_execution()
            .validate_native_execution(aggregate)
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
    pub fn validate_registry(
        &self,
        registry: &LogicalVoiceRegistry,
    ) -> Result<(), PresentationTimelineError> {
        let invalid = |s: &str| PresentationTimelineError::InvalidTimeline(s.into());
        if self.registry_generation != registry.generation() {
            return Err(invalid("timeline registry generation is not current"));
        }
        for span in &self.spans {
            let id = match span {
                NativeSpeechSpan::Legacy(s) => s.logical_voice_id.as_deref(),
                NativeSpeechSpan::Layered(s) | NativeSpeechSpan::EngineLayered(s) => {
                    Some(s.logical_voice_id.as_str())
                }
            };
            let Some(id) = id else {
                continue;
            };
            let definition = registry.registered_definitions().iter().find(|d| match d {
                RegisteredVoiceDefinition::Legacy(d) => d.id == id,
                RegisteredVoiceDefinition::Layered(d) => d.id == id,
                RegisteredVoiceDefinition::EngineLayered(d) => d.id == id,
            });
            if !matches!(
                (span, definition),
                (
                    NativeSpeechSpan::Legacy(_),
                    Some(RegisteredVoiceDefinition::Legacy(_))
                ) | (
                    NativeSpeechSpan::Layered(_),
                    Some(RegisteredVoiceDefinition::Layered(_))
                ) | (
                    NativeSpeechSpan::EngineLayered(_),
                    Some(RegisteredVoiceDefinition::EngineLayered(_))
                )
            ) {
                return Err(invalid(
                    "timeline span mode does not match its registered definition",
                ));
            }
        }
        Ok(())
    }
}
pub fn encode_timeline_v5(
    t: &PresentationTimelineV5,
    aggregate: bool,
) -> Result<String, PresentationTimelineError> {
    t.validate(aggregate)?;
    let bytes = serde_json::to_vec(t).map_err(PresentationTimelineError::InvalidJson)?;
    if bytes.len() > limit(aggregate) {
        return Err(size_error(aggregate));
    }
    Ok(STANDARD.encode(bytes))
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
pub fn decode_timeline_v5(
    payload: &str,
    declared: Option<usize>,
) -> Result<PresentationTimelineV5, PresentationTimelineDecodeError> {
    let aggregate = declared.is_some();
    let decode = || {
        if declared.is_some_and(|n| n == 0 || n > limit(aggregate)) {
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
        if declared.is_some_and(|n| n != bytes.len()) {
            return Err(PresentationTimelineError::InvalidTimeline(
                "multipart decoded size differs from declaration".into(),
            ));
        }
        serde_json::from_slice::<crate::control::DuplicateFreeJson>(&bytes)
            .map_err(PresentationTimelineError::InvalidJson)?;
        serde_json::from_slice::<PresentationTimelineV5>(&bytes)
            .map_err(PresentationTimelineError::InvalidJson)
    };
    let timeline = decode().map_err(|e| PresentationTimelineDecodeError::new(None, e))?;
    timeline
        .validate(aggregate)
        .map_err(|e| PresentationTimelineDecodeError::new(timeline.tracking_identity(), e))?;
    Ok(timeline)
}
#[cfg(test)]
mod tests;
