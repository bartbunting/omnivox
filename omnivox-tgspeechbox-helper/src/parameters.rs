//! Voice-quality adjustments relative to the selected profile and phoneme frames.
use super::{TgSpeechBoxTtsEngine, ENGINE_ID};
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::engine_parameters::{validate_query, CatalogueError};
use omnivox_tts::helper_protocol::parameters::*;
use omnivox_tts::native_parameters::*;
use omnivox_tts::voice_choices::VoiceStylePatch;
use omnivox_tts::{SynthesisRequest, TtsError};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const SCHEMA: &str = "tgspeechbox.voice-quality.v1";
const IDS: [&str; 5] = [
    "breathiness",
    "creakiness",
    "brightness",
    "jitter",
    "shimmer",
];
static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);

pub(super) struct ParameterState {
    identity: CatalogueIdentity,
    descriptors: Vec<ParameterDescriptor>,
    next_plan: AtomicU64,
    recent: Mutex<VecDeque<(String, ExplanationResult)>>,
}

#[derive(Clone)]
pub(super) struct Prepared {
    pub quality: [f64; 5],
    plan: Option<NativePlan>,
    degradation: Option<String>,
}

impl ParameterState {
    pub fn new(sample_rate: u32) -> Self {
        let labels = [
            "Additional breathiness",
            "Additional creakiness",
            "Brightness offset",
            "Additional jitter",
            "Additional shimmer",
        ];
        let help = [
            "Adds breath noise while retaining the profile and phoneme variation. Zero preserves the original voice.",
            "Adds a creaky texture while retaining the profile and phoneme variation. Zero preserves the original voice.",
            "Adds decibels to the selected profile's high-shelf gain. Zero preserves its brightness.",
            "Adds irregular pitch-period variation. Zero preserves the profile and phoneme variation.",
            "Adds irregular amplitude variation. Zero preserves the profile and phoneme variation.",
        ];
        let descriptors: Vec<_> = IDS
            .iter()
            .enumerate()
            .map(|(i, id)| ParameterDescriptor {
                id: (*id).into(),
                label: labels[i].into(),
                help: help[i].into(),
                group: "voice-quality".into(),
                order: i as u32,
                unit: Some(if i == 2 { "dB" } else { "amount" }.into()),
                value_type: ValueType::Number {
                    minimum: if i == 2 { -12.0 } else { 0.0 },
                    maximum: if i == 2 { 12.0 } else { 1.0 },
                    step: if i == 2 { 0.5 } else { 0.05 },
                },
                scope: ParameterScope::Voice,
                adjustable: true,
                availability: ParameterAvailability {
                    status: AvailabilityStatus::Supported,
                    reason: None,
                },
                default: ParameterDefault {
                    source: DefaultSource::QualifiedProfile,
                    value: Some(NativeValue::Number(0.0)),
                    reset_supported: true,
                },
                side_effects: vec![],
            })
            .collect();
        Self {
            identity: CatalogueIdentity {
                schema_id: SCHEMA.into(),
                profile_id: format!("tgspeechbox.25b0e1a.dsp9.{sample_rate}.v1"),
                catalogue_revision: catalogue_revision(&descriptors, &[]),
                runtime_generation: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            },
            descriptors,
            next_plan: AtomicU64::new(1),
            recent: Mutex::new(VecDeque::new()),
        }
    }

    fn catalogue(&self, voice_id: Option<String>) -> ParameterCatalogue {
        ParameterCatalogue {
            engine_id: ENGINE_ID.into(),
            identity: self.identity.clone(),
            voice_id,
            parameters: self.descriptors.clone(),
            mappings: vec![],
        }
    }

    pub fn query(
        &self,
        engine: &TgSpeechBoxTtsEngine,
        query: CatalogueQuery,
    ) -> Result<CatalogueResult, CatalogueError> {
        validate_query(&query)?;
        if query.engine_id != ENGINE_ID {
            return Err(CatalogueError::Invalid(
                "Wrong engine for TGSpeechBox catalogue".into(),
            ));
        }
        if query.cursor.is_some()
            || query
                .expected_catalogue_revision
                .as_ref()
                .is_some_and(|r| r != &self.identity.catalogue_revision)
        {
            return Err(CatalogueError::Stale(
                "TGSpeechBox catalogue has changed or cursor is invalid".into(),
            ));
        }
        if query
            .voice_id
            .as_deref()
            .is_some_and(|v| engine.selection(v).is_err())
        {
            return Ok(CatalogueResult::Unavailable {
                reason: CatalogueUnavailable::VoiceUnavailable,
                message: "TGSpeechBox voice is unavailable".into(),
            });
        }
        Ok(CatalogueResult::Ready {
            identity: self.identity.clone(),
            voice_id: query.voice_id,
            parameters: self.descriptors.clone(),
            mappings: vec![],
            next_cursor: None,
        })
    }

    fn plan(
        &self,
        voice_id: &str,
        parameters: Option<&VoiceParameters>,
    ) -> Result<NativePlan, TtsError> {
        let catalogue = self.catalogue(Some(voice_id.into()));
        // These controls have no common ACSS mappings. Contextual pitch/rate
        // remains independent and cannot silently become a texture setting.
        compose(
            &catalogue,
            parameters
                .map(|p| &p.expected_identity)
                .unwrap_or(&self.identity),
            &PhysicalVoiceId::new(ENGINE_ID, voice_id),
            &BTreeMap::new(),
            parameters.map(|p| &p.native),
            &VoiceStylePatch::default(),
        )
        .map_err(|e| TtsError::InvalidParameter(e.to_string()))
    }

    pub fn prepare(
        &self,
        engine: &TgSpeechBoxTtsEngine,
        request: &SynthesisRequest,
        parameters: &VoiceParameters,
    ) -> Result<Prepared, TtsError> {
        omnivox_tts::native_synthesis::validate_request(parameters)?;
        let voice = request.voice_id_for_engine(ENGINE_ID)?;
        engine.selection(voice)?;
        if parameters.native.engine_id != ENGINE_ID
            || parameters.native.schema_id != SCHEMA
            || parameters.expected_identity != self.identity
        {
            if parameters.unavailable_policy == UnavailablePolicy::CommonOnly {
                return Ok(Prepared {
                    quality: [0.0; 5],
                    plan: None,
                    degradation: Some(
                        "TGSpeechBox native settings belong to a different schema or runtime"
                            .into(),
                    ),
                });
            }
            return Err(TtsError::InvalidParameter(
                "TGSpeechBox native identity is stale or unsupported".into(),
            ));
        }
        let plan = self.plan(voice, Some(parameters))?;
        let mut quality = [0.0; 5];
        for (i, id) in IDS.iter().enumerate() {
            quality[i] = match plan.parameters[*id].value {
                Some(NativeValue::Number(value)) => value,
                Some(NativeValue::Integer(value)) => value as f64,
                _ => {
                    return Err(TtsError::InvalidParameter(
                        "Expected numeric TGSpeechBox adjustment".into(),
                    ))
                }
            };
        }
        Ok(Prepared {
            quality,
            plan: Some(plan),
            degradation: None,
        })
    }

    fn explanation(
        plan: &NativePlan,
        evidence: Evidence,
        plan_id: Option<String>,
    ) -> ExplanationResult {
        ExplanationResult::Ready {
            evidence,
            plan_id,
            realized: RealizedVoice {
                engine_id: plan.realized.engine_id.clone(),
                voice_id: plan.realized.voice_id.clone(),
            },
            identity: plan.identity.clone(),
            parameters: plan
                .parameters
                .iter()
                .map(|(id, p)| ParameterEvidence {
                    id: id.clone(),
                    value: p.value.clone(),
                    origin: p.origin,
                    masked_native: p.masked_native,
                    read_back: false,
                })
                .collect(),
        }
    }

    pub fn applied(&self, prepared: Prepared) -> NativeApplication {
        let Some(plan) = prepared.plan else {
            return NativeApplication {
                status: ApplicationStatus::CommonOnly,
                plan_id: None,
                identity: None,
                masked_parameters: vec![],
                reason: prepared.degradation,
            };
        };
        let id = format!(
            "tgs-{}-{}",
            self.identity.runtime_generation,
            self.next_plan.fetch_add(1, Ordering::Relaxed)
        );
        let explanation = Self::explanation(&plan, Evidence::AdapterApplied, Some(id.clone()));
        let mut recent = self.recent.lock().unwrap();
        if recent.len() == 32 {
            recent.pop_front();
        }
        recent.push_back((id.clone(), explanation));
        NativeApplication {
            status: ApplicationStatus::Applied,
            plan_id: Some(id),
            identity: Some(plan.identity),
            masked_parameters: vec![],
            reason: None,
        }
    }

    pub fn explain(
        &self,
        engine: &TgSpeechBoxTtsEngine,
        source: ExplanationSource,
    ) -> Result<ExplanationResult, CatalogueError> {
        omnivox_tts::voice_explanation::validate_helper_source(&source)
            .map_err(CatalogueError::Invalid)?;
        match source {
            ExplanationSource::Applied { plan_id } => Ok(self
                .recent
                .lock()
                .unwrap()
                .iter()
                .find(|(id, _)| id == &plan_id)
                .map(|(_, e)| e.clone())
                .unwrap_or(ExplanationResult::Unavailable {
                    reason: ExplanationUnavailable::PlanExpired,
                    message: "TGSpeechBox plan is no longer retained".into(),
                })),
            ExplanationSource::Draft {
                settings,
                voice_parameters,
            } => {
                let voice = settings
                    .voice_id
                    .as_deref()
                    .or(engine.descriptor.default_voice_id.as_deref())
                    .ok_or_else(|| CatalogueError::Invalid("No TGSpeechBox voice".into()))?;
                if engine.selection(voice).is_err() {
                    return Ok(ExplanationResult::Unavailable {
                        reason: ExplanationUnavailable::VoiceUnavailable,
                        message: "TGSpeechBox voice is unavailable".into(),
                    });
                }
                let plan = self
                    .plan(voice, voice_parameters.as_deref())
                    .map_err(|e| CatalogueError::Invalid(e.to_string()))?;
                Ok(Self::explanation(&plan, Evidence::Planned, None))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::voice_choices::Adjustment;

    fn parameters(state: &ParameterState) -> VoiceParameters {
        VoiceParameters {
            native: NativePatch {
                engine_id: ENGINE_ID.into(),
                schema_id: SCHEMA.into(),
                parameters: BTreeMap::new(),
            },
            expected_identity: state.identity.clone(),
            context_dimensions: vec![],
            unavailable_policy: UnavailablePolicy::Require,
        }
    }

    #[test]
    fn default_and_zero_restore_the_profile_after_a_custom_style() {
        let state = ParameterState::new(44_100);
        state
            .catalogue(Some("en-us/adam".into()))
            .validate()
            .unwrap();
        let mut p = parameters(&state);
        p.native.parameters.insert(
            "breathiness".into(),
            Adjustment::Set {
                value: NativeValue::Number(0.8),
            },
        );
        assert_eq!(
            state.plan("en-us/adam", Some(&p)).unwrap().parameters["breathiness"].value,
            Some(NativeValue::Number(0.8))
        );
        p.native
            .parameters
            .insert("breathiness".into(), Adjustment::Default {});
        let default = state.plan("en-us/adam", Some(&p)).unwrap();
        assert_eq!(
            default.parameters["breathiness"].origin,
            ValueOrigin::NativeDefault
        );
        assert_eq!(
            default.parameters["breathiness"].value,
            Some(NativeValue::Number(0.0))
        );
        p.native.parameters.insert(
            "breathiness".into(),
            Adjustment::Set {
                value: NativeValue::Number(0.0),
            },
        );
        assert_eq!(
            state.plan("en-us/adam", Some(&p)).unwrap().parameters["breathiness"].origin,
            ValueOrigin::NativeSet
        );
        assert_eq!(
            state.plan("en-us/beth", None).unwrap().parameters["breathiness"].value,
            Some(NativeValue::Number(0.0))
        );
    }

    #[test]
    fn invalid_values_and_stale_runtimes_cannot_form_a_native_plan() {
        let state = ParameterState::new(44_100);
        for (id, value) in [
            ("breathiness", -0.1),
            ("jitter", 1.1),
            ("brightness", 12.1),
            ("brightness", -12.1),
        ] {
            let mut p = parameters(&state);
            p.native.parameters.insert(
                id.into(),
                Adjustment::Set {
                    value: NativeValue::Number(value),
                },
            );
            assert!(state.plan("en-us/adam", Some(&p)).is_err());
        }
        let mut p = parameters(&state);
        p.expected_identity.runtime_generation += 1;
        assert!(state.plan("en-us/adam", Some(&p)).is_err());
    }

    #[test]
    fn explanations_retain_actual_voice_and_bound_the_plan_cache() {
        let state = ParameterState::new(44_100);
        let mut first = None;
        for n in 0..33 {
            let receipt = state.applied(Prepared {
                quality: [0.0; 5],
                plan: Some(state.plan("en-us/beth", None).unwrap()),
                degradation: None,
            });
            receipt.validate().unwrap();
            if n == 0 {
                first = receipt.plan_id;
            }
        }
        let recent = state.recent.lock().unwrap();
        assert_eq!(recent.len(), 32);
        assert!(recent.iter().all(|(id, _)| Some(id) != first.as_ref()));
        for (_, result) in recent.iter() {
            result.validate().unwrap();
            assert!(
                matches!(result, ExplanationResult::Ready { realized, evidence: Evidence::AdapterApplied, .. } if realized.voice_id == "en-us/beth")
            );
        }
    }
}
