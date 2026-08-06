//! Persistent runtime engine health and recovery-probe circuit breaking.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use omnivox_tts::contracts::{EngineDescriptor, EngineHealth};

const FIRST_COOLDOWN: Duration = Duration::from_secs(5);
const SECOND_COOLDOWN: Duration = Duration::from_secs(15);
const THIRD_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePermit {
    Normal,
    RecoveryProbe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineAccess {
    Permit(EnginePermit),
    Denied { reason: String },
}

pub struct RuntimeInventorySnapshot {
    pub generation: u64,
    pub engines: Vec<EngineDescriptor>,
}

#[derive(Debug)]
enum CircuitState {
    Open {
        failures: u32,
        reason: String,
        retry_at: Instant,
    },
    Ready {
        failures: u32,
        reason: String,
    },
    Probing {
        failures: u32,
        reason: String,
    },
}

#[derive(Default)]
struct RuntimeHealthInner {
    generation: u64,
    circuits: BTreeMap<String, CircuitState>,
}

#[derive(Default)]
pub struct RuntimeEngineHealth {
    inner: Mutex<RuntimeHealthInner>,
}

impl RuntimeEngineHealth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(
        &self,
        base_generation: u64,
        inventory: Vec<EngineDescriptor>,
    ) -> RuntimeInventorySnapshot {
        self.snapshot_at(base_generation, inventory, Instant::now())
    }

    pub fn acquire(&self, engine_id: &str) -> EngineAccess {
        self.acquire_at(engine_id, Instant::now())
    }

    pub fn record_failure(&self, engine_id: &str, reason: impl Into<String>) -> Duration {
        self.record_failure_at(engine_id, reason.into(), Instant::now())
    }

    pub fn record_success(&self, engine_id: &str, permit: EnginePermit) {
        if permit != EnginePermit::RecoveryProbe {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if matches!(
            inner.circuits.get(engine_id),
            Some(CircuitState::Probing { .. })
        ) {
            inner.circuits.remove(engine_id);
            advance_generation(&mut inner);
        }
    }

    pub fn release_probe(&self, engine_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        let Some(CircuitState::Probing { failures, reason }) = inner.circuits.remove(engine_id)
        else {
            return;
        };
        inner.circuits.insert(
            engine_id.to_owned(),
            CircuitState::Ready { failures, reason },
        );
        advance_generation(&mut inner);
    }

    fn snapshot_at(
        &self,
        base_generation: u64,
        mut inventory: Vec<EngineDescriptor>,
        now: Instant,
    ) -> RuntimeInventorySnapshot {
        let mut inner = self.inner.lock().unwrap();
        for descriptor in &mut inventory {
            promote_ready(&mut inner, &descriptor.id, now);
            if let Some(state) = inner.circuits.get(&descriptor.id) {
                descriptor.health = projected_health(state);
            }
        }
        RuntimeInventorySnapshot {
            generation: base_generation.saturating_add(inner.generation),
            engines: inventory,
        }
    }

    fn acquire_at(&self, engine_id: &str, now: Instant) -> EngineAccess {
        let mut inner = self.inner.lock().unwrap();
        promote_ready(&mut inner, engine_id, now);
        match inner.circuits.remove(engine_id) {
            None => EngineAccess::Permit(EnginePermit::Normal),
            Some(CircuitState::Ready { failures, reason }) => {
                inner.circuits.insert(
                    engine_id.to_owned(),
                    CircuitState::Probing { failures, reason },
                );
                advance_generation(&mut inner);
                EngineAccess::Permit(EnginePermit::RecoveryProbe)
            }
            Some(state @ CircuitState::Open { .. }) => {
                let reason = denial_reason(&state);
                inner.circuits.insert(engine_id.to_owned(), state);
                EngineAccess::Denied { reason }
            }
            Some(state @ CircuitState::Probing { .. }) => {
                let reason = denial_reason(&state);
                inner.circuits.insert(engine_id.to_owned(), state);
                EngineAccess::Denied { reason }
            }
        }
    }

    fn record_failure_at(&self, engine_id: &str, reason: String, now: Instant) -> Duration {
        let mut inner = self.inner.lock().unwrap();
        let previous_failures = match inner.circuits.remove(engine_id) {
            Some(CircuitState::Open { failures, .. })
            | Some(CircuitState::Ready { failures, .. })
            | Some(CircuitState::Probing { failures, .. }) => failures,
            None => 0,
        };
        let failures = previous_failures.saturating_add(1);
        let cooldown = cooldown_for(failures);
        inner.circuits.insert(
            engine_id.to_owned(),
            CircuitState::Open {
                failures,
                reason,
                retry_at: now + cooldown,
            },
        );
        advance_generation(&mut inner);
        cooldown
    }
}

fn promote_ready(inner: &mut RuntimeHealthInner, engine_id: &str, now: Instant) {
    let ready = matches!(
        inner.circuits.get(engine_id),
        Some(CircuitState::Open { retry_at, .. }) if now >= *retry_at
    );
    if !ready {
        return;
    }
    let Some(CircuitState::Open {
        failures, reason, ..
    }) = inner.circuits.remove(engine_id)
    else {
        unreachable!("the circuit state was checked before removal")
    };
    inner.circuits.insert(
        engine_id.to_owned(),
        CircuitState::Ready { failures, reason },
    );
    advance_generation(inner);
}

fn projected_health(state: &CircuitState) -> EngineHealth {
    match state {
        CircuitState::Open { reason, .. } => EngineHealth::Failed {
            reason: format!("runtime failure: {reason}; recovery probe pending"),
        },
        CircuitState::Ready { reason, .. } => EngineHealth::Degraded {
            reason: format!("runtime failure: {reason}; recovery probe ready"),
        },
        CircuitState::Probing { reason, .. } => EngineHealth::Failed {
            reason: format!("runtime failure: {reason}; recovery probe in progress"),
        },
    }
}

fn denial_reason(state: &CircuitState) -> String {
    match state {
        CircuitState::Open { reason, .. } => {
            format!("runtime failure: {reason}; recovery probe pending")
        }
        CircuitState::Probing { reason, .. } => {
            format!("runtime failure: {reason}; recovery probe in progress")
        }
        CircuitState::Ready { .. } => unreachable!("ready circuits issue a probe permit"),
    }
}

fn cooldown_for(failures: u32) -> Duration {
    match failures {
        0 | 1 => FIRST_COOLDOWN,
        2 => SECOND_COOLDOWN,
        3 => THIRD_COOLDOWN,
        _ => MAX_COOLDOWN,
    }
}

fn advance_generation(inner: &mut RuntimeHealthInner) {
    inner.generation = inner.generation.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::contracts::{
        AcssCapabilities, AudioOutputMode, Availability, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, MarkerCapabilities,
    };

    fn descriptor(engine_id: &str) -> EngineDescriptor {
        EngineDescriptor {
            id: engine_id.to_owned(),
            display_name: engine_id.to_owned(),
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
                native_extensions: Vec::new(),
            },
            voices: Vec::new(),
            default_voice_id: None,
        }
    }

    #[test]
    fn first_failure_opens_the_circuit_for_five_seconds() {
        let health = RuntimeEngineHealth::new();
        let now = Instant::now();

        let cooldown = health.record_failure_at("dectalk", "helper exited".to_owned(), now);
        let snapshot = health.snapshot_at(7, vec![descriptor("dectalk")], now);

        assert_eq!(cooldown, Duration::from_secs(5));
        assert_eq!(snapshot.generation, 8);
        assert!(matches!(
            snapshot.engines[0].health,
            EngineHealth::Failed { .. }
        ));
        assert!(matches!(
            health.acquire_at("dectalk", now + Duration::from_secs(4)),
            EngineAccess::Denied { .. }
        ));
    }

    #[test]
    fn public_snapshot_reports_a_new_runtime_failure() {
        let health = RuntimeEngineHealth::new();

        assert_eq!(
            health.record_failure("winrt", "runtime error"),
            Duration::from_secs(5)
        );
        let snapshot = health.snapshot(4, vec![descriptor("winrt")]);

        assert_eq!(snapshot.generation, 5);
        assert!(matches!(
            snapshot.engines[0].health,
            EngineHealth::Failed { .. }
        ));
    }

    #[test]
    fn cooldown_allows_exactly_one_recovery_probe() {
        let health = RuntimeEngineHealth::new();
        let now = Instant::now();
        health.record_failure_at("dectalk", "helper exited".to_owned(), now);

        let ready =
            health.snapshot_at(2, vec![descriptor("dectalk")], now + Duration::from_secs(5));
        let permit = health.acquire_at("dectalk", now + Duration::from_secs(5));
        let probing =
            health.snapshot_at(2, vec![descriptor("dectalk")], now + Duration::from_secs(5));

        assert!(matches!(
            ready.engines[0].health,
            EngineHealth::Degraded { .. }
        ));
        assert_eq!(permit, EngineAccess::Permit(EnginePermit::RecoveryProbe));
        assert!(matches!(
            health.acquire_at("dectalk", now + Duration::from_secs(5)),
            EngineAccess::Denied { .. }
        ));
        assert!(matches!(
            probing.engines[0].health,
            EngineHealth::Failed { .. }
        ));
    }

    #[test]
    fn successful_probe_closes_the_circuit() {
        let health = RuntimeEngineHealth::new();
        let now = Instant::now();
        health.record_failure_at("dectalk", "helper exited".to_owned(), now);
        let permit = health.acquire_at("dectalk", now + Duration::from_secs(5));

        health.record_success("dectalk", EnginePermit::RecoveryProbe);
        let snapshot =
            health.snapshot_at(3, vec![descriptor("dectalk")], now + Duration::from_secs(5));

        assert_eq!(permit, EngineAccess::Permit(EnginePermit::RecoveryProbe));
        assert!(matches!(snapshot.engines[0].health, EngineHealth::Healthy));
        assert_eq!(
            health.acquire("dectalk"),
            EngineAccess::Permit(EnginePermit::Normal)
        );
    }

    #[test]
    fn failed_probes_follow_the_bounded_backoff_sequence() {
        let health = RuntimeEngineHealth::new();
        let start = Instant::now();
        let mut now = start;
        let expected = [5, 15, 30, 60, 60];

        for seconds in expected {
            let cooldown = health.record_failure_at("dectalk", "still down".to_owned(), now);
            assert_eq!(cooldown, Duration::from_secs(seconds));
            now += cooldown;
            assert_eq!(
                health.acquire_at("dectalk", now),
                EngineAccess::Permit(EnginePermit::RecoveryProbe)
            );
        }
    }

    #[test]
    fn cancelled_probe_returns_to_ready_without_counting_a_failure() {
        let health = RuntimeEngineHealth::new();
        let now = Instant::now();
        health.record_failure_at("dectalk", "helper exited".to_owned(), now);
        assert_eq!(
            health.acquire_at("dectalk", now + Duration::from_secs(5)),
            EngineAccess::Permit(EnginePermit::RecoveryProbe)
        );

        health.release_probe("dectalk");

        assert_eq!(
            health.acquire_at("dectalk", now + Duration::from_secs(5)),
            EngineAccess::Permit(EnginePermit::RecoveryProbe)
        );
    }
}
