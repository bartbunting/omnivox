use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::OnceLock;

const EXTERNAL_INITIALIZATIONS: usize = 4;
pub const EXTERNAL_STARTUP_BUDGET: Duration = Duration::from_secs(120);

/// Includes attempts whose callers have timed out while launch, I/O or cleanup
/// is still running. Explicit recovery shares the startup limit.
#[derive(Default)]
pub(super) struct InitializationSlots {
    active: AtomicUsize,
}

impl InitializationSlots {
    fn acquire(
        self: &Arc<Self>,
        deadline: Instant,
    ) -> Result<InitializationSlot, HelperEngineError> {
        loop {
            check_budget(deadline)?;
            if self
                .active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                    (active < EXTERNAL_INITIALIZATIONS).then_some(active + 1)
                })
                .is_ok()
            {
                return Ok(InitializationSlot(Arc::clone(self)));
            }
            thread::sleep(
                HELPER_CLEANUP_POLL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

struct InitializationSlot(Arc<InitializationSlots>);

impl Drop for InitializationSlot {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

fn check_budget(deadline: Instant) -> Result<(), HelperEngineError> {
    if Instant::now() >= deadline {
        Err(HelperEngineError::Timeout("external startup budget"))
    } else {
        Ok(())
    }
}

pub(super) fn publication_lock<T>(
    lock: &RwLock<T>,
    deadline: Option<Instant>,
) -> Result<std::sync::RwLockWriteGuard<'_, T>, HelperEngineError> {
    let Some(deadline) = deadline else {
        return lock
            .write()
            .map_err(|_| HelperEngineError::Transport("helper publication lock poisoned".into()));
    };
    loop {
        check_budget(deadline)?;
        match lock.try_write() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err(HelperEngineError::Transport(
                    "helper publication lock poisoned".into(),
                ));
            }
            Err(TryLockError::WouldBlock) => thread::sleep(
                HELPER_CLEANUP_POLL.min(deadline.saturating_duration_since(Instant::now())),
            ),
        }
    }
}

struct PreparedInitialization {
    connection: Arc<dyn HelperConnection>,
    descriptor: EngineDescriptor,
    protocol_version: u16,
    accepted: mpsc::SyncSender<()>,
}

// Field order keeps the slot occupied through the engine's final Drop, if the
// host has already gone away. The worker owns no join handle outside the engine.
struct InitializationTask {
    engine: Arc<HelperTtsEngine>,
    _slot: InitializationSlot,
}

impl InitializationTask {
    fn run(
        self,
        deadline: Instant,
        results: mpsc::SyncSender<Result<Option<PreparedInitialization>, HelperEngineError>>,
    ) {
        let mut results = Some(results);
        let mut initialize = || {
            let _lifecycle = cleanup_lock(&self.engine.lifecycle, deadline)?;
            check_budget(deadline)?;
            if self.engine.current_connection().is_ok() {
                let _ = results.take().unwrap().send(Ok(None));
                return Ok(());
            }
            self.engine.install_fresh_connection_with(
                Some(deadline),
                |connection, descriptor, protocol_version| {
                    let (accepted, decision) = mpsc::sync_channel(1);
                    results
                        .take()
                        .unwrap()
                        .send(Ok(Some(PreparedInitialization {
                            connection,
                            descriptor,
                            protocol_version,
                            accepted,
                        })))
                        .map_err(|_| HelperEngineError::Timeout("external startup budget"))?;
                    // The result sender is gone before waiting for acceptance.
                    // Dropping the receiver therefore drops even an unread
                    // candidate and wakes this wait. Lifecycle and retirement
                    // remain held until acceptance or cleanup.
                    decision
                        .recv()
                        .map_err(|_| HelperEngineError::Timeout("external startup budget"))
                },
            )
        };
        if let Err(error) = initialize() {
            if let Some(results) = results {
                let _ = results.send(Err(error));
            }
        }
    }
}

impl HelperTtsEngine {
    /// Wait for initialization within an admission budget. Unfinished launch,
    /// protocol I/O and cleanup retain this engine and a process-wide slot after
    /// timeout. A later call joins the completed attempt before retrying.
    pub fn initialize_before(
        self: &Arc<Self>,
        deadline: Instant,
    ) -> Result<bool, HelperEngineError> {
        static SLOTS: OnceLock<Arc<InitializationSlots>> = OnceLock::new();
        self.initialize_with_slots(deadline, SLOTS.get_or_init(Default::default))
    }

    pub(super) fn initialize_with_slots(
        self: &Arc<Self>,
        deadline: Instant,
        slots: &Arc<InitializationSlots>,
    ) -> Result<bool, HelperEngineError> {
        check_budget(deadline)?;
        let mut pending = cleanup_lock(&self.initialization, deadline)?;
        Self::join_initialization_before(&mut pending, deadline)?;
        check_budget(deadline)?;
        if self.current_connection().is_ok() {
            return Ok(false);
        }
        let task = InitializationTask {
            engine: Arc::clone(self),
            _slot: slots.acquire(deadline)?,
        };
        let (results, waiting) = mpsc::sync_channel(1);
        *pending = Some(
            thread::Builder::new()
                .name("omnivox-helper-initialization".into())
                .spawn(move || task.run(deadline, results))
                .map_err(|error| {
                    HelperEngineError::Transport(format!("could not start initialization: {error}"))
                })?,
        );
        match waiting.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(prepared)) => {
                // A queued response can be received after the deadline. Dropping
                // it tells the owner to retire, without any late publication.
                check_budget(deadline)?;
                if let Some(prepared) = prepared {
                    self.publish_connection(
                        prepared.connection,
                        prepared.descriptor,
                        prepared.protocol_version,
                        Some(deadline),
                    )?;
                    let _ = prepared.accepted.send(());
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Ok(Err(error)) => Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err(HelperEngineError::Timeout("external startup budget"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(HelperEngineError::Transport(
                "helper initialization worker failed".into(),
            )),
        }
    }

    pub(super) fn join_initialization_before(
        pending: &mut Option<JoinHandle<()>>,
        deadline: Instant,
    ) -> Result<(), HelperEngineError> {
        if let Some(worker) = pending.as_ref() {
            while !worker.is_finished() {
                check_budget(deadline)?;
                thread::sleep(
                    HELPER_CLEANUP_POLL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
        if let Some(worker) = pending.take() {
            let _ = worker.join();
        }
        Ok(())
    }
}

/// The prepared engine is retained on both success and failure. An explicit
/// rescan must reuse it; cleanup ownership cannot be replaced with a new object.
pub struct ExternalHelperInitialization {
    pub engine_id: String,
    pub engine: Option<Arc<HelperTtsEngine>>,
    pub result: Result<Duration, String>,
}

pub fn initialize_external_helpers(
    mut configs: Vec<HelperEngineConfig>,
    requested: &str,
) -> Vec<ExternalHelperInitialization> {
    configs.sort_by(|a, b| {
        (a.engine_id != requested, &a.engine_id).cmp(&(b.engine_id != requested, &b.engine_id))
    });
    let prepared: Vec<_> = configs
        .into_iter()
        .map(|config| {
            let id = config.engine_id.clone();
            let engine = HelperTtsEngine::prepare(config)
                .map(Arc::new)
                .map_err(|e| e.to_string());
            (id, engine)
        })
        .collect();
    let results = run_initializations(
        &prepared,
        EXTERNAL_STARTUP_BUDGET,
        |(_, engine), deadline| match engine {
            Ok(engine) => engine
                .initialize_before(deadline)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(error) => Err(error.clone()),
        },
    );
    prepared
        .into_iter()
        .zip(results)
        .map(
            |((engine_id, engine), result)| ExternalHelperInitialization {
                engine_id,
                engine: engine.ok(),
                result,
            },
        )
        .collect()
}

/// Dedicated external slots keep the queue independent of shipped startup.
/// Scoped admission waiters are joined, including panic paths. Their owned I/O
/// attempts can outlive a timeout, retaining slots without publishing late.
pub(super) fn run_initializations<T: Sync>(
    jobs: &[T],
    budget: Duration,
    initialize: impl Fn(&T, Instant) -> Result<(), String> + Sync,
) -> Vec<Result<Duration, String>> {
    let deadline = Instant::now() + budget;
    let pending = Mutex::new((0..jobs.len()).collect::<VecDeque<_>>());
    let results = Mutex::new((0..jobs.len()).map(|_| None).collect::<Vec<_>>());
    let mut worker_failed = false;
    thread::scope(|scope| {
        let mut workers = Vec::new();
        for slot in 0..jobs.len().min(EXTERNAL_INITIALIZATIONS) {
            let pending = &pending;
            let results = &results;
            let initialize = &initialize;
            match thread::Builder::new()
                .name(format!("omnivox-external-init-{slot}"))
                .spawn_scoped(scope, move || loop {
                    let next = {
                        let mut pending = pending.lock().unwrap();
                        if Instant::now() >= deadline {
                            break;
                        }
                        pending.pop_front()
                    };
                    let Some(index) = next else {
                        break;
                    };
                    let started = Instant::now();
                    let result = initialize(&jobs[index], deadline).map(|()| started.elapsed());
                    results.lock().unwrap()[index] = Some(result);
                }) {
                Ok(worker) => workers.push(worker),
                Err(_) => worker_failed = true,
            }
        }
        for worker in workers {
            if worker.join().is_err() {
                worker_failed = true;
            }
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|result| {
            result.unwrap_or_else(|| {
                Err(if worker_failed {
                    "external initialization worker failed"
                } else {
                    "external startup budget exhausted before admission"
                }
                .into())
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrency_is_bounded_and_every_admitted_job_runs_once() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let seen = Mutex::new(Vec::new());
        let jobs = (0..32).collect::<Vec<_>>();
        let results = run_initializations(&jobs, Duration::from_secs(10), |id, _| {
            let count = active.fetch_add(1, Ordering::AcqRel) + 1;
            peak.fetch_max(count, Ordering::AcqRel);
            seen.lock().unwrap().push(*id);
            thread::sleep(Duration::from_millis(5));
            active.fetch_sub(1, Ordering::AcqRel);
            if id % 2 == 0 {
                Ok(())
            } else {
                Err("optional runtime unavailable".into())
            }
        });
        assert!(peak.load(Ordering::Acquire) <= EXTERNAL_INITIALIZATIONS);
        assert!(peak.load(Ordering::Acquire) > 1);
        assert_eq!(active.load(Ordering::Acquire), 0);
        let mut seen = seen.into_inner().unwrap();
        seen.sort_unstable();
        assert_eq!(seen, jobs);
        for (id, result) in results.into_iter().enumerate() {
            assert_eq!(result.is_ok(), id % 2 == 0);
        }
    }

    #[test]
    fn one_batch_deadline_retires_active_work_and_does_not_admit_the_rest() {
        let admitted = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let deadline_seen = Mutex::new(Vec::new());
        let started = Instant::now();
        let results = run_initializations(&[0; 32], Duration::from_millis(100), |_, deadline| {
            admitted.fetch_add(1, Ordering::AcqRel);
            deadline_seen.lock().unwrap().push(deadline);
            thread::sleep(deadline.saturating_duration_since(Instant::now()));
            finished.fetch_add(1, Ordering::AcqRel);
            Err("retired at admission deadline".into())
        });
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(results.iter().all(Result::is_err));
        assert!(admitted.load(Ordering::Acquire) <= EXTERNAL_INITIALIZATIONS);
        assert_eq!(
            finished.load(Ordering::Acquire),
            admitted.load(Ordering::Acquire)
        );
        assert!(results
            .iter()
            .any(|result| result.as_ref().unwrap_err().contains("before admission")));
        let deadlines = deadline_seen.into_inner().unwrap();
        assert!(deadlines.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn expired_budget_never_invokes_a_job_and_panics_do_not_escape_the_batch() {
        let expired = run_initializations(&[0; 2], Duration::ZERO, |_, _| panic!("must not start"));
        assert!(expired.iter().all(Result::is_err));
        let panicked = run_initializations(&[0; 2], Duration::from_secs(1), |_, _| {
            panic!("failed worker")
        });
        assert!(panicked
            .iter()
            .all(|result| result.as_ref().unwrap_err().contains("worker failed")));
    }
}
