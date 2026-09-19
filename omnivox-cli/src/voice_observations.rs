//! Request-local evidence, updated at acceptance and at actual source consumption.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use omnivox_tts::voice_choices::AudioChoiceIdentity;
use omnivox_tts::voice_preview_v3::NativeAudioChoiceIdentity;

use crate::routing::choice::PreparedVoiceAttempt;

use omnivox_tts::voice_preview_v2::MAX_ACCEPTED_AUDIO_CHOICES;

#[derive(Clone)]
pub(crate) struct VoiceObservation {
    identity: Arc<NativeAudioChoiceIdentity>,
    started: Arc<AtomicBool>,
    source_started: Arc<AtomicBool>,
    last_started: Arc<Mutex<Option<Arc<NativeAudioChoiceIdentity>>>>,
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
    failure_message: Option<String>,
    native_plans: Option<Arc<crate::native_plans::NativePlanReferences>>,
    accepted: Vec<VoiceObservation>,
    truncated: bool,
    last_started: Arc<Mutex<Option<Arc<NativeAudioChoiceIdentity>>>>,
}

pub(crate) struct VoiceObservationSnapshot {
    pub accepted: Vec<(AudioChoiceIdentity, bool)>,
    pub truncated: bool,
    pub last_started: Option<AudioChoiceIdentity>,
}

pub(crate) struct NativeVoiceObservationSnapshot {
    pub accepted: Vec<(NativeAudioChoiceIdentity, bool)>,
    pub truncated: bool,
    pub last_started: Option<NativeAudioChoiceIdentity>,
}

impl VoiceObservations {
    pub(crate) fn reject_preparation(&mut self, message: &str) {
        let mut end = message
            .len()
            .min(omnivox_tts::voice_preview_v2::MAX_PREVIEW_MESSAGE_BYTES);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        self.failure_message = Some(message[..end].to_owned());
    }
    pub(crate) fn failure_message(&self) -> Option<String> {
        self.failure_message.clone()
    }

    pub(crate) fn native(plans: Arc<crate::native_plans::NativePlanReferences>) -> Self {
        Self {
            native_plans: Some(plans),
            ..Default::default()
        }
    }
    pub(crate) fn supports_native(&self) -> bool {
        self.native_plans.is_some()
    }

    /// The synthesis producer prepares and publishes sources sequentially.
    /// Allocate the handle before enqueue: null playback can win the ack race.
    pub(crate) fn prepare(
        &self,
        attempt: &PreparedVoiceAttempt,
    ) -> Result<VoiceObservation, String> {
        let application = self.native_plans.as_ref().and_then(|plans| {
            attempt.native_application.as_ref().map(|application| {
                plans.publish(
                    &attempt.native_runtime,
                    &attempt.resolution.realized,
                    application,
                    attempt.choice_id.as_deref(),
                )
            })
        });
        let identity = NativeAudioChoiceIdentity::from_parts(attempt.audio_identity(), application);
        identity.validate()?;
        let existing = self.accepted.iter().find(|item| *item.identity == identity);
        Ok(VoiceObservation {
            identity: existing.map_or_else(|| Arc::new(identity), |item| item.identity.clone()),
            started: existing.map_or_else(
                || Arc::new(AtomicBool::new(false)),
                |item| item.started.clone(),
            ),
            source_started: Arc::new(AtomicBool::new(false)),
            last_started: self.last_started.clone(),
        })
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
        let native = self.native_snapshot();
        VoiceObservationSnapshot {
            accepted: native
                .accepted
                .into_iter()
                .map(|(id, started)| (id.common(), started))
                .collect(),
            truncated: native.truncated,
            last_started: native.last_started.map(|id| id.common()),
        }
    }
    pub(crate) fn native_snapshot(&self) -> NativeVoiceObservationSnapshot {
        let last_started = self.last_started.lock().unwrap().clone();
        NativeVoiceObservationSnapshot {
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
            kind: crate::routing::choice::VoiceAttemptKind::Layered,
            native: omnivox_tts::engine_voice_choices::NativeChoiceExecution::NotRequested,
            native_application: None,
            native_runtime: None,
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
        let observation = evidence.prepare(&attempt("first")).unwrap();
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
        let observation = evidence.prepare(&attempt("first")).unwrap();
        evidence.accept(&observation);
        drop(observation);
        let snapshot = evidence.snapshot();
        assert!(!snapshot.accepted[0].1);
        assert_eq!(snapshot.last_started, None);
    }

    #[test]
    fn repeated_sources_share_started_evidence_but_each_updates_last_once() {
        let mut evidence = VoiceObservations::default();
        let first = evidence.prepare(&attempt("first")).unwrap();
        evidence.accept(&first);
        let second = evidence.prepare(&attempt("second")).unwrap();
        evidence.accept(&second);
        let repeated = evidence.prepare(&attempt("first")).unwrap();
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
        let initial = evidence.prepare(&changed).unwrap();
        evidence.accept(&initial);
        changed.resolution.reason = ResolutionReason::GlobalDefault;
        changed.choice_id = None;
        let policy = evidence.prepare(&changed).unwrap();
        evidence.accept(&policy);
        changed
            .acss
            .omitted
            .push(omnivox_tts::contracts::AcssDimension::Richness);
        let degraded = evidence.prepare(&changed).unwrap();
        evidence.accept(&degraded);
        assert_eq!(evidence.snapshot().accepted.len(), 3);
    }

    #[test]
    fn last_started_survives_list_truncation_and_unconsumed_later_attempts() {
        let mut evidence = VoiceObservations::default();
        for index in 0..34 {
            let observation = evidence
                .prepare(&attempt(&format!("choice-{index}")))
                .unwrap();
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
    #[test]
    fn native_evidence_retains_accepted_without_inventing_start_and_distinguishes_plans() {
        let mut observations = VoiceObservations::native(Arc::new(
            crate::native_plans::NativePlanReferences::default(),
        ));
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/engine-voice-parameters.json"
        ))
        .unwrap();
        let mut attempt = attempt("same-choice");
        attempt.native_application = Some(
            serde_json::from_value(
                fixture["messages"]["playback_receipt"]["native_application"].clone(),
            )
            .unwrap(),
        );
        for index in 0..34 {
            let observation = observations.prepare(&attempt).unwrap();
            observations.accept(&observation);
            if index == 0 {
                assert!(observations.native_snapshot().last_started.is_none());
            }
            if index < 33 {
                observation.first_frame();
            }
        }
        let snapshot = observations.native_snapshot();
        assert_eq!(snapshot.accepted.len(), 32);
        assert!(snapshot.truncated);
        assert!(snapshot.accepted.iter().all(|(_, started)| *started));
        assert_ne!(
            snapshot.accepted[0].0.native_application,
            snapshot.accepted[1].0.native_application
        );
        assert_eq!(
            snapshot
                .last_started
                .unwrap()
                .native_application
                .unwrap()
                .plan_id
                .as_deref(),
            Some("native-plan-33")
        );
    }
}
