//! Request-local evidence, updated at acceptance and at actual source consumption.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use omnivox_tts::voice_choices::AudioChoiceIdentity;

use crate::routing::choice::PreparedVoiceAttempt;

use omnivox_tts::voice_preview_v2::MAX_ACCEPTED_AUDIO_CHOICES;

#[derive(Clone)]
pub(crate) struct VoiceObservation {
    identity: Arc<AudioChoiceIdentity>,
    started: Arc<AtomicBool>,
    source_started: Arc<AtomicBool>,
    last_started: Arc<Mutex<Option<Arc<AudioChoiceIdentity>>>>,
}

impl VoiceObservation {
    /// Called only by the source's first nonempty frame cue. Work is bounded:
    /// no formatting, allocation, registry lookup, producer lock or output I/O.
    pub(crate) fn first_frame(&self) {
        if self.source_started.swap(true, Ordering::AcqRel) {
            return;
        }
        self.started.store(true, Ordering::Release);
        *self.last_started.lock().unwrap() = Some(Arc::clone(&self.identity));
    }
}

#[derive(Default)]
pub(crate) struct VoiceObservations {
    accepted: Vec<VoiceObservation>,
    truncated: bool,
    last_started: Arc<Mutex<Option<Arc<AudioChoiceIdentity>>>>,
}

pub(crate) struct VoiceObservationSnapshot {
    pub accepted: Vec<(AudioChoiceIdentity, bool)>,
    pub truncated: bool,
    pub last_started: Option<AudioChoiceIdentity>,
}

impl VoiceObservations {
    /// The synthesis producer prepares and publishes sources sequentially.
    /// Allocate the handle before enqueue: null playback can win the ack race.
    pub(crate) fn prepare(&self, attempt: &PreparedVoiceAttempt) -> VoiceObservation {
        let identity = attempt.audio_identity();
        let existing = self.accepted.iter().find(|item| *item.identity == identity);
        VoiceObservation {
            identity: existing.map_or_else(|| Arc::new(identity), |item| item.identity.clone()),
            started: existing.map_or_else(
                || Arc::new(AtomicBool::new(false)),
                |item| item.started.clone(),
            ),
            source_started: Arc::new(AtomicBool::new(false)),
            last_started: self.last_started.clone(),
        }
    }

    /// Success of a nonempty enqueue, or published progressive frames even on error.
    pub(crate) fn accept(&mut self, observation: &VoiceObservation) {
        if self
            .accepted
            .iter()
            .any(|item| item.identity == observation.identity)
        {
            return;
        }
        if self.accepted.len() == MAX_ACCEPTED_AUDIO_CHOICES {
            self.truncated = true;
        } else {
            self.accepted.push(observation.clone());
        }
    }

    /// Terminal callers must wait for ALL source tickets before taking this snapshot.
    pub(crate) fn snapshot(&self) -> VoiceObservationSnapshot {
        let last_started = self.last_started.lock().unwrap().clone();
        VoiceObservationSnapshot {
            accepted: self
                .accepted
                .iter()
                .map(|item| {
                    (
                        (*item.identity).clone(),
                        item.started.load(Ordering::Acquire),
                    )
                })
                .collect(),
            truncated: self.truncated,
            last_started: last_started.map(|identity| (*identity).clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::contracts::{NormalizedAcss, PhysicalVoiceId, PostSynthesisStyle};
    use omnivox_tts::resolver::{ResolutionReason, VoiceResolution};
    use omnivox_tts::TtsSettings;

    fn attempt(choice: &str) -> PreparedVoiceAttempt {
        PreparedVoiceAttempt {
            registry_generation: 41,
            resolution: VoiceResolution {
                logical_voice_id: "bolden".to_owned(),
                requested: None,
                realized: PhysicalVoiceId::new("eloquence", "Reed"),
                reason: ResolutionReason::Preferred,
                failed_attempts: Vec::new(),
            },
            choice_index: Some(0),
            choice_id: Some(choice.to_owned()),
            settings: TtsSettings::default(),
            acss: NormalizedAcss::default().degrade_for(&Default::default()),
            effects: PostSynthesisStyle::default().degrade_for(&[]),
        }
    }

    #[test]
    fn consumption_before_acceptance_bookkeeping_is_preserved() {
        let mut evidence = VoiceObservations::default();
        let observation = evidence.prepare(&attempt("first"));
        observation.first_frame();
        evidence.accept(&observation);
        let snapshot = evidence.snapshot();
        assert_eq!(snapshot.accepted.len(), 1);
        assert!(snapshot.accepted[0].1);
        assert_eq!(snapshot.last_started, Some(snapshot.accepted[0].0.clone()));
    }

    #[test]
    fn accepted_and_cancelled_before_consumption_has_no_last_started_voice() {
        let mut evidence = VoiceObservations::default();
        let observation = evidence.prepare(&attempt("first"));
        evidence.accept(&observation);
        drop(observation);
        let snapshot = evidence.snapshot();
        assert!(!snapshot.accepted[0].1);
        assert_eq!(snapshot.last_started, None);
    }

    #[test]
    fn repeated_sources_share_started_evidence_but_each_updates_last_once() {
        let mut evidence = VoiceObservations::default();
        let first = evidence.prepare(&attempt("first"));
        evidence.accept(&first);
        let second = evidence.prepare(&attempt("second"));
        evidence.accept(&second);
        let repeated = evidence.prepare(&attempt("first"));
        evidence.accept(&repeated);
        repeated.first_frame();
        second.first_frame();
        repeated.first_frame(); // A duplicate callback cannot reorder history.
        let snapshot = evidence.snapshot();
        assert_eq!(snapshot.accepted.len(), 2);
        assert!(snapshot.accepted.iter().all(|(_, started)| *started));
        assert_eq!(
            snapshot.last_started.unwrap().choice_id.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn equal_physical_voices_with_different_resolution_or_degradation_are_distinct() {
        let mut evidence = VoiceObservations::default();
        let mut changed = attempt("first");
        let initial = evidence.prepare(&changed);
        evidence.accept(&initial);
        changed.resolution.reason = ResolutionReason::GlobalDefault;
        changed.choice_id = None;
        let policy = evidence.prepare(&changed);
        evidence.accept(&policy);
        changed
            .acss
            .omitted
            .push(omnivox_tts::contracts::AcssDimension::Richness);
        let degraded = evidence.prepare(&changed);
        evidence.accept(&degraded);
        assert_eq!(evidence.snapshot().accepted.len(), 3);
    }

    #[test]
    fn last_started_survives_list_truncation_and_unconsumed_later_attempts() {
        let mut evidence = VoiceObservations::default();
        for index in 0..34 {
            let observation = evidence.prepare(&attempt(&format!("choice-{index}")));
            evidence.accept(&observation);
            if index < 33 {
                observation.first_frame();
            }
        }
        let snapshot = evidence.snapshot();
        assert_eq!(snapshot.accepted.len(), 32);
        assert!(snapshot.truncated);
        assert_eq!(
            snapshot.last_started.unwrap().choice_id.as_deref(),
            Some("choice-32")
        );
    }
}
