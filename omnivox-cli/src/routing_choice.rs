//! CLI-owned preparation and commitment of a single actual voice attempt.

use super::*;
use omnivox_tts::voice_choices::VoiceStylePatch;

/// A span's immutable input. Layered context never lives on mutable routing state.
pub(crate) enum AttemptStyle<'a> {
    Legacy {
        settings: &'a TtsSettings,
        acss: Option<&'a NormalizedAcss>,
    },
    Layered {
        context: &'a VoiceStylePatch,
        base_rate: f32,
        placement_pan: Option<f32>,
    },
}

/// Values belong to the actual attempt, independently of subsequent route mutations.
#[derive(Clone, Debug)]
pub(crate) struct PreparedVoiceAttempt {
    pub registry_generation: u64,
    pub resolution: VoiceResolution,
    #[cfg_attr(not(test), expect(dead_code))]
    pub choice_index: Option<usize>,
    pub choice_id: Option<String>,
    pub settings: TtsSettings,
    pub acss: AcssApplication,
    pub effects: PostSynthesisApplication,
}

impl PreparedVoiceAttempt {
    pub(crate) fn audio_identity(&self) -> omnivox_tts::voice_choices::AudioChoiceIdentity {
        omnivox_tts::voice_choices::AudioChoiceIdentity {
            choice_id: self.choice_id.clone(),
            reason: self.resolution.reason.clone(),
            realized: self.resolution.realized.clone(),
            degraded_acss: self.acss.omitted.clone(),
            degraded_effects: self.effects.omitted.clone(),
        }
    }
}

impl AttemptStyle<'_> {
    pub(crate) fn prepare(
        &self,
        routing: &LogicalVoiceRoutingSnapshot,
        route: &LogicalRoute,
        descriptor: &EngineDescriptor,
    ) -> Result<PreparedVoiceAttempt, String> {
        let (mut settings, acss, effects, choice_index, choice_id) = match self {
            Self::Legacy { settings, acss } => (
                (*settings).clone(),
                acss.map_or_else(
                    || route.acss.clone(),
                    |style| style.clone().degrade_for(&descriptor.capabilities.acss),
                ),
                route.effects.clone(),
                None,
                None,
            ),
            Self::Layered {
                context,
                base_rate,
                placement_pan,
            } => {
                let definition = routing
                    .layered_definitions
                    .iter()
                    .find(|definition| definition.id == route.logical_voice_id)
                    .ok_or("layered request has no admitted layered definition")?;
                let index = definition
                    .selected_choice(&route.resolution)
                    .map_err(|error| error.to_string())?;
                let composed = definition
                    .compose(index, context, *base_rate, *placement_pan)
                    .map_err(|error| error.to_string())?;
                (
                    TtsSettings {
                        rate: *base_rate,
                        ..TtsSettings::default()
                    },
                    composed.acss.degrade_for(&descriptor.capabilities.acss),
                    composed
                        .effects
                        .degrade_for(&descriptor.capabilities.post_synthesis_dimensions),
                    index,
                    composed.choice_id,
                )
            }
        };
        settings.voice = route.realized.voice_id.clone();
        apply_normalized_acss(&mut settings, &acss.style);
        let prepared = PreparedVoiceAttempt {
            registry_generation: routing.registry_generation,
            resolution: route.resolution.clone(),
            choice_index,
            choice_id,
            settings,
            acss,
            effects,
        };
        if matches!(self, Self::Layered { .. }) {
            omnivox_tts::voice_preview_v2::validate_audio_choice_identity(
                &prepared.audio_identity(),
            )?;
        }
        Ok(prepared)
    }
}

/// The engine-facing stream adapter publishes identity and style together.
pub(crate) trait RoutedPlaybackSink {
    fn start_attempt(
        &mut self,
        attempt: &PreparedVoiceAttempt,
        start: SynthesisStreamStart,
    ) -> Result<(), TtsError>;
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError>;
    fn markers(
        &mut self,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<ResolvedAnchor>,
    ) -> Result<(), TtsError>;
}

pub(crate) struct LegacyPlaybackSink<'a>(pub &'a mut dyn SynthesisStreamSink);

impl RoutedPlaybackSink for LegacyPlaybackSink<'_> {
    fn start_attempt(
        &mut self,
        _attempt: &PreparedVoiceAttempt,
        start: SynthesisStreamStart,
    ) -> Result<(), TtsError> {
        self.0.start(start)
    }

    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        self.0.audio(audio)
    }

    fn markers(
        &mut self,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<ResolvedAnchor>,
    ) -> Result<(), TtsError> {
        self.0.markers(markers, anchors)
    }
}

pub(crate) enum PreparedSynthesisOutcome {
    Streamed(SynthesisStreamCompletion),
    Buffered {
        result: Box<SynthesisResult>,
        attempt: Box<PreparedVoiceAttempt>,
    },
    Cancelled,
    Failed,
    Exhausted,
}

impl PreparedSynthesisOutcome {
    pub(crate) fn into_legacy(self) -> RuntimeProgressiveSynthesisOutcome {
        match self {
            Self::Streamed(completion) => RuntimeProgressiveSynthesisOutcome::Streamed(completion),
            Self::Buffered { result, .. } => RuntimeProgressiveSynthesisOutcome::Buffered(result),
            Self::Cancelled => RuntimeProgressiveSynthesisOutcome::Cancelled,
            Self::Failed => RuntimeProgressiveSynthesisOutcome::Failed,
            Self::Exhausted => RuntimeProgressiveSynthesisOutcome::Exhausted,
        }
    }

    pub(crate) fn from_retry(outcome: RuntimeSynthesisOutcome) -> Self {
        match outcome {
            RuntimeSynthesisOutcome::Ready(_) => unreachable!("retry selection never succeeds"),
            RuntimeSynthesisOutcome::Cancelled => Self::Cancelled,
            RuntimeSynthesisOutcome::Failed => Self::Failed,
            RuntimeSynthesisOutcome::Exhausted => Self::Exhausted,
        }
    }
}
