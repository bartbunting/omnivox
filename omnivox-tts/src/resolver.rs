//! Deterministic resolution of portable logical voices against runtime inventory.

use crate::contracts::{
    Availability, EngineDescriptor, EngineHealth, FallbackPolicy, LogicalVoiceDefinition,
    PhysicalVoiceId, TextRepertoire, VoiceDescriptor, VoiceSelector,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stage at which a selector was considered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum ResolutionStage {
    Preference { index: usize },
    SameLanguageOnRequestedEngine,
    PreferredEngine { index: usize },
    GlobalDefault,
    FallbackEngine { index: usize },
}

/// Why a selector could not be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolutionFailure {
    EngineNotFound {
        engine_id: String,
    },
    EngineUnavailable {
        engine_id: String,
        reason: String,
    },
    EngineFailed {
        engine_id: String,
        reason: String,
    },
    VoiceNotFound {
        id: PhysicalVoiceId,
    },
    VoiceUnavailable {
        id: PhysicalVoiceId,
        reason: String,
    },
    TextUnsupported {
        engine_id: String,
        text_repertoire: TextRepertoire,
        utf8_offset: usize,
        codepoint: u32,
    },
    NoMatchingVoice,
}

/// One failed selector, retained for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionAttempt {
    pub stage: ResolutionStage,
    pub selector: VoiceSelector,
    pub failure: ResolutionFailure,
}

/// Why the realized voice was selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ResolutionReason {
    Preferred,
    ExplicitAlternative { preference_index: usize },
    SameLanguageOnRequestedEngine,
    PreferredEngine { preferred_index: usize },
    GlobalDefault,
    FallbackEngine { fallback_index: usize },
}

/// Successful late binding of one logical voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceResolution {
    pub logical_voice_id: String,
    pub requested: Option<VoiceSelector>,
    pub realized: PhysicalVoiceId,
    pub reason: ResolutionReason,
    pub failed_attempts: Vec<ResolutionAttempt>,
}

/// No configured or policy fallback could speak the logical voice.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("no usable physical voice for logical voice {logical_voice_id}")]
pub struct VoiceResolutionError {
    pub logical_voice_id: String,
    pub attempts: Vec<ResolutionAttempt>,
}

/// Resolve a portable logical definition against current engine inventory.
///
/// Resolution is intentionally pure. Callers can rerun it after an engine or
/// voice failure using an updated inventory, without retaining stale bindings.
pub fn resolve_voice(
    engines: &[EngineDescriptor],
    definition: &LogicalVoiceDefinition,
    policy: &FallbackPolicy,
) -> Result<VoiceResolution, VoiceResolutionError> {
    resolve_voice_inner(engines, definition, policy, None)
}

/// Resolve a logical voice while excluding engines that cannot preserve TEXT.
pub fn resolve_voice_for_text(
    engines: &[EngineDescriptor],
    definition: &LogicalVoiceDefinition,
    policy: &FallbackPolicy,
    text: &str,
) -> Result<VoiceResolution, VoiceResolutionError> {
    resolve_voice_inner(engines, definition, policy, Some(text))
}

fn resolve_voice_inner(
    engines: &[EngineDescriptor],
    definition: &LogicalVoiceDefinition,
    policy: &FallbackPolicy,
    text: Option<&str>,
) -> Result<VoiceResolution, VoiceResolutionError> {
    let requested = definition.preferences.first().cloned();
    let mut attempts = Vec::new();

    for (index, selector) in definition.preferences.iter().enumerate() {
        let stage = ResolutionStage::Preference { index };
        match evaluate_selector(engines, selector, &policy.preferred_engines, text) {
            Ok(realized) => {
                let reason = if index == 0 {
                    ResolutionReason::Preferred
                } else {
                    ResolutionReason::ExplicitAlternative {
                        preference_index: index,
                    }
                };
                return Ok(success(definition, requested, realized, reason, attempts));
            }
            Err(failure) => attempts.push(ResolutionAttempt {
                stage,
                selector: selector.clone(),
                failure,
            }),
        }
    }

    if policy.allow_same_language_on_requested_engine {
        if let (Some(engine_id), Some(language)) = (
            requested.as_ref().and_then(VoiceSelector::engine_id),
            definition.language.as_ref(),
        ) {
            let selector = VoiceSelector::Properties {
                engine_id: Some(engine_id.to_owned()),
                language: Some(language.clone()),
                gender: None,
            };
            match evaluate_selector(engines, &selector, &policy.preferred_engines, text) {
                Ok(realized) => {
                    return Ok(success(
                        definition,
                        requested,
                        realized,
                        ResolutionReason::SameLanguageOnRequestedEngine,
                        attempts,
                    ));
                }
                Err(failure) => attempts.push(ResolutionAttempt {
                    stage: ResolutionStage::SameLanguageOnRequestedEngine,
                    selector,
                    failure,
                }),
            }
        }
    }

    for (index, engine_id) in policy.preferred_engines.iter().enumerate() {
        let selector = VoiceSelector::EngineDefault {
            engine_id: engine_id.clone(),
        };
        match evaluate_selector(engines, &selector, &policy.preferred_engines, text) {
            Ok(realized) => {
                return Ok(success(
                    definition,
                    requested,
                    realized,
                    ResolutionReason::PreferredEngine {
                        preferred_index: index,
                    },
                    attempts,
                ));
            }
            Err(failure) => attempts.push(ResolutionAttempt {
                stage: ResolutionStage::PreferredEngine { index },
                selector,
                failure,
            }),
        }
    }

    if let Some(selector) = &policy.global_default {
        match evaluate_selector(engines, selector, &policy.preferred_engines, text) {
            Ok(realized) => {
                return Ok(success(
                    definition,
                    requested,
                    realized,
                    ResolutionReason::GlobalDefault,
                    attempts,
                ));
            }
            Err(failure) => attempts.push(ResolutionAttempt {
                stage: ResolutionStage::GlobalDefault,
                selector: selector.clone(),
                failure,
            }),
        }
    }

    for (index, engine_id) in policy.fallback_engines.iter().enumerate() {
        let selector = VoiceSelector::EngineDefault {
            engine_id: engine_id.clone(),
        };
        match evaluate_selector(engines, &selector, &policy.preferred_engines, text) {
            Ok(realized) => {
                return Ok(success(
                    definition,
                    requested,
                    realized,
                    ResolutionReason::FallbackEngine {
                        fallback_index: index,
                    },
                    attempts,
                ));
            }
            Err(failure) => attempts.push(ResolutionAttempt {
                stage: ResolutionStage::FallbackEngine { index },
                selector,
                failure,
            }),
        }
    }

    Err(VoiceResolutionError {
        logical_voice_id: definition.id.clone(),
        attempts,
    })
}

fn success(
    definition: &LogicalVoiceDefinition,
    requested: Option<VoiceSelector>,
    realized: PhysicalVoiceId,
    reason: ResolutionReason,
    failed_attempts: Vec<ResolutionAttempt>,
) -> VoiceResolution {
    VoiceResolution {
        logical_voice_id: definition.id.clone(),
        requested,
        realized,
        reason,
        failed_attempts,
    }
}

fn evaluate_selector(
    engines: &[EngineDescriptor],
    selector: &VoiceSelector,
    preferred_engines: &[String],
    text: Option<&str>,
) -> Result<PhysicalVoiceId, ResolutionFailure> {
    match selector {
        VoiceSelector::Exact(id) => evaluate_exact(engines, id, text),
        VoiceSelector::EngineDefault { engine_id } => {
            let engine = find_usable_engine(engines, engine_id)?;
            let voice = choose_voice(engine, None, None)?;
            ensure_text_supported(engine, text)?;
            Ok(voice)
        }
        VoiceSelector::Properties {
            engine_id,
            language,
            gender,
        } => {
            if let Some(engine_id) = engine_id {
                let engine = find_usable_engine(engines, engine_id)?;
                let voice = choose_voice(engine, language.as_deref(), *gender)?;
                ensure_text_supported(engine, text)?;
                Ok(voice)
            } else {
                choose_across_engines(
                    engines,
                    language.as_deref(),
                    *gender,
                    preferred_engines,
                    text,
                )
            }
        }
    }
}

fn evaluate_exact(
    engines: &[EngineDescriptor],
    id: &PhysicalVoiceId,
    text: Option<&str>,
) -> Result<PhysicalVoiceId, ResolutionFailure> {
    let engine = find_usable_engine(engines, &id.engine_id)?;
    let voice = engine
        .voices
        .iter()
        .find(|voice| voice.id.voice_id == id.voice_id)
        .ok_or_else(|| ResolutionFailure::VoiceNotFound { id: id.clone() })?;

    match &voice.availability {
        Availability::Available => {
            ensure_text_supported(engine, text)?;
            Ok(voice.id.clone())
        }
        Availability::Unavailable { reason } => Err(ResolutionFailure::VoiceUnavailable {
            id: id.clone(),
            reason: reason.clone(),
        }),
    }
}

fn find_usable_engine<'a>(
    engines: &'a [EngineDescriptor],
    engine_id: &str,
) -> Result<&'a EngineDescriptor, ResolutionFailure> {
    let engine = engines
        .iter()
        .find(|engine| engine.id == engine_id)
        .ok_or_else(|| ResolutionFailure::EngineNotFound {
            engine_id: engine_id.to_owned(),
        })?;

    match &engine.availability {
        Availability::Unavailable { reason } => {
            return Err(ResolutionFailure::EngineUnavailable {
                engine_id: engine_id.to_owned(),
                reason: reason.clone(),
            });
        }
        Availability::Available => {}
    }

    if let EngineHealth::Failed { reason } = &engine.health {
        return Err(ResolutionFailure::EngineFailed {
            engine_id: engine_id.to_owned(),
            reason: reason.clone(),
        });
    }

    Ok(engine)
}

fn ensure_text_supported(
    engine: &EngineDescriptor,
    text: Option<&str>,
) -> Result<(), ResolutionFailure> {
    match text.and_then(|text| text_failure(engine, text)) {
        Some(failure) => Err(failure),
        None => Ok(()),
    }
}

fn text_failure(engine: &EngineDescriptor, text: &str) -> Option<ResolutionFailure> {
    engine
        .capabilities
        .text_repertoire
        .first_unsupported(text)
        .map(
            |(utf8_offset, character)| ResolutionFailure::TextUnsupported {
                engine_id: engine.id.clone(),
                text_repertoire: engine.capabilities.text_repertoire,
                utf8_offset,
                codepoint: u32::from(character),
            },
        )
}

fn choose_across_engines(
    engines: &[EngineDescriptor],
    language: Option<&str>,
    gender: Option<crate::contracts::VoiceGender>,
    preferred_engines: &[String],
    text: Option<&str>,
) -> Result<PhysicalVoiceId, ResolutionFailure> {
    let mut candidates: Vec<&EngineDescriptor> = engines
        .iter()
        .filter(|engine| engine.can_synthesize())
        .collect();
    candidates.sort_by(|left, right| {
        let left_priority = preferred_engines
            .iter()
            .position(|engine_id| engine_id == &left.id)
            .unwrap_or(usize::MAX);
        let right_priority = preferred_engines
            .iter()
            .position(|engine_id| engine_id == &right.id)
            .unwrap_or(usize::MAX);
        left_priority
            .cmp(&right_priority)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut first_text_failure = None;
    for engine in candidates {
        let Ok(voice) = choose_voice(engine, language, gender) else {
            continue;
        };
        if let Some(failure) = text.and_then(|text| text_failure(engine, text)) {
            first_text_failure.get_or_insert(failure);
            continue;
        }
        return Ok(voice);
    }

    Err(first_text_failure.unwrap_or(ResolutionFailure::NoMatchingVoice))
}

fn choose_voice(
    engine: &EngineDescriptor,
    language: Option<&str>,
    gender: Option<crate::contracts::VoiceGender>,
) -> Result<PhysicalVoiceId, ResolutionFailure> {
    let mut voices: Vec<&VoiceDescriptor> = engine
        .voices
        .iter()
        .filter(|voice| voice.availability.is_available())
        .filter(|voice| language_matches(voice, language))
        .filter(|voice| gender.is_none() || voice.gender == gender)
        .collect();

    voices.sort_by(|left, right| {
        let left_default = engine.default_voice_id.as_deref() == Some(&left.id.voice_id);
        let right_default = engine.default_voice_id.as_deref() == Some(&right.id.voice_id);
        right_default
            .cmp(&left_default)
            .then_with(|| left.id.voice_id.cmp(&right.id.voice_id))
    });

    voices
        .first()
        .map(|voice| voice.id.clone())
        .ok_or(ResolutionFailure::NoMatchingVoice)
}

fn language_matches(voice: &VoiceDescriptor, requested: Option<&str>) -> bool {
    match (voice.language.as_deref(), requested) {
        (_, None) => true,
        (Some(actual), Some(requested)) => actual.eq_ignore_ascii_case(requested),
        (None, Some(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{
        AcssCapabilities, AudioOutputMode, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, MarkerCapabilities, NormalizedAcss, VoiceGender,
    };
    use crate::VoiceQuality;

    fn voice(engine_id: &str, voice_id: &str, language: &str) -> VoiceDescriptor {
        VoiceDescriptor {
            id: PhysicalVoiceId::new(engine_id, voice_id),
            display_name: voice_id.to_owned(),
            language: Some(language.to_owned()),
            gender: None,
            quality: VoiceQuality::Enhanced,
            availability: Availability::Available,
        }
    }

    fn engine(id: &str, default_voice_id: &str, voices: Vec<VoiceDescriptor>) -> EngineDescriptor {
        EngineDescriptor {
            id: id.to_owned(),
            display_name: id.to_owned(),
            version: None,
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: EngineCapabilities {
                acss: AcssCapabilities::default(),
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::PlaybackOnly,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities::default(),
                language_switching: false,
                text_repertoire: crate::contracts::TextRepertoire::Unicode,
                post_synthesis_dimensions: Vec::new(),
                native_extensions: Vec::new(),
            },
            voices,
            default_voice_id: Some(default_voice_id.to_owned()),
        }
    }

    fn logical(preferences: Vec<VoiceSelector>) -> LogicalVoiceDefinition {
        LogicalVoiceDefinition {
            id: "source-code".to_owned(),
            language: Some("en-US".to_owned()),
            preferences,
            acss: NormalizedAcss::default(),
            effects: Default::default(),
        }
    }

    fn exact(engine_id: &str, voice_id: &str) -> VoiceSelector {
        VoiceSelector::Exact(PhysicalVoiceId::new(engine_id, voice_id))
    }

    #[test]
    fn exact_preferred_voice_wins() {
        let engines = vec![engine(
            "dectalk",
            "paul",
            vec![voice("dectalk", "paul", "en-US")],
        )];

        let resolution = resolve_voice(
            &engines,
            &logical(vec![exact("dectalk", "paul")]),
            &FallbackPolicy::default(),
        )
        .unwrap();

        assert_eq!(resolution.realized, PhysicalVoiceId::new("dectalk", "paul"));
        assert_eq!(resolution.reason, ResolutionReason::Preferred);
        assert!(resolution.failed_attempts.is_empty());
    }

    #[test]
    fn text_resolution_records_repertoire_degradation_before_fallback() {
        let mut helper = engine("eloquence", "v1", vec![voice("eloquence", "v1", "en-US")]);
        helper.capabilities.text_repertoire = TextRepertoire::Windows1252;
        let mut unicode = engine("espeak", "en-us", vec![voice("espeak", "en-us", "en-US")]);
        unicode.capabilities.text_repertoire = TextRepertoire::Unicode;
        let policy = FallbackPolicy {
            fallback_engines: vec!["espeak".to_owned()],
            ..FallbackPolicy::default()
        };

        let resolution = resolve_voice_for_text(
            &[helper, unicode],
            &logical(vec![exact("eloquence", "v1")]),
            &policy,
            "Élan 日本",
        )
        .unwrap();

        assert_eq!(resolution.realized, PhysicalVoiceId::new("espeak", "en-us"));
        assert!(matches!(
            resolution.failed_attempts[0].failure,
            ResolutionFailure::TextUnsupported {
                ref engine_id,
                text_repertoire: TextRepertoire::Windows1252,
                utf8_offset: 6,
                codepoint: 0x65e5,
            } if engine_id == "eloquence"
        ));
    }

    #[test]
    fn missing_preferred_voice_uses_explicit_alternative() {
        let engines = vec![engine(
            "eloquence",
            "reed",
            vec![voice("eloquence", "reed", "en-US")],
        )];
        let definition = logical(vec![exact("dectalk", "paul"), exact("eloquence", "reed")]);

        let resolution = resolve_voice(&engines, &definition, &FallbackPolicy::default()).unwrap();

        assert_eq!(
            resolution.realized,
            PhysicalVoiceId::new("eloquence", "reed")
        );
        assert_eq!(
            resolution.reason,
            ResolutionReason::ExplicitAlternative {
                preference_index: 1
            }
        );
        assert_eq!(resolution.failed_attempts.len(), 1);
    }

    #[test]
    fn same_language_fallback_prefers_engine_default() {
        let engines = vec![engine(
            "winrt",
            "zira",
            vec![
                voice("winrt", "david", "en-US"),
                voice("winrt", "zira", "en-US"),
            ],
        )];
        let policy = FallbackPolicy {
            allow_same_language_on_requested_engine: true,
            ..FallbackPolicy::default()
        };

        let resolution =
            resolve_voice(&engines, &logical(vec![exact("winrt", "missing")]), &policy).unwrap();

        assert_eq!(resolution.realized, PhysicalVoiceId::new("winrt", "zira"));
        assert_eq!(
            resolution.reason,
            ResolutionReason::SameLanguageOnRequestedEngine
        );
    }

    #[test]
    fn global_default_follows_logical_preferences() {
        let engines = vec![engine(
            "winrt",
            "david",
            vec![voice("winrt", "david", "en-US")],
        )];
        let policy = FallbackPolicy {
            global_default: Some(exact("winrt", "david")),
            ..FallbackPolicy::default()
        };

        let resolution =
            resolve_voice(&engines, &logical(vec![exact("dectalk", "paul")]), &policy).unwrap();

        assert_eq!(resolution.reason, ResolutionReason::GlobalDefault);
        assert_eq!(resolution.realized, PhysicalVoiceId::new("winrt", "david"));
    }

    #[test]
    fn configured_fallback_engines_are_tried_in_order() {
        let mut failed = engine("dectalk", "paul", vec![voice("dectalk", "paul", "en-US")]);
        failed.health = EngineHealth::Failed {
            reason: "helper exited".to_owned(),
        };
        let engines = vec![
            failed,
            engine("espeak", "en-us", vec![voice("espeak", "en-us", "en-US")]),
        ];
        let policy = FallbackPolicy {
            fallback_engines: vec!["dectalk".to_owned(), "espeak".to_owned()],
            ..FallbackPolicy::default()
        };

        let resolution = resolve_voice(&engines, &logical(Vec::new()), &policy).unwrap();

        assert_eq!(resolution.realized, PhysicalVoiceId::new("espeak", "en-us"));
        assert_eq!(
            resolution.reason,
            ResolutionReason::FallbackEngine { fallback_index: 1 }
        );
    }

    #[test]
    fn global_preferred_engines_follow_explicit_logical_alternatives() {
        let engines = vec![
            engine("winrt", "david", vec![voice("winrt", "david", "en-US")]),
            engine(
                "eloquence",
                "reed",
                vec![voice("eloquence", "reed", "en-US")],
            ),
        ];
        let policy = FallbackPolicy {
            preferred_engines: vec!["eloquence".to_owned(), "winrt".to_owned()],
            ..FallbackPolicy::default()
        };

        let global = resolve_voice(&engines, &logical(Vec::new()), &policy).unwrap();
        let explicit = resolve_voice(
            &engines,
            &logical(vec![exact("winrt", "david")]),
            &policy,
        )
        .unwrap();

        assert_eq!(global.realized, PhysicalVoiceId::new("eloquence", "reed"));
        assert_eq!(
            global.reason,
            ResolutionReason::PreferredEngine { preferred_index: 0 }
        );
        assert_eq!(explicit.realized, PhysicalVoiceId::new("winrt", "david"));
        assert_eq!(explicit.reason, ResolutionReason::Preferred);
    }

    #[test]
    fn property_selector_late_binds_to_each_environment() {
        let definition = logical(vec![VoiceSelector::Properties {
            engine_id: None,
            language: Some("en-US".to_owned()),
            gender: Some(VoiceGender::Male),
        }]);
        let mut david = voice("winrt", "david", "en-US");
        david.gender = Some(VoiceGender::Male);
        let mut reed = voice("eloquence", "reed", "en-US");
        reed.gender = Some(VoiceGender::Male);

        let work = resolve_voice(
            &[engine("winrt", "david", vec![david])],
            &definition,
            &FallbackPolicy::default(),
        )
        .unwrap();
        let home = resolve_voice(
            &[engine("eloquence", "reed", vec![reed])],
            &definition,
            &FallbackPolicy::default(),
        )
        .unwrap();

        assert_eq!(work.realized, PhysicalVoiceId::new("winrt", "david"));
        assert_eq!(home.realized, PhysicalVoiceId::new("eloquence", "reed"));
    }

    #[test]
    fn degraded_engine_remains_eligible() {
        let mut degraded = engine("dectalk", "paul", vec![voice("dectalk", "paul", "en-US")]);
        degraded.health = EngineHealth::Degraded {
            reason: "markers unavailable".to_owned(),
        };

        let resolution = resolve_voice(
            &[degraded],
            &logical(vec![exact("dectalk", "paul")]),
            &FallbackPolicy::default(),
        )
        .unwrap();

        assert_eq!(resolution.realized, PhysicalVoiceId::new("dectalk", "paul"));
    }

    #[test]
    fn resolution_can_be_repeated_after_runtime_failure() {
        let primary = engine("dectalk", "paul", vec![voice("dectalk", "paul", "en-US")]);
        let fallback = engine(
            "eloquence",
            "reed",
            vec![voice("eloquence", "reed", "en-US")],
        );
        let definition = logical(vec![exact("dectalk", "paul"), exact("eloquence", "reed")]);

        let first = resolve_voice(
            &[primary.clone(), fallback.clone()],
            &definition,
            &FallbackPolicy::default(),
        )
        .unwrap();
        let mut failed_primary = primary;
        failed_primary.health = EngineHealth::Failed {
            reason: "synthesis failed".to_owned(),
        };
        let retried = resolve_voice(
            &[failed_primary, fallback],
            &definition,
            &FallbackPolicy::default(),
        )
        .unwrap();

        assert_eq!(first.realized, PhysicalVoiceId::new("dectalk", "paul"));
        assert_eq!(retried.realized, PhysicalVoiceId::new("eloquence", "reed"));
    }

    #[test]
    fn total_failure_returns_all_diagnostic_attempts() {
        let policy = FallbackPolicy {
            global_default: Some(exact("winrt", "david")),
            fallback_engines: vec!["espeak".to_owned()],
            ..FallbackPolicy::default()
        };

        let error =
            resolve_voice(&[], &logical(vec![exact("dectalk", "paul")]), &policy).unwrap_err();

        assert_eq!(error.logical_voice_id, "source-code");
        assert_eq!(error.attempts.len(), 3);
        assert!(matches!(
            error.attempts[0].failure,
            ResolutionFailure::EngineNotFound { .. }
        ));
    }
}
