// Injected only into an isolated release snapshot by reproduce.py.
// These tests assert the observed defect; they are not regression tests for a fix.
use super::*;
use crate::{ProgressiveSilenceTrimmer, SilenceTrimmer};

const INPUT_FRAMES: usize = 2048;
const CAPTURE_SAMPLES: usize = 8192;

fn ramp() -> Vec<f32> {
    // Distinct, exactly representable values identify every interleaved sample.
    (1..=INPUT_FRAMES * 2).map(|v| v as f32 / 8192.0).collect()
}

fn after_onset(output: &[f32]) -> (usize, &[f32]) {
    let start = output.iter().position(|v| *v != 0.0).expect("audio");
    assert_eq!(start % 2, 0);
    (start / 2, &output[start..])
}

fn check_defect(label: &str, output: &[f32], input: &[f32]) {
    let (idle_frames, output) = after_onset(output);
    let doubled: Vec<f32> = input[..512].iter().flat_map(|v| [*v, *v]).collect();
    assert_eq!(&output[..1024], doubled, "first 256 frames: {label}");
    assert_eq!(
        &output[1024..input.len() + 512],
        &input[512..],
        "tail: {label}"
    );
    assert!(output[input.len() + 512..].iter().all(|v| *v == 0.0));
    println!("{label}: idle_frames={idle_frames}, lost_input_frames=0, stretched_input_frames=256, stretched_output_frames=512, exact_tail_frames={}", input.len() / 2 - 256);
}

#[derive(Clone, Copy, Debug)]
enum Path {
    Buffered,
    Tracked,
    Progressive,
}

fn append(control: &AudioControl, path: Path, samples: Vec<f32>) {
    match path {
        Path::Buffered => {
            assert!(control
                .queue(StreamType::Speech, &AudioBuffer::new(samples))
                .is_ok());
        }
        Path::Tracked => {
            control
                .queue_tracked(StreamType::Speech, &AudioBuffer::new(samples))
                .unwrap()
                .unwrap();
        }
        Path::Progressive => {
            let (mut producer, _ticket) = control
                .queue_progressive_speech_with_cue_callback_cancellable_if(
                    |_| {},
                    CancellationToken::new(),
                    || true,
                )
                .unwrap()
                .unwrap();
            producer.use_letter_navigation_prebuffer().unwrap();
            // Keep audio plus completion within capacity before driving the mixer.
            for chunk in samples.chunks(4096) {
                producer
                    .push_audio(AudioBuffer::new(chunk.to_vec()))
                    .unwrap();
            }
            producer.finish().unwrap();
        }
    }
}

// Diagnostic control only: the queue contains canonical 44.1 kHz stereo sources.
// Override the idle queue metadata without changing any PCM. Production repair
// still needs review of all source formats, devices and cancellation behavior.
struct CanonicalQueue(rodio::queue::SourcesQueueOutput<f32>);

impl Iterator for CanonicalQueue {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        self.0.next()
    }
}

impl Source for CanonicalQueue {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        44100
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn setup(canonical_metadata: bool) -> (AudioControl, rodio::dynamic_mixer::DynamicMixer<f32>) {
    let (speech, queue) = Sink::new_idle();
    let (tone, _) = Sink::new_idle();
    let (sound, _) = Sink::new_idle();
    let control = AudioControl::new(
        ManagedSink::Device(Arc::new(speech)),
        ManagedSink::Device(Arc::new(tone)),
        ManagedSink::Device(Arc::new(sound)),
        16,
        16,
        16,
    );
    let (controller, mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 44100);
    if canonical_metadata {
        controller.add(CanonicalQueue(queue));
    } else {
        controller.add(queue);
    }
    (control, mixer)
}

#[test]
fn idle_onset_repro_after_idle() {
    for path in [Path::Buffered, Path::Tracked, Path::Progressive] {
        let (control, mut mixer) = setup(false);
        for repetition in 0..5 {
            // Advance the mixer by one second. No wall-clock sleep or device.
            assert!(mixer.by_ref().take(88200).all(|s| s == 0.0));
            let samples = ramp();
            append(&control, path, samples.clone());
            let output: Vec<f32> = mixer.by_ref().take(CAPTURE_SAMPLES).collect();
            check_defect(
                &format!("{path:?} repetition={repetition}"),
                &output,
                &samples,
            );
        }
    }
}

#[test]
fn idle_onset_repro_cold_and_contiguous() {
    for path in [Path::Buffered, Path::Tracked, Path::Progressive] {
        let (control, mut mixer) = setup(false);
        let samples = ramp();
        append(&control, path, samples.clone());
        append(&control, path, samples.clone());
        let output: Vec<f32> = mixer.by_ref().take(12288).collect();
        let (_, onset) = after_onset(&output);
        let first_length = samples.len() + 512;
        let mut first = onset[..first_length].to_vec();
        first.extend([0.0; 2]);
        check_defect(&format!("cold {path:?}"), &first, &samples);
        assert_eq!(&onset[first_length..first_length + samples.len()], samples);
        println!("contiguous {path:?}: second source preserved exactly");
    }
}

#[test]
fn idle_onset_repro_canonical_metadata_control() {
    for path in [Path::Buffered, Path::Tracked, Path::Progressive] {
        let (control, mut mixer) = setup(true);
        for repetition in 0..5 {
            assert!(mixer.by_ref().take(88200).all(|s| s == 0.0));
            let samples = ramp();
            append(&control, path, samples.clone());
            let output: Vec<f32> = mixer.by_ref().take(CAPTURE_SAMPLES).collect();
            let (_, onset) = after_onset(&output);
            assert_eq!(&onset[..samples.len()], samples);
            assert!(onset[samples.len()..].iter().all(|s| *s == 0.0));
            println!("canonical queue metadata {path:?} repetition={repetition}: all {INPUT_FRAMES} frames preserved exactly");
        }
    }
}

#[test]
fn idle_onset_repro_tick_and_internal_silence() {
    for lead_frames in [882, 2646] {
        for amplitude in [0.011, 0.02, 0.5] {
            let mut input = vec![0.0; lead_frames * 2];
            input[..2].copy_from_slice(&[amplitude, amplitude]);
            input.extend(vec![0.1; 2048]);
            let mut buffered = AudioBuffer::new(input.clone());
            SilenceTrimmer::with_asymmetric_padding(0.01, 0.0, 0.005)
                .process_with_report(&mut buffered)
                .unwrap();
            assert_eq!(buffered.samples, input);
            for chunk in [1, 128, 256, 512, 882, 1024, 4096] {
                let mut trimmer =
                    ProgressiveSilenceTrimmer::with_asymmetric_padding(0.01, 0.0, 0.005);
                let mut output = Vec::new();
                for window in input.chunks(chunk * 2) {
                    output.extend(
                        trimmer
                            .process_window(AudioBuffer::new(window.to_vec()))
                            .unwrap()
                            .samples,
                    );
                }
                output.extend(trimmer.finish().unwrap().0.samples);
                assert_eq!(output, input);
                println!("tick/internal silence: lead_frames={lead_frames} amplitude={amplitude} window_frames={chunk}: preserved exactly");
            }
            let (control, mut mixer) = setup(false);
            assert!(mixer.by_ref().take(88200).all(|s| s == 0.0));
            append(&control, Path::Progressive, input.clone());
            let capture: Vec<f32> = mixer.by_ref().take(12288).collect();
            check_defect(
                &format!("tick through playback lead_frames={lead_frames} amplitude={amplitude}"),
                &capture,
                &input,
            );
        }
    }
}
