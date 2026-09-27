// Isolate fault-injection budgets from concurrently running tests. Production
// initialize_before uses the same implementation with process-wide slots.
fn initialize_for_test(
    engine: &Arc<HelperTtsEngine>,
    deadline: Instant,
) -> Result<bool, HelperEngineError> {
    engine.initialize_with_slots(
        deadline,
        &Arc::new(initialization::InitializationSlots::default()),
    )
}

fn join_initialization_for_test(engine: &HelperTtsEngine) {
    HelperTtsEngine::join_initialization_before(
        &mut engine.initialization.lock().unwrap(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
}

#[test]
fn first_initialization_failure_retains_ownership_until_cleanup_is_confirmed() {
    let failed = Arc::new(MockConnection::new(
        helper_descriptor("wrong", "1"),
        MockSynthesisMode::Complete,
    ));
    failed.cleanup_fails.store(true, Ordering::Release);
    let recovered = Arc::new(MockConnection::new(
        helper_descriptor("eloquence", "2"),
        MockSynthesisMode::Complete,
    ));
    let engine = HelperTtsEngine::without_connection(
        mock_config("eloquence"),
        Arc::new(MockConnector::new(vec![failed.clone(), recovered.clone()])),
        None,
    )
    .unwrap();
    let engine = Arc::new(engine);
    for _ in 0..2 {
        assert!(matches!(
            initialize_for_test(&engine, Instant::now() + Duration::from_secs(2)),
            Err(HelperEngineError::Timeout("mock cleanup"))
        ));
        assert!(engine.current_connection().is_err());
        assert!(recovered.sent.lock().unwrap().is_empty());
    }
    failed.cleanup_fails.store(false, Ordering::Release);
    assert!(initialize_for_test(&engine, Instant::now() + Duration::from_secs(2)).unwrap());
    assert_eq!(engine.descriptor().version.as_deref(), Some("2"));
    assert!(!initialize_for_test(&engine, Instant::now() + Duration::from_secs(2)).unwrap());
}

#[test]
fn expired_initialization_budget_does_not_spawn_a_process() {
    let engine = Arc::new(
        HelperTtsEngine::prepare(HelperEngineConfig::new(
            "org.example",
            "/must-not-start-a-helper",
        ))
        .unwrap(),
    );
    assert!(matches!(
        engine.initialize_before(Instant::now()),
        Err(HelperEngineError::Timeout("external startup budget"))
    ));
    assert!(engine.current_connection().is_err());
}

struct StalledInitialization {
    inner: MockConnection,
    describe: bool,
    block_write: bool,
}

impl HelperConnection for StalledInitialization {
    fn send(&self, request: &HelperRequest) -> Result<(), HelperEngineError> {
        let targeted = if self.describe {
            matches!(request.body, HelperRequestBody::Describe)
        } else {
            matches!(request.body, HelperRequestBody::Hello { .. })
        };
        if targeted {
            if self.block_write {
                // Model a writer which only process termination can unblock.
                let mut responses = self.inner.responses.lock().unwrap();
                while !self.inner.terminated.load(Ordering::Acquire) {
                    responses = self
                        .inner
                        .response_ready
                        .wait_timeout(responses, Duration::from_millis(50))
                        .unwrap()
                        .0;
                }
                return Err(HelperEngineError::Exited);
            }
            return Ok(()); // No response: the read must obey the same deadline.
        }
        self.inner.send(request)
    }
    fn receive(&self, timeout: Duration) -> Result<HelperResponse, HelperEngineError> {
        self.inner.receive(timeout)
    }
    fn terminate(&self) -> Result<(), HelperEngineError> {
        self.inner.terminate()
    }
}

struct InitializationConnector(Arc<StalledInitialization>, AtomicUsize);
impl HelperConnector for InitializationConnector {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        self.1.fetch_add(1, Ordering::AcqRel);
        Ok(self.0.clone())
    }
}

#[test]
fn initialization_watchdog_bounds_hello_and_descriptor_writes_and_reads() {
    for describe in [false, true] {
        for block_write in [false, true] {
            let peer = Arc::new(StalledInitialization {
                inner: MockConnection::new(
                    helper_descriptor("eloquence", "1"),
                    MockSynthesisMode::Complete,
                ),
                describe,
                block_write,
            });
            let engine = HelperTtsEngine::without_connection(
                mock_config("eloquence"),
                Arc::new(InitializationConnector(peer.clone(), AtomicUsize::new(0))),
                None,
            )
            .unwrap();
            let engine = Arc::new(engine);
            let started = Instant::now();
            assert!(matches!(
                initialize_for_test(&engine, started + Duration::from_millis(100)),
                Err(HelperEngineError::Timeout("external startup budget"))
            ));
            assert!(started.elapsed() < Duration::from_secs(2));
            join_initialization_for_test(&engine);
            assert!(peer.inner.terminated.load(Ordering::Acquire));
            assert!(engine.current_connection().is_err());
            assert!(engine.retiring_connection.lock().unwrap().is_none());
        }
    }
}

#[cfg(unix)]
#[test]
fn stalled_real_helper_is_reaped_at_the_batch_deadline() {
    let mut config = HelperEngineConfig::new("org.example", "/bin/sh");
    config.arguments = vec!["-c".into(), "exec sleep 10".into()];
    let engine = Arc::new(HelperTtsEngine::prepare(config).unwrap());
    let started = Instant::now();
    assert!(matches!(
        initialize_for_test(&engine, started + Duration::from_millis(100)),
        Err(HelperEngineError::Timeout("external startup budget"))
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    join_initialization_for_test(&engine);
    assert!(engine.current_connection().is_err());
    assert!(engine.retiring_connection.lock().unwrap().is_none());
}

// These regressions release their blocked operations after 500ms and join the
// attempts before assertions, including when the timing expectation fails.
struct BlockedStartupConnector {
    release: Arc<AtomicBool>,
    peer: Arc<MockConnection>,
    attempts: AtomicUsize,
}

impl HelperConnector for BlockedStartupConnector {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        self.attempts.fetch_add(1, Ordering::AcqRel);
        while !self.release.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(self.peer.clone())
    }
}

#[test]
fn startup_deadline_does_not_wait_for_a_blocked_connector() {
    let release = Arc::new(AtomicBool::new(false));
    let peer = Arc::new(MockConnection::new(
        helper_descriptor("eloquence", "1"),
        MockSynthesisMode::Complete,
    ));
    let engine = HelperTtsEngine::without_connection(
        mock_config("eloquence"),
        Arc::new(BlockedStartupConnector {
            release: release.clone(),
            peer: peer.clone(),
            attempts: AtomicUsize::new(0),
        }),
        None,
    )
    .unwrap();
    let engine = Arc::new(engine);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        release.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let result = initialize_for_test(&engine, started + Duration::from_millis(100));
    let elapsed = started.elapsed();
    releaser.join().unwrap();
    join_initialization_for_test(&engine);
    assert!(result.is_err());
    assert!(engine.current_connection().is_err());
    assert!(engine.descriptor.read().unwrap().is_none());
    assert!(peer.terminated.load(Ordering::Acquire));
    assert!(peer.sent.lock().unwrap().is_empty());
    eprintln!(
        "blocked connector: budget_ms=100 return_ms={}",
        elapsed.as_millis()
    );
    assert!(
        elapsed < Duration::from_millis(300),
        "startup waited for blocked connector: {elapsed:?}"
    );
}

// Release even when an assertion unwinds, so no test leaves a blocked attempt.
struct ReleaseStartup(Arc<AtomicBool>);
impl Drop for ReleaseStartup {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn unfinished_initializations_keep_the_batch_limit_and_forbid_replacement() {
    let release = Arc::new(AtomicBool::new(false));
    let _release_on_drop = ReleaseStartup(release.clone());
    let slots = Arc::new(initialization::InitializationSlots::default());
    let mut engines = Vec::new();
    let mut connectors = Vec::new();
    for _ in 0..5 {
        let connector = Arc::new(BlockedStartupConnector {
            release: release.clone(),
            peer: Arc::new(MockConnection::new(
                helper_descriptor("eloquence", "1"),
                MockSynthesisMode::Complete,
            )),
            attempts: AtomicUsize::new(0),
        });
        engines.push(Arc::new(
            HelperTtsEngine::without_connection(mock_config("eloquence"), connector.clone(), None)
                .unwrap(),
        ));
        connectors.push(connector);
    }
    let started = Instant::now();
    let results = initialization::run_initializations(
        &engines,
        Duration::from_millis(200),
        |engine, deadline| {
            engine
                .initialize_with_slots(deadline, &slots)
                .map(|_| ())
                .map_err(|error| error.to_string())
        },
    );
    let elapsed = started.elapsed();
    let admitted = connectors
        .iter()
        .map(|connector| connector.attempts.load(Ordering::Acquire))
        .collect::<Vec<_>>();
    // Same owner cannot replace its unfinished attempt. A different owner cannot
    // bypass the process-wide slots after the batch has already returned.
    let retry =
        engines[0].initialize_with_slots(Instant::now() + Duration::from_millis(50), &slots);
    let extra =
        engines[4].initialize_with_slots(Instant::now() + Duration::from_millis(50), &slots);
    let attempts_after_retries = connectors
        .iter()
        .map(|connector| connector.attempts.load(Ordering::Acquire))
        .collect::<Vec<_>>();
    release.store(true, Ordering::Release);
    for engine in &engines {
        join_initialization_for_test(engine);
    }
    assert!(elapsed < Duration::from_millis(500));
    assert!(results.iter().all(Result::is_err));
    assert_eq!(admitted, [1, 1, 1, 1, 0]);
    assert!(retry.is_err() && extra.is_err());
    assert_eq!(attempts_after_retries, admitted);
    for engine in &engines {
        assert!(engine.current_connection().is_err());
        assert!(engine.descriptor.read().unwrap().is_none());
    }
    // Finishing the old operations returns their slots, so an explicit later
    // admission can initialize the helper that never started in this batch.
    assert!(engines[4]
        .initialize_with_slots(Instant::now() + Duration::from_secs(2), &slots)
        .unwrap());
    join_initialization_for_test(&engines[4]);
}

#[test]
fn dropping_the_host_retains_a_blocked_initialization_and_cleans_its_late_child() {
    let release = Arc::new(AtomicBool::new(false));
    let _release_on_drop = ReleaseStartup(release.clone());
    let peer = Arc::new(MockConnection::new(
        helper_descriptor("eloquence", "1"),
        MockSynthesisMode::Complete,
    ));
    let connector = Arc::new(BlockedStartupConnector {
        release: release.clone(),
        peer: peer.clone(),
        attempts: AtomicUsize::new(0),
    });
    let engine = Arc::new(
        HelperTtsEngine::without_connection(mock_config("eloquence"), connector.clone(), None)
            .unwrap(),
    );
    assert!(initialize_for_test(&engine, Instant::now() + Duration::from_millis(100)).is_err());
    let owner = Arc::downgrade(&engine);
    let started = Instant::now();
    drop(engine);
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(owner.upgrade().is_some());
    release.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Arc::strong_count(&connector) != 1 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        owner.strong_count(),
        0,
        "worker must not retain itself after completion"
    );
    assert_eq!(
        Arc::strong_count(&connector),
        1,
        "final Drop must finish without joining itself"
    );
    assert!(peer.terminated.load(Ordering::Acquire));
    assert!(peer.sent.lock().unwrap().is_empty());
}

#[test]
fn a_ready_connection_cannot_publish_after_admission_expires() {
    let peer = Arc::new(MockConnection::new(
        helper_descriptor("eloquence", "1"),
        MockSynthesisMode::Complete,
    ));
    let engine = Arc::new(
        HelperTtsEngine::without_connection(
            mock_config("eloquence"),
            Arc::new(MockConnector::new(vec![peer.clone()])),
            None,
        )
        .unwrap(),
    );
    // Let negotiation complete, but prevent the caller from publishing its
    // descriptor until after it has timed out and declined the candidate.
    let reading_descriptor = engine.descriptor.read().unwrap();
    let started = Instant::now();
    let result = initialize_for_test(&engine, started + Duration::from_millis(100));
    let elapsed = started.elapsed();
    drop(reading_descriptor);
    join_initialization_for_test(&engine);
    assert!(matches!(
        result,
        Err(HelperEngineError::Timeout("external startup budget"))
    ));
    assert!(elapsed < Duration::from_millis(300));
    assert!(peer
        .sent
        .lock()
        .unwrap()
        .iter()
        .any(|request| matches!(request.body, HelperRequestBody::Describe)));
    assert!(peer.terminated.load(Ordering::Acquire));
    assert!(engine.current_connection().is_err());
    assert!(engine.descriptor.read().unwrap().is_none());
}

#[test]
fn startup_deadline_does_not_wait_for_unconfirmed_writer_cleanup() {
    let peer = Arc::new(StalledInitialization {
        inner: MockConnection::new(
            helper_descriptor("eloquence", "1"),
            MockSynthesisMode::Complete,
        ),
        describe: false,
        block_write: true,
    });
    peer.inner.cleanup_fails.store(true, Ordering::Release);
    let connector = Arc::new(InitializationConnector(peer.clone(), AtomicUsize::new(0)));
    let engine =
        HelperTtsEngine::without_connection(mock_config("eloquence"), connector.clone(), None)
            .unwrap();
    let engine = Arc::new(engine);
    let released = peer.clone();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        released.inner.cleanup_fails.store(false, Ordering::Release);
        released.inner.terminate().unwrap();
    });
    let started = Instant::now();
    let result = initialize_for_test(&engine, started + Duration::from_millis(100));
    let elapsed = started.elapsed();
    let retry = initialize_for_test(&engine, Instant::now() + Duration::from_millis(50));
    releaser.join().unwrap();
    join_initialization_for_test(&engine);
    assert!(result.is_err());
    assert!(retry.is_err());
    assert_eq!(connector.1.load(Ordering::Acquire), 1);
    assert!(engine.current_connection().is_err());
    assert!(engine.descriptor.read().unwrap().is_none());
    eprintln!(
        "unconfirmed writer cleanup: budget_ms=100 return_ms={}",
        elapsed.as_millis()
    );
    assert!(
        elapsed < Duration::from_millis(300),
        "startup waited for unconfirmed cleanup: {elapsed:?}"
    );
}

struct PanickingStartupConnector;
impl HelperConnector for PanickingStartupConnector {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        panic!("controlled initialization panic");
    }
}

#[test]
fn panicked_initializations_report_failure_and_release_their_slots() {
    let slots = Arc::new(initialization::InitializationSlots::default());
    for _ in 0..4 {
        let engine = Arc::new(
            HelperTtsEngine::without_connection(
                mock_config("eloquence"),
                Arc::new(PanickingStartupConnector),
                None,
            )
            .unwrap(),
        );
        let error = engine
            .initialize_with_slots(Instant::now() + Duration::from_secs(2), &slots)
            .unwrap_err();
        join_initialization_for_test(&engine);
        assert!(error.to_string().contains("initialization worker failed"));
        assert!(engine.current_connection().is_err());
    }
    let peer = Arc::new(MockConnection::new(
        helper_descriptor("eloquence", "1"),
        MockSynthesisMode::Complete,
    ));
    let healthy = Arc::new(
        HelperTtsEngine::without_connection(
            mock_config("eloquence"),
            Arc::new(MockConnector::new(vec![peer])),
            None,
        )
        .unwrap(),
    );
    assert!(healthy
        .initialize_with_slots(Instant::now() + Duration::from_secs(2), &slots)
        .unwrap());
    join_initialization_for_test(&healthy);
}
