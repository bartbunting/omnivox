// Reuse the protocol peer for recovery, with real pipes for blocked submission.
struct CancellationConnector(Mutex<VecDeque<Arc<dyn HelperConnection>>>);

impl HelperConnector for CancellationConnector {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(HelperEngineError::Exited)
    }
}

fn cancellation_engine(
    connection: Arc<dyn HelperConnection>,
    streaming: bool,
    idle_timeout: Duration,
) -> (Arc<HelperTtsEngine>, Arc<MockConnection>) {
    let mut descriptor = helper_descriptor("eloquence", "recovered");
    if streaming {
        descriptor.capabilities.audio_output = AudioOutputMode::StreamingPcm;
    }
    let recovered = Arc::new(MockConnection::new(
        descriptor,
        if streaming {
            MockSynthesisMode::StreamComplete
        } else {
            MockSynthesisMode::Complete
        },
    ));
    let mut config = mock_config("eloquence");
    config.synthesis_idle_timeout = idle_timeout;
    let peers: VecDeque<Arc<dyn HelperConnection>> =
        VecDeque::from([connection, recovered.clone()]);
    let engine =
        HelperTtsEngine::with_connector(config, Arc::new(CancellationConnector(Mutex::new(peers))))
            .unwrap();
    (Arc::new(engine), recovered)
}

fn cancellation_synthesize(
    engine: &HelperTtsEngine,
    streaming: bool,
    text: &str,
) -> Result<(), TtsError> {
    if streaming {
        engine
            .synthesize_stream(
                &synthesis_request(text),
                &mut RecordingStreamSink::default(),
            )
            .map(|_| ())
    } else {
        engine.synthesize(&synthesis_request(text)).map(|_| ())
    }
}

#[cfg(unix)]
fn blocked_synthesis_pipe(streaming: bool, protocol_version: u16, stop: bool) {
    let mut descriptor = helper_descriptor("eloquence", "blocked");
    if streaming {
        descriptor.capabilities.audio_output = AudioOutputMode::StreamingPcm;
    }
    let hello = HelperResponse::for_request_version(
        protocol_version,
        1,
        HelperResponseBody::Hello {
            selected_protocol_version: protocol_version,
            helper_name: "blocked pipe fixture".into(),
            helper_version: "1".into(),
        },
    );
    let inventory = HelperResponse::for_request_version(
        protocol_version,
        2,
        HelperResponseBody::Descriptor { descriptor },
    );
    // No native engine or audio device. Exec leaves one child to reap, and its
    // finite sleep bounds the fixture even if the implementation regresses.
    let script = "IFS= read -r hello; printf '%s\\n' \"$1\"; \
                  IFS= read -r describe; printf '%s\\n' \"$2\"; exec sleep 10";
    let connection = Arc::new(
        ProcessHelperConnection::spawn(
            "eloquence",
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                script.into(),
                "blocked-pipe-fixture".into(),
                serde_json::to_string(&hello).unwrap().into(),
                serde_json::to_string(&inventory).unwrap().into(),
            ],
            &LaunchEnvironment::capture(),
        )
        .unwrap(),
    );
    let idle_timeout = if stop {
        Duration::from_secs(5)
    } else {
        Duration::from_millis(100)
    };
    let (engine, recovered) = cancellation_engine(connection.clone(), streaming, idle_timeout);
    let (finished, completion) = mpsc::channel();
    let worker = engine.clone();
    let synthesis = thread::spawn(move || {
        let result = cancellation_synthesize(&worker, streaming, &"x".repeat(128 * 1024));
        finished.send(result).unwrap();
    });

    // Observe entry into the pipe write, independently of active-request
    // publication: the original bug published that identity only after writing.
    let wait_until = Instant::now() + Duration::from_secs(2);
    let entered_write = loop {
        if matches!(connection.writer.try_lock(), Err(TryLockError::WouldBlock)) {
            break true;
        }
        if Instant::now() >= wait_until || synthesis.is_finished() {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };
    let (stopped, stop_completion) = mpsc::channel();
    let stopper = stop.then(|| {
        let owner = engine.clone();
        thread::spawn(move || {
            owner.stop();
            stopped.send(()).unwrap();
        })
    });
    let stop_returned = !stop
        || stop_completion
            .recv_timeout(Duration::from_millis(750))
            .is_ok();
    let result = completion.recv_timeout(Duration::from_millis(750));
    // Always release and join before asserting, including on the old code.
    connection.terminate().unwrap();
    synthesis.join().unwrap();
    if let Some(stopper) = stopper {
        stopper.join().unwrap();
    }
    assert!(entered_write, "fixture never entered the pipe write");
    assert!(
        stop_returned,
        "Stop remained behind the blocked synthesis write"
    );
    assert!(
        matches!(result, Ok(Err(_))),
        "blocked synthesis did not retire: {result:?}"
    );
    assert!(!engine.is_speaking());
    assert!(connection
        .child
        .lock()
        .unwrap()
        .try_wait()
        .unwrap()
        .is_some());
    assert!(connection.reader_handle.lock().unwrap().is_none());
    assert!(cancellation_synthesize(&engine, streaming, "replacement speech").is_ok());
    assert!(!recovered.terminated.load(Ordering::Acquire));
}

#[cfg(unix)]
#[test]
fn stop_bounds_a_blocked_buffered_synthesis_pipe() {
    for version in [5, 6] {
        blocked_synthesis_pipe(false, version, true);
    }
}

#[cfg(unix)]
#[test]
fn stop_bounds_a_blocked_progressive_synthesis_pipe() {
    for version in [5, 6] {
        blocked_synthesis_pipe(true, version, true);
    }
}

#[cfg(unix)]
#[test]
fn synthesis_idle_deadline_includes_blocked_pipe_writes() {
    for streaming in [false, true] {
        for version in [5, 6] {
            blocked_synthesis_pipe(streaming, version, false);
        }
    }
}

struct BlockedCancelWrite {
    inner: Arc<MockConnection>,
    complete_target: bool,
}

impl HelperConnection for BlockedCancelWrite {
    fn send(&self, request: &HelperRequest) -> Result<(), HelperEngineError> {
        if matches!(request.body, HelperRequestBody::Cancel { .. }) {
            if self.complete_target {
                // The target reply can arrive while the cancel write is still
                // blocked. Target completion alone must not disarm that deadline.
                self.inner.send(request)?;
            }
            let mut responses = self.inner.responses.lock().unwrap();
            while !self.inner.terminated.load(Ordering::Acquire) {
                responses = self
                    .inner
                    .response_ready
                    .wait_timeout(responses, Duration::from_millis(10))
                    .unwrap()
                    .0;
            }
            return Err(HelperEngineError::Exited);
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

#[test]
fn stop_bounds_a_blocked_cancel_write_even_after_target_completion() {
    for streaming in [false, true] {
        for complete_target in [false, true] {
            let mut descriptor = helper_descriptor("eloquence", "blocked");
            if streaming {
                descriptor.capabilities.audio_output = AudioOutputMode::StreamingPcm;
            }
            let peer = Arc::new(MockConnection::new(
                descriptor,
                MockSynthesisMode::WaitForCancel,
            ));
            let connection = Arc::new(BlockedCancelWrite {
                inner: peer.clone(),
                complete_target,
            });
            let (engine, recovered) =
                cancellation_engine(connection, streaming, Duration::from_secs(5));
            let worker = engine.clone();
            let synthesis =
                thread::spawn(move || cancellation_synthesize(&worker, streaming, "old speech"));
            let wait_until = Instant::now() + Duration::from_secs(2);
            while !engine.is_speaking() && Instant::now() < wait_until {
                thread::sleep(Duration::from_millis(1));
            }
            let (finished, completion) = mpsc::channel();
            let owner = engine.clone();
            let stopper = thread::spawn(move || {
                owner.stop();
                finished.send(()).unwrap();
            });
            let returned = completion.recv_timeout(Duration::from_millis(750)).is_ok();
            peer.terminate().unwrap();
            stopper.join().unwrap();
            let result = synthesis.join().unwrap();
            assert!(
                returned,
                "Stop waited for cancel write; terminal={complete_target}"
            );
            assert!(result.is_err());
            assert!(!engine.is_speaking());
            assert!(cancellation_synthesize(&engine, streaming, "replacement speech").is_ok());
            thread::sleep(HELPER_CANCEL_GRACE + Duration::from_millis(25));
            assert!(!recovered.terminated.load(Ordering::Acquire));
        }
    }
}
