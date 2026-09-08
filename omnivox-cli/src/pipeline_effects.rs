//! Stateful effects belong to one committed voice choice within a dispatch.

use omnivox_audio::{AudioBuffer, PostSynthesisParameters, PostSynthesisProcessor};
use omnivox_tts::contracts::PhysicalVoiceId;

use crate::routing::choice::PreparedVoiceAttempt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EffectOwner {
    Legacy,
    Layered {
        registry_generation: u64,
        logical_voice_id: String,
        choice_id: Option<String>,
        realized: PhysicalVoiceId,
    },
}

impl EffectOwner {
    pub(crate) fn layered(attempt: &PreparedVoiceAttempt) -> Self {
        Self::Layered {
            registry_generation: attempt.registry_generation,
            logical_voice_id: attempt.resolution.logical_voice_id.clone(),
            choice_id: attempt.choice_id.clone(),
            realized: attempt.resolution.realized.clone(),
        }
    }
}

pub(crate) struct DispatchEffects {
    owner: EffectOwner,
    processor: PostSynthesisProcessor,
}

impl DispatchEffects {
    pub(crate) fn new() -> Self {
        Self {
            owner: EffectOwner::Legacy,
            processor: PostSynthesisProcessor::new(),
        }
    }

    /// Flush with the OLD parameters, including placement, before resetting.
    /// Selecting the same owner preserves filter history and parameter ramps.
    pub(crate) fn select(&mut self, owner: EffectOwner) -> Option<AudioBuffer> {
        if self.owner == owner {
            return None;
        }
        let tail = self.processor.finish();
        self.processor = PostSynthesisProcessor::new();
        self.owner = owner;
        tail
    }

    pub(crate) fn process_window(
        &mut self,
        audio: &AudioBuffer,
        parameters: PostSynthesisParameters,
        final_window: bool,
    ) -> omnivox_audio::post_synthesis::ProcessedEffectWindow {
        self.processor
            .process_window(audio, parameters, final_window)
    }

    /// Output failure must not turn rejected PCM into a later audible tail.
    pub(crate) fn discard(&mut self) {
        self.processor = PostSynthesisProcessor::new();
    }

    pub(crate) fn finish(&mut self) -> Option<AudioBuffer> {
        self.processor.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(choice: &str) -> EffectOwner {
        EffectOwner::Layered {
            registry_generation: 41,
            logical_voice_id: "bolden".to_owned(),
            choice_id: Some(choice.to_owned()),
            realized: PhysicalVoiceId::new("eloquence", "Reed"),
        }
    }

    #[test]
    fn same_owner_keeps_filter_history_but_duplicate_physical_rows_are_isolated() {
        let parameters = PostSynthesisParameters {
            low_pass_hz: Some(200.0),
            ..Default::default()
        };
        let mut processor = DispatchEffects::new();
        processor.select(owner("first"));
        processor.process_window(&AudioBuffer::new(vec![0.8; 2000]), parameters, false);
        assert!(processor.select(owner("first")).is_none());
        let silence = AudioBuffer::new(vec![0.0; 100]);
        let continued = processor.process_window(&silence, parameters, false);
        assert!(continued.audio.samples.iter().any(|sample| *sample != 0.0));
        processor.select(owner("second"));
        let clean = processor.process_window(&silence, parameters, false);
        assert!(clean.audio.samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn departed_owner_tail_keeps_its_pan_and_cannot_enter_the_new_owner() {
        let mut processor = DispatchEffects::new();
        processor.select(owner("first"));
        let parameters = PostSynthesisParameters {
            pan: -1.0,
            echo: 0.8,
            ..Default::default()
        };
        // Set placement before the impulse so the boundary ramp is not itself
        // stored as right-channel echo energy.
        processor.process_window(&AudioBuffer::empty(), parameters, false);
        processor.process_window(&AudioBuffer::new(vec![0.5; 4000]), parameters, false);
        let tail = processor
            .select(owner("second"))
            .expect("previous echo tail");
        assert!(tail.frame_count() <= omnivox_audio::MAX_EFFECT_TAIL_FRAMES);
        assert!(tail.samples.chunks_exact(2).any(|frame| frame[0] != 0.0));
        assert!(tail
            .samples
            .chunks_exact(2)
            .all(|frame| frame[1].abs() < 0.000001));
        assert!(processor.finish().is_none());
        let clean = processor.process_window(&AudioBuffer::new(vec![0.0; 4000]), parameters, false);
        assert!(clean.audio.samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn layered_boundary_ends_a_legacy_run_and_an_unused_attempt_has_no_tail() {
        let mut processor = DispatchEffects::new();
        let parameters = PostSynthesisParameters {
            low_pass_hz: Some(200.0),
            ..Default::default()
        };
        processor.process_window(&AudioBuffer::new(vec![0.8; 2000]), parameters, false);
        assert!(processor.select(owner("unused")).is_none());
        assert!(processor.select(EffectOwner::Legacy).is_none());
        let next_legacy =
            processor.process_window(&AudioBuffer::new(vec![0.0; 100]), parameters, false);
        assert!(next_legacy
            .audio
            .samples
            .iter()
            .all(|sample| *sample == 0.0));
    }
}
