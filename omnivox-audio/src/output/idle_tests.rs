use super::*;
use crate::rodio_output::new_idle_sink;
use rodio::dynamic_mixer::{mixer, DynamicMixer};

fn fixture() -> (AudioControl, DynamicMixer<f32>) {
    let (speech, speech_output) = new_idle_sink();
    let (tone, tone_output) = new_idle_sink();
    let (sound, sound_output) = new_idle_sink();
    let (controller, output) = mixer(CHANNELS, SAMPLE_RATE);
    controller.add(speech_output);
    controller.add(tone_output);
    controller.add(sound_output);
    (
        AudioControl::new(
            ManagedSink::Device(Arc::new(speech)),
            ManagedSink::Device(Arc::new(tone)),
            ManagedSink::Device(Arc::new(sound)),
            8,
            8,
            8,
        ),
        output,
    )
}

fn samples(frames: usize) -> AudioBuffer {
    AudioBuffer::new(
        (1..=frames * 2)
            .map(|index| {
                let value = index as f32 / 16_384.0;
                if index % 2 == 0 {
                    -value
                } else {
                    value
                }
            })
            .collect(),
    )
}

fn assert_exact(output: &mut DynamicMixer<f32>, expected: &[f32]) {
    let mut first = None;
    for offset in 0_usize..2048 {
        let value = output.next().expect("live mixer");
        if value != 0.0 {
            assert!(offset.is_multiple_of(CHANNELS as usize));
            first = Some(value);
            break;
        }
    }
    assert_eq!(first, Some(expected[0]), "first sample after idle");
    let remaining: Vec<_> = output.by_ref().take(expected.len() - 1).collect();
    assert_eq!(
        remaining,
        expected[1..],
        "PCM changed at the queue boundary"
    );
}

#[test]
fn buffered_and_tracked_onsets_survive_cold_start_idle_and_adjacency() {
    for stream in [StreamType::Speech, StreamType::Tone, StreamType::Sound] {
        for tracked in [false, true] {
            let (control, mut output) = fixture();
            for idle_frames in [0, 44_100, 1, 511, 44_100] {
                assert!(output
                    .by_ref()
                    .take(idle_frames * 2)
                    .all(|value| value == 0.0));
                let mut expected = Vec::new();
                let mut tickets = Vec::new();
                // Short sources on either side of the faulty 256-frame boundary,
                // followed immediately by a longer source on the same queue.
                for frames in [1, 255, 256, 257, 2048] {
                    let audio = samples(frames);
                    if tracked {
                        tickets.push(control.queue_tracked(stream, &audio).unwrap().unwrap());
                    } else {
                        assert!(control.queue(stream, &audio).unwrap());
                    }
                    expected.extend(audio.samples);
                }
                assert_exact(&mut output, &expected);
                assert!(output.by_ref().take(1024).all(|value| value == 0.0));
                for ticket in tickets {
                    assert_eq!(ticket.wait(), PlaybackStatus::Completed);
                }
                assert_eq!(control.pending(stream), 0);
            }
        }
    }
}

#[test]
fn progressive_onsets_preserve_samples_cues_and_completion_after_idle() {
    for letter_reserve in [false, true] {
        let (control, mut output) = fixture();
        for idle_frames in [0, 44_100, 44_100] {
            assert!(output
                .by_ref()
                .take(idle_frames * 2)
                .all(|value| value == 0.0));
            let (sender, receiver) = mpsc::channel();
            let (mut producer, ticket) = control
                .queue_progressive_speech_with_cue_callback_cancellable_if(
                    move |cue| sender.send(cue).unwrap(),
                    CancellationToken::new(),
                    || true,
                )
                .unwrap()
                .unwrap();
            if letter_reserve {
                producer.use_letter_navigation_prebuffer().unwrap();
            }
            let cues: Vec<_> = [0, 255, 256, 1024, 3072]
                .into_iter()
                .map(|frame_offset| PlaybackCue {
                    frame_offset,
                    identifier: frame_offset,
                })
                .collect();
            producer.push_cues(cues.clone()).unwrap();
            let audio = samples(3072);
            // Three PCM windows plus completion fit the bounded queue.
            for window in audio.samples.chunks(2048) {
                producer
                    .push_audio(AudioBuffer::new(window.to_vec()))
                    .unwrap();
            }
            producer.finish().unwrap();
            assert_exact(&mut output, &audio.samples);
            assert!(output.by_ref().take(1024).all(|value| value == 0.0));
            assert_eq!(ticket.wait(), PlaybackStatus::Completed);
            assert_eq!(receiver.try_iter().collect::<Vec<_>>(), cues);
        }
    }
}

#[test]
fn stopped_speech_keeps_its_fade_and_the_next_onset_is_exact() {
    let (control, mut output) = fixture();
    let cancelled = control
        .queue_tracked(StreamType::Speech, &AudioBuffer::new(vec![0.5; 8192]))
        .unwrap()
        .unwrap();
    assert_eq!(output.next(), Some(0.5));
    assert_eq!(output.next(), Some(0.5));
    control.stop(StreamType::Speech);
    let tail: Vec<_> = output.by_ref().take(SPEECH_STOP_FADE_FRAMES * 2).collect();
    assert_eq!(tail.first(), Some(&0.5));
    assert_eq!(tail.last(), Some(&0.0));
    assert!(tail.chunks_exact(2).all(|frame| frame[0] == frame[1]));
    assert!(output.by_ref().take(88_200).all(|value| value == 0.0));
    assert_eq!(cancelled.wait(), PlaybackStatus::Cancelled);
    let audio = samples(2048);
    control.queue(StreamType::Speech, &audio).unwrap();
    assert_exact(&mut output, &audio.samples);
}

#[test]
fn canonical_queue_still_converts_to_the_device_format() {
    for (channels, rate) in [(1, 44_100), (2, 48_000), (1, 48_000), (6, 44_100)] {
        let (sink, queue) = new_idle_sink();
        let audio = samples(2048);
        sink.append(BufferSource::new(audio.samples.clone()));
        let (controller, mut output) = mixer::<f32>(channels, rate);
        controller.add(queue);
        let reference = rodio::source::UniformSourceIterator::<_, f32>::new(
            BufferSource::new(audio.samples),
            channels,
            rate,
        );
        // Compare the onset with the same PCM converted without any queue.
        let expected: Vec<_> = reference.take(512 * channels as usize).collect();
        assert_eq!(
            output.by_ref().take(expected.len()).collect::<Vec<_>>(),
            expected,
            "device format {channels} channels at {rate} Hz"
        );
    }
}
