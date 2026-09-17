// Included in the existing helper engine tests to reuse its bounded fake PCM peer.
mod native_parent {
    use super::*;
    use crate::helper_protocol::parameters as p;
    use crate::native_parameters::CatalogueIdentity;
    use serde_json::json;

    struct NativePeer {
        inner: MockConnection,
        requests: Mutex<HashMap<u64, p::Request>>,
        replies: Mutex<HashMap<u64, p::Response>>,
        fault: AtomicUsize,
        stall_queries: AtomicBool,
        stall_query_write: AtomicBool,
    }
    impl NativePeer {
        fn new(streaming: bool) -> Arc<Self> {
            let mut descriptor = helper_descriptor("eloquence", "1.0");
            if streaming {
                descriptor.capabilities.audio_output = AudioOutputMode::StreamingPcm;
            }
            Arc::new(Self {
                inner: MockConnection::new(
                    descriptor,
                    if streaming {
                        MockSynthesisMode::StreamComplete
                    } else {
                        MockSynthesisMode::Complete
                    },
                ),
                requests: Mutex::new(HashMap::new()),
                replies: Mutex::new(HashMap::new()),
                fault: AtomicUsize::new(0),
                stall_queries: AtomicBool::new(false),
                stall_query_write: AtomicBool::new(false),
            })
        }
        fn queue(&self, request_id: u64, body: p::ResponseBody) {
            self.replies.lock().unwrap().insert(
                request_id,
                p::Response {
                    protocol_version: 6,
                    request_id,
                    body,
                },
            );
            self.inner
                .push(self.inner.response(request_id, HelperResponseBody::Pong));
        }
    }
    impl HelperConnection for NativePeer {
        fn send(&self, request: &HelperRequest) -> Result<(), HelperEngineError> {
            session::validate_common_request(request)?;
            assert_eq!(request.protocol_version, 6);
            let mut common = request.clone();
            common.protocol_version = 5;
            self.inner.send(&common)
        }
        fn send_parameters(&self, request: &p::Request) -> Result<(), HelperEngineError> {
            request.validate()?;
            self.requests
                .lock()
                .unwrap()
                .insert(request.request_id, request.clone());
            match &request.body {
                p::RequestBody::Synthesize {
                    text,
                    settings,
                    anchors,
                    ..
                } => self.inner.send(&HelperRequest::with_version(
                    5,
                    request.request_id,
                    HelperRequestBody::Synthesize {
                        text: text.clone(),
                        settings: settings.clone(),
                        anchors: Some(anchors.clone()),
                    },
                ))?,
                p::RequestBody::GetEngineParametersV1(q) => {
                    if self.stall_query_write.load(Ordering::Acquire) {
                        let mut queue = self.inner.responses.lock().unwrap();
                        while !self.inner.terminated.load(Ordering::Acquire) {
                            queue = self
                                .inner
                                .response_ready
                                .wait_timeout(queue, Duration::from_millis(50))
                                .unwrap()
                                .0;
                        }
                        return Err(HelperEngineError::Exited);
                    }
                    if !self.stall_queries.load(Ordering::Acquire) {
                        self.queue(
                            request.request_id,
                            p::ResponseBody::EngineParametersV1 {
                                engine_id: q.engine_id.clone(),
                                result: p::CatalogueResult::Ready {
                                    identity: identity(),
                                    voice_id: q.voice_id.clone(),
                                    parameters: vec![],
                                    mappings: vec![],
                                    next_cursor: None,
                                },
                            },
                        );
                    }
                }
                p::RequestBody::ExplainVoiceParametersV1 { source } => {
                    let (evidence, plan_id) = match source {
                        p::ExplanationSource::Draft { .. } => (p::Evidence::Planned, None),
                        p::ExplanationSource::Applied { plan_id } => {
                            (p::Evidence::AdapterApplied, Some(plan_id.clone()))
                        }
                    };
                    let mut identity = identity();
                    if self.fault.load(Ordering::Acquire) == 2 {
                        identity.runtime_generation += 1;
                    }
                    self.queue(
                        request.request_id,
                        p::ResponseBody::VoiceParametersExplainedV1 {
                            result: p::ExplanationResult::Ready {
                                evidence,
                                plan_id,
                                realized: p::RealizedVoice {
                                    engine_id: "eloquence".into(),
                                    voice_id: "reed".into(),
                                },
                                identity,
                                parameters: vec![],
                            },
                        },
                    );
                }
            }
            Ok(())
        }
        fn receive(&self, _: Duration) -> Result<HelperResponse, HelperEngineError> {
            unreachable!("parent must use negotiated reader")
        }
        fn receive_message(
            &self,
            timeout: Duration,
        ) -> Result<session::Response, HelperEngineError> {
            let mut response = self.inner.receive(timeout)?;
            if let Some(r) = self
                .replies
                .lock()
                .unwrap()
                .remove(&response.request_id.unwrap())
            {
                return Ok(session::Response::Parameters(r));
            }
            response.protocol_version = 6;
            if let HelperResponseBody::Hello {
                selected_protocol_version,
                ..
            } = &mut response.body
            {
                *selected_protocol_version = 6;
            }
            if let HelperResponseBody::SynthesisStarted {
                format,
                actual_voice_id,
            } = response.body
            {
                let id = response.request_id.unwrap();
                let request = self.requests.lock().unwrap()[&id].clone();
                let p::RequestBody::Synthesize {
                    voice_parameters, ..
                } = request.body
                else {
                    unreachable!()
                };
                let mut application = voice_parameters.map(|p| p::NativeApplication {
                    status: p::ApplicationStatus::Applied,
                    plan_id: Some(format!("plan-{id}")),
                    identity: Some(p.expected_identity),
                    masked_parameters: vec![],
                    reason: None,
                });
                match self.fault.load(Ordering::Acquire) {
                    1 => application = None,
                    2 => {
                        if let Some(a) = &mut application {
                            a.identity.as_mut().unwrap().runtime_generation += 1;
                        }
                    }
                    3 => {
                        application = Some(p::NativeApplication {
                            status: p::ApplicationStatus::CommonOnly,
                            plan_id: None,
                            identity: None,
                            masked_parameters: vec![],
                            reason: Some("Unavailable".into()),
                        })
                    }
                    _ => {}
                }
                return Ok(session::Response::Parameters(p::Response {
                    protocol_version: 6,
                    request_id: id,
                    body: p::ResponseBody::SynthesisStarted {
                        format,
                        actual_voice_id,
                        native_application: application,
                    },
                }));
            }
            Ok(session::Response::Common(response))
        }
        fn terminate(&self) -> Result<(), HelperEngineError> {
            self.inner.terminate()
        }
    }
    struct Connector(Mutex<VecDeque<Arc<NativePeer>>>);
    impl HelperConnector for Connector {
        fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(HelperEngineError::Exited)?)
        }
    }
    fn engine(peers: Vec<Arc<NativePeer>>) -> HelperTtsEngine {
        HelperTtsEngine::with_connector(
            mock_config("eloquence"),
            Arc::new(Connector(Mutex::new(peers.into()))),
        )
        .unwrap()
    }
    fn identity() -> CatalogueIdentity {
        serde_json::from_value(json!({"schema_id":"eloquence.eci.v1", "profile_id":"test.v1", "catalogue_revision":"a".repeat(64), "runtime_generation":1})).unwrap()
    }
    fn native() -> p::VoiceParameters {
        serde_json::from_value(json!({"native":{"engine_id":"eloquence","schema_id":"eloquence.eci.v1","parameters":{"pitch":{"op":"set","value":40}}},
            "context_dimensions":["average_pitch"],"expected_identity":identity(),"unavailable_policy":"require"})).unwrap()
    }
    fn query() -> p::CatalogueQuery {
        p::CatalogueQuery {
            engine_id: "eloquence".into(),
            voice_id: None,
            cursor: None,
            expected_catalogue_revision: None,
        }
    }
    fn applied(a: &p::NativeApplication) -> p::ExplanationSource {
        p::ExplanationSource::Applied {
            plan_id: a.plan_id.clone().unwrap(),
        }
    }

    #[test]
    fn ordinary_and_native_speech_keep_receipts_separate_and_reset_per_request() {
        for streaming in [false, true] {
            let peer = NativePeer::new(streaming);
            let engine = engine(vec![Arc::clone(&peer)]);
            assert!(matches!(
                engine.query_parameters(query()).unwrap(),
                p::CatalogueResult::Ready { .. }
            ));
            let (result, receipt) = engine
                .synthesize_with_parameters(&synthesis_request("hello"), &native())
                .unwrap();
            assert!(!result.audio.samples.is_empty());
            assert_eq!(receipt.status, p::ApplicationStatus::Applied);
            assert!(matches!(
                engine.explain_parameters(applied(&receipt)).unwrap(),
                p::ExplanationResult::Ready {
                    evidence: p::Evidence::AdapterApplied,
                    ..
                }
            ));
            assert!(engine.synthesize(&synthesis_request("ordinary")).is_ok());
            let requests = peer.requests.lock().unwrap();
            let ordinary = requests
                .values()
                .filter(|r| {
                    matches!(
                        r.body,
                        p::RequestBody::Synthesize {
                            voice_parameters: None,
                            ..
                        }
                    )
                })
                .count();
            assert_eq!(ordinary, 1);
        }
    }

    struct OrderedSink {
        receipt: Arc<AtomicBool>,
        audio: usize,
    }
    impl SynthesisStreamSink for OrderedSink {
        fn start(&mut self, _: SynthesisStreamStart) -> Result<(), TtsError> {
            assert!(self.receipt.load(Ordering::Acquire));
            Ok(())
        }
        fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
            assert!(self.receipt.load(Ordering::Acquire));
            self.audio += audio.samples.len();
            Ok(())
        }
        fn markers(
            &mut self,
            _: Vec<SynthesisMarker>,
            _: Vec<ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            Ok(())
        }
    }
    #[test]
    fn receipt_precedes_buffered_and_progressive_sink_start() {
        for streaming in [false, true] {
            let engine = engine(vec![NativePeer::new(streaming)]);
            let observed = Arc::new(AtomicBool::new(false));
            let mut sink = OrderedSink {
                receipt: Arc::clone(&observed),
                audio: 0,
            };
            engine
                .synthesize_stream_with_parameters(
                    &synthesis_request("hello"),
                    &native(),
                    &mut sink,
                    &mut |_| {
                        assert!(!observed.swap(true, Ordering::AcqRel));
                    },
                )
                .unwrap();
            assert!(sink.audio > 0);
        }
    }
    #[test]
    fn invalid_receipts_never_reach_sink_or_application_callback() {
        for fault in [1, 2, 3] {
            let peer = NativePeer::new(true);
            peer.fault.store(fault, Ordering::Release);
            let engine = engine(vec![Arc::clone(&peer)]);
            let mut sink = RecordingStreamSink::default();
            assert!(engine
                .synthesize_stream_with_parameters(
                    &synthesis_request("hello"),
                    &native(),
                    &mut sink,
                    &mut |_| panic!("unvalidated evidence escaped")
                )
                .is_err());
            assert!(sink.starts.is_empty() && sink.audio.is_empty());
            assert!(peer.inner.terminated.load(Ordering::Acquire));
        }
    }
    #[test]
    fn old_helper_requires_explicit_common_only_policy() {
        let peer = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::Complete,
        ));
        let engine = mock_engine(vec![Arc::clone(&peer)]).unwrap();
        assert!(matches!(
            engine.query_parameters(query()).unwrap(),
            p::CatalogueResult::Unavailable {
                reason: p::CatalogueUnavailable::UnsupportedHelper,
                ..
            }
        ));
        assert!(engine
            .synthesize_with_parameters(&synthesis_request("strict"), &native())
            .is_err());
        assert!(!peer
            .sent
            .lock()
            .unwrap()
            .iter()
            .any(|r| matches!(r.body, HelperRequestBody::Synthesize { .. })));
        let mut settings = native();
        settings.unavailable_policy = p::UnavailablePolicy::CommonOnly;
        let (_, receipt) = engine
            .synthesize_with_parameters(&synthesis_request("ordinary"), &settings)
            .unwrap();
        assert_eq!(receipt.status, p::ApplicationStatus::CommonOnly);
        assert!(receipt.plan_id.is_none());
        assert!(!peer.terminated.load(Ordering::Acquire));
    }
    #[test]
    fn query_during_speech_is_busy_and_cancellation_keeps_connection_usable() {
        let peer = NativePeer::new(true);
        peer.inner.set_mode(MockSynthesisMode::StreamWaitForCancel);
        let engine = Arc::new(engine(vec![Arc::clone(&peer)]));
        let worker = Arc::clone(&engine);
        let (started, ready) = mpsc::sync_channel(1);
        let task = thread::spawn(move || {
            worker.synthesize_stream_with_parameters(
                &synthesis_request("wait"),
                &native(),
                &mut RecordingStreamSink::default(),
                &mut |_| {
                    started.send(()).unwrap();
                },
            )
        });
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            engine.query_parameters(query()).unwrap(),
            p::CatalogueResult::Busy { .. }
        ));
        assert!(!peer
            .requests
            .lock()
            .unwrap()
            .values()
            .any(|r| matches!(r.body, p::RequestBody::GetEngineParametersV1(_))));
        engine.stop();
        assert!(task.join().unwrap().is_err());
        assert!(matches!(
            engine.query_parameters(query()).unwrap(),
            p::CatalogueResult::Ready { .. }
        ));
        peer.inner.set_mode(MockSynthesisMode::StreamComplete);
        assert!(engine
            .synthesize(&synthesis_request("after cancel"))
            .is_ok());
        assert!(!peer.inner.terminated.load(Ordering::Acquire));
    }
    #[test]
    fn stalled_query_retires_connection_and_old_applied_references_expire() {
        let first = NativePeer::new(false);
        let second = NativePeer::new(false);
        let engine = engine(vec![Arc::clone(&first), Arc::clone(&second)]);
        let (_, receipt) = engine
            .synthesize_with_parameters(&synthesis_request("hello"), &native())
            .unwrap();
        first.stall_queries.store(true, Ordering::Release);
        assert!(matches!(
            engine.query_parameters(query()),
            Err(HelperEngineError::Timeout(_))
        ));
        assert!(first.inner.terminated.load(Ordering::Acquire));
        assert!(matches!(
            engine.explain_parameters(applied(&receipt)).unwrap(),
            p::ExplanationResult::Unavailable {
                reason: p::ExplanationUnavailable::PlanExpired,
                ..
            }
        ));
        assert!(engine.synthesize(&synthesis_request("reconnected")).is_ok());
        assert!(matches!(
            engine.explain_parameters(applied(&receipt)).unwrap(),
            p::ExplanationResult::Unavailable {
                reason: p::ExplanationUnavailable::PlanExpired,
                ..
            }
        ));
        assert!(!second
            .requests
            .lock()
            .unwrap()
            .values()
            .any(|r| matches!(r.body, p::RequestBody::ExplainVoiceParametersV1 { .. })));
    }
    #[test]
    fn retained_explanation_cannot_change_identity_and_history_is_bounded() {
        let peer = NativePeer::new(false);
        let engine = engine(vec![Arc::clone(&peer)]);
        let (_, first) = engine
            .synthesize_with_parameters(&synthesis_request("first"), &native())
            .unwrap();
        let mut last = first.clone();
        for _ in 0..64 {
            last = engine
                .synthesize_with_parameters(&synthesis_request("next utterance"), &native())
                .unwrap()
                .1;
        }
        assert!(matches!(
            engine.explain_parameters(applied(&first)).unwrap(),
            p::ExplanationResult::Unavailable {
                reason: p::ExplanationUnavailable::PlanExpired,
                ..
            }
        ));
        assert!(matches!(
            engine.explain_parameters(applied(&last)).unwrap(),
            p::ExplanationResult::Ready { .. }
        ));
        peer.fault.store(2, Ordering::Release);
        assert!(engine.explain_parameters(applied(&last)).is_err());
        assert!(peer.inner.terminated.load(Ordering::Acquire));
    }
    #[test]
    fn metadata_queries_do_not_start_deferred_helpers() {
        let peer = NativePeer::new(false);
        let engine = HelperTtsEngine::with_deferred_connector(
            mock_config("eloquence"),
            Arc::new(Connector(Mutex::new(vec![Arc::clone(&peer)].into()))),
            peer.inner.descriptor.clone(),
        )
        .unwrap();
        assert!(matches!(
            engine.query_parameters(query()).unwrap(),
            p::CatalogueResult::Unavailable {
                reason: p::CatalogueUnavailable::EngineUnavailable,
                ..
            }
        ));
        assert!(peer.inner.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn parameter_query_watchdog_wakes_a_blocked_write_without_accumulating_workers() {
        let peer = NativePeer::new(false);
        peer.stall_query_write.store(true, Ordering::Release);
        let engine = engine(vec![Arc::clone(&peer)]);
        let started = Instant::now();
        assert!(matches!(
            engine.query_parameters(query()),
            Err(HelperEngineError::Timeout("parameter query"))
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(peer.inner.terminated.load(Ordering::Acquire));
        assert!(engine.current_connection().is_err());
    }
}
