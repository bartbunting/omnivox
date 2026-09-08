//! Reader, queue, and worker acceptance for mixed timelines.
use super::*;
use omnivox_tts::timeline_v4::encode_timeline_v4;

fn command(timeline: &PresentationTimelineV4) -> Command {
    Command::new(
        CommandId::EmacsvoxTimeline,
        Some(encode_timeline_v4(timeline, false).unwrap()),
    )
}

fn parts(timeline: &PresentationTimelineV4) -> Vec<String> {
    let encoded = encode_timeline_v4(timeline, true).unwrap();
    let padding = encoded.bytes().rev().take_while(|b| *b == b'=').count();
    let bytes = encoded.len() / 4 * 3 - padding;
    let count = if encoded.len() > 500_000 { 3 } else { 2 };
    let width = encoded.len() / (count * 4) * 4;
    (0..count)
        .map(|index| {
            let end = if index + 1 == count {
                encoded.len()
            } else {
                (index + 1) * width
            };
            let fragment = &encoded[index * width..end];
            format!(
                "4 {} {} {index} {count} {bytes} {fragment}",
                timeline.generation, timeline.dispatch_id
            )
        })
        .collect()
}

#[test]
fn multipart_v4_requires_one_complete_version_and_rejects_replay() {
    let timeline = timeline();
    let frames = parts(&timeline);
    let generations = PresentationGenerations::default();
    let prefix = MultipartTimelineAssembler::start(&frames[0]).unwrap();
    assert!(!prefix.is_complete());
    assert!(prefix.finish(&generations).is_err());
    let mut assembler = MultipartTimelineAssembler::start(&frames[0]).unwrap();
    assembler.push(&frames[1]).unwrap();
    let prepared = assembler.finish(&generations).unwrap().unwrap();
    assert_eq!(
        prepared.timeline,
        TimelineDocument::Layered(timeline.clone())
    );
    for (first, second) in [
        (frames[0].clone(), frames[1].replacen("4 ", "3 ", 1)),
        (frames[0].replacen("4 ", "3 ", 1), frames[1].clone()),
        (frames[0].clone(), frames[0].clone()),
    ] {
        let mut assembler = MultipartTimelineAssembler::start(&first).unwrap();
        assert!(assembler.push(&second).is_err());
    }
    let mut wrong_header =
        MultipartTimelineAssembler::start(&frames[0].replacen("4 ", "3 ", 1)).unwrap();
    wrong_header
        .push(&frames[1].replacen("4 ", "3 ", 1))
        .unwrap();
    assert!(wrong_header.finish(&generations).is_err());
    let mut committed = generations;
    committed.commit(timeline.generation);
    let mut replay = MultipartTimelineAssembler::start(&frames[0]).unwrap();
    replay.push(&frames[1]).unwrap();
    assert!(replay.finish(&committed).unwrap().is_none());
    assert!(omnivox_tts::timeline_protocol::decode_presentation_timeline_part(&frames[0]).is_err());
    assert!(decode_presentation_timeline(&command(&timeline).args.unwrap()).is_err());
}

#[test]
fn reader_rejects_invalid_v4_before_any_replacement_can_be_prepared() {
    let engines = EngineRegistry::new();
    let voices = registry(&engines, definition());
    let generations = PresentationGenerations::default();
    let (_sender, receiver) = mpsc::channel();
    let cancellation = KeyedCancellationRegistry::default();
    let mut valid = timeline();
    valid.delivery_policy = PresentationDeliveryPolicy::Replaceable;
    valid.replacement_key = Some("navigation".to_owned());
    let StructuredSubmissionRead::Prepared(waiting) = read_structured_submission(
        &generations,
        command(&valid),
        &receiver,
        &TtsState::default(),
        &voices,
    )
    .unwrap() else {
        panic!("valid request rejected")
    };
    let mut lease = begin_keyed_cancellation(&cancellation, &waiting.timeline).unwrap();
    lease.activate();
    for invalid in 0..5 {
        let mut bad = valid.clone();
        bad.generation += 1;
        bad.dispatch_id += 1;
        let MixedSpeechSpan::Layered(span) = &mut bad.spans[2] else {
            unreachable!()
        };
        match invalid {
            0 => bad.registry_generation += 1,
            1 => span.logical_voice_id = "missing".to_owned(),
            2 => span.logical_voice_id = "annotation".to_owned(),
            3 => span.text = "x".repeat(400_000),
            4 => {
                bad.actions = (0..513)
                    .map(|id| PresentationTimelineAction {
                        id: format!("action-{id}"),
                        lifecycle_anchor: PresentationLifecycleAnchor::Object,
                        position: PresentationTimelinePosition::SpanBoundary {
                            span_id: span.id,
                            affinity: PresentationAffinity::After,
                        },
                        action: PresentationAction::SemanticEvent,
                    })
                    .collect()
            }
            _ => unreachable!(),
        }
        let (multipart_sender, multipart_receiver) = mpsc::channel();
        let (first, input) = if invalid == 3 {
            let frames = parts(&bad);
            for frame in &frames[1..] {
                multipart_sender
                    .send(Ok(format!("emacsvox_timeline_part {frame}")))
                    .unwrap();
            }
            (
                Command::new(CommandId::EmacsvoxTimelinePart, Some(frames[0].clone())),
                &multipart_receiver,
            )
        } else {
            (command(&bad), &receiver)
        };
        assert!(matches!(
            read_structured_submission(&generations, first, input, &TtsState::default(), &voices,)
                .unwrap(),
            StructuredSubmissionRead::Rejected(RejectedStructuredSubmission {
                status: BatchStatus::Failed,
                ..
            })
        ));
        assert!(!lease.token().is_cancelled());
        assert_eq!(cancellation.active.lock().unwrap().len(), 1);
    }
    let frames = parts(&valid);
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(format!("emacsvox_timeline_part {}", frames[1])))
        .unwrap();
    assert!(matches!(
        read_structured_submission(
            &generations,
            Command::new(CommandId::EmacsvoxTimelinePart, Some(frames[0].clone())),
            &receiver,
            &TtsState::default(),
            &voices,
        )
        .unwrap(),
        StructuredSubmissionRead::Prepared(_)
    ));
    assert!(!lease.token().is_cancelled());
}

#[test]
fn mixed_queue_and_active_replacement_domains_keep_v3_separate() {
    let engines = EngineRegistry::new();
    let voices = registry(&engines, definition());
    let cancellation = KeyedCancellationRegistry::default();
    let mut mixed = timeline();
    mixed.delivery_policy = PresentationDeliveryPolicy::Replaceable;
    mixed.replacement_key = Some("navigation".to_owned());
    let old = PresentationTimelineEnvelope {
        protocol_version: 3,
        generation: 7,
        dispatch_id: 90,
        delivery_policy: Some(PresentationDeliveryPolicy::Replaceable),
        replacement_key: mixed.replacement_key.clone(),
        spans: vec![],
        actions: vec![],
    };
    let mut old_lease = begin_keyed_cancellation(&cancellation, &old.clone().into()).unwrap();
    old_lease.activate();
    let mut mixed_lease = begin_keyed_cancellation(&cancellation, &mixed.clone().into()).unwrap();
    mixed_lease.activate();
    assert!(!old_lease.token().is_cancelled());
    let (sender, receiver) = synthesis_channel();
    let make_request = |timeline| SynthRequest::Timeline {
        timeline,
        state: TtsState::default(),
        logical_voice_routing: LogicalVoiceRoutingSnapshot::capture(&voices, &engines),
        cancellation: None,
        lifecycle: RequestLifecycle::default(),
        gen: 1,
    };
    assert!(sender.try_send(make_request(old.into())).accepted);
    assert!(sender.try_send(make_request(mixed.clone().into())).accepted);
    mixed.generation += 1;
    mixed.dispatch_id += 1;
    let replacement = sender.try_send(make_request(mixed.clone().into()));
    assert!(replacement.accepted);
    assert_eq!(replacement.retired.len(), 1);
    assert_eq!(replacement.retired[0].reason, RetirementReason::Replaced);
    assert!(matches!(
        receiver.recv().unwrap(),
        SynthRequest::Timeline {
            timeline: TimelineDocument::Legacy(_),
            ..
        }
    ));
    let request = receiver.recv().unwrap();
    assert!(
        request.queued_payload_bytes() >= mixed.spans.iter().map(|s| s.text().len()).sum::<usize>()
    );
    assert!(
        matches!(request, SynthRequest::Timeline { timeline: TimelineDocument::Layered(t), .. } if t.dispatch_id == mixed.dispatch_id)
    );
    let mut replacement_lease = begin_keyed_cancellation(&cancellation, &mixed.into()).unwrap();
    replacement_lease.activate();
    assert!(mixed_lease.token().is_cancelled());
    assert!(!old_lease.token().is_cancelled());
}

#[test]
fn mixed_reader_to_worker_reports_consumed_choices_before_one_terminal_record() {
    for behavior in [Behavior::Buffered, Behavior::Stream] {
        let first = PreviewEngine::new("first", "one", Behavior::FailBeforeAudio);
        let second = PreviewEngine::new("second", "two", behavior);
        let mut engines = EngineRegistry::new();
        engines.register(first.clone()).unwrap();
        engines.register(second.clone()).unwrap();
        let mut voices = registry(&engines, definition());
        let mut generations = PresentationGenerations::default();
        let (_input, input) = mpsc::channel();
        let StructuredSubmissionRead::Prepared(prepared) = read_structured_submission(
            &generations,
            command(&timeline()),
            &input,
            &TtsState::default(),
            &voices,
        )
        .unwrap() else {
            panic!("reader rejected valid timeline")
        };
        let (work_sender, receiver) = synthesis_channel();
        execute_structured_presentation(
            prepared,
            &mut generations,
            &TtsState {
                speech_rate: 0.65,
                ..Default::default()
            },
            1,
            None,
            &engines,
            &RoutingPolicyRegistry::new("first"),
            &voices,
            &work_sender,
        );
        // A later registration must not change the already admitted document.
        voices
            .register_v2(42, vec![], FallbackPolicy::default(), &engines.inventory())
            .unwrap();
        drop(work_sender);
        let streams = AudioStreams::new_with_backend(8, 8, 8, AudioBackend::Null).unwrap();
        let control = streams.control();
        let capture = Capture::default();
        let (output, writer) =
            crate::marker_events::spawn_marker_event_reporter_with_writer(capture.clone());
        let (sender, tracker) = spawn_tracked_playback_reporter(output.clone());
        synthesis_worker(
            receiver,
            Arc::new(AtomicU64::new(1)),
            first,
            Arc::new(engines),
            Arc::new(RuntimeEngineHealth::new()),
            control.clone(),
            AudioFileLoader::with_cache(),
            sender,
            output,
        );
        tracker.join().unwrap();
        writer.join().unwrap();
        control.drain();
        let records = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let lines = records.lines().collect::<Vec<_>>();
        assert_eq!(lines.last(), Some(&"__EMACSVOX_TRACKED__ 91 completed"));
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("__EMACSVOX_TRACKED__"))
                .count(),
            1
        );
        let events = lines[..lines.len() - 1]
            .iter()
            .map(|line| decode_marker_event(line.split_whitespace().last().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.event, MarkerEvent::UtteranceStarted { .. }))
                .count(),
            5
        );
        let mut receipt_spans = vec![];
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event.protocol_version, 3);
            if let MarkerEvent::VoiceChoiceApplied(receipt) = &event.event {
                assert!(matches!(
                    events[index - 1].event,
                    MarkerEvent::UtteranceStarted { .. }
                ));
                assert_eq!(event.sequence, events[index - 1].sequence + 1);
                assert_eq!(receipt.registry_generation, 41);
                assert_eq!(receipt.choice.choice_id.as_deref(), Some("fallback"));
                receipt_spans.push(receipt.span_id);
            }
        }
        assert_eq!(receipt_spans, vec![3, 5]);
        assert_eq!(second.requests.lock().unwrap().len(), 5);
    }
}
