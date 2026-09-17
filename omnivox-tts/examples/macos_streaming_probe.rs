//! Native silent acceptance. Run on macOS; the main thread owns the Cocoa loop.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This probe requires native macOS.");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
fn main() {
    let worker = std::thread::spawn(|| {
        let result = std::panic::catch_unwind(native::run);
        omnivox_tts::macos::stop_main_runloop();
        result
    });
    omnivox_tts::macos::run_main_runloop();
    worker.join().unwrap().unwrap();
}

#[cfg(target_os = "macos")]
mod native {
    use omnivox_tts::contracts::{AudioOutputMode, PhysicalVoiceId};
    use omnivox_tts::macos::MacOsTtsEngine;
    use omnivox_tts::*;
    use std::time::{Duration, Instant};

    struct Sink<'a> {
        engine: &'a MacOsTtsEngine,
        started: Instant,
        first: Option<Duration>,
        frames: u64,
        windows: usize,
        expected: PhysicalVoiceId,
        interrupt: bool,
        fail: bool,
    }
    impl SynthesisStreamSink for Sink<'_> {
        fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
            assert_eq!(start.engine_id, "macos");
            assert_eq!(start.actual_voice.as_ref(), Some(&self.expected));
            Ok(())
        }
        fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
            assert!(!audio.is_empty());
            assert!(audio.samples.iter().all(|value| value.is_finite()));
            self.frames += audio.frame_count() as u64;
            self.windows += 1;
            if self.first.is_none() {
                self.first = Some(self.started.elapsed());
                // Let the native queue fill. Stop must wake its producer even
                // while this consumer is busy, and replacement must recover.
                std::thread::sleep(Duration::from_millis(250));
                if self.interrupt {
                    let stop = Instant::now();
                    self.engine.stop();
                    assert!(stop.elapsed() < Duration::from_millis(100));
                }
                if self.fail {
                    return Err(TtsError::SynthesisFailed("probe sink failure".into()));
                }
            }
            Ok(())
        }
        fn markers(
            &mut self,
            markers: Vec<SynthesisMarker>,
            anchors: Vec<ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            assert!(markers.is_empty());
            assert!(anchors
                .iter()
                .all(|anchor| anchor.resolution == AnchorResolution::Omitted));
            Ok(())
        }
    }

    pub(super) fn run() {
        let engine = MacOsTtsEngine::new().unwrap();
        assert_eq!(
            engine.descriptor().capabilities.audio_output,
            AudioOutputMode::StreamingPcm
        );
        let voices: Vec<_> = engine
            .available_voices()
            .into_iter()
            .filter(|v| v.language.starts_with("en"))
            .take(3)
            .collect();
        assert!(!voices.is_empty(), "native runner has no English voice");
        for voice in voices {
            let physical = PhysicalVoiceId::new("macos", &voice.identifier);
            println!("VOICE {} {}", voice.identifier, voice.name);
            let text = "Streaming must preserve every word, including the end of this sentence. "
                .repeat(12);
            let request = SynthesisRequest::new(&text, TtsSettings::default())
                .with_route("probe", physical.clone());
            let mut sink = Sink {
                engine: &engine,
                started: Instant::now(),
                first: None,
                frames: 0,
                windows: 0,
                expected: physical.clone(),
                interrupt: false,
                fail: false,
            };
            let completed = engine.synthesize_stream(&request, &mut sink).unwrap();
            assert_eq!(completed.frame_count, sink.frames);
            assert!(sink.windows > 8, "long speech did not arrive progressively");
            println!(
                "STREAM first_ms={} total_ms={} windows={} frames={}",
                sink.first.unwrap().as_millis(),
                sink.started.elapsed().as_millis(),
                sink.windows,
                sink.frames
            );
            // The compatibility API must retain the entire utterance, too.
            let short = SynthesisRequest::new("A complete short sentence.", TtsSettings::default())
                .with_route("probe", physical.clone());
            let buffered = engine.synthesize(&short).unwrap();
            assert_eq!(buffered.actual_voice.as_ref(), Some(&physical));
            assert!(!buffered.audio.is_empty());
            for iteration in 0..4 {
                let mut sink = Sink {
                    engine: &engine,
                    started: Instant::now(),
                    first: None,
                    frames: 0,
                    windows: 0,
                    expected: physical.clone(),
                    interrupt: iteration % 2 == 0,
                    fail: iteration % 2 != 0,
                };
                assert!(engine.synthesize_stream(&request, &mut sink).is_err());
                assert_eq!(sink.windows, 1, "audio escaped retirement");
                assert!(!engine.is_speaking());
                assert!(
                    !engine.synthesize(&short).unwrap().audio.is_empty(),
                    "replacement failed"
                );
            }
            let cancelled = SynthesisCancellationToken::new();
            cancelled.cancel();
            assert!(engine
                .synthesize(&short.clone().with_cancellation(cancelled))
                .is_err());
            println!("PASS native streaming, buffered compatibility, cancellation, sink failure and recovery");
        }
    }
}
