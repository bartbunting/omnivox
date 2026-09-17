use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::voice_choices::Adjustment;

fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Native units; booleans and integers remain distinct, including false and zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NativeValue {
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Enum(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueType {
    Integer {
        minimum: i64,
        maximum: i64,
        step: i64,
    },
    Number {
        minimum: f64,
        maximum: f64,
        step: f64,
    },
    Boolean,
    Enum {
        choices: Vec<EnumChoice>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnumChoice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterScope {
    Voice,
    Engine,
    Startup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityStatus {
    Supported,
    VoiceUnavailable,
    RuntimeUnsupported,
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterAvailability {
    pub status: AvailabilityStatus,
    #[serde(deserialize_with = "nullable")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultSource {
    RuntimeReadback,
    QualifiedProfile,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterDefault {
    pub source: DefaultSource,
    #[serde(deserialize_with = "nullable")]
    pub value: Option<NativeValue>,
    pub reset_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterDescriptor {
    pub id: String,
    pub label: String,
    pub help: String,
    pub group: String,
    pub order: u32,
    #[serde(deserialize_with = "nullable")]
    pub unit: Option<String>,
    pub value_type: ValueType,
    pub scope: ParameterScope,
    pub adjustable: bool,
    pub availability: ParameterAvailability,
    pub default: ParameterDefault,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommonInput {
    Rate,
    RateOffset,
    AveragePitch,
    PitchRange,
    Stress,
    Richness,
    Volume,
    Gain,
    LowPass,
    HighPass,
    Pan,
    Reverb,
    Echo,
    Chorus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommonMapping {
    pub common_inputs: Vec<CommonInput>,
    pub native_outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueIdentity {
    pub schema_id: String,
    pub profile_id: String,
    pub catalogue_revision: String,
    pub runtime_generation: u64,
}

/// Complete assembled catalogue. Pagination/control envelopes are a later layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterCatalogue {
    pub engine_id: String,
    pub identity: CatalogueIdentity,
    #[serde(deserialize_with = "nullable")]
    pub voice_id: Option<String>,
    pub parameters: Vec<ParameterDescriptor>,
    pub mappings: Vec<CommonMapping>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePatch {
    pub engine_id: String,
    pub schema_id: String,
    pub parameters: BTreeMap<String, Adjustment<NativeValue>>,
}
