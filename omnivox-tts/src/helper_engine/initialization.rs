use super::*;

const EXTERNAL_INITIALIZATIONS: usize = 4;
pub const EXTERNAL_STARTUP_BUDGET: Duration = Duration::from_secs(120);

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
/// Scoped workers are all joined, including panic paths; none can publish late.
fn run_initializations<T: Sync>(
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
    use std::sync::atomic::AtomicUsize;

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
