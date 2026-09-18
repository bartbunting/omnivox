//! Strict native private previews and bounded accepted/started evidence.
use crate::contracts::{AcssDimension, PhysicalVoiceId, PostSynthesisDimension};
use crate::control::{
    format_control_event, ChoiceFallbackPolicy, ControlCodecError, ControlResponse,
    ControlResponseEnvelope, PreviewStatus, CONTROL_PROTOCOL_VERSION,
};
use crate::engine_voice_choices::{EngineLayeredVoiceDefinition, EngineVoiceChoice};
use crate::native_synthesis::NativeApplication;
use crate::resolver::ResolutionReason;
use crate::voice_choices::{
    required_nullable, AudioChoiceIdentity, SharedVoiceStyle, VoiceStylePatch,
};
use crate::voice_preview_v2::{
    PrivatePreviewVoice, VoicePlacement, VoicePreviewRequestV2, VoicePreviewSelection,
    MAX_ACCEPTED_AUDIO_CHOICES, MAX_PREVIEW_MESSAGE_BYTES, PRIVATE_PREVIEW_VOICE_ID,
};
use serde::{Deserialize, Serialize};

pub const MAX_NATIVE_AUDIO_CHOICE_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateNativePreviewVoice {
    #[serde(deserialize_with = "required_nullable")]
    pub language: Option<String>,
    pub shared: SharedVoiceStyle,
    pub choices: Vec<EngineVoiceChoice>,
}
impl PrivateNativePreviewVoice {
    pub fn definition(&self) -> EngineLayeredVoiceDefinition {
        EngineLayeredVoiceDefinition {
            id: PRIVATE_PREVIEW_VOICE_ID.into(),
            language: self.language.clone(),
            shared: self.shared.clone(),
            choices: self.choices.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePreviewRequestV3 {
    pub text: String,
    pub voice: PrivateNativePreviewVoice,
    pub context: VoiceStylePatch,
    pub placement: VoicePlacement,
    pub selection: VoicePreviewSelection,
    pub fallback_policy: ChoiceFallbackPolicy,
    pub disabled_engine_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_base_rate: Option<f32>,
}
impl VoicePreviewRequestV3 {
    pub fn validate(&self, base_rate: f32) -> Result<Option<usize>, String> {
        let definition = self.voice.definition();
        definition.validate().map_err(|e| e.to_string())?;
        // This projection reuses only common input validation; it never executes.
        let common = definition.common_projection();
        VoicePreviewRequestV2 {
            text: self.text.clone(),
            voice: PrivatePreviewVoice {
                language: common.language,
                shared: common.shared,
                choices: common.choices,
            },
            context: self.context.clone(),
            placement: self.placement.clone(),
            selection: self.selection.clone(),
            fallback_policy: self.fallback_policy.clone(),
            disabled_engine_ids: self.disabled_engine_ids.clone(),
            expected_base_rate: self.expected_base_rate,
        }
        .validate(base_rate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAudioChoiceIdentity {
    #[serde(deserialize_with = "required_nullable")]
    pub choice_id: Option<String>,
    pub reason: ResolutionReason,
    pub realized: PhysicalVoiceId,
    pub degraded_acss: Vec<AcssDimension>,
    pub degraded_effects: Vec<PostSynthesisDimension>,
    #[serde(deserialize_with = "required_nullable")]
    pub native_application: Option<NativeApplication>,
}
impl NativeAudioChoiceIdentity {
    pub fn from_parts(
        identity: AudioChoiceIdentity,
        native_application: Option<NativeApplication>,
    ) -> Self {
        Self {
            choice_id: identity.choice_id,
            reason: identity.reason,
            realized: identity.realized,
            degraded_acss: identity.degraded_acss,
            degraded_effects: identity.degraded_effects,
            native_application,
        }
    }
    pub fn common(&self) -> AudioChoiceIdentity {
        AudioChoiceIdentity {
            choice_id: self.choice_id.clone(),
            reason: self.reason.clone(),
            realized: self.realized.clone(),
            degraded_acss: self.degraded_acss.clone(),
            degraded_effects: self.degraded_effects.clone(),
        }
    }
    /// Checked by the producer before any PCM can be accepted.
    pub fn validate(&self) -> Result<(), String> {
        crate::voice_preview_v2::validate_audio_choice_identity(&self.common())?;
        if let Some(application) = &self.native_application {
            application.validate().map_err(|e| e.to_string())?;
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len()
            > MAX_NATIVE_AUDIO_CHOICE_BYTES
        {
            return Err("native voice evidence exceeds the output budget".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedNativeAudioChoice {
    #[serde(deserialize_with = "required_nullable")]
    pub choice_id: Option<String>,
    pub reason: ResolutionReason,
    pub realized: PhysicalVoiceId,
    pub degraded_acss: Vec<AcssDimension>,
    pub degraded_effects: Vec<PostSynthesisDimension>,
    #[serde(deserialize_with = "required_nullable")]
    pub native_application: Option<NativeApplication>,
    pub playback_started: bool,
}
impl From<(NativeAudioChoiceIdentity, bool)> for AcceptedNativeAudioChoice {
    fn from((identity, playback_started): (NativeAudioChoiceIdentity, bool)) -> Self {
        Self {
            choice_id: identity.choice_id,
            reason: identity.reason,
            realized: identity.realized,
            degraded_acss: identity.degraded_acss,
            degraded_effects: identity.degraded_effects,
            native_application: identity.native_application,
            playback_started,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePreviewResponseV3 {
    pub status: PreviewStatus,
    pub accepted_audio: Vec<AcceptedNativeAudioChoice>,
    pub accepted_audio_truncated: bool,
    #[serde(deserialize_with = "required_nullable")]
    pub last_started: Option<NativeAudioChoiceIdentity>,
    #[serde(deserialize_with = "required_nullable")]
    pub message: Option<String>,
    pub base_rate: f32,
    pub effective_disabled_engine_ids: Vec<String>,
}

impl VoicePreviewResponseV3 {
    pub fn validate(&self) -> Result<(), String> {
        if !self.base_rate.is_finite()
            || !(0.0..=2.0).contains(&self.base_rate)
            || self.accepted_audio.len() > MAX_ACCEPTED_AUDIO_CHOICES
            || self
                .message
                .as_ref()
                .is_some_and(|m| m.len() > MAX_PREVIEW_MESSAGE_BYTES)
        {
            return Err("invalid native preview terminal bounds".into());
        }
        for accepted in &self.accepted_audio {
            NativeAudioChoiceIdentity {
                choice_id: accepted.choice_id.clone(),
                reason: accepted.reason.clone(),
                realized: accepted.realized.clone(),
                degraded_acss: accepted.degraded_acss.clone(),
                degraded_effects: accepted.degraded_effects.clone(),
                native_application: accepted.native_application.clone(),
            }
            .validate()?;
        }
        if let Some(last) = &self.last_started {
            last.validate()?;
        }
        Ok(())
    }

    /// Reserve room for a bounded identity and maximally escaped diagnostic
    /// before admission. Acceptance entries can later be shortened independently.
    pub fn validate_metadata_budget(base_rate: f32, disabled: &[String]) -> Result<(), String> {
        let envelope = ControlResponseEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: Some(u64::MAX),
            response: ControlResponse::PreviewVoiceCompletedV3(Self {
                status: PreviewStatus::Cancelled,
                accepted_audio: Vec::new(),
                accepted_audio_truncated: false,
                last_started: None,
                message: None,
                base_rate,
                effective_disabled_engine_ids: disabled.to_vec(),
            }),
        };
        let fixed = serde_json::to_vec(&envelope)
            .map_err(|error| error.to_string())?
            .len();
        if fixed
            .saturating_add(MAX_NATIVE_AUDIO_CHOICE_BYTES)
            .saturating_add(6 * MAX_PREVIEW_MESSAGE_BYTES)
            > crate::control::MAX_CONTROL_PAYLOAD_BYTES
        {
            return Err(
                "preview frozen metadata leaves insufficient terminal output space".to_owned(),
            );
        }
        Ok(())
    }

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
            response: ControlResponse::PreviewVoiceCompletedV3(self),
        };
        loop {
            match format_control_event(&envelope) {
                Ok(record) => return Ok(record),
                Err(ControlCodecError::PayloadTooLarge) => {
                    let ControlResponse::PreviewVoiceCompletedV3(response) = &mut envelope.response
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
mod tests;
