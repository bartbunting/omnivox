//! Bounded connection-local references to helper-owned applied plans.
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::native_parameters::CatalogueIdentity;
use omnivox_tts::native_synthesis::{ApplicationStatus, NativeApplication};
use omnivox_tts::TtsEngine;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};

#[derive(Default)]
pub(crate) struct NativePlanReferences(Mutex<State>);
#[derive(Default)]
struct State {
    next: u64,
    plans: VecDeque<PlanReference>,
}
#[derive(Clone)]
pub(crate) struct PlanReference {
    pub public_id: String,
    pub helper_id: String,
    pub voice: PhysicalVoiceId,
    pub choice_id: Option<String>,
    pub identity: CatalogueIdentity,
    pub owner: Weak<dyn TtsEngine>,
    pub epoch: Option<u64>,
}
impl NativePlanReferences {
    /// Called on the synthesis side, before audio can be consumed. Compact
    /// receipts survive eviction; only their optional detail lookup expires.
    pub(crate) fn publish(
        &self,
        runtime: &Option<(Weak<dyn TtsEngine>, u64)>,
        voice: &PhysicalVoiceId,
        application: &NativeApplication,
        choice_id: Option<&str>,
    ) -> NativeApplication {
        let mut application = application.clone();
        if application.status != ApplicationStatus::Applied {
            return application;
        }
        let mut state = self.0.lock().unwrap();
        // Exhaustion is not a recoverable state in one bounded worker lifetime.
        state.next = state
            .next
            .checked_add(1)
            .expect("native plan identity exhausted");
        let public_id = format!("native-plan-{}", state.next);
        if let (Some((owner, epoch)), Some(helper_id), Some(identity)) = (
            runtime,
            application.plan_id.as_ref(),
            application.identity.as_ref(),
        ) {
            state.plans.push_back(PlanReference {
                public_id: public_id.clone(),
                helper_id: helper_id.clone(),
                voice: voice.clone(),
                choice_id: choice_id.map(str::to_owned),
                identity: identity.clone(),
                owner: owner.clone(),
                epoch: Some(*epoch),
            });
            while state.plans.len() > 64 {
                state.plans.pop_front();
            }
        }
        application.plan_id = Some(public_id);
        application
    }
    pub(crate) fn lookup(&self, id: &str, engines: &EngineRegistry) -> Option<PlanReference> {
        let reference = self
            .0
            .try_lock()
            .ok()?
            .plans
            .iter()
            .find(|p| p.public_id == id)?
            .clone();
        let current = engines.engine(&reference.voice.engine_id)?;
        let owner = reference.owner.upgrade()?;
        (Arc::ptr_eq(&owner, &current)
            && reference.epoch.is_some()
            && reference.epoch == current.parameter_cache_epoch())
        .then_some(reference)
    }
}
