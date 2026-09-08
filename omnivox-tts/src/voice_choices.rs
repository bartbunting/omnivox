//! Typed per-choice style layers, independent of selection and playback.
//!
//! Admission and execution are added separately. No capability is advertised
//! merely because these values can be decoded and composed.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::contracts::{
    apply_rate_offset, AcssDimension, FallbackPolicy, LogicalVoiceDefinition, NormalizedAcss,
    PhysicalVoiceId, PostSynthesisDimension, PostSynthesisStyle, VoiceGender, VoiceSelector,
};
use crate::logical_voices::{
    validate_registration, LogicalVoiceRegistryError, MAX_LOGICAL_VOICE_ID_BYTES,
    MAX_VOICE_PREFERENCES,
};
use crate::resolver::{ResolutionReason, VoiceResolution};

#[derive(Debug, Error)]
pub enum ChoiceTuningError {
    #[error("invalid per-choice tuning: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Registry(#[from] LogicalVoiceRegistryError),
}

/// Identity of accepted/consumed audio, independent of mutable routing state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioChoiceIdentity {
    pub choice_id: Option<String>,
    pub reason: ResolutionReason,
    pub realized: PhysicalVoiceId,
    pub degraded_acss: Vec<AcssDimension>,
    pub degraded_effects: Vec<PostSynthesisDimension>,
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// A supplied patch member. Absence is represented by the containing field.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Adjustment<T> {
    Set { value: T },
    Default {},
}

fn present_adjustment<'de, D, T>(deserializer: D) -> Result<Option<Adjustment<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Adjustment::<T>::deserialize(deserializer).map(Some)
}

fn normalized(value: f32) -> Result<(), ChoiceTuningError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ChoiceTuningError::Invalid(
            "normalized value must be finite in [0, 1]",
        ))
    }
}

fn offset(value: i16) -> Result<(), ChoiceTuningError> {
    if (-20..=20).contains(&value) {
        Ok(())
    } else {
        Err(ChoiceTuningError::Invalid(
            "rate offset must be in [-20, 20]",
        ))
    }
}

/// Complete nullable base fields; omitted keys are invalid on this new wire path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedAcss {
    #[serde(deserialize_with = "required_nullable")]
    pub rate: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub average_pitch: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub pitch_range: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub stress: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub richness: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub volume: Option<f32>,
}

impl From<&SharedAcss> for NormalizedAcss {
    fn from(value: &SharedAcss) -> Self {
        Self {
            rate: value.rate,
            average_pitch: value.average_pitch,
            pitch_range: value.pitch_range,
            stress: value.stress,
            richness: value.richness,
            volume: value.volume,
        }
    }
}

/// Complete nullable base fields; omitted keys are invalid on this new wire path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedEffects {
    #[serde(deserialize_with = "required_nullable")]
    pub gain: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub low_pass: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub high_pass: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub pan: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub reverb: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub echo: Option<f32>,
    #[serde(deserialize_with = "required_nullable")]
    pub chorus: Option<f32>,
}

impl From<&SharedEffects> for PostSynthesisStyle {
    fn from(value: &SharedEffects) -> Self {
        Self {
            gain: value.gain,
            low_pass: value.low_pass,
            high_pass: value.high_pass,
            pan: value.pan,
            reverb: value.reverb,
            echo: value.echo,
            chorus: value.chorus,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedVoiceStyle {
    pub acss: SharedAcss,
    #[serde(deserialize_with = "required_nullable")]
    pub rate_offset: Option<i16>,
    pub effects: SharedEffects,
}

impl SharedVoiceStyle {
    pub fn validate(&self) -> Result<(), ChoiceTuningError> {
        for value in [
            self.acss.rate,
            self.acss.average_pitch,
            self.acss.pitch_range,
            self.acss.stress,
            self.acss.richness,
            self.acss.volume,
            self.effects.gain,
            self.effects.low_pass,
            self.effects.high_pass,
            self.effects.pan,
            self.effects.reverb,
            self.effects.echo,
            self.effects.chorus,
        ]
        .into_iter()
        .flatten()
        {
            normalized(value)?;
        }
        if let Some(value) = self.rate_offset {
            offset(value)?;
            if self.acss.rate.is_some() {
                return Err(ChoiceTuningError::Invalid(
                    "absolute rate and rate offset cannot coexist",
                ));
            }
        }
        Ok(())
    }
}

/// Sparse replacement operations, retaining explicit zero and default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceStylePatch {
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub average_pitch: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub pitch_range: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub stress: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub richness: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub gain: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub low_pass: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub high_pass: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub pan: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub reverb: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub echo: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub chorus: Option<Adjustment<f32>>,
    #[serde(
        default,
        deserialize_with = "present_adjustment",
        skip_serializing_if = "Option::is_none"
    )]
    pub rate_offset: Option<Adjustment<i16>>,
}

impl VoiceStylePatch {
    pub fn validate(&self) -> Result<(), ChoiceTuningError> {
        for operation in [
            self.average_pitch,
            self.pitch_range,
            self.stress,
            self.richness,
            self.gain,
            self.low_pass,
            self.high_pass,
            self.pan,
            self.reverb,
            self.echo,
            self.chorus,
        ]
        .into_iter()
        .flatten()
        {
            if let Adjustment::Set { value } = operation {
                normalized(value)?;
            }
        }
        if let Some(Adjustment::Set { value }) = self.rate_offset {
            offset(value)?;
        }
        Ok(())
    }

    fn apply(
        &self,
        acss: &mut NormalizedAcss,
        effects: &mut PostSynthesisStyle,
        rate: &mut Option<i16>,
    ) {
        replace(self.average_pitch, &mut acss.average_pitch);
        replace(self.pitch_range, &mut acss.pitch_range);
        replace(self.stress, &mut acss.stress);
        replace(self.richness, &mut acss.richness);
        replace(self.gain, &mut effects.gain);
        replace(self.low_pass, &mut effects.low_pass);
        replace(self.high_pass, &mut effects.high_pass);
        replace(self.pan, &mut effects.pan);
        replace(self.reverb, &mut effects.reverb);
        replace(self.echo, &mut effects.echo);
        replace(self.chorus, &mut effects.chorus);
        if self.rate_offset.is_some() {
            acss.rate = None;
            replace(self.rate_offset, rate);
        }
    }
}

fn replace<T: Copy>(operation: Option<Adjustment<T>>, destination: &mut Option<T>) {
    match operation {
        Some(Adjustment::Set { value }) => *destination = Some(value),
        Some(Adjustment::Default {}) => *destination = None,
        None => {}
    }
}

// Keep the existing selector semantics while rejecting unknown fields only on
// the new path. Do not tighten legacy requests by changing VoiceSelector.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StrictSelector {
    Exact {
        engine_id: String,
        voice_id: String,
    },
    EngineDefault {
        engine_id: String,
    },
    Properties {
        engine_id: Option<String>,
        language: Option<String>,
        gender: Option<VoiceGender>,
    },
}

fn choice_selector<'de, D>(deserializer: D) -> Result<VoiceSelector, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match StrictSelector::deserialize(deserializer)? {
        StrictSelector::Exact {
            engine_id,
            voice_id,
        } => VoiceSelector::Exact(PhysicalVoiceId {
            engine_id,
            voice_id,
        }),
        StrictSelector::EngineDefault { engine_id } => VoiceSelector::EngineDefault { engine_id },
        StrictSelector::Properties {
            engine_id,
            language,
            gender,
        } => VoiceSelector::Properties {
            engine_id,
            language,
            gender,
        },
    })
}

pub(crate) fn optional_choice_selector<'de, D>(
    deserializer: D,
) -> Result<Option<VoiceSelector>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(transparent)]
    struct Selector(#[serde(deserialize_with = "choice_selector")] VoiceSelector);
    Ok(Option::<Selector>::deserialize(deserializer)?.map(|selector| selector.0))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceChoice {
    pub id: String,
    #[serde(deserialize_with = "choice_selector")]
    pub selector: VoiceSelector,
    pub adjustments: VoiceStylePatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayeredVoiceDefinition {
    pub id: String,
    #[serde(deserialize_with = "required_nullable")]
    pub language: Option<String>,
    pub shared: SharedVoiceStyle,
    pub choices: Vec<VoiceChoice>,
}

/// One member of a complete replacement registry, not a separate generation domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "definition",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RegisteredVoiceDefinition {
    Legacy(LogicalVoiceDefinition),
    Layered(LayeredVoiceDefinition),
}

/// Complete request state before capability adaptation, never failed-attempt state.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposedVoiceStyle {
    pub choice_id: Option<String>,
    pub acss: NormalizedAcss,
    pub effects: PostSynthesisStyle,
}

impl LayeredVoiceDefinition {
    pub fn validate(&self) -> Result<(), ChoiceTuningError> {
        self.shared.validate()?;
        if self.choices.len() > MAX_VOICE_PREFERENCES {
            return Err(ChoiceTuningError::Invalid("too many choices"));
        }
        let mut ids = HashSet::new();
        for choice in &self.choices {
            if choice.id.is_empty()
                || choice.id.len() > MAX_LOGICAL_VOICE_ID_BYTES
                || !choice
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
            {
                return Err(ChoiceTuningError::Invalid(
                    "choice ID must contain 1..128 ASCII identifier characters",
                ));
            }
            if !ids.insert(&choice.id) {
                return Err(ChoiceTuningError::Invalid("duplicate choice ID"));
            }
            choice.adjustments.validate()?;
        }
        validate_registration(&[self.legacy_projection()], &FallbackPolicy::default())?;
        Ok(())
    }

    /// Shared settings and bare selectors, for old operations and diagnostics only.
    pub fn legacy_projection(&self) -> LogicalVoiceDefinition {
        LogicalVoiceDefinition {
            id: self.id.clone(),
            language: self.language.clone(),
            preferences: self
                .choices
                .iter()
                .map(|choice| choice.selector.clone())
                .collect(),
            acss: (&self.shared.acss).into(),
            effects: (&self.shared.effects).into(),
        }
    }

    /// Preserve the resolver's original preference occurrence; never join physical IDs.
    pub fn selected_choice(
        &self,
        resolution: &VoiceResolution,
    ) -> Result<Option<usize>, ChoiceTuningError> {
        if resolution.logical_voice_id != self.id {
            return Err(ChoiceTuningError::Invalid(
                "resolution belongs to another definition",
            ));
        }
        let index = match resolution.reason {
            ResolutionReason::Preferred => Some(0),
            ResolutionReason::ExplicitAlternative { preference_index } if preference_index > 0 => {
                Some(preference_index)
            }
            ResolutionReason::ExplicitAlternative { .. } => {
                return Err(ChoiceTuningError::Invalid("invalid alternative index"))
            }
            _ => None,
        };
        if index.is_some_and(|index| index >= self.choices.len()) {
            return Err(ChoiceTuningError::Invalid(
                "resolution has no matching choice record",
            ));
        }
        Ok(index)
    }

    /// Compose fresh shared/actual-choice/context layers, then existing placement.
    /// Zero/default offset preserves a host rate above one; nonzero uses the old clamp.
    pub fn compose(
        &self,
        choice_index: Option<usize>,
        context: &VoiceStylePatch,
        base_rate: f32,
        placement_pan: Option<f32>,
    ) -> Result<ComposedVoiceStyle, ChoiceTuningError> {
        self.validate()?;
        context.validate()?;
        if !base_rate.is_finite() || !(0.0..=2.0).contains(&base_rate) {
            return Err(ChoiceTuningError::Invalid(
                "host rate must be finite in [0, 2]",
            ));
        }
        if let Some(pan) = placement_pan {
            normalized(pan)?;
        }
        let choice = choice_index
            .map(|index| {
                self.choices
                    .get(index)
                    .ok_or(ChoiceTuningError::Invalid("unknown choice index"))
            })
            .transpose()?;
        let mut acss = NormalizedAcss::from(&self.shared.acss);
        let mut effects = PostSynthesisStyle::from(&self.shared.effects);
        let mut rate = self.shared.rate_offset;
        if let Some(choice) = choice {
            choice.adjustments.apply(&mut acss, &mut effects, &mut rate);
        }
        context.apply(&mut acss, &mut effects, &mut rate);
        if let Some(offset) = rate.filter(|offset| *offset != 0) {
            acss.rate = Some(apply_rate_offset(base_rate, offset));
        }
        if placement_pan.is_some() {
            effects.pan = placement_pan;
        }
        Ok(ComposedVoiceStyle {
            choice_id: choice.map(|row| row.id.clone()),
            acss,
            effects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/voice-choice-tuning.json"
        ))
        .unwrap()
    }

    fn definition() -> LayeredVoiceDefinition {
        serde_json::from_value(
            fixture()["messages"]["registration"]["definitions"][0]["definition"].clone(),
        )
        .unwrap()
    }

    fn close(actual: Option<f32>, expected: &Value) {
        if expected.is_null() {
            assert_eq!(actual, None);
        } else {
            assert!(
                (actual.unwrap() - expected.as_f64().unwrap() as f32).abs() < 1e-6,
                "{actual:?} differs from {expected}"
            );
        }
    }

    #[test]
    fn independent_wire_composition_examples() {
        for case in fixture()["composition_cases"].as_array().unwrap() {
            let mut definition = definition();
            if let Some(rate) = case.get("shared_absolute_rate") {
                definition.shared.acss.rate = Some(rate.as_f64().unwrap() as f32);
                definition.shared.rate_offset = None;
            }
            let before = definition.clone();
            let index = case["choice_id"].as_str().map(|id| {
                definition
                    .choices
                    .iter()
                    .position(|row| row.id == id)
                    .unwrap()
            });
            let context: VoiceStylePatch = serde_json::from_value(case["context"].clone()).unwrap();
            let base = case["base_rate"].as_f64().unwrap() as f32;
            let result = definition
                .compose(index, &context, base, None)
                .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
            let expected = &case["expected"];
            close(result.acss.average_pitch, &expected["average_pitch"]);
            close(result.acss.richness, &expected["richness"]);
            close(result.effects.low_pass, &expected["low_pass"]);
            close(
                Some(result.acss.rate.unwrap_or(base)),
                &expected["effective_rate"],
            );
            assert_eq!(result.choice_id.as_deref(), case["choice_id"].as_str());
            assert_eq!(definition, before);
        }
    }

    #[test]
    fn independent_malformed_patches_are_rejected_without_normalizing() {
        for case in fixture()["invalid_patch_cases"].as_array().unwrap() {
            if let Ok(patch) =
                serde_json::from_str::<VoiceStylePatch>(case["json"].as_str().unwrap())
            {
                assert!(patch.validate().is_err(), "accepted {}", case["reason"]);
            }
        }
    }

    #[test]
    fn inherited_zero_and_default_round_trip_distinctly() {
        for value in [
            json!({}),
            json!({"richness":{"op":"set","value":0.0}}),
            json!({"richness":{"op":"default"}}),
        ] {
            let patch: VoiceStylePatch = serde_json::from_value(value.clone()).unwrap();
            patch.validate().unwrap();
            assert_eq!(serde_json::to_value(patch).unwrap(), value);
        }
        assert_ne!(
            VoiceStylePatch::default(),
            serde_json::from_value::<VoiceStylePatch>(json!({"richness":{"op":"default"}}))
                .unwrap()
        );
    }

    #[test]
    fn audio_identity_matches_preview_and_receipt_contract_examples() {
        let examples = fixture();
        let expected = &examples["messages"]["preview_completed"]["last_started"];
        let identity: AudioChoiceIdentity = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(&identity).unwrap(), *expected);
        assert_eq!(
            examples["messages"]["playback_receipt"]["choice"],
            *expected
        );
        let mut extra = expected.clone();
        extra["unknown"] = json!(true);
        assert!(serde_json::from_value::<AudioChoiceIdentity>(extra).is_err());
    }

    #[test]
    fn shared_base_requires_every_nullable_field_and_rejects_unknowns() {
        let shared = serde_json::to_value(definition().shared).unwrap();
        for part in ["acss", "effects"] {
            for field in shared[part].as_object().unwrap().keys() {
                let mut missing = shared.clone();
                missing[part].as_object_mut().unwrap().remove(field);
                assert!(
                    serde_json::from_value::<SharedVoiceStyle>(missing).is_err(),
                    "missing {part}.{field}"
                );
            }
            let mut extra = shared.clone();
            extra[part]["unknown"] = json!(0);
            assert!(serde_json::from_value::<SharedVoiceStyle>(extra).is_err());
        }
        let mut missing = shared.clone();
        missing.as_object_mut().unwrap().remove("rate_offset");
        assert!(serde_json::from_value::<SharedVoiceStyle>(missing).is_err());
        let mut ambiguous = definition().shared;
        ambiguous.acss.rate = Some(0.5);
        assert!(ambiguous.validate().is_err());
        ambiguous.rate_offset = Some(0);
        assert!(ambiguous.validate().is_err());
        ambiguous.rate_offset = None;
        ambiguous.validate().unwrap();
        ambiguous.acss.richness = Some(f32::NAN);
        assert!(ambiguous.validate().is_err());
    }

    #[test]
    fn new_selector_and_wrapper_fields_are_strict_without_changing_legacy_types() {
        let definitions = fixture()["messages"]["registration"]["definitions"].clone();
        let rows: Vec<RegisteredVoiceDefinition> =
            serde_json::from_value(definitions.clone()).unwrap();
        assert!(matches!(&rows[0], RegisteredVoiceDefinition::Layered(_)));
        assert!(matches!(&rows[1], RegisteredVoiceDefinition::Legacy(_)));
        let mut extra = definitions[0].clone();
        extra["unexpected"] = json!(true);
        assert!(serde_json::from_value::<RegisteredVoiceDefinition>(extra).is_err());
        let mut extra = definitions[0]["definition"].clone();
        extra["choices"][0]["selector"]["scope"] = json!("local");
        assert!(serde_json::from_value::<LayeredVoiceDefinition>(extra).is_err());
        let mut old = definitions[1]["definition"].clone();
        old["legacy_extension"] = json!(true);
        assert!(serde_json::from_value::<LogicalVoiceDefinition>(old).is_ok());
    }

    #[test]
    fn choice_bounds_and_identity_are_validated_before_composition() {
        let original = definition();
        for id in [
            "".to_owned(),
            "has space".to_owned(),
            "é".to_owned(),
            "a".repeat(129),
        ] {
            let mut voice = original.clone();
            voice.choices[0].id = id;
            assert!(voice.validate().is_err());
        }
        let mut duplicate = original.clone();
        duplicate.choices[1].id = duplicate.choices[0].id.clone();
        assert!(duplicate.validate().is_err());
        let mut overflow = original.clone();
        overflow.choices = (0..33)
            .map(|index| {
                let mut row = original.choices[0].clone();
                row.id = format!("row-{index}");
                row
            })
            .collect();
        assert!(overflow.validate().is_err());
        assert!(original
            .compose(Some(99), &VoiceStylePatch::default(), 0.65, None)
            .is_err());
        for rate in [-0.1, 2.1, f32::INFINITY, f32::NAN] {
            assert!(original
                .compose(None, &VoiceStylePatch::default(), rate, None)
                .is_err());
        }
    }

    #[test]
    fn record_selection_uses_resolution_reason_not_physical_identity() {
        let voice = definition();
        assert_eq!(voice.choices[1].selector, voice.choices[2].selector);
        let mut resolution = VoiceResolution {
            logical_voice_id: voice.id.clone(),
            requested: Some(voice.choices[1].selector.clone()),
            realized: PhysicalVoiceId::new("eloquence", "Reed"),
            reason: ResolutionReason::ExplicitAlternative {
                preference_index: 2,
            },
            failed_attempts: vec![],
        };
        let index = voice.selected_choice(&resolution).unwrap();
        assert_eq!(index, Some(2));
        assert_eq!(
            voice
                .compose(index, &VoiceStylePatch::default(), 0.65, None)
                .unwrap()
                .choice_id,
            Some("eloquence-reed-soft".to_owned())
        );
        resolution.reason = ResolutionReason::GlobalDefault;
        assert_eq!(voice.selected_choice(&resolution).unwrap(), None);
        resolution.reason = ResolutionReason::ExplicitAlternative {
            preference_index: 99,
        };
        assert!(voice.selected_choice(&resolution).is_err());
        resolution.reason = ResolutionReason::Preferred;
        resolution.logical_voice_id = "another-preset".to_owned();
        assert!(voice.selected_choice(&resolution).is_err());
    }

    #[test]
    fn defaults_clear_values_before_capability_adaptation_and_placement_stays_separate() {
        let voice = definition();
        let context: VoiceStylePatch = serde_json::from_value(json!({
            "average_pitch": {"op":"default"}, "richness":{"op":"default"},
            "rate_offset":{"op":"default"}, "low_pass":{"op":"default"},
            "pan":{"op":"set","value":0}
        }))
        .unwrap();
        let result = voice.compose(Some(1), &context, 1.4, Some(0.7)).unwrap();
        assert_eq!(result.acss.average_pitch, None);
        assert_eq!(result.acss.richness, None);
        assert_eq!(result.acss.rate, None);
        assert_eq!(result.effects.low_pass, None);
        assert_eq!(result.effects.pan, Some(0.7));
        let next = voice
            .compose(Some(0), &VoiceStylePatch::default(), 0.65, None)
            .unwrap();
        assert_eq!(next.acss.average_pitch, Some(0.2));
        assert_eq!(next.acss.richness, Some(0.8));
        assert_eq!(next.effects.pan, None);
        assert!(voice.compose(None, &context, 0.65, Some(f32::NAN)).is_err());
    }

    #[test]
    fn compatibility_projection_does_not_bake_a_relative_rate_or_choice_patch() {
        let voice = definition();
        let old = voice.legacy_projection();
        assert_eq!(old.preferences.len(), voice.choices.len());
        assert_eq!(old.acss.rate, None);
        assert_eq!(old.acss.richness, Some(0.5));
        assert_eq!(old.effects.low_pass, Some(0.75));
        assert_eq!(old.preferences[1], old.preferences[2]);
    }
}
