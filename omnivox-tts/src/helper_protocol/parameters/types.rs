use serde::{Deserialize, Deserializer, Serialize};

use super::super::{HelperAudioFormat, HelperSynthesisSettings};
use crate::native_parameters::{
    CatalogueIdentity, CommonInput, CommonMapping, NativePatch, NativeValue, ParameterDescriptor,
    ValueOrigin,
};
use crate::{AnchorAffinity, RequestedAnchor};

pub(super) fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailablePolicy {
    Require,
    CommonOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceParameters {
    pub native: NativePatch,
    pub context_dimensions: Vec<CommonInput>,
    pub expected_identity: CatalogueIdentity,
    pub unavailable_policy: UnavailablePolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueQuery {
    pub engine_id: String,
    #[serde(deserialize_with = "nullable")]
    pub voice_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub cursor: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub expected_catalogue_revision: Option<String>,
}

// Do not change shared synthesis/physical-voice readers to strengthen a new wire shape.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAnchor {
    id: String,
    text_offset: u32,
    affinity: AnchorAffinity,
}
fn anchors<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RequestedAnchor>, D::Error> {
    Ok(Vec::<WireAnchor>::deserialize(d)?
        .into_iter()
        .map(|a| RequestedAnchor::new(a.id, a.text_offset, a.affinity))
        .collect())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExplanationSource {
    Draft {
        settings: HelperSynthesisSettings,
        #[serde(deserialize_with = "nullable")]
        voice_parameters: Option<Box<VoiceParameters>>,
    },
    Applied {
        plan_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestBody {
    Synthesize {
        text: String,
        settings: HelperSynthesisSettings,
        #[serde(deserialize_with = "anchors")]
        anchors: Vec<RequestedAnchor>,
        #[serde(deserialize_with = "nullable")]
        voice_parameters: Option<VoiceParameters>,
    },
    GetEngineParametersV1(CatalogueQuery),
    ExplainVoiceParametersV1 {
        source: ExplanationSource,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogueUnavailable {
    EngineUnavailable,
    VoiceUnavailable,
    NotDescribed,
    UnsupportedHelper,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CatalogueResult {
    Ready {
        identity: CatalogueIdentity,
        #[serde(deserialize_with = "nullable")]
        voice_id: Option<String>,
        parameters: Vec<ParameterDescriptor>,
        mappings: Vec<CommonMapping>,
        #[serde(deserialize_with = "nullable")]
        next_cursor: Option<String>,
    },
    Busy {
        retry_after_ms: u16,
    },
    Unavailable {
        reason: CatalogueUnavailable,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Planned,
    AdapterApplied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealizedVoice {
    pub engine_id: String,
    pub voice_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterEvidence {
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub value: Option<NativeValue>,
    pub origin: ValueOrigin,
    pub masked_native: bool,
    pub read_back: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplanationUnavailable {
    PlanExpired,
    NativeUnavailable,
    VoiceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExplanationResult {
    Ready {
        evidence: Evidence,
        #[serde(deserialize_with = "nullable")]
        plan_id: Option<String>,
        realized: RealizedVoice,
        identity: CatalogueIdentity,
        parameters: Vec<ParameterEvidence>,
    },
    Busy {
        retry_after_ms: u16,
    },
    Unavailable {
        reason: ExplanationUnavailable,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationStatus {
    Applied,
    CommonOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeApplication {
    pub status: ApplicationStatus,
    #[serde(deserialize_with = "nullable")]
    pub plan_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub identity: Option<CatalogueIdentity>,
    pub masked_parameters: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseBody {
    SynthesisStarted {
        format: HelperAudioFormat,
        actual_voice_id: String,
        #[serde(deserialize_with = "nullable")]
        native_application: Option<NativeApplication>,
    },
    EngineParametersV1 {
        engine_id: String,
        result: CatalogueResult,
    },
    VoiceParametersExplainedV1 {
        result: ExplanationResult,
    },
}
