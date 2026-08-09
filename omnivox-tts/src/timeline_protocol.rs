//! Bounded Base64-JSON transport for structured Emacsvox presentations.

use std::collections::HashSet;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::{
    NormalizedAcss, PostSynthesisStyle, MAX_RATE_OFFSET_POINTS, MIN_RATE_OFFSET_POINTS,
};

/// Original structured presentation-timeline protocol version.
pub const PRESENTATION_TIMELINE_PROTOCOL_V1: u32 = 1;
/// Structured timeline version carrying delivery policy and replacement identity.
pub const PRESENTATION_TIMELINE_PROTOCOL_V2: u32 = 2;
/// Current structured presentation-timeline protocol version.
pub const PRESENTATION_TIMELINE_PROTOCOL_VERSION: u32 = PRESENTATION_TIMELINE_PROTOCOL_V2;
/// Maximum decoded JSON accepted in one timeline request.
pub const MAX_TIMELINE_PAYLOAD_BYTES: usize = 256 * 1024;
/// Conservative maximum encoded size for the decoded request bound.
pub const MAX_TIMELINE_ENCODED_BYTES: usize = (MAX_TIMELINE_PAYLOAD_BYTES / 3) * 4 + 8;
/// Maximum number of speech spans in one request.
pub const MAX_TIMELINE_SPANS: usize = 4096;
/// Maximum number of non-speech actions in one request.
pub const MAX_TIMELINE_ACTIONS: usize = 4096;
/// Maximum UTF-8 size of a logical voice, action, or effect-state ID.
pub const MAX_TIMELINE_ID_BYTES: usize = 128;
/// Maximum UTF-8 size of one resource path.
pub const MAX_TIMELINE_RESOURCE_PATH_BYTES: usize = 4096;

/// One atomic presentation and its tracked playback identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationTimelineEnvelope {
    pub protocol_version: u32,
    pub generation: u64,
    pub dispatch_id: u64,
    /// Version 2 delivery policy. Version 1 omits this and is replaceable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_policy: Option<PresentationDeliveryPolicy>,
    /// Version 2 replacement domain, present only for replaceable work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_key: Option<String>,
    pub spans: Vec<PresentationSpeechSpan>,
    #[serde(default)]
    pub actions: Vec<PresentationTimelineAction>,
}

impl PresentationTimelineEnvelope {
    /// Return the validated effective delivery policy.
    pub fn effective_delivery_policy(&self) -> PresentationDeliveryPolicy {
        self.delivery_policy
            .unwrap_or(PresentationDeliveryPolicy::Replaceable)
    }

    /// Return whether adjacent validated envelopes share one replacement domain.
    pub fn shares_replacement_domain(&self, other: &Self) -> bool {
        if self.effective_delivery_policy() != PresentationDeliveryPolicy::Replaceable
            || other.effective_delivery_policy() != PresentationDeliveryPolicy::Replaceable
        {
            return false;
        }

        match (self.protocol_version, other.protocol_version) {
            (PRESENTATION_TIMELINE_PROTOCOL_V1, PRESENTATION_TIMELINE_PROTOCOL_V1) => true,
            (PRESENTATION_TIMELINE_PROTOCOL_V2, PRESENTATION_TIMELINE_PROTOCOL_V2) => {
                self.replacement_key == other.replacement_key
            }
            _ => false,
        }
    }
}

/// Scheduling contract for one complete structured presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationDeliveryPolicy {
    Ordered,
    Replaceable,
    Urgent,
}

/// One ordered speech span with an independently routable logical voice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationSpeechSpan {
    pub id: u64,
    pub text: String,
    #[serde(default)]
    pub logical_voice_id: Option<String>,
    #[serde(default)]
    pub acss: NormalizedAcss,
    /// Signed points relative to the server's current normalized speech rate.
    /// This is intentionally separate from the absolute `acss.rate` value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_offset: Option<i16>,
    #[serde(default)]
    pub effects: PresentationEffectDirective,
}

/// Persistent complete-state operation applied before a speech span.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PresentationEffectDirective {
    /// Continue the complete effect state used by the preceding span.
    #[default]
    Retain,
    /// Replace the complete state. Missing dimensions have neutral values.
    Replace {
        state_id: String,
        style: PostSynthesisStyle,
    },
    /// End the persistent effect state and return to neutral processing.
    End,
}

/// One non-speech operation attached to a source position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationTimelineAction {
    pub id: String,
    pub position: PresentationTimelinePosition,
    /// Semantic lifecycle metadata; it does not determine audio placement.
    pub lifecycle_anchor: PresentationLifecycleAnchor,
    #[serde(flatten)]
    pub action: PresentationAction,
}

/// Physical source position retained until synthesis resolves an audio frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "position", rename_all = "snake_case")]
pub enum PresentationTimelinePosition {
    SpanBoundary {
        span_id: u64,
        affinity: PresentationAffinity,
    },
    TextOffset {
        span_id: u64,
        utf8_offset: u32,
        affinity: PresentationAffinity,
    },
}

impl PresentationTimelinePosition {
    pub fn span_id(&self) -> u64 {
        match self {
            Self::SpanBoundary { span_id, .. } | Self::TextOffset { span_id, .. } => *span_id,
        }
    }
}

/// Which side of one source position owns an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationAffinity {
    Before,
    After,
}

/// Semantic lifecycle retained independently from physical audio placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationLifecycleAnchor {
    Object,
    Run,
    Transition,
}

/// One operation rendered on the common presentation timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PresentationAction {
    Audio {
        path: String,
        mode: PresentationAudioMode,
        #[serde(default = "unity")]
        volume: f32,
        #[serde(default = "center")]
        pan: f32,
        #[serde(default)]
        effect_bus: PresentationEffectBus,
    },
    Tone {
        frequency_hz: f32,
        duration_ms: u32,
        mode: PresentationAudioMode,
        #[serde(default = "unity")]
        volume: f32,
        #[serde(default = "center")]
        pan: f32,
        #[serde(default)]
        effect_bus: PresentationEffectBus,
    },
    Silence {
        duration_ms: u32,
    },
    SemanticEvent,
}

/// Whether an audio action advances the primary speech clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationAudioMode {
    Insert,
    Overlay,
}

/// Whether an audio action uses the active speech effect state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationEffectBus {
    #[default]
    Dry,
    Speech,
}

#[derive(Debug, Error)]
pub enum PresentationTimelineError {
    #[error("timeline payload exceeds the {MAX_TIMELINE_PAYLOAD_BYTES}-byte limit")]
    PayloadTooLarge,

    #[error("timeline payload is not valid Base64: {0}")]
    InvalidBase64(#[source] base64::DecodeError),

    #[error("timeline payload is not valid JSON: {0}")]
    InvalidJson(#[source] serde_json::Error),

    #[error("invalid presentation timeline: {0}")]
    InvalidTimeline(String),
}

/// Encode and validate one structured presentation as an unwrapped Base64 word.
pub fn encode_presentation_timeline(
    timeline: &PresentationTimelineEnvelope,
) -> Result<String, PresentationTimelineError> {
    validate_presentation_timeline(timeline)?;
    let json = serde_json::to_vec(timeline).map_err(PresentationTimelineError::InvalidJson)?;
    if json.len() > MAX_TIMELINE_PAYLOAD_BYTES {
        return Err(PresentationTimelineError::PayloadTooLarge);
    }
    Ok(STANDARD.encode(json))
}

/// Decode, validate, and bound one structured presentation payload.
pub fn decode_presentation_timeline(
    payload: &str,
) -> Result<PresentationTimelineEnvelope, PresentationTimelineError> {
    let payload = payload
        .trim()
        .strip_prefix('{')
        .and_then(|payload| payload.strip_suffix('}'))
        .unwrap_or_else(|| payload.trim());
    if payload.len() > MAX_TIMELINE_ENCODED_BYTES {
        return Err(PresentationTimelineError::PayloadTooLarge);
    }
    let json = STANDARD
        .decode(payload)
        .map_err(PresentationTimelineError::InvalidBase64)?;
    if json.len() > MAX_TIMELINE_PAYLOAD_BYTES {
        return Err(PresentationTimelineError::PayloadTooLarge);
    }
    let timeline = serde_json::from_slice(&json).map_err(PresentationTimelineError::InvalidJson)?;
    validate_presentation_timeline(&timeline)?;
    Ok(timeline)
}

/// Validate all cross-references and resource bounds before playback changes.
pub fn validate_presentation_timeline(
    timeline: &PresentationTimelineEnvelope,
) -> Result<(), PresentationTimelineError> {
    match timeline.protocol_version {
        PRESENTATION_TIMELINE_PROTOCOL_V1 => {
            invalid_if(
                timeline.delivery_policy.is_some() || timeline.replacement_key.is_some(),
                "version 1 timelines cannot contain version 2 delivery fields",
            )?;
        }
        PRESENTATION_TIMELINE_PROTOCOL_V2 => {
            let Some(delivery_policy) = timeline.delivery_policy else {
                return Err(PresentationTimelineError::InvalidTimeline(
                    "version 2 timelines require a delivery policy".to_owned(),
                ));
            };
            match delivery_policy {
                PresentationDeliveryPolicy::Replaceable => {
                    let Some(replacement_key) = timeline.replacement_key.as_deref() else {
                        return Err(PresentationTimelineError::InvalidTimeline(
                            "replaceable timelines require a replacement key".to_owned(),
                        ));
                    };
                    validate_id(replacement_key, "replacement key")?;
                }
                PresentationDeliveryPolicy::Ordered | PresentationDeliveryPolicy::Urgent => {
                    invalid_if(
                        timeline.replacement_key.is_some(),
                        "ordered and urgent timelines cannot contain a replacement key",
                    )?;
                }
            }
        }
        version => {
            return Err(PresentationTimelineError::InvalidTimeline(format!(
                "unsupported protocol version {version}"
            )));
        }
    }
    invalid_if(timeline.generation == 0, "generation must be positive")?;
    invalid_if(timeline.dispatch_id == 0, "dispatch ID must be positive")?;
    invalid_if(
        timeline.spans.is_empty(),
        "at least one speech span is required",
    )?;
    invalid_if(
        timeline.spans.len() > MAX_TIMELINE_SPANS,
        format!("more than {MAX_TIMELINE_SPANS} speech spans"),
    )?;
    invalid_if(
        timeline.actions.len() > MAX_TIMELINE_ACTIONS,
        format!("more than {MAX_TIMELINE_ACTIONS} actions"),
    )?;

    let mut span_ids = HashSet::with_capacity(timeline.spans.len());
    for span in &timeline.spans {
        invalid_if(span.id == 0, "speech span ID must be positive")?;
        invalid_if(
            !span_ids.insert(span.id),
            format!("duplicate speech span ID {}", span.id),
        )?;
        invalid_if(
            span.text.is_empty(),
            format!("speech span {} is empty", span.id),
        )?;
        if let Some(logical_voice_id) = &span.logical_voice_id {
            validate_id(logical_voice_id, "logical voice")?;
        }
        validate_acss(&span.acss, span.id)?;
        if let Some(rate_offset) = span.rate_offset {
            invalid_if(
                !(MIN_RATE_OFFSET_POINTS..=MAX_RATE_OFFSET_POINTS).contains(&rate_offset),
                format!(
                    "speech span {} rate offset must be between {} and {} points",
                    span.id, MIN_RATE_OFFSET_POINTS, MAX_RATE_OFFSET_POINTS
                ),
            )?;
            invalid_if(
                span.acss.rate.is_some(),
                format!(
                    "speech span {} cannot combine absolute rate and rate offset",
                    span.id
                ),
            )?;
        }
        if let PresentationEffectDirective::Replace { state_id, style } = &span.effects {
            validate_id(state_id, "effect state")?;
            validate_effects(style, span.id)?;
        }
    }

    let spans = timeline
        .spans
        .iter()
        .map(|span| (span.id, span))
        .collect::<std::collections::HashMap<_, _>>();
    let mut action_ids = HashSet::with_capacity(timeline.actions.len());
    for action in &timeline.actions {
        validate_id(&action.id, "action")?;
        invalid_if(
            action.id.starts_with("omnivox."),
            "action IDs beginning with omnivox. are reserved",
        )?;
        invalid_if(
            !action_ids.insert(action.id.as_str()),
            format!("duplicate action ID {}", action.id),
        )?;
        let Some(span) = spans.get(&action.position.span_id()) else {
            return Err(PresentationTimelineError::InvalidTimeline(format!(
                "action {} references an unknown speech span",
                action.id
            )));
        };
        if let PresentationTimelinePosition::TextOffset { utf8_offset, .. } = action.position {
            let offset = utf8_offset as usize;
            invalid_if(
                offset > span.text.len() || !span.text.is_char_boundary(offset),
                format!("action {} has an invalid UTF-8 offset", action.id),
            )?;
        }
        validate_action(action)?;
    }
    Ok(())
}

fn validate_action(action: &PresentationTimelineAction) -> Result<(), PresentationTimelineError> {
    match &action.action {
        PresentationAction::Audio {
            path, volume, pan, ..
        } => {
            invalid_if(
                path.is_empty(),
                format!("audio action {} has an empty path", action.id),
            )?;
            invalid_if(
                path.len() > MAX_TIMELINE_RESOURCE_PATH_BYTES,
                format!("audio action {} path is too long", action.id),
            )?;
            validate_normalized(*volume, &format!("audio action {} volume", action.id))?;
            validate_normalized(*pan, &format!("audio action {} pan", action.id))
        }
        PresentationAction::Tone {
            frequency_hz,
            duration_ms,
            volume,
            pan,
            ..
        } => {
            invalid_if(
                !frequency_hz.is_finite() || *frequency_hz <= 0.0 || *frequency_hz > 24_000.0,
                format!("tone action {} has an invalid frequency", action.id),
            )?;
            invalid_if(
                *duration_ms == 0 || *duration_ms > 60_000,
                format!("tone action {} has an invalid duration", action.id),
            )?;
            validate_normalized(*volume, &format!("tone action {} volume", action.id))?;
            validate_normalized(*pan, &format!("tone action {} pan", action.id))
        }
        PresentationAction::Silence { duration_ms } => invalid_if(
            *duration_ms == 0 || *duration_ms > 60_000,
            format!("silence action {} has an invalid duration", action.id),
        ),
        PresentationAction::SemanticEvent => Ok(()),
    }
}

fn validate_acss(style: &NormalizedAcss, span_id: u64) -> Result<(), PresentationTimelineError> {
    for (name, value) in [
        ("rate", style.rate),
        ("average_pitch", style.average_pitch),
        ("pitch_range", style.pitch_range),
        ("stress", style.stress),
        ("richness", style.richness),
        ("volume", style.volume),
    ] {
        if let Some(value) = value {
            validate_normalized(value, &format!("speech span {span_id} ACSS {name}"))?;
        }
    }
    Ok(())
}

fn validate_effects(
    style: &PostSynthesisStyle,
    span_id: u64,
) -> Result<(), PresentationTimelineError> {
    for (name, value) in [
        ("gain", style.gain),
        ("low_pass", style.low_pass),
        ("high_pass", style.high_pass),
        ("pan", style.pan),
        ("reverb", style.reverb),
        ("echo", style.echo),
    ] {
        if let Some(value) = value {
            validate_normalized(value, &format!("speech span {span_id} effect {name}"))?;
        }
    }
    Ok(())
}

fn validate_id(value: &str, kind: &str) -> Result<(), PresentationTimelineError> {
    invalid_if(
        value.is_empty() || value.len() > MAX_TIMELINE_ID_BYTES,
        format!("{kind} ID must contain 1 to {MAX_TIMELINE_ID_BYTES} UTF-8 bytes"),
    )
}

fn validate_normalized(value: f32, field: &str) -> Result<(), PresentationTimelineError> {
    invalid_if(
        !value.is_finite() || !(0.0..=1.0).contains(&value),
        format!("{field} must be finite and normalized"),
    )
}

fn invalid_if(
    condition: bool,
    message: impl Into<String>,
) -> Result<(), PresentationTimelineError> {
    if condition {
        Err(PresentationTimelineError::InvalidTimeline(message.into()))
    } else {
        Ok(())
    }
}

fn unity() -> f32 {
    1.0
}

fn center() -> f32 {
    0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION_ONE_INTEROP_FIXTURE: &str =
        "eyJwcm90b2NvbF92ZXJzaW9uIjoxLCJnZW5lcmF0aW9uIjoyNywiZGlzcGF0Y2hfaWQiOjkxLCJzcGFucyI6W3siaWQiOjEsInRleHQiOiJjYWbDqSDml6XmnKwifV0sImFjdGlvbnMiOltdfQ==";
    const VERSION_TWO_INTEROP_FIXTURE: &str =
        "eyJwcm90b2NvbF92ZXJzaW9uIjoyLCJnZW5lcmF0aW9uIjoyNywiZGlzcGF0Y2hfaWQiOjkxLCJkZWxpdmVyeV9wb2xpY3kiOiJyZXBsYWNlYWJsZSIsInJlcGxhY2VtZW50X2tleSI6InNwZWFrZXIiLCJzcGFucyI6W3siaWQiOjEsInRleHQiOiJjYWbDqSDml6XmnKwifV0sImFjdGlvbnMiOltdfQ==";

    fn timeline() -> PresentationTimelineEnvelope {
        PresentationTimelineEnvelope {
            protocol_version: PRESENTATION_TIMELINE_PROTOCOL_VERSION,
            generation: 7,
            dispatch_id: 19,
            delivery_policy: Some(PresentationDeliveryPolicy::Replaceable),
            replacement_key: Some("speaker".to_owned()),
            spans: vec![PresentationSpeechSpan {
                id: 1,
                text: "café 日本".to_owned(),
                logical_voice_id: Some("comment".to_owned()),
                acss: NormalizedAcss {
                    rate: Some(0.7),
                    ..NormalizedAcss::default()
                },
                rate_offset: None,
                effects: PresentationEffectDirective::Replace {
                    state_id: "comment-effects".to_owned(),
                    style: PostSynthesisStyle {
                        reverb: Some(0.2),
                        pan: Some(0.75),
                        ..PostSynthesisStyle::default()
                    },
                },
            }],
            actions: vec![
                PresentationTimelineAction {
                    id: "open-cue".to_owned(),
                    position: PresentationTimelinePosition::SpanBoundary {
                        span_id: 1,
                        affinity: PresentationAffinity::Before,
                    },
                    lifecycle_anchor: PresentationLifecycleAnchor::Object,
                    action: PresentationAction::Audio {
                        path: "/tmp/open.ogg".to_owned(),
                        mode: PresentationAudioMode::Overlay,
                        volume: 0.8,
                        pan: 0.25,
                        effect_bus: PresentationEffectBus::Dry,
                    },
                },
                PresentationTimelineAction {
                    id: "spoken-word".to_owned(),
                    position: PresentationTimelinePosition::TextOffset {
                        span_id: 1,
                        utf8_offset: 6,
                        affinity: PresentationAffinity::Before,
                    },
                    lifecycle_anchor: PresentationLifecycleAnchor::Run,
                    action: PresentationAction::SemanticEvent,
                },
            ],
        }
    }

    #[test]
    fn timeline_round_trip_preserves_unicode_and_separate_anchors() {
        let mut timeline = timeline();
        timeline.spans[0].acss.rate = None;
        timeline.spans[0].rate_offset = Some(-4);
        let encoded = encode_presentation_timeline(&timeline).unwrap();

        assert_eq!(decode_presentation_timeline(&encoded).unwrap(), timeline);
        assert_eq!(
            decode_presentation_timeline(&format!("{{{encoded}}}")).unwrap(),
            timeline
        );
    }

    #[test]
    fn documented_emacsvox_fixtures_decode_for_both_versions() {
        let version_one = decode_presentation_timeline(VERSION_ONE_INTEROP_FIXTURE).unwrap();
        assert_eq!(
            version_one.protocol_version,
            PRESENTATION_TIMELINE_PROTOCOL_V1
        );
        assert_eq!(version_one.spans[0].text, "café 日本");
        assert_eq!(
            version_one.effective_delivery_policy(),
            PresentationDeliveryPolicy::Replaceable
        );

        let version_two = decode_presentation_timeline(VERSION_TWO_INTEROP_FIXTURE).unwrap();
        assert_eq!(
            version_two.protocol_version,
            PRESENTATION_TIMELINE_PROTOCOL_V2
        );
        assert_eq!(
            version_two.delivery_policy,
            Some(PresentationDeliveryPolicy::Replaceable)
        );
        assert_eq!(version_two.replacement_key.as_deref(), Some("speaker"));
    }

    #[test]
    fn version_one_remains_implicitly_replaceable() {
        let mut version_one = timeline();
        version_one.protocol_version = PRESENTATION_TIMELINE_PROTOCOL_V1;
        version_one.delivery_policy = None;
        version_one.replacement_key = None;

        let encoded = encode_presentation_timeline(&version_one).unwrap();
        let decoded = decode_presentation_timeline(&encoded).unwrap();

        assert_eq!(
            decoded.effective_delivery_policy(),
            PresentationDeliveryPolicy::Replaceable
        );
        assert!(decoded.shares_replacement_domain(&version_one));
    }

    #[test]
    fn version_two_validates_policy_and_replacement_identity() {
        let mut missing_policy = timeline();
        missing_policy.delivery_policy = None;
        assert!(matches!(
            validate_presentation_timeline(&missing_policy),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));

        let mut missing_key = timeline();
        missing_key.replacement_key = None;
        assert!(matches!(
            validate_presentation_timeline(&missing_key),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));

        let mut ordered_with_key = timeline();
        ordered_with_key.delivery_policy = Some(PresentationDeliveryPolicy::Ordered);
        assert!(matches!(
            validate_presentation_timeline(&ordered_with_key),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));

        let mut ordered = ordered_with_key;
        ordered.replacement_key = None;
        assert!(validate_presentation_timeline(&ordered).is_ok());
    }

    #[test]
    fn only_same_key_replaceable_timelines_share_a_domain() {
        let navigation = timeline();
        let mut same_key = timeline();
        same_key.generation += 1;
        let mut different_key = same_key.clone();
        different_key.replacement_key = Some("completion".to_owned());
        let mut ordered = same_key.clone();
        ordered.delivery_policy = Some(PresentationDeliveryPolicy::Ordered);
        ordered.replacement_key = None;
        let mut urgent = ordered.clone();
        urgent.delivery_policy = Some(PresentationDeliveryPolicy::Urgent);

        assert!(navigation.shares_replacement_domain(&same_key));
        assert!(!navigation.shares_replacement_domain(&different_key));
        assert!(!navigation.shares_replacement_domain(&ordered));
        assert!(!navigation.shares_replacement_domain(&urgent));
    }

    #[test]
    fn validation_rejects_duplicate_ids_and_unknown_spans() {
        let mut duplicate = timeline();
        duplicate.spans.push(duplicate.spans[0].clone());
        assert!(matches!(
            validate_presentation_timeline(&duplicate),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));

        let mut unknown = timeline();
        unknown.actions[0].position = PresentationTimelinePosition::SpanBoundary {
            span_id: 99,
            affinity: PresentationAffinity::After,
        };
        assert!(matches!(
            validate_presentation_timeline(&unknown),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));
    }

    #[test]
    fn validation_rejects_non_boundary_offsets_and_non_finite_values() {
        let mut bad_offset = timeline();
        bad_offset.actions[1].position = PresentationTimelinePosition::TextOffset {
            span_id: 1,
            utf8_offset: 4,
            affinity: PresentationAffinity::Before,
        };
        assert!(matches!(
            validate_presentation_timeline(&bad_offset),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));

        let mut nan = timeline();
        nan.spans[0].acss.rate = Some(f32::NAN);
        assert!(matches!(
            validate_presentation_timeline(&nan),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));
    }

    #[test]
    fn validation_rejects_invalid_or_ambiguous_rate_offsets() {
        let mut out_of_range = timeline();
        out_of_range.spans[0].acss.rate = None;
        out_of_range.spans[0].rate_offset = Some(MAX_RATE_OFFSET_POINTS + 1);
        assert!(matches!(
            validate_presentation_timeline(&out_of_range),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));

        let mut ambiguous = timeline();
        ambiguous.spans[0].rate_offset = Some(1);
        assert!(matches!(
            validate_presentation_timeline(&ambiguous),
            Err(PresentationTimelineError::InvalidTimeline(_))
        ));
    }

    #[test]
    fn encoded_payload_is_bounded_before_decoding() {
        let encoded = "A".repeat(MAX_TIMELINE_ENCODED_BYTES + 1);
        assert!(matches!(
            decode_presentation_timeline(&encoded),
            Err(PresentationTimelineError::PayloadTooLarge)
        ));
    }
}
