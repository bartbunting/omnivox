//! Bounded Base64-JSON transport for structured Emacsvox presentations.

use std::collections::HashSet;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::{NormalizedAcss, PostSynthesisStyle};

/// Current structured presentation-timeline protocol version.
pub const PRESENTATION_TIMELINE_PROTOCOL_VERSION: u32 = 1;
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

/// One atomic, replaceable presentation and its tracked playback identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationTimelineEnvelope {
    pub protocol_version: u32,
    pub generation: u64,
    pub dispatch_id: u64,
    pub spans: Vec<PresentationSpeechSpan>,
    #[serde(default)]
    pub actions: Vec<PresentationTimelineAction>,
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
        #[serde(default)]
        effect_bus: PresentationEffectBus,
    },
    Tone {
        frequency_hz: f32,
        duration_ms: u32,
        mode: PresentationAudioMode,
        #[serde(default = "unity")]
        volume: f32,
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
    invalid_if(
        timeline.protocol_version != PRESENTATION_TIMELINE_PROTOCOL_VERSION,
        format!("unsupported protocol version {}", timeline.protocol_version),
    )?;
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
        PresentationAction::Audio { path, volume, .. } => {
            invalid_if(
                path.is_empty(),
                format!("audio action {} has an empty path", action.id),
            )?;
            invalid_if(
                path.len() > MAX_TIMELINE_RESOURCE_PATH_BYTES,
                format!("audio action {} path is too long", action.id),
            )?;
            validate_normalized(*volume, &format!("audio action {} volume", action.id))
        }
        PresentationAction::Tone {
            frequency_hz,
            duration_ms,
            volume,
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
            validate_normalized(*volume, &format!("tone action {} volume", action.id))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline() -> PresentationTimelineEnvelope {
        PresentationTimelineEnvelope {
            protocol_version: PRESENTATION_TIMELINE_PROTOCOL_VERSION,
            generation: 7,
            dispatch_id: 19,
            spans: vec![PresentationSpeechSpan {
                id: 1,
                text: "café 日本".to_owned(),
                logical_voice_id: Some("comment".to_owned()),
                acss: NormalizedAcss {
                    rate: Some(0.7),
                    ..NormalizedAcss::default()
                },
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
        let timeline = timeline();
        let encoded = encode_presentation_timeline(&timeline).unwrap();

        assert_eq!(decode_presentation_timeline(&encoded).unwrap(), timeline);
        assert_eq!(
            decode_presentation_timeline(&format!("{{{encoded}}}")).unwrap(),
            timeline
        );
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
    fn encoded_payload_is_bounded_before_decoding() {
        let encoded = "A".repeat(MAX_TIMELINE_ENCODED_BYTES + 1);
        assert!(matches!(
            decode_presentation_timeline(&encoded),
            Err(PresentationTimelineError::PayloadTooLarge)
        ));
    }
}
