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
    for _ in 0..2 {
        assert!(matches!(
            engine.initialize_before(Instant::now() + Duration::from_secs(2)),
            Err(HelperEngineError::Timeout("mock cleanup"))
        ));
        assert!(engine.current_connection().is_err());
        assert!(recovered.sent.lock().unwrap().is_empty());
    }
    failed.cleanup_fails.store(false, Ordering::Release);
    assert!(engine
        .initialize_before(Instant::now() + Duration::from_secs(2))
        .unwrap());
    assert_eq!(engine.descriptor().version.as_deref(), Some("2"));
    assert!(!engine
        .initialize_before(Instant::now() + Duration::from_secs(2))
        .unwrap());
}

#[test]
fn expired_initialization_budget_does_not_spawn_a_process() {
    let engine = HelperTtsEngine::prepare(HelperEngineConfig::new(
        "org.example",
        "/must-not-start-a-helper",
    ))
    .unwrap();
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

struct InitializationConnector(Arc<StalledInitialization>);
impl HelperConnector for InitializationConnector {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
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
                Arc::new(InitializationConnector(peer.clone())),
                None,
            )
            .unwrap();
            let started = Instant::now();
            assert!(matches!(
                engine.initialize_before(started + Duration::from_millis(100)),
                Err(HelperEngineError::Timeout("external startup budget"))
            ));
            assert!(started.elapsed() < Duration::from_secs(2));
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
    let engine = HelperTtsEngine::prepare(config).unwrap();
    let started = Instant::now();
    assert!(matches!(
        engine.initialize_before(started + Duration::from_millis(100)),
        Err(HelperEngineError::Timeout("external startup budget"))
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(engine.current_connection().is_err());
    assert!(engine.retiring_connection.lock().unwrap().is_none());
}
