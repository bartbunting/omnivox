//! Strict private layered previews and bounded terminal evidence.

use serde::{Deserialize, Serialize};

use crate::contracts::{AcssDimension, PhysicalVoiceId, PostSynthesisDimension};
use crate::control::{
    format_control_event, ChoiceFallbackPolicy, ControlCodecError, ControlResponse,
    ControlResponseEnvelope, PreviewStatus, CONTROL_PROTOCOL_VERSION, MAX_PREVIEW_TEXT_BYTES,
};
use crate::logical_voices::validate_registration;
use crate::resolver::ResolutionReason;
use crate::routing_policy::{RoutingPolicy, RoutingPolicyRegistry};
use crate::voice_choices::{
    AudioChoiceIdentity, LayeredVoiceDefinition, SharedVoiceStyle, VoiceChoice, VoiceStylePatch,
};

pub const PRIVATE_PREVIEW_VOICE_ID: &str = "__omnivox_preview__";
pub const MAX_ACCEPTED_AUDIO_CHOICES: usize = 32;
pub const MAX_PREVIEW_MESSAGE_BYTES: usize = 1024;

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePlacement {
    #[serde(deserialize_with = "required_nullable")]
    pub pan: Option<f32>,
}

impl VoicePlacement {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .pan
            .is_some_and(|pan| !pan.is_finite() || !(0.0..=1.0).contains(&pan))
        {
            return Err("placement pan must be null or finite in [0, 1]".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoicePreviewSelection {
    Automatic {},
    Choice { choice_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivatePreviewVoice {
    #[serde(deserialize_with = "required_nullable")]
    pub language: Option<String>,
    pub shared: SharedVoiceStyle,
    pub choices: Vec<VoiceChoice>,
}

impl PrivatePreviewVoice {
    pub fn definition(&self) -> LayeredVoiceDefinition {
        LayeredVoiceDefinition {
            id: PRIVATE_PREVIEW_VOICE_ID.to_owned(),
            language: self.language.clone(),
            shared: self.shared.clone(),
            choices: self.choices.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePreviewRequestV2 {
    pub text: String,
    pub voice: PrivatePreviewVoice,
    pub context: VoiceStylePatch,
    pub placement: VoicePlacement,
    pub selection: VoicePreviewSelection,
    pub fallback_policy: ChoiceFallbackPolicy,
    pub disabled_engine_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_base_rate: Option<f32>,
}

impl VoicePreviewRequestV2 {
    /// Validate all private inputs before admission, including policies unused
    /// by an individual audition. The returned index remains in the full chain.
    pub fn validate(&self, base_rate: f32) -> Result<Option<usize>, String> {
        if self.text.is_empty() || self.text.len() > MAX_PREVIEW_TEXT_BYTES {
            return Err(format!(
                "preview text must contain 1..={MAX_PREVIEW_TEXT_BYTES} UTF-8 bytes"
            ));
        }
        if !base_rate.is_finite() || !(0.0..=2.0).contains(&base_rate) {
            return Err("base rate must be finite in [0, 2]".to_owned());
        }
        if let Some(expected) = self.expected_base_rate {
            if !expected.is_finite() || !(0.0..=2.0).contains(&expected) {
                return Err("expected base rate must be finite in [0, 2]".to_owned());
            }
            if expected != base_rate {
                return Err("Comparison rate changed; restart comparison".to_owned());
            }
        }
        let definition = self.voice.definition();
        definition.validate().map_err(|error| error.to_string())?;
        self.context.validate().map_err(|error| error.to_string())?;
        self.placement.validate()?;
        validate_registration(&[], &self.fallback_policy.clone().into())
            .map_err(|error| error.to_string())?;
        RoutingPolicyRegistry::new("preview")
            .register(
                1,
                RoutingPolicy {
                    disabled_engine_ids: self.disabled_engine_ids.clone(),
                    ..RoutingPolicy::default()
                },
            )
            .map_err(|error| error.to_string())?;
        match &self.selection {
            VoicePreviewSelection::Automatic {} => Ok(None),
            VoicePreviewSelection::Choice { choice_id } => definition
                .choices
                .iter()
                .position(|choice| choice.id == *choice_id)
                .map(Some)
                .ok_or_else(|| "preview choice ID is absent from the draft".to_owned()),
        }
    }
}

/// Flat wire identity with its independently observed playback flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedAudioChoice {
    #[serde(deserialize_with = "required_nullable")]
    pub choice_id: Option<String>,
    pub reason: ResolutionReason,
    pub realized: PhysicalVoiceId,
    pub degraded_acss: Vec<AcssDimension>,
    pub degraded_effects: Vec<PostSynthesisDimension>,
    pub playback_started: bool,
}

impl From<(AudioChoiceIdentity, bool)> for AcceptedAudioChoice {
    fn from((identity, playback_started): (AudioChoiceIdentity, bool)) -> Self {
        Self {
            choice_id: identity.choice_id,
            reason: identity.reason,
            realized: identity.realized,
            degraded_acss: identity.degraded_acss,
            degraded_effects: identity.degraded_effects,
            playback_started,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePreviewResponseV2 {
    pub status: PreviewStatus,
    pub accepted_audio: Vec<AcceptedAudioChoice>,
    pub accepted_audio_truncated: bool,
    #[serde(deserialize_with = "required_nullable")]
    pub last_started: Option<AudioChoiceIdentity>,
    #[serde(deserialize_with = "required_nullable")]
    pub message: Option<String>,
    pub base_rate: f32,
    pub effective_disabled_engine_ids: Vec<String>,
}

impl VoicePreviewResponseV2 {
    /// Shrink only the acceptance list and diagnostic text. Never lose the
    /// independent last-started identity, frozen inputs or typed status.
    pub fn bounded_event(mut self, request_id: u64) -> Result<String, ControlCodecError> {
        if self.accepted_audio.len() > MAX_ACCEPTED_AUDIO_CHOICES {
            self.accepted_audio.truncate(MAX_ACCEPTED_AUDIO_CHOICES);
            self.accepted_audio_truncated = true;
        }
        if let Some(message) = &mut self.message {
            let mut end = message.len().min(MAX_PREVIEW_MESSAGE_BYTES);
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        let mut envelope = ControlResponseEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: Some(request_id),
            response: ControlResponse::PreviewVoiceCompletedV2(self),
        };
        loop {
            match format_control_event(&envelope) {
                Ok(record) => return Ok(record),
                Err(ControlCodecError::PayloadTooLarge) => {
                    let ControlResponse::PreviewVoiceCompletedV2(response) = &mut envelope.response
                    else {
                        unreachable!()
                    };
                    if response.accepted_audio.pop().is_none() {
                        return Err(ControlCodecError::PayloadTooLarge);
                    }
                    response.accepted_audio_truncated = true;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{decode_request, decode_response, ControlRequest, CONTROL_EVENT_PREFIX};
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde_json::{json, Value};

    fn fixture(name: &str) -> Value {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/voice-choice-tuning.json"
        ))
        .unwrap();
        fixture["messages"][name].clone()
    }

    fn request(value: &Value) -> Result<VoicePreviewRequestV2, String> {
        let decoded = decode_request(&STANDARD.encode(value.to_string()))
            .map_err(|error| error.to_string())?;
        let ControlRequest::PreviewVoiceV2(request) = decoded.request else {
            panic!("wrong request")
        };
        Ok(request)
    }

    #[test]
    fn independent_preview_fixture_roundtrips_and_preserves_original_choice_index() {
        let value = fixture("preview");
        let mut preview = request(&value).unwrap();
        assert_eq!(preview.validate(0.65).unwrap(), None);
        let roundtrip: VoicePreviewRequestV2 =
            serde_json::from_str(&serde_json::to_string(&preview).unwrap()).unwrap();
        assert_eq!(roundtrip, preview);
        assert_eq!(preview.voice.choices.len(), 3);
        assert_eq!(preview.voice.choices[1].id, "eloquence-reed");
        preview.selection = VoicePreviewSelection::Choice {
            choice_id: "eloquence-reed-soft".to_owned(),
        };
        assert_eq!(preview.validate(0.65).unwrap(), Some(2));
        assert!(preview.validate(0.66).is_err());
        preview.selection = VoicePreviewSelection::Choice {
            choice_id: "missing".to_owned(),
        };
        assert!(preview.validate(0.65).is_err());
    }

    #[test]
    fn new_preview_rejects_missing_extension_duplicate_and_invalid_nested_fields() {
        let original = fixture("preview");
        for field in [
            "text",
            "voice",
            "context",
            "placement",
            "selection",
            "fallback_policy",
            "disabled_engine_ids",
        ] {
            let mut bad = original.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(request(&bad).is_err(), "{field}");
        }
        for pointer in [
            "",
            "/voice",
            "/context",
            "/placement",
            "/selection",
            "/fallback_policy",
            "/voice/choices/0/selector",
        ] {
            let mut bad = original.clone();
            bad.pointer_mut(pointer).unwrap()["extra"] = json!(true);
            assert!(request(&bad).is_err(), "{pointer}");
        }
        for (pointer, value) in [
            ("/context/richness", Value::Null),
            ("/placement", json!({})),
            (
                "/selection",
                json!({"mode":"automatic", "choice_id":"dectalk-paul"}),
            ),
            ("/voice/shared/acss", json!({})),
        ] {
            let mut bad = original.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(request(&bad).is_err(), "{pointer}");
        }
        let text = original.to_string().replacen(
            "\"request_id\":702",
            "\"request_id\":702,\"request_id\":703",
            1,
        );
        assert!(decode_request(&STANDARD.encode(text)).is_err());
        for (pointer, value) in [
            ("/placement/pan", json!(1.1)),
            ("/context/richness/value", json!(-0.1)),
            ("/voice/shared/rate_offset", json!(21)),
            ("/disabled_engine_ids", json!(["bad id"])),
            ("/expected_base_rate", json!(3)),
        ] {
            let mut bad = original.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(request(&bad).unwrap().validate(0.65).is_err(), "{pointer}");
        }
    }

    #[test]
    fn terminal_fixture_roundtrips_and_requires_playback_flag() {
        let value = fixture("preview_completed");
        let envelope: ControlResponseEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&serde_json::to_string(&envelope).unwrap()).unwrap(),
            value
        );
        let mut bad = value;
        bad["accepted_audio"][0]
            .as_object_mut()
            .unwrap()
            .remove("playback_started");
        assert!(serde_json::from_value::<ControlResponseEnvelope>(bad).is_err());
    }

    #[test]
    fn terminal_budget_keeps_status_last_started_and_utf8_message() {
        let envelope: ControlResponseEnvelope =
            serde_json::from_value(fixture("preview_completed")).unwrap();
        let ControlResponse::PreviewVoiceCompletedV2(mut response) = envelope.response else {
            panic!("wrong response")
        };
        let last = response.last_started.clone();
        let mut big = response.accepted_audio[0].clone();
        big.realized.voice_id = "\u{1}".repeat(4096);
        response.accepted_audio = (0..32)
            .map(|index| {
                let mut item = big.clone();
                item.choice_id = Some(format!("row-{index}"));
                item
            })
            .collect();
        response.status = PreviewStatus::Failed;
        response.message = Some("é".repeat(600));
        let record = response.bounded_event(702).unwrap();
        assert!(record.len() < 512 * 1024);
        let payload = record.strip_prefix(CONTROL_EVENT_PREFIX).unwrap().trim();
        let decoded = decode_response(payload).unwrap();
        let ControlResponse::PreviewVoiceCompletedV2(response) = decoded.response else {
            panic!("wrong response")
        };
        assert_eq!(response.status, PreviewStatus::Failed);
        assert_eq!(response.last_started, last);
        assert!(response.accepted_audio_truncated);
        assert!(response.accepted_audio.len() < 32);
        assert_eq!(
            response.accepted_audio[0].choice_id.as_deref(),
            Some("row-0")
        );
        assert_eq!(response.message.unwrap(), "é".repeat(512));
    }
}
