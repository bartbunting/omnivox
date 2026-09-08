//! Explicit mixed-span timelines. No legacy flattening is used for execution.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::logical_voices::LogicalVoiceRegistry;
use crate::timeline_protocol::*;
use crate::voice_choices::{RegisteredVoiceDefinition, VoiceStylePatch};
use crate::voice_preview_v2::VoicePlacement;

pub const PRESENTATION_TIMELINE_PROTOCOL_V4: u32 = 4;

/// A decoded document retains its original representation through admission.
#[derive(Debug, Clone, PartialEq)]
pub enum TimelineDocument {
    Legacy(PresentationTimelineEnvelope),
    Layered(PresentationTimelineV4),
}

impl From<PresentationTimelineEnvelope> for TimelineDocument {
    fn from(timeline: PresentationTimelineEnvelope) -> Self {
        Self::Legacy(timeline)
    }
}

impl From<PresentationTimelineV4> for TimelineDocument {
    fn from(timeline: PresentationTimelineV4) -> Self {
        Self::Layered(timeline)
    }
}

impl TimelineDocument {
    pub fn protocol_version(&self) -> u32 {
        match self {
            Self::Legacy(t) => t.protocol_version,
            Self::Layered(t) => t.protocol_version,
        }
    }
    pub fn generation(&self) -> u64 {
        match self {
            Self::Legacy(t) => t.generation,
            Self::Layered(t) => t.generation,
        }
    }
    pub fn dispatch_id(&self) -> u64 {
        match self {
            Self::Legacy(t) => t.dispatch_id,
            Self::Layered(t) => t.dispatch_id,
        }
    }
    pub fn effective_delivery_policy(&self) -> PresentationDeliveryPolicy {
        match self {
            Self::Legacy(t) => t.effective_delivery_policy(),
            Self::Layered(t) => t.delivery_policy,
        }
    }
    pub fn replacement_key(&self) -> Option<&str> {
        match self {
            Self::Legacy(t) => t.replacement_key.as_deref(),
            Self::Layered(t) => t.replacement_key.as_deref(),
        }
    }
    pub fn tracking_identity(&self) -> Option<PresentationTimelineIdentity> {
        match self {
            Self::Legacy(t) => t.tracking_identity(),
            Self::Layered(t) => t.tracking_identity(),
        }
    }
    pub fn span_text(&self, index: usize) -> Option<&str> {
        match self {
            Self::Legacy(t) => t.spans.get(index).map(|span| span.text.as_str()),
            Self::Layered(t) => t.spans.get(index).map(MixedSpeechSpan::text),
        }
    }
    pub fn shares_replacement_domain(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Legacy(a), Self::Legacy(b)) => a.shares_replacement_domain(b),
            (Self::Layered(a), Self::Layered(b)) => a.shares_replacement_domain(b),
            _ => false,
        }
    }
}

/// Select the versioned decoder within existing transport bounds. Old decoder
/// entry points remain available with their original accepted versions.
pub fn decode_timeline_document(
    payload: &str,
    aggregate_bytes: Option<usize>,
) -> Result<TimelineDocument, PresentationTimelineDecodeError> {
    #[derive(Deserialize)]
    struct Version {
        protocol_version: u32,
    }
    let aggregate = aggregate_bytes.is_some();
    let limit = if aggregate {
        MAX_TIMELINE_AGGREGATE_BYTES
    } else {
        MAX_TIMELINE_PAYLOAD_BYTES
    };
    let version = || {
        let payload = payload.trim();
        let payload = payload
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(payload);
        if payload.len() > limit.div_ceil(3) * 4 || aggregate_bytes.is_some_and(|size| size > limit)
        {
            return Err(size_error(aggregate));
        }
        let bytes = STANDARD
            .decode(payload)
            .map_err(PresentationTimelineError::InvalidBase64)?;
        if bytes.len() > limit {
            return Err(size_error(aggregate));
        }
        serde_json::from_slice::<Version>(&bytes)
            .map(|value| value.protocol_version)
            .map_err(PresentationTimelineError::InvalidJson)
    };
    let version = version().map_err(|error| PresentationTimelineDecodeError::new(None, error))?;
    if version == PRESENTATION_TIMELINE_PROTOCOL_V4 {
        decode_timeline_v4(payload, aggregate_bytes).map(TimelineDocument::Layered)
    } else if let Some(bytes) = aggregate_bytes {
        decode_multipart_presentation_timeline(payload, bytes)
            .map(TimelineDocument::Legacy)
            .map_err(|error| PresentationTimelineDecodeError::new(None, error))
    } else {
        decode_presentation_timeline(payload).map(TimelineDocument::Legacy)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayeredSpeechSpan {
    pub id: u64,
    pub text: String,
    pub logical_voice_id: String,
    pub context: VoiceStylePatch,
    pub placement: VoicePlacement,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "span",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MixedSpeechSpan {
    Legacy(PresentationSpeechSpan),
    Layered(LayeredSpeechSpan),
}

impl MixedSpeechSpan {
    pub fn id(&self) -> u64 {
        match self {
            Self::Legacy(span) => span.id,
            Self::Layered(span) => span.id,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Legacy(span) => &span.text,
            Self::Layered(span) => &span.text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationTimelineV4 {
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
    pub spans: Vec<MixedSpeechSpan>,
    pub actions: Vec<PresentationTimelineAction>,
}

// A supplied null is still a forbidden delivery field for ordered/urgent work.
fn present_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

impl PresentationTimelineV4 {
    pub fn tracking_identity(&self) -> Option<PresentationTimelineIdentity> {
        PresentationTimelineIdentity::new(self.generation, self.dispatch_id)
    }

    pub fn shares_replacement_domain(&self, other: &Self) -> bool {
        self.protocol_version == other.protocol_version
            && self.delivery_policy == PresentationDeliveryPolicy::Replaceable
            && other.delivery_policy == PresentationDeliveryPolicy::Replaceable
            && self.replacement_key == other.replacement_key
    }

    /// Registry ownership is checked separately at admission, after syntax and
    /// whole-document validation and before any cancellation or queue mutation.
    pub fn validate_registry(
        &self,
        registry: &LogicalVoiceRegistry,
    ) -> Result<(), PresentationTimelineError> {
        require(
            self.registry_generation == registry.generation(),
            "timeline registry generation is not current",
        )?;
        let layered = registry
            .registered_definitions()
            .iter()
            .filter_map(|definition| match definition {
                RegisteredVoiceDefinition::Layered(voice) => Some(voice.id.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        for span in &self.spans {
            if let MixedSpeechSpan::Layered(span) = span {
                require(
                    layered.contains(span.logical_voice_id.as_str()),
                    "layered span references a missing or legacy definition",
                )?;
            }
        }
        Ok(())
    }

    pub fn validate(&self, aggregate: bool) -> Result<(), PresentationTimelineError> {
        require(
            self.protocol_version == PRESENTATION_TIMELINE_PROTOCOL_V4,
            "expected timeline protocol version 4",
        )?;
        require(
            self.generation > 0 && self.dispatch_id > 0 && self.registry_generation > 0,
            "timeline and registry generations and dispatch ID must be positive",
        )?;
        match self.delivery_policy {
            PresentationDeliveryPolicy::Replaceable => validate_id(
                self.replacement_key
                    .as_deref()
                    .ok_or_else(|| invalid("replaceable timeline requires a replacement key"))?,
                "replacement key",
            )?,
            _ => require(
                self.replacement_key.is_none(),
                "ordered and urgent timelines forbid replacement keys",
            )?,
        }
        let span_limit = if aggregate {
            MAX_TIMELINE_AGGREGATE_SPANS
        } else {
            MAX_TIMELINE_SPANS
        };
        require(
            !self.spans.is_empty() && self.spans.len() <= span_limit,
            "timeline speech span count is outside its transport bound",
        )?;
        require(
            self.actions.len() <= MAX_TIMELINE_ACTIONS,
            "timeline has too many actions",
        )?;
        let mut spans = HashMap::with_capacity(self.spans.len());
        for span in &self.spans {
            require(span.id() > 0, "speech span ID must be positive")?;
            require(!span.text().is_empty(), "speech span text cannot be empty")?;
            require(
                spans.insert(span.id(), span.text()).is_none(),
                "duplicate speech span ID",
            )?;
            match span {
                MixedSpeechSpan::Legacy(span) => {
                    if let Some(id) = &span.logical_voice_id {
                        validate_id(id, "logical voice")?;
                    }
                    validate_legacy_span_style(span)?;
                }
                MixedSpeechSpan::Layered(span) => {
                    validate_id(&span.logical_voice_id, "logical voice")?;
                    span.context
                        .validate()
                        .map_err(|error| invalid(&error.to_string()))?;
                    span.placement.validate().map_err(|error| invalid(&error))?;
                }
            }
        }
        let mut action_ids = HashSet::with_capacity(self.actions.len());
        for action in &self.actions {
            validate_id(&action.id, "action")?;
            require(
                !action.id.starts_with("omnivox."),
                "action IDs beginning with omnivox. are reserved",
            )?;
            require(action_ids.insert(&action.id), "duplicate action ID")?;
            let text = spans
                .get(&action.position.span_id())
                .ok_or_else(|| invalid("action references an unknown speech span"))?;
            if let PresentationTimelinePosition::TextOffset { utf8_offset, .. } = action.position {
                let offset = utf8_offset as usize;
                require(
                    offset <= text.len() && text.is_char_boundary(offset),
                    "action has an invalid UTF-8 offset",
                )?;
            }
            validate_action(action)?;
        }
        Ok(())
    }
}

fn invalid(message: &str) -> PresentationTimelineError {
    PresentationTimelineError::InvalidTimeline(message.to_owned())
}

fn require(valid: bool, message: &str) -> Result<(), PresentationTimelineError> {
    if valid {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

pub fn encode_timeline_v4(
    timeline: &PresentationTimelineV4,
    aggregate: bool,
) -> Result<String, PresentationTimelineError> {
    timeline.validate(aggregate)?;
    let bytes = serde_json::to_vec(timeline).map_err(PresentationTimelineError::InvalidJson)?;
    let limit = if aggregate {
        MAX_TIMELINE_AGGREGATE_BYTES
    } else {
        MAX_TIMELINE_PAYLOAD_BYTES
    };
    if bytes.len() > limit {
        return Err(size_error(aggregate));
    }
    Ok(STANDARD.encode(bytes))
}

/// A declared aggregate size selects the multipart bound; None is one frame.
/// Transport assembly must additionally verify every part's identity/version.
pub fn decode_timeline_v4(
    payload: &str,
    declared_aggregate_bytes: Option<usize>,
) -> Result<PresentationTimelineV4, PresentationTimelineDecodeError> {
    let aggregate = declared_aggregate_bytes.is_some();
    let limit = if aggregate {
        MAX_TIMELINE_AGGREGATE_BYTES
    } else {
        MAX_TIMELINE_PAYLOAD_BYTES
    };
    let decode = || {
        if declared_aggregate_bytes.is_some_and(|bytes| bytes == 0 || bytes > limit) {
            return Err(size_error(aggregate));
        }
        let payload = payload.trim();
        let payload = payload
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(payload);
        if payload.len() > limit.div_ceil(3) * 4 {
            return Err(size_error(aggregate));
        }
        let bytes = STANDARD
            .decode(payload)
            .map_err(PresentationTimelineError::InvalidBase64)?;
        if bytes.len() > limit {
            return Err(size_error(aggregate));
        }
        require(
            declared_aggregate_bytes.is_none_or(|declared| declared == bytes.len()),
            "multipart timeline decoded size differs from its declaration",
        )?;
        serde_json::from_slice::<crate::control::DuplicateFreeJson>(&bytes)
            .map_err(PresentationTimelineError::InvalidJson)?;
        serde_json::from_slice::<PresentationTimelineV4>(&bytes)
            .map_err(PresentationTimelineError::InvalidJson)
    };
    let timeline = decode().map_err(|error| PresentationTimelineDecodeError::new(None, error))?;
    timeline.validate(aggregate).map_err(|error| {
        PresentationTimelineDecodeError::new(timeline.tracking_identity(), error)
    })?;
    Ok(timeline)
}

fn size_error(aggregate: bool) -> PresentationTimelineError {
    if aggregate {
        PresentationTimelineError::AggregatePayloadTooLarge
    } else {
        PresentationTimelineError::PayloadTooLarge
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture() -> Value {
        let value: Value = serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/voice-choice-tuning.json"
        ))
        .unwrap();
        value["messages"]["timeline"].clone()
    }

    #[test]
    fn independent_mixed_fixture_roundtrips_without_flattening() {
        let value = fixture();
        let timeline = decode_timeline_v4(&STANDARD.encode(value.to_string()), None).unwrap();
        assert!(timeline
            .spans
            .iter()
            .any(|span| matches!(span, MixedSpeechSpan::Layered(_))));
        assert!(timeline
            .spans
            .iter()
            .any(|span| matches!(span, MixedSpeechSpan::Legacy(_))));
        assert_eq!(
            decode_timeline_v4(&encode_timeline_v4(&timeline, false).unwrap(), None).unwrap(),
            timeline
        );
    }

    #[test]
    fn new_timeline_rejects_unknown_missing_mixed_and_duplicate_keys() {
        let original = fixture();
        for field in [
            "protocol_version",
            "generation",
            "dispatch_id",
            "registry_generation",
            "delivery_policy",
            "spans",
            "actions",
        ] {
            let mut value = original.clone();
            value.as_object_mut().unwrap().remove(field);
            assert!(
                decode_timeline_v4(&STANDARD.encode(value.to_string()), None).is_err(),
                "{field}"
            );
        }
        let layered = original["spans"]
            .as_array()
            .unwrap()
            .iter()
            .position(|span| span["mode"] == "layered")
            .unwrap();
        for field in ["acss", "rate_offset", "effects", "unknown"] {
            let mut value = original.clone();
            value["spans"][layered]["span"][field] = json!({});
            assert!(
                decode_timeline_v4(&STANDARD.encode(value.to_string()), None).is_err(),
                "{field}"
            );
        }
        let mut value = original.clone();
        value["replacement_key"] = Value::Null;
        assert!(decode_timeline_v4(&STANDARD.encode(value.to_string()), None).is_err());
        let raw = original.to_string().replacen(
            "\"dispatch_id\":",
            "\"dispatch_id\":999,\"dispatch_id\":",
            1,
        );
        assert!(decode_timeline_v4(&STANDARD.encode(raw), None)
            .unwrap_err()
            .identity()
            .is_none());
    }

    #[test]
    fn registry_references_require_current_layered_definitions_but_allow_unresolved() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/voice-choice-tuning.json"
        ))
        .unwrap();
        let timeline: PresentationTimelineV4 =
            serde_json::from_value(fixture["messages"]["timeline"].clone()).unwrap();
        let registration: crate::control::ControlRequestEnvelope =
            serde_json::from_value(fixture["messages"]["registration"].clone()).unwrap();
        let crate::control::ControlRequest::RegisterLogicalVoicesV2(registration) =
            registration.request
        else {
            panic!("registration")
        };
        let mut registry = LogicalVoiceRegistry::default();
        registry
            .register_v2(
                registration.registry_generation,
                registration.definitions,
                registration.fallback_policy.into(),
                &[],
            )
            .unwrap();
        timeline.validate_registry(&registry).unwrap(); // No installed voices is still admissible.
        let mut stale = timeline.clone();
        stale.registry_generation += 1;
        assert!(stale.validate_registry(&registry).is_err());
        let layered = timeline
            .spans
            .iter()
            .position(|span| matches!(span, MixedSpeechSpan::Layered(_)))
            .unwrap();
        let mut missing = timeline;
        let MixedSpeechSpan::Layered(span) = &mut missing.spans[layered] else {
            unreachable!()
        };
        span.logical_voice_id = "missing".to_owned();
        assert!(missing.validate_registry(&registry).is_err());
        let MixedSpeechSpan::Layered(span) = &mut missing.spans[layered] else {
            unreachable!()
        };
        span.logical_voice_id = "voice-annotate".to_owned();
        assert!(registry
            .definitions()
            .iter()
            .any(|voice| voice.id == "voice-annotate"));
        assert!(missing.validate_registry(&registry).is_err());
    }

    #[test]
    fn semantic_rejection_preserves_trustworthy_dispatch_and_checks_whole_document() {
        let original = fixture();
        for change in [0, 1, 2] {
            let mut value = original.clone();
            match change {
                0 => value["registry_generation"] = json!(0),
                1 => value["spans"][1]["span"]["id"] = value["spans"][0]["span"]["id"].clone(),
                _ => {
                    value["delivery_policy"] = json!("ordered");
                    value["replacement_key"] = json!("navigation");
                }
            }
            let error = decode_timeline_v4(&STANDARD.encode(value.to_string()), None).unwrap_err();
            assert_eq!(
                error.identity().unwrap().dispatch_id(),
                original["dispatch_id"].as_u64().unwrap()
            );
        }
    }

    #[test]
    fn aggregate_bound_does_not_expand_single_frame_or_action_limits() {
        let mut timeline: PresentationTimelineV4 = serde_json::from_value(fixture()).unwrap();
        let span = timeline.spans[0].clone();
        timeline.actions.clear();
        timeline.spans = (1..=4097)
            .map(|id| {
                let mut span = span.clone();
                match &mut span {
                    MixedSpeechSpan::Legacy(span) => span.id = id,
                    MixedSpeechSpan::Layered(span) => span.id = id,
                };
                span
            })
            .collect();
        assert!(timeline.validate(false).is_err());
        timeline.validate(true).unwrap();
        let encoded = encode_timeline_v4(&timeline, true).unwrap();
        let size = STANDARD.decode(&encoded).unwrap().len();
        assert!(decode_timeline_v4(&encoded, None).is_err());
        assert!(decode_timeline_v4(&encoded, Some(size + 1)).is_err());
        assert_eq!(decode_timeline_v4(&encoded, Some(size)).unwrap(), timeline);
    }

    #[test]
    fn action_validation_crosses_both_span_kinds_and_preserves_utf8_boundaries() {
        let mut timeline: PresentationTimelineV4 = serde_json::from_value(fixture()).unwrap();
        let MixedSpeechSpan::Layered(span) = &mut timeline.spans[0] else {
            panic!("layered fixture")
        };
        span.text = "é heading".to_owned();
        timeline.actions = vec![PresentationTimelineAction {
            id: "cue".to_owned(),
            lifecycle_anchor: PresentationLifecycleAnchor::Object,
            position: PresentationTimelinePosition::TextOffset {
                span_id: 1,
                utf8_offset: 2,
                affinity: PresentationAffinity::Before,
            },
            action: PresentationAction::SemanticEvent,
        }];
        timeline.validate(false).unwrap();
        for (span_id, offset, valid) in [(1, 1, false), (2, 1, true), (999, 0, false)] {
            timeline.actions[0].position = PresentationTimelinePosition::TextOffset {
                span_id,
                utf8_offset: offset,
                affinity: PresentationAffinity::Before,
            };
            assert_eq!(timeline.validate(false).is_ok(), valid);
        }
        timeline.actions[0].position = PresentationTimelinePosition::SpanBoundary {
            span_id: 2,
            affinity: PresentationAffinity::After,
        };
        timeline.actions.push(timeline.actions[0].clone());
        assert!(timeline.validate(false).is_err());
        timeline.actions = vec![timeline.actions[0].clone(); MAX_TIMELINE_ACTIONS + 1];
        assert!(timeline.validate(true).is_err());
    }
}
