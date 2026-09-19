//! Read-only explanations of one private choice or a retained application.
use crate::control::ChoiceFallbackPolicy;
use crate::helper_protocol::parameters as helper;
use crate::native_parameters::CatalogueIdentity;
use crate::voice_choices::{required_nullable, VoiceStylePatch};
use crate::voice_preview_v2::{VoicePlacement, VoicePreviewSelection};
use crate::voice_preview_v3::{PrivateNativePreviewVoice, VoicePreviewRequestV3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationRequest {
    pub source: ExplanationSource,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExplanationSource {
    Draft {
        voice: Box<PrivateNativePreviewVoice>,
        context: VoiceStylePatch,
        placement: VoicePlacement,
        selection: VoicePreviewSelection,
        fallback_policy: Box<ChoiceFallbackPolicy>,
        disabled_engine_ids: Vec<String>,
        #[serde(deserialize_with = "required_nullable")]
        expected_base_rate: Option<f32>,
    },
    Applied {
        plan_id: String,
    },
}
impl ExplanationSource {
    /// Reuse private admission without providing text for routing or synthesis.
    pub fn preview_inputs(&self) -> Option<VoicePreviewRequestV3> {
        match self {
            Self::Draft {
                voice,
                context,
                placement,
                selection,
                fallback_policy,
                disabled_engine_ids,
                expected_base_rate,
            } => Some(VoicePreviewRequestV3 {
                text: "validation only".into(),
                voice: voice.as_ref().clone(),
                context: context.clone(),
                placement: placement.clone(),
                selection: selection.clone(),
                fallback_policy: fallback_policy.as_ref().clone(),
                disabled_engine_ids: disabled_engine_ids.clone(),
                expected_base_rate: *expected_base_rate,
            }),
            Self::Applied { .. } => None,
        }
    }
    pub fn validate(&self, base_rate: f32) -> Result<(), String> {
        if let Some(request) = self.preview_inputs() {
            if request.validate(base_rate)?.is_none() {
                return Err("Draft explanations require one selected choice".into());
            }
        } else if let Self::Applied { plan_id } = self {
            validate_helper_source(&helper::ExplanationSource::Applied {
                plan_id: plan_id.clone(),
            })?;
        }
        Ok(())
    }
}
pub fn validate_helper_source(source: &helper::ExplanationSource) -> Result<(), String> {
    helper::Request {
        protocol_version: helper::PROTOCOL_VERSION,
        request_id: 1,
        body: helper::RequestBody::ExplainVoiceParametersV1 {
            source: source.clone(),
        },
    }
    .validate()
    .map_err(|e| e.to_string())
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationResponse {
    pub result: ExplanationResult,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExplanationResult {
    Ready {
        evidence: helper::Evidence,
        #[serde(deserialize_with = "required_nullable")]
        plan_id: Option<String>,
        choice_id: String,
        realized: helper::RealizedVoice,
        identity: CatalogueIdentity,
        parameters: Vec<helper::ParameterEvidence>,
    },
    Busy {
        retry_after_ms: u16,
    },
    Unavailable {
        reason: helper::ExplanationUnavailable,
        message: String,
    },
}
impl ExplanationResult {
    pub fn from_helper(
        result: helper::ExplanationResult,
        choice_id: String,
        public_plan: Option<String>,
    ) -> Self {
        match result {
            helper::ExplanationResult::Ready {
                evidence,
                realized,
                identity,
                parameters,
                ..
            } => Self::Ready {
                evidence,
                plan_id: public_plan,
                choice_id,
                realized,
                identity,
                parameters,
            },
            helper::ExplanationResult::Busy { retry_after_ms } => Self::Busy { retry_after_ms },
            helper::ExplanationResult::Unavailable { reason, message } => {
                Self::Unavailable { reason, message }
            }
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let helper = match self {
            Self::Ready {
                evidence,
                plan_id,
                choice_id,
                realized,
                identity,
                parameters,
            } => {
                if choice_id.is_empty()
                    || choice_id.len() > 128
                    || !choice_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
                {
                    return Err("invalid explanation choice ID".into());
                }
                helper::ExplanationResult::Ready {
                    evidence: *evidence,
                    plan_id: plan_id.clone(),
                    realized: realized.clone(),
                    identity: identity.clone(),
                    parameters: parameters.clone(),
                }
            }
            Self::Busy { retry_after_ms } => helper::ExplanationResult::Busy {
                retry_after_ms: *retry_after_ms,
            },
            Self::Unavailable { reason, message } => helper::ExplanationResult::Unavailable {
                reason: *reason,
                message: message.clone(),
            },
        };
        helper.validate().map_err(|e| e.to_string())
    }
}
#[cfg(test)]
mod tests;
