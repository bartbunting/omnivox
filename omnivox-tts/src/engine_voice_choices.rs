//! Native choice admission and actual-choice preparation, without engine I/O.
//!
//! Public registration uses these types; native speech integration remains separate.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::contracts::{EngineDescriptor, PhysicalVoiceId, VoiceSelector};
use crate::native_parameters::{
    AvailabilityStatus, NativePatch, ParameterCatalogue, ParameterError, ParameterScope,
};
use crate::native_synthesis::{UnavailablePolicy, VoiceParameters};
use crate::resolver::VoiceResolution;
use crate::voice_choices::{
    Adjustment, ChoiceTuningError, ComposedVoiceStyle, LayeredVoiceDefinition,
    RegisteredVoiceDefinition, SharedVoiceStyle, VoiceChoice, VoiceStylePatch,
};

#[derive(Debug, thiserror::Error)]
pub enum NativeChoiceError {
    #[error(transparent)]
    Common(#[from] ChoiceTuningError),
    #[error(transparent)]
    Native(#[from] ParameterError),
    #[error("native voice settings unavailable: {0}")]
    Unavailable(&'static str),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineVoiceChoice {
    pub id: String,
    #[serde(deserialize_with = "crate::voice_choices::choice_selector")]
    pub selector: VoiceSelector,
    pub adjustments: VoiceStylePatch,
    #[serde(deserialize_with = "crate::voice_choices::required_nullable")]
    pub native: Option<NativePatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineLayeredVoiceDefinition {
    pub id: String,
    #[serde(deserialize_with = "crate::voice_choices::required_nullable")]
    pub language: Option<String>,
    pub shared: SharedVoiceStyle,
    pub choices: Vec<EngineVoiceChoice>,
}

/// Separate reader: the existing v2 definition reader cannot accept this new mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "definition",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EngineRegisteredVoiceDefinition {
    Legacy(
        #[serde(deserialize_with = "legacy_definition")] crate::contracts::LogicalVoiceDefinition,
    ),
    Layered(LayeredVoiceDefinition),
    EngineLayered(EngineLayeredVoiceDefinition),
}

// New tagged forms are strict without changing the legacy request readers.
fn legacy_definition<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<crate::contracts::LogicalVoiceDefinition, D::Error> {
    use serde::de::Error;
    let value = serde_json::Value::deserialize(d)?;
    let check = |object: &serde_json::Value, keys: &[&str]| -> Result<(), D::Error> {
        let object = object
            .as_object()
            .ok_or_else(|| D::Error::custom("expected legacy object"))?;
        if object.keys().any(|key| !keys.contains(&key.as_str())) {
            return Err(D::Error::custom("unknown legacy definition field"));
        }
        Ok(())
    };
    check(
        &value,
        &["id", "language", "preferences", "acss", "effects"],
    )?;
    check(
        &value["acss"],
        &[
            "rate",
            "average_pitch",
            "pitch_range",
            "stress",
            "richness",
            "volume",
        ],
    )?;
    if let Some(effects) = value.get("effects") {
        check(
            effects,
            &[
                "gain",
                "low_pass",
                "high_pass",
                "pan",
                "reverb",
                "echo",
                "chorus",
            ],
        )?;
    }
    #[derive(Deserialize)]
    struct Selector(
        #[serde(deserialize_with = "crate::voice_choices::choice_selector")] VoiceSelector,
    );
    let selectors = value["preferences"]
        .as_array()
        .ok_or_else(|| D::Error::custom("expected preference array"))?;
    for selector in selectors {
        let Selector(_selector) =
            serde_json::from_value(selector.clone()).map_err(D::Error::custom)?;
    }
    serde_json::from_value(value).map_err(D::Error::custom)
}

impl From<EngineRegisteredVoiceDefinition> for RegisteredVoiceDefinition {
    fn from(value: EngineRegisteredVoiceDefinition) -> Self {
        match value {
            EngineRegisteredVoiceDefinition::Legacy(v) => Self::Legacy(v),
            EngineRegisteredVoiceDefinition::Layered(v) => Self::Layered(v),
            EngineRegisteredVoiceDefinition::EngineLayered(v) => Self::EngineLayered(v),
        }
    }
}

/// Complete v3 registry replacement, shared by internal and public admission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceRegistrationV3 {
    pub registry_generation: u64,
    pub definitions: Vec<EngineRegisteredVoiceDefinition>,
    pub fallback_policy: crate::control::ChoiceFallbackPolicy,
}

impl VoiceRegistrationV3 {
    pub fn from_json(json: &[u8]) -> Result<Self, NativeChoiceError> {
        let result: Self = crate::native_parameters::decode(json)?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), NativeChoiceError> {
        if self.registry_generation == 0
            || self.definitions.len() > crate::logical_voices::MAX_LOGICAL_VOICES
        {
            return Err(
                ChoiceTuningError::Invalid("invalid native registry generation/count").into(),
            );
        }
        let mut projection = Vec::with_capacity(self.definitions.len());
        for definition in &self.definitions {
            match definition {
                EngineRegisteredVoiceDefinition::Legacy(v) => {
                    // The new reader rejects nonfinite values, including JSON
                    // numbers that overflow f32. Legacy APIs retain their clamps.
                    if [
                        v.acss.rate,
                        v.acss.average_pitch,
                        v.acss.pitch_range,
                        v.acss.stress,
                        v.acss.richness,
                        v.acss.volume,
                        v.effects.gain,
                        v.effects.low_pass,
                        v.effects.high_pass,
                        v.effects.pan,
                        v.effects.reverb,
                        v.effects.echo,
                        v.effects.chorus,
                    ]
                    .into_iter()
                    .flatten()
                    .any(|value| !value.is_finite())
                    {
                        return Err(ParameterError::Invalid("nonfinite legacy style value").into());
                    }
                    projection.push(v.clone());
                }
                EngineRegisteredVoiceDefinition::Layered(v) => {
                    v.validate()?;
                    projection.push(v.legacy_projection());
                }
                EngineRegisteredVoiceDefinition::EngineLayered(v) => {
                    v.validate()?;
                    projection.push(v.common_projection().legacy_projection());
                }
            }
        }
        crate::logical_voices::validate_registration(
            &projection,
            &self.fallback_policy.clone().into(),
        )
        .map_err(ChoiceTuningError::from)?;
        if serde_json::to_vec(self)
            .map_err(|_| ParameterError::Invalid("invalid registration values"))?
            .len()
            > crate::control::MAX_CONTROL_PAYLOAD_BYTES
        {
            return Err(ParameterError::Invalid("native registry exceeds control bound").into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSupport {
    Supported,
    Deferred,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeChoiceStatus {
    pub logical_voice_id: String,
    pub choice_id: String,
    pub status: NativeSupport,
    pub reason: Option<String>,
}

/// Metadata supplied by the connection owner, never fetched by registration.
/// Busy/unqueried engines are deferred; explicit unsupported/unavailable replies
/// remain unavailable. A ready entry contains one complete, validated catalogue.
#[derive(Debug, Clone, Copy)]
pub enum ParameterKnowledge<'a> {
    Ready(&'a ParameterCatalogue),
    Deferred { engine_id: &'a str },
    Unavailable { engine_id: &'a str },
}

/// Borrowed immutable metadata for one connection/runtime observation.
/// Do not combine entries from different connections or retain this as saved state.
pub struct NativeCatalogueSnapshot<'a> {
    inventory: &'a [EngineDescriptor],
    ready: BTreeMap<(&'a str, Option<&'a str>), &'a ParameterCatalogue>,
    unavailable: BTreeMap<&'a str, NativeSupport>,
}

impl<'a> NativeCatalogueSnapshot<'a> {
    pub fn new(
        inventory: &'a [EngineDescriptor],
        entries: &[ParameterKnowledge<'a>],
    ) -> Result<Self, NativeChoiceError> {
        let mut result = Self {
            inventory,
            ready: BTreeMap::new(),
            unavailable: BTreeMap::new(),
        };
        let mut generations = BTreeMap::new();
        for entry in entries {
            match *entry {
                ParameterKnowledge::Ready(c) => {
                    c.validate()?;
                    if result
                        .ready
                        .insert((&c.engine_id, c.voice_id.as_deref()), c)
                        .is_some()
                        || generations
                            .insert(c.engine_id.as_str(), c.identity.runtime_generation)
                            .is_some_and(|old| old != c.identity.runtime_generation)
                    {
                        return Err(
                            ParameterError::Invalid("conflicting catalogue snapshots").into()
                        );
                    }
                }
                ParameterKnowledge::Deferred { engine_id }
                | ParameterKnowledge::Unavailable { engine_id } => {
                    let status = if matches!(entry, ParameterKnowledge::Deferred { .. }) {
                        NativeSupport::Deferred
                    } else {
                        NativeSupport::Unavailable
                    };
                    if result.unavailable.insert(engine_id, status).is_some() {
                        return Err(ParameterError::Invalid("duplicate engine knowledge").into());
                    }
                }
            }
        }
        if result
            .ready
            .keys()
            .any(|(engine, _)| result.unavailable.contains_key(engine))
        {
            return Err(ParameterError::Invalid("conflicting engine knowledge").into());
        }
        Ok(result)
    }

    pub(crate) fn inventory(&self) -> &[EngineDescriptor] {
        self.inventory
    }

    fn catalogue(&self, engine: &str, voice: Option<&str>) -> Option<&ParameterCatalogue> {
        self.ready
            .get(&(engine, voice))
            .or_else(|| self.ready.get(&(engine, None)))
            .copied()
    }

    fn qualify(
        &self,
        patch: &NativePatch,
        selector: &VoiceSelector,
    ) -> Result<
        (
            NativeSupport,
            Option<&'static str>,
            Option<&ParameterCatalogue>,
        ),
        NativeChoiceError,
    > {
        let engine = self.inventory.iter().find(|e| e.id == patch.engine_id);
        let Some(engine) = engine.filter(|e| e.can_synthesize()) else {
            return Ok((
                NativeSupport::Unavailable,
                Some("Engine is unavailable"),
                None,
            ));
        };
        let voice = match selector {
            VoiceSelector::Exact(id) => Some(id.voice_id.as_str()),
            _ => None,
        };
        let Some(catalogue) = self.catalogue(&patch.engine_id, voice) else {
            let status = self
                .unavailable
                .get(patch.engine_id.as_str())
                .copied()
                .unwrap_or(NativeSupport::Deferred);
            return Ok((
                status,
                Some("No current native catalogue is available"),
                None,
            ));
        };
        if catalogue.identity.schema_id != patch.schema_id {
            return Ok((
                NativeSupport::Unavailable,
                Some("Native schema is not supported by this runtime"),
                None,
            ));
        }
        let mut status = NativeSupport::Supported;
        for (id, operation) in &patch.parameters {
            let descriptor = catalogue
                .parameters
                .iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| ParameterError::Parameter {
                    id: id.clone(),
                    reason: "not described by runtime",
                })?;
            if descriptor.scope != ParameterScope::Voice
                || (descriptor.availability.status == AvailabilityStatus::Supported
                    && !descriptor.adjustable)
            {
                return Err(ParameterError::Parameter {
                    id: id.clone(),
                    reason: "not an adjustable voice parameter",
                }
                .into());
            }
            if let Adjustment::Set { value } = operation {
                if !descriptor.value_type.accepts(value) {
                    return Err(ParameterError::Parameter {
                        id: id.clone(),
                        reason: "value outside declared type/range",
                    }
                    .into());
                }
            }
            match descriptor.availability.status {
                AvailabilityStatus::Supported => {}
                AvailabilityStatus::NotChecked if status == NativeSupport::Supported => {
                    status = NativeSupport::Deferred
                }
                AvailabilityStatus::RuntimeUnsupported | AvailabilityStatus::VoiceUnavailable => {
                    status = NativeSupport::Unavailable
                }
                _ => {}
            }
        }
        if voice.is_some_and(|id| {
            engine
                .voice(id)
                .is_none_or(|v| !v.availability.is_available())
        }) {
            status = NativeSupport::Unavailable;
        }
        Ok((
            status,
            (status != NativeSupport::Supported)
                .then_some("Native block is not currently applicable"),
            Some(catalogue),
        ))
    }
}

/// A prepared native block never claims application or PCM acceptance.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeChoiceExecution {
    NotRequested,
    Parameters(Box<VoiceParameters>),
    CommonOnly { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedEngineVoiceStyle {
    pub common: ComposedVoiceStyle,
    pub native: NativeChoiceExecution,
}

impl EngineLayeredVoiceDefinition {
    pub fn from_json(json: &[u8]) -> Result<Self, NativeChoiceError> {
        let result: Self = crate::native_parameters::decode(json)?;
        result.validate()?;
        Ok(result)
    }

    pub fn common_projection(&self) -> LayeredVoiceDefinition {
        LayeredVoiceDefinition {
            id: self.id.clone(),
            language: self.language.clone(),
            shared: self.shared.clone(),
            choices: self
                .choices
                .iter()
                .map(|c| VoiceChoice {
                    id: c.id.clone(),
                    selector: c.selector.clone(),
                    adjustments: c.adjustments.clone(),
                })
                .collect(),
        }
    }

    pub fn validate(&self) -> Result<(), NativeChoiceError> {
        self.common_projection().validate()?;
        for choice in &self.choices {
            if let Some(native) = &choice.native {
                native.validate_shape()?;
                if choice.selector.engine_id() != Some(native.engine_id.as_str()) {
                    return Err(ParameterError::IdentityMismatch.into());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn statuses(
        &self,
        snapshot: &NativeCatalogueSnapshot<'_>,
    ) -> Result<Vec<NativeChoiceStatus>, NativeChoiceError> {
        self.choices
            .iter()
            .filter_map(|choice| {
                choice.native.as_ref().map(|native| {
                    let (status, reason, _) = snapshot.qualify(native, &choice.selector)?;
                    Ok(NativeChoiceStatus {
                        logical_voice_id: self.id.clone(),
                        choice_id: choice.id.clone(),
                        status,
                        reason: reason.map(str::to_owned),
                    })
                })
            })
            .collect()
    }

    /// Recompose the actual choice from the admitted definition and context.
    /// Policy fallback has no choice patch. No native mapping formula runs here.
    pub fn prepare(
        &self,
        resolution: &VoiceResolution,
        context: &VoiceStylePatch,
        base_rate: f32,
        placement_pan: Option<f32>,
        snapshot: &NativeCatalogueSnapshot<'_>,
        policy: UnavailablePolicy,
    ) -> Result<PreparedEngineVoiceStyle, NativeChoiceError> {
        self.validate()?;
        let common_definition = self.common_projection();
        let index = common_definition.selected_choice(resolution)?;
        let common = common_definition.compose(index, context, base_rate, placement_pan)?;
        let Some(patch) = index.and_then(|i| self.choices[i].native.as_ref()) else {
            return Ok(PreparedEngineVoiceStyle {
                common,
                native: NativeChoiceExecution::NotRequested,
            });
        };
        let selected = &self.choices[index.unwrap()].selector;
        if selected.engine_id() != Some(resolution.realized.engine_id.as_str())
            || matches!(selected, VoiceSelector::Exact(id) if id != &resolution.realized)
        {
            return Err(ParameterError::IdentityMismatch.into());
        }
        let actual = VoiceSelector::Exact(PhysicalVoiceId::new(
            &resolution.realized.engine_id,
            &resolution.realized.voice_id,
        ));
        let qualified = snapshot.qualify(patch, &actual);
        let native = match qualified {
            Ok((NativeSupport::Supported, _, Some(c))) => {
                NativeChoiceExecution::Parameters(Box::new(VoiceParameters {
                    native: patch.clone(),
                    context_dimensions: crate::native_parameters::contextual_inputs(context)?
                        .into_iter()
                        .collect(),
                    expected_identity: c.identity.clone(),
                    unavailable_policy: policy,
                }))
            }
            other => {
                let reason = match other {
                    Ok((_, reason, _)) => reason.unwrap_or("Native catalogue is unavailable"),
                    Err(_) => "Saved native block is invalid for this runtime",
                };
                if policy == UnavailablePolicy::Require {
                    return Err(NativeChoiceError::Unavailable(reason));
                }
                NativeChoiceExecution::CommonOnly {
                    reason: reason.into(),
                }
            }
        };
        Ok(PreparedEngineVoiceStyle { common, native })
    }
}

#[cfg(test)]
mod tests;
