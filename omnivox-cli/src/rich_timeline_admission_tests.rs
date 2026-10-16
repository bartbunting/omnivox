use super::*;
use omnivox_tts::timeline_v6::encode_timeline_v6;

fn extended_document() -> omnivox_tts::timeline_v6::PresentationTimelineV6 {
    let mut timeline = rich_document();
    timeline.protocol_version = 7;
    let shape = timeline.actions[0].shape.as_mut().unwrap();
    shape.waveform = omnivox_core::tone::Waveform::Pulse;
    shape.pulse_width = Some(0.25);
    shape.end_frequency_hz = Some(1320.0);
    shape.envelope_curve = Some(omnivox_core::tone::EnvelopeCurve::Exponential);
    timeline
}

#[test]
fn rich_multipart_preserves_shapes_and_rejects_version_or_size_mismatch() {
    for timeline in [rich_document(), extended_document()] {
        let version = timeline.protocol_version;
        let encoded = encode_timeline_v6(&timeline, true).unwrap();
        let bytes =
            encoded.len() / 4 * 3 - encoded.bytes().rev().take_while(|b| *b == b'=').count();
        let midpoint = encoded.len() / 8 * 4;
        let parts = [
            format!("{version} 8 91 0 2 {bytes} {}", &encoded[..midpoint]),
            format!("{version} 8 91 1 2 {bytes} {}", &encoded[midpoint..]),
        ];
        let generations = PresentationGenerations::default();
        let mut assembler = MultipartTimelineAssembler::start(&parts[0]).unwrap();
        assembler.push(&parts[1]).unwrap();
        assert_eq!(
            assembler.finish(&generations).unwrap().unwrap().timeline,
            TimelineDocument::Rich(timeline)
        );
        for wrong in [4, 5] {
            let mut assembler = MultipartTimelineAssembler::start(&parts[0]).unwrap();
            assert!(assembler
                .push(&parts[1].replacen(&format!("{version} "), &format!("{wrong} "), 1))
                .is_err());
        }
        let mut assembler = MultipartTimelineAssembler::start(
            &parts[0].replace(&format!("{bytes} "), &format!("{} ", bytes + 1)),
        )
        .unwrap();
        assembler
            .push(&parts[1].replace(&format!("{bytes} "), &format!("{} ", bytes + 1)))
            .unwrap();
        assert!(assembler.finish(&generations).is_err());
    }
}

#[test]
fn rich_admission_and_replacement_keep_old_versions_and_other_keys_independent() {
    let engines = EngineRegistry::new();
    let voices = registry(&engines, definition());
    for mut rich in [rich_document(), extended_document()] {
        rich.delivery_policy = PresentationDeliveryPolicy::Replaceable;
        rich.replacement_key = Some("navigation".into());
        let generations = PresentationGenerations::default();
        let cancellation = KeyedCancellationRegistry::default();
        let old = TimelineDocument::Native(rich.clone().into_execution_parts().0);
        let mut old_lease = begin_keyed_cancellation(&cancellation, &old).unwrap();
        old_lease.activate();
        let current = TimelineDocument::Rich(rich.clone());
        let mut current_lease = begin_keyed_cancellation(&cancellation, &current).unwrap();
        current_lease.activate();
        assert!(!old_lease.token().is_cancelled());
        let mut other = rich.clone();
        other.replacement_key = Some("status".into());
        let mut other_lease =
            begin_keyed_cancellation(&cancellation, &TimelineDocument::Rich(other)).unwrap();
        other_lease.activate();
        for bad_registry in [true, false] {
            let mut bad = rich.clone();
            if bad_registry {
                bad.registry_generation += 1;
            } else {
                bad.actions[0]
                    .shape
                    .as_mut()
                    .unwrap()
                    .envelope
                    .as_mut()
                    .unwrap()
                    .sustain_level = 1.5;
            }
            // Encode unchecked JSON to exercise the actual admission decoder.
            let command = Command::new(
                CommandId::EmacsvoxTimeline,
                Some(
                    omnivox_tts::presentation::encode_presentation_script(
                        &serde_json::to_string(&bad).unwrap(),
                    )
                    .unwrap(),
                ),
            );
            assert!(matches!(
                validate_structured_admission(
                    prepare_structured_presentation(&generations, &command),
                    &TtsState::default(),
                    &voices
                ),
                StructuredSubmissionRead::Rejected(_)
            ));
            assert!(!current_lease.token().is_cancelled());
        }
        let (sender, receiver) = synthesis_channel();
        let make_request = |timeline| SynthRequest::Timeline {
            timeline,
            letter: false,
            state: TtsState::default(),
            logical_voice_routing: LogicalVoiceRoutingSnapshot::capture(&voices, &engines),
            cancellation: None,
            lifecycle: RequestLifecycle::default(),
            gen: 1,
        };
        assert!(sender.try_send(make_request(old)).accepted);
        assert!(sender.try_send(make_request(current)).accepted);
        rich.generation += 1;
        rich.dispatch_id += 1;
        // A Classic-only update still uses version six and replaces its rich predecessor.
        rich.actions[0].shape = None;
        let replacement = TimelineDocument::Rich(rich);
        let queued = sender.try_send(make_request(replacement.clone()));
        assert!(queued.accepted);
        assert_eq!(queued.retired.len(), 1);
        assert!(matches!(
            receiver.recv().unwrap(),
            SynthRequest::Timeline {
                timeline: TimelineDocument::Native(_),
                ..
            }
        ));
        assert!(matches!(
            receiver.recv().unwrap(),
            SynthRequest::Timeline {
                timeline: TimelineDocument::Rich(_),
                ..
            }
        ));
        let mut lease = begin_keyed_cancellation(&cancellation, &replacement).unwrap();
        lease.activate();
        assert!(current_lease.token().is_cancelled());
        assert!(!old_lease.token().is_cancelled());
        assert!(!other_lease.token().is_cancelled());
    }
}
