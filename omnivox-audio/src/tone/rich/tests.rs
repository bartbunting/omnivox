use super::*;
use std::cell::Cell;

fn tone(waveform: Waveform, envelope: Option<AdsrEnvelope>) -> ToneParameters {
    ToneParameters {
        frequency_hz: 441.0,
        duration_ms: 200,
        volume: 1.0,
        shape: ToneShape { waveform, envelope },
    }
}

fn adsr(attack_ms: u32, decay_ms: u32, sustain_level: f32, release_ms: u32) -> AdsrEnvelope {
    AdsrEnvelope {
        attack_ms,
        decay_ms,
        sustain_level,
        release_ms,
    }
}

#[test]
fn classic_shaped_and_interruptible_entry_points_preserve_every_sample() {
    for frequency_hz in [1.0, 440.0, 725.5, 20000.0, 24000.0] {
        for duration_ms in [1, 2, 10, 75] {
            for volume in [0.0, 0.35, 1.0] {
                let tone = ToneParameters {
                    frequency_hz,
                    duration_ms,
                    volume,
                    shape: ToneShape::default(),
                };
                let old = ToneGenerator::generate(frequency_hz, duration_ms, volume);
                let interruptible = ToneGenerator::generate_interruptible(
                    frequency_hz,
                    duration_ms,
                    volume,
                    &|| false,
                )
                .unwrap();
                let shaped = ToneGenerator::generate_shaped(&tone, &|| false).unwrap();
                assert_eq!(old.samples, interruptible.samples);
                assert_eq!(old.samples, shaped.samples);
            }
        }
    }
}

#[test]
fn envelope_stages_and_compression_have_fixed_endpoints() {
    let normal = Envelope::new(adsr(200, 300, 0.4, 100), 1001, 1000);
    for (frame, expected) in [
        (0, 0.0),
        (100, 0.5),
        (200, 1.0),
        (350, 0.7),
        (500, 0.4),
        (900, 0.4),
        (950, 0.2),
        (1000, 0.0),
    ] {
        assert!(
            (normal.level(frame) - expected).abs() < 0.000001,
            "frame {frame}"
        );
    }
    let compressed = Envelope::new(adsr(10, 20, 0.4, 10), 1001, 10);
    for (frame, expected) in [
        (125, 0.5),
        (250, 1.0),
        (500, 0.7),
        (750, 0.4),
        (875, 0.2),
        (1000, 0.0),
    ] {
        assert!((compressed.level(frame) - expected).abs() < 0.000001);
    }
    let immediate = Envelope::new(adsr(0, 0, 0.3, 0), 1001, 1000);
    assert_eq!(immediate.level(0), 0.0);
    assert_eq!(immediate.level(44), 0.3);
    assert_eq!(immediate.level(500), 0.3);
    assert_eq!(immediate.level(1000), 0.0);
    // Endpoint cap is a minimum, not a second multiplier on the attack.
    assert!((normal.level(22) - 0.11).abs() < 0.000001);
}

#[test]
fn zero_and_short_stages_keep_duration_stereo_bounds_and_silent_endpoints() {
    for waveform in [Waveform::Sine, Waveform::Triangle, Waveform::Square] {
        for envelope in [
            None,
            Some(adsr(0, 0, 1.0, 0)),
            Some(adsr(60000, 60000, 0.25, 60000)),
            Some(adsr(0, 10, 0.0, 0)),
            Some(adsr(10, 0, 0.5, 15)),
        ] {
            for duration_ms in [1, 2, 10, 75, 200] {
                let tone = ToneParameters {
                    duration_ms,
                    ..tone(waveform, envelope)
                };
                let audio = ToneGenerator::generate_shaped(&tone, &|| false).unwrap();
                assert_eq!(
                    audio.frame_count(),
                    SAMPLE_RATE as usize * duration_ms as usize / 1000
                );
                assert!(audio
                    .samples
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 1.0));
                assert_eq!(&audio.samples[..2], &[0.0, 0.0]);
                assert_eq!(&audio.samples[audio.samples.len() - 2..], &[0.0, 0.0]);
                assert!(audio
                    .samples
                    .chunks_exact(2)
                    .any(|frame| frame[0] != frame[1]));
            }
        }
    }
}

fn amplitude(audio: &AudioBuffer, frequency: f64, frames: usize) -> f64 {
    let (mut sine, mut cosine) = (0.0, 0.0);
    for frame in 441..441 + frames {
        let phase = TAU * frequency * frame as f64 / SAMPLE_RATE as f64;
        sine += audio.left(frame) as f64 * phase.sin();
        cosine += audio.left(frame) as f64 * phase.cos();
    }
    2.0 * sine.hypot(cosine) / frames as f64
}

#[test]
fn harmonic_shapes_have_expected_spectra_and_exclude_above_nyquist_partials() {
    for (waveform, ratio) in [
        (Waveform::Sine, 0.0),
        (Waveform::Triangle, 1.0 / 9.0),
        (Waveform::Square, 1.0 / 3.0),
    ] {
        let audio = ToneGenerator::generate_shaped(&tone(waveform, None), &|| false).unwrap();
        let fundamental = amplitude(&audio, 441.0, 1000);
        let third = amplitude(&audio, 1323.0, 1000);
        assert!((third / fundamental - ratio).abs() < 0.00001);
        assert!(amplitude(&audio, 882.0, 1000) / fundamental < 0.00001);
        let high = ToneParameters {
            frequency_hz: 8000.0,
            ..tone(waveform, None)
        };
        let audio = ToneGenerator::generate_shaped(&high, &|| false).unwrap();
        let fundamental = amplitude(&audio, 8000.0, 4410);
        assert!(fundamental > 0.1);
        assert!(amplitude(&audio, 20100.0, 4410) / fundamental < 0.00001);
    }
}

#[test]
fn harmonic_work_stops_at_32_partials_even_at_low_frequencies() {
    for waveform in [Waveform::Triangle, Waveform::Square] {
        let tone = ToneParameters {
            frequency_hz: 44.1,
            ..tone(waveform, None)
        };
        let audio = ToneGenerator::generate_shaped(&tone, &|| false).unwrap();
        assert!(amplitude(&audio, 44.1 * 63.0, 1000) > 0.0001);
        assert!(amplitude(&audio, 44.1 * 65.0, 1000) < 0.000001);
    }
}

#[test]
fn gain_is_linear_without_changing_the_envelope_or_duration() {
    for waveform in [Waveform::Sine, Waveform::Triangle, Waveform::Square] {
        let loud = tone(waveform, Some(adsr(10, 20, 0.3, 15)));
        let quiet = ToneParameters {
            volume: 0.25,
            ..loud
        };
        let zero = ToneParameters {
            volume: 0.0,
            ..loud
        };
        let loud = ToneGenerator::generate_shaped(&loud, &|| false).unwrap();
        let quiet = ToneGenerator::generate_shaped(&quiet, &|| false).unwrap();
        let zero = ToneGenerator::generate_shaped(&zero, &|| false).unwrap();
        assert_eq!(loud.frame_count(), quiet.frame_count());
        for (loud, quiet) in loud.samples.iter().zip(&quiet.samples) {
            assert!((loud * 0.25 - quiet).abs() < 0.000001);
        }
        assert!(zero.samples.iter().all(|sample| *sample == 0.0));
    }
}

#[test]
fn cancellation_discards_classic_and_shaped_pcm_before_during_and_after_generation() {
    for waveform in [Waveform::Sine, Waveform::Triangle, Waveform::Square] {
        for envelope in [None, Some(adsr(1, 0, 1.0, 1))] {
            for cancel_at in [1, 3, 4] {
                // 441 frames: checks at entry, before allocation, at frame 256,
                // and before publishing. Classic delegates its allocation check.
                let tone = ToneParameters {
                    duration_ms: 10,
                    ..tone(waveform, envelope)
                };
                let checks = Cell::new(0);
                let cancelled = || {
                    checks.set(checks.get() + 1);
                    checks.get() >= cancel_at
                };
                assert!(matches!(
                    ToneGenerator::generate_shaped(&tone, &cancelled),
                    Err(AudioError::Cancelled)
                ));
                assert_eq!(checks.get(), cancel_at);
            }
        }
    }
    let checks = Cell::new(0);
    let cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() == 2
    };
    assert!(matches!(
        ToneGenerator::generate_interruptible(440.0, 60000, 1.0, &cancelled),
        Err(AudioError::Cancelled)
    ));
    assert_eq!(checks.get(), 2);
}

#[test]
fn renderer_rejects_invalid_programmatic_values_before_generating() {
    for tone in [
        ToneParameters {
            frequency_hz: f32::NAN,
            ..tone(Waveform::Sine, None)
        },
        ToneParameters {
            duration_ms: 0,
            ..tone(Waveform::Sine, None)
        },
        ToneParameters {
            volume: 1.1,
            ..tone(Waveform::Sine, None)
        },
        tone(Waveform::Square, Some(adsr(60001, 0, 1.0, 0))),
    ] {
        assert!(matches!(
            ToneGenerator::generate_shaped(&tone, &|| false),
            Err(AudioError::InvalidFormat(_))
        ));
    }
}
