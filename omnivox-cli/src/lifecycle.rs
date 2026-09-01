//! Correlated, process-local speech lifecycle timing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

const UNRECORDED: u64 = 0;

#[derive(Debug, Default)]
struct LifecycleState {
    admitted_at: OnceLock<Instant>,
    audio_queued_us: AtomicU64,
    mixer_source_started_us: AtomicU64,
}

/// Shared monotonic timings for one synthesis request.
///
/// Stored durations are offset by one so zero microseconds remains a valid
/// measurement while zero in the atomic fields means "not recorded".
#[derive(Clone, Debug, Default)]
pub(crate) struct RequestLifecycle {
    state: Arc<LifecycleState>,
}

impl RequestLifecycle {
    pub(crate) fn commit_admission(&self) {
        let _ = self.state.admitted_at.set(Instant::now());
    }

    pub(crate) fn admitted_at(&self) -> Option<Instant> {
        self.state.admitted_at.get().copied()
    }

    pub(crate) fn elapsed_us_at(&self, observed_at: Instant) -> Option<u64> {
        self.admitted_at()
            .map(|admitted_at| duration_us(observed_at.saturating_duration_since(admitted_at)))
    }

    pub(crate) fn elapsed_us(&self) -> Option<u64> {
        self.elapsed_us_at(Instant::now())
    }

    pub(crate) fn record_audio_queued_at(&self, queued_at: Instant) {
        self.record_first_elapsed(&self.state.audio_queued_us, queued_at);
    }

    pub(crate) fn audio_queued_us(&self) -> Option<u64> {
        recorded_elapsed(&self.state.audio_queued_us)
    }

    pub(crate) fn record_mixer_source_started(&self) {
        self.record_first_elapsed(&self.state.mixer_source_started_us, Instant::now());
    }

    pub(crate) fn mixer_source_started_us(&self) -> Option<u64> {
        recorded_elapsed(&self.state.mixer_source_started_us)
    }

    fn record_first_elapsed(&self, destination: &AtomicU64, observed_at: Instant) {
        if let Some(elapsed_us) = self.elapsed_us_at(observed_at) {
            let stored = elapsed_us.saturating_add(1);
            let _ = destination.compare_exchange(
                UNRECORDED,
                stored,
                Ordering::Release,
                Ordering::Relaxed,
            );
        }
    }
}

fn duration_us(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn recorded_elapsed(source: &AtomicU64) -> Option<u64> {
    source.load(Ordering::Acquire).checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_each_lifecycle_boundary_once_after_admission() {
        let lifecycle = RequestLifecycle::default();
        lifecycle.record_audio_queued_at(Instant::now());
        lifecycle.record_mixer_source_started();
        assert_eq!(lifecycle.audio_queued_us(), None);
        assert_eq!(lifecycle.mixer_source_started_us(), None);

        lifecycle.commit_admission();
        let admitted_at = lifecycle.admitted_at().unwrap();
        lifecycle.record_audio_queued_at(admitted_at);
        lifecycle.record_mixer_source_started();
        let first_source = lifecycle.mixer_source_started_us().unwrap();

        lifecycle.record_audio_queued_at(Instant::now());
        lifecycle.record_mixer_source_started();
        assert_eq!(lifecycle.audio_queued_us(), Some(0));
        assert_eq!(lifecycle.mixer_source_started_us(), Some(first_source));
    }

    #[test]
    fn clones_share_the_same_measurements() {
        let lifecycle = RequestLifecycle::default();
        lifecycle.commit_admission();
        let clone = lifecycle.clone();

        clone.record_mixer_source_started();

        assert_eq!(
            lifecycle.mixer_source_started_us(),
            clone.mixer_source_started_us()
        );
    }
}
