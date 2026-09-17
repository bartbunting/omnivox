use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::{PhysicalVoiceId, VoiceSelector};
use crate::voice_choices::{Adjustment, VoiceStylePatch};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueOrigin {
    EngineDefault,
    CommonMapping,
    ContextMapping,
    NativeSet,
    NativeDefault,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedParameter {
    /// None means a verified engine reset whose numerical value is unknown.
    pub value: Option<NativeValue>,
    pub origin: ValueOrigin,
    pub masked_native: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativePlan {
    pub realized: PhysicalVoiceId,
    pub identity: CatalogueIdentity,
    pub parameters: BTreeMap<String, PlannedParameter>,
    /// After resetting the pristine voice, apply in this order. It orders
    /// intended values, not native function calls; the adapter owns execution.
    pub application_order: Vec<String>,
}

fn contextual_inputs(context: &VoiceStylePatch) -> Result<BTreeSet<CommonInput>, ParameterError> {
    context
        .validate()
        .map_err(|_| ParameterError::Invalid("invalid contextual patch"))?;
    use CommonInput::*;
    Ok([
        (RateOffset, context.rate_offset.is_some()),
        (AveragePitch, context.average_pitch.is_some()),
        (PitchRange, context.pitch_range.is_some()),
        (Stress, context.stress.is_some()),
        (Richness, context.richness.is_some()),
        (Gain, context.gain.is_some()),
        (LowPass, context.low_pass.is_some()),
        (HighPass, context.high_pass.is_some()),
        (Pan, context.pan.is_some()),
        (Reverb, context.reverb.is_some()),
        (Echo, context.echo.is_some()),
        (Chorus, context.chorus.is_some()),
    ]
    .into_iter()
    .filter_map(|(id, present)| present.then_some(id))
    .collect())
}

/// Compose for one actual physical voice, without changing any engine state.
///
/// `mapped_common` is the adapter's final common mapping, including context and
/// its established native clamping. Mapping formulas remain adapter-owned.
/// Catalogue default evidence must belong to this exact physical voice.
pub fn compose(
    catalogue: &ParameterCatalogue,
    expected_identity: &CatalogueIdentity,
    realized: &PhysicalVoiceId,
    mapped_common: &BTreeMap<String, NativeValue>,
    native: Option<&NativePatch>,
    context: &VoiceStylePatch,
) -> Result<NativePlan, ParameterError> {
    catalogue.validate()?;
    if &catalogue.identity != expected_identity {
        return Err(ParameterError::StaleIdentity);
    }
    if catalogue.engine_id != realized.engine_id
        || catalogue.voice_id.as_deref() != Some(realized.voice_id.as_str())
    {
        return Err(ParameterError::IdentityMismatch);
    }
    if let Some(native) = native {
        native.validate_for(catalogue, &VoiceSelector::Exact(realized.clone()))?;
    }
    let explicit = contextual_inputs(context)?;
    let contextual: BTreeSet<_> = catalogue
        .mappings
        .iter()
        .filter(|m| m.common_inputs.iter().any(|i| explicit.contains(i)))
        .flat_map(|m| m.native_outputs.iter().cloned())
        .collect();
    let descriptors: BTreeMap<_, _> = catalogue
        .parameters
        .iter()
        .filter(|p| {
            p.scope == ParameterScope::Voice
                && p.availability.status == AvailabilityStatus::Supported
                && p.default.reset_supported
        })
        .map(|p| (p.id.clone(), p))
        .collect();
    let mut parameters: BTreeMap<_, _> = descriptors
        .iter()
        .map(|(id, p)| {
            (
                id.clone(),
                PlannedParameter {
                    value: p.default.value.clone(),
                    origin: ValueOrigin::EngineDefault,
                    masked_native: false,
                },
            )
        })
        .collect();
    for (id, value) in mapped_common {
        let p = descriptors
            .get(id)
            .ok_or(ParameterError::Invalid("unavailable common mapping output"))?;
        if !p.value_type.accepts(value) {
            return Err(p.error("mapped value outside qualified range"));
        }
        let result = parameters.get_mut(id).expect("descriptor inserted");
        result.value = Some(value.clone());
        result.origin = ValueOrigin::CommonMapping;
    }
    // A default context operation may intentionally omit the mapped value;
    // the pristine baseline still wins over a native choice override.
    for id in &contextual {
        if let Some(result) = parameters.get_mut(id) {
            result.origin = ValueOrigin::ContextMapping;
        }
    }
    if let Some(native) = native {
        for (id, operation) in &native.parameters {
            let result = parameters.get_mut(id).expect("native patch validated");
            if contextual.contains(id) {
                result.masked_native = true;
                continue;
            }
            match operation {
                Adjustment::Set { value } => {
                    result.value = Some(value.clone());
                    result.origin = ValueOrigin::NativeSet;
                }
                Adjustment::Default {} => {
                    result.value = descriptors[id].default.value.clone();
                    result.origin = ValueOrigin::NativeDefault;
                }
            }
        }
    }
    // Kahn ordering, deterministic for independent controls. A -> B means
    // applying A can change B, so B must be restored after A.
    let mut incoming: BTreeMap<String, usize> =
        parameters.keys().map(|id| (id.clone(), 0)).collect();
    for p in descriptors.values() {
        for target in &p.side_effects {
            let Some(count) = incoming.get_mut(target) else {
                return Err(p.error("side effect reaches an unavailable/non-voice control"));
            };
            if parameters[target].value.is_none() {
                return Err(p.error("side effect target has no restorable value"));
            }
            *count += 1;
        }
    }
    let mut ready: BTreeSet<String> = incoming
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut application_order = Vec::new();
    while let Some(id) = ready.pop_first() {
        for target in &descriptors[&id].side_effects {
            let count = incoming.get_mut(target).expect("validated target");
            *count -= 1;
            if *count == 0 {
                ready.insert(target.clone());
            }
        }
        application_order.push(id);
    }
    if application_order.len() != parameters.len() {
        return Err(ParameterError::DependencyCycle);
    }
    Ok(NativePlan {
        realized: realized.clone(),
        identity: catalogue.identity.clone(),
        parameters,
        application_order,
    })
}
