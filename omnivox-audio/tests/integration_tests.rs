use omnivox_audio::buffer::{AudioBuffer, CHANNELS, SAMPLE_RATE};
use omnivox_audio::effects::{ChannelRouter, SilenceTrimmer, VolumeAdjust};
use omnivox_audio::loader::AudioFileLoader;
use omnivox_audio::pipeline::{AudioEffect, AudioPipeline};
use omnivox_audio::tone::ToneGenerator;
use omnivox_core::ChannelMode;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helper: generate a minimal WAV file (mono, 16-bit PCM)
// ---------------------------------------------------------------------------

fn write_wav_file(path: &Path, sample_rate: u32, samples_i16: &[i16]) {
    let num_channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate: u32 = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
    let block_align: u16 = num_channels * bits_per_sample / 8;
    let data_size: u32 = samples_i16.len() as u32 * block_align as u32;

    let mut wav: Vec<u8> = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&num_channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples_i16 {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, &wav).expect("failed to write WAV file");
}

fn sine_i16(freq_hz: f32, sample_rate: u32, num_samples: usize) -> Vec<i16> {
    let two_pi = 2.0 * std::f32::consts::PI;
    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            ((two_pi * freq_hz * t).sin() * 32767.0) as i16
        })
        .collect()
}

// ===========================================================================
// 1. End-to-end pipeline tests
// ===========================================================================

#[test]
fn tone_through_full_pipeline() {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(SilenceTrimmer::with_settings(0.01, 0.0)));
    pipeline.push(Box::new(VolumeAdjust::new(0.5)));
    pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Both)));

    let mut buf = ToneGenerator::generate(440.0, 100, 1.0);
    let original_frames = buf.frame_count();

    pipeline.process(&mut buf).unwrap();

    // Buffer should still be stereo f32 @ 44100
    assert_eq!(buf.channels(), CHANNELS);
    assert_eq!(buf.sample_rate(), SAMPLE_RATE);
    // Tone has no leading/trailing silence beyond the fade region,
    // so frame count should remain the same (fades are above threshold)
    // or be very close.
    assert!(buf.frame_count() > 0);
    assert!(buf.frame_count() <= original_frames);
    // Volume was halved
    assert!(buf.peak_amplitude() <= 0.55);
}

#[test]
fn known_buffer_through_effects() {
    // Build a buffer with known content: 0.8 amplitude stereo samples
    let frame_count = 100;
    let mut samples = Vec::with_capacity(frame_count * 2);
    for _ in 0..frame_count {
        samples.push(0.8);
        samples.push(-0.8);
    }
    let mut buf = AudioBuffer::new(samples);

    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(0.5)));
    pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Left)));
    pipeline.process(&mut buf).unwrap();

    // Volume halved: 0.4, -0.4.  Then left routing: right channel zeroed.
    for i in 0..buf.frame_count() {
        assert!((buf.left(i) - 0.4).abs() < 1e-6, "frame {} left", i);
        assert_eq!(buf.right(i), 0.0, "frame {} right", i);
    }
}

// ===========================================================================
// 2. Silence trimming tests
// ===========================================================================

#[test]
fn trim_leading_silence() {
    let trimmer = SilenceTrimmer::with_settings(0.01, 0.0);

    // 500 frames of silence then 100 frames of sound
    let silent_frames = 500;
    let sound_frames = 100;
    let mut samples = vec![0.0; silent_frames * 2];
    for _ in 0..sound_frames {
        samples.push(0.5);
        samples.push(-0.5);
    }
    let mut buf = AudioBuffer::new(samples);
    trimmer.process(&mut buf).unwrap();

    assert_eq!(buf.frame_count(), sound_frames);
    assert!((buf.left(0) - 0.5).abs() < 1e-6);
}

#[test]
fn trim_trailing_silence() {
    let trimmer = SilenceTrimmer::with_settings(0.01, 0.0);

    let sound_frames = 100;
    let silent_frames = 500;
    let mut samples = Vec::new();
    for _ in 0..sound_frames {
        samples.push(0.5);
        samples.push(-0.5);
    }
    samples.extend(vec![0.0; silent_frames * 2]);
    let mut buf = AudioBuffer::new(samples);
    trimmer.process(&mut buf).unwrap();

    assert_eq!(buf.frame_count(), sound_frames);
}

#[test]
fn trim_both_ends() {
    let trimmer = SilenceTrimmer::with_settings(0.01, 0.0);

    let mut samples = vec![0.0; 200]; // 100 silent frames
    for _ in 0..50 {
        samples.push(0.7);
        samples.push(-0.7);
    }
    samples.extend(vec![0.0; 200]); // 100 silent frames
    let mut buf = AudioBuffer::new(samples);
    trimmer.process(&mut buf).unwrap();

    assert_eq!(buf.frame_count(), 50);
    assert!((buf.left(0) - 0.7).abs() < 1e-6);
}

#[test]
fn trim_preserves_minimum_padding() {
    let padding_secs = 0.005;
    let trimmer = SilenceTrimmer::with_settings(0.01, padding_secs);
    let padding_frames = (padding_secs * SAMPLE_RATE as f32) as usize;

    // Plenty of silence on both sides
    let silent = padding_frames + 100;
    let mut samples = vec![0.0; silent * 2];
    samples.push(0.5);
    samples.push(-0.5); // 1 sound frame
    samples.extend(vec![0.0; silent * 2]);
    let mut buf = AudioBuffer::new(samples);
    trimmer.process(&mut buf).unwrap();

    // Should have at least the sound frame + padding on each side
    assert!(
        buf.frame_count() >= 1 + padding_frames,
        "frame_count {} should be >= {}",
        buf.frame_count(),
        1 + padding_frames
    );
}

#[test]
fn trim_does_not_over_trim() {
    // Use a very high threshold; everything below 0.9 is "silence"
    let trimmer = SilenceTrimmer::with_settings(0.9, 0.0);

    let mut samples = Vec::new();
    for _ in 0..10 {
        samples.push(0.1);
        samples.push(0.1); // below threshold
    }
    samples.push(1.0);
    samples.push(1.0); // above threshold
    for _ in 0..10 {
        samples.push(0.1);
        samples.push(0.1); // below threshold
    }
    let mut buf = AudioBuffer::new(samples);
    trimmer.process(&mut buf).unwrap();

    // Only the loud frame should survive
    assert_eq!(buf.frame_count(), 1);
    assert!((buf.left(0) - 1.0).abs() < 1e-6);
}

#[test]
fn trim_with_different_thresholds() {
    // Low threshold: most stuff survives
    let low = SilenceTrimmer::with_settings(0.001, 0.0);
    let mut buf1 = AudioBuffer::new(vec![0.005, 0.005, 0.5, -0.5, 0.005, 0.005]);
    low.process(&mut buf1).unwrap();
    // 0.005 > 0.001, so all frames survive
    assert_eq!(buf1.frame_count(), 3);

    // High threshold: only loud stuff survives
    let high = SilenceTrimmer::with_settings(0.1, 0.0);
    let mut buf2 = AudioBuffer::new(vec![0.005, 0.005, 0.5, -0.5, 0.005, 0.005]);
    high.process(&mut buf2).unwrap();
    assert_eq!(buf2.frame_count(), 1);
}

// ===========================================================================
// 3. Effect chain tests
// ===========================================================================

#[test]
fn volume_halves_samples() {
    let vol = VolumeAdjust::new(0.5);
    let mut buf = AudioBuffer::new(vec![1.0, -1.0, 0.6, -0.4]);
    vol.process(&mut buf).unwrap();

    assert!((buf.samples[0] - 0.5).abs() < 1e-6);
    assert!((buf.samples[1] - (-0.5)).abs() < 1e-6);
    assert!((buf.samples[2] - 0.3).abs() < 1e-6);
    assert!((buf.samples[3] - (-0.2)).abs() < 1e-6);
}

#[test]
fn channel_left_zeros_right() {
    let router = ChannelRouter::new(ChannelMode::Left);
    let mut buf = AudioBuffer::new(vec![0.5, 0.3, 0.7, 0.9]);
    router.process(&mut buf).unwrap();

    assert_eq!(buf.left(0), 0.5);
    assert_eq!(buf.right(0), 0.0);
    assert_eq!(buf.left(1), 0.7);
    assert_eq!(buf.right(1), 0.0);
}

#[test]
fn channel_right_zeros_left() {
    let router = ChannelRouter::new(ChannelMode::Right);
    let mut buf = AudioBuffer::new(vec![0.5, 0.3, 0.7, 0.9]);
    router.process(&mut buf).unwrap();

    assert_eq!(buf.left(0), 0.0);
    assert_eq!(buf.right(0), 0.3);
    assert_eq!(buf.left(1), 0.0);
    assert_eq!(buf.right(1), 0.9);
}

#[test]
fn multiple_effects_applied_in_order() {
    let mut pipeline = AudioPipeline::new();
    // First: volume at 0.5, second: route to left only
    pipeline.push(Box::new(VolumeAdjust::new(0.5)));
    pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Left)));

    let mut buf = AudioBuffer::new(vec![0.8, -0.6, 0.4, -0.2]);
    pipeline.process(&mut buf).unwrap();

    // After volume: [0.4, -0.3, 0.2, -0.1]
    // After left routing: right zeroed
    assert!((buf.left(0) - 0.4).abs() < 1e-6);
    assert_eq!(buf.right(0), 0.0);
    assert!((buf.left(1) - 0.2).abs() < 1e-6);
    assert_eq!(buf.right(1), 0.0);
}

#[test]
fn effect_order_matters_trim_then_volume_vs_volume_then_trim() {
    // Trim then volume: silence removed first, then volume applied to remaining
    let mut pipeline_tv = AudioPipeline::new();
    pipeline_tv.push(Box::new(SilenceTrimmer::with_settings(0.01, 0.0)));
    pipeline_tv.push(Box::new(VolumeAdjust::new(2.0)));

    let mut samples = vec![0.0; 20]; // silence
    samples.extend_from_slice(&[0.3, -0.3]); // 1 sound frame
    let mut buf_tv = AudioBuffer::new(samples);
    pipeline_tv.process(&mut buf_tv).unwrap();

    assert_eq!(buf_tv.frame_count(), 1);
    assert!((buf_tv.left(0) - 0.6).abs() < 1e-6);

    // Volume then trim: volume applied to everything first
    let mut pipeline_vt = AudioPipeline::new();
    pipeline_vt.push(Box::new(VolumeAdjust::new(2.0)));
    pipeline_vt.push(Box::new(SilenceTrimmer::with_settings(0.01, 0.0)));

    let mut samples2 = vec![0.0; 20];
    samples2.extend_from_slice(&[0.3, -0.3]);
    let mut buf_vt = AudioBuffer::new(samples2);
    pipeline_vt.process(&mut buf_vt).unwrap();

    // Volume doubles the sound frame to [0.6, -0.6], silence stays at 0.0
    // Trim removes leading silence, leaves [0.6, -0.6]
    assert_eq!(buf_vt.frame_count(), 1);
    assert!((buf_vt.left(0) - 0.6).abs() < 1e-6);
}

#[test]
fn three_effects_in_chain() {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(SilenceTrimmer::with_settings(0.01, 0.0)));
    pipeline.push(Box::new(VolumeAdjust::new(0.5)));
    pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Right)));

    // Leading silence, 2 sound frames, trailing silence
    let mut samples = vec![0.0; 10];
    samples.extend_from_slice(&[0.8, -0.6, 0.4, -0.2]);
    samples.extend(vec![0.0; 10]);
    let mut buf = AudioBuffer::new(samples);
    pipeline.process(&mut buf).unwrap();

    // After trim: [0.8, -0.6, 0.4, -0.2]
    // After volume 0.5: [0.4, -0.3, 0.2, -0.1]
    // After right routing: left zeroed
    assert_eq!(buf.frame_count(), 2);
    assert_eq!(buf.left(0), 0.0);
    assert!((buf.right(0) - (-0.3)).abs() < 1e-6);
    assert_eq!(buf.left(1), 0.0);
    assert!((buf.right(1) - (-0.1)).abs() < 1e-6);
}

// ===========================================================================
// 4. Tone accuracy tests
// ===========================================================================

#[test]
fn tone_440hz_frequency_via_zero_crossings() {
    let buf = ToneGenerator::generate(440.0, 200, 1.0);
    let fade_frames = (SAMPLE_RATE as f32 * 0.010) as usize;

    let start = fade_frames;
    let end = buf.frame_count() - fade_frames;
    assert!(end > start, "tone too short for frequency analysis");

    let left: Vec<f32> = (start..end).map(|i| buf.left(i)).collect();
    let mut crossings = 0u32;
    for i in 1..left.len() {
        if left[i - 1].signum() != left[i].signum() {
            crossings += 1;
        }
    }

    let duration = (end - start) as f32 / SAMPLE_RATE as f32;
    let measured_freq = crossings as f32 / 2.0 / duration;

    assert!(
        (measured_freq - 440.0).abs() < 5.0,
        "measured {}Hz, expected ~440Hz",
        measured_freq
    );
}

#[test]
fn tone_1000hz_frequency_via_zero_crossings() {
    let buf = ToneGenerator::generate(1000.0, 200, 1.0);
    let fade_frames = (SAMPLE_RATE as f32 * 0.010) as usize;

    let start = fade_frames;
    let end = buf.frame_count() - fade_frames;
    let left: Vec<f32> = (start..end).map(|i| buf.left(i)).collect();

    let mut crossings = 0u32;
    for i in 1..left.len() {
        if left[i - 1].signum() != left[i].signum() {
            crossings += 1;
        }
    }

    let duration = (end - start) as f32 / SAMPLE_RATE as f32;
    let measured_freq = crossings as f32 / 2.0 / duration;

    assert!(
        (measured_freq - 1000.0).abs() < 10.0,
        "measured {}Hz, expected ~1000Hz",
        measured_freq
    );
}

#[test]
fn tone_duration_matches_expected_sample_count() {
    for &dur_ms in &[10, 50, 100, 250, 500] {
        let buf = ToneGenerator::generate(440.0, dur_ms, 1.0);
        let expected_frames = (SAMPLE_RATE as f64 * dur_ms as f64 / 1000.0) as usize;
        assert_eq!(
            buf.frame_count(),
            expected_frames,
            "duration {}ms: expected {} frames, got {}",
            dur_ms,
            expected_frames,
            buf.frame_count()
        );
        assert_eq!(buf.samples.len(), expected_frames * 2);
    }
}

#[test]
fn tone_stereo_output_and_spatial_delay() {
    let buf = ToneGenerator::generate(1000.0, 100, 1.0);
    let fade_frames = (SAMPLE_RATE as f32 * 0.010) as usize;

    // Verify stereo format
    assert_eq!(buf.samples.len() % 2, 0);
    assert_eq!(buf.channels(), CHANNELS);

    // In the steady-state region, left and right should differ slightly
    // due to the sub-sample delay
    let mid = fade_frames + (buf.frame_count() - 2 * fade_frames) / 2;
    let left = buf.left(mid);
    let right = buf.right(mid);

    // They differ but are close (sub-sample delay is very small)
    let diff = (left - right).abs();
    assert!(
        diff < 0.1,
        "left={}, right={}, diff={} should be small",
        left,
        right,
        diff
    );
    // For a 1000Hz tone, the delay is more noticeable than for lower freqs
    // At 1000Hz, 0.01ms delay = 0.01 * 1000 / 1000 = 0.01 cycles phase shift
    // This is detectable but small
}

// ===========================================================================
// 5. Audio format tests
// ===========================================================================

#[test]
fn all_buffers_stereo_f32_44100() {
    // Tone generator output
    let tone = ToneGenerator::generate(440.0, 100, 1.0);
    assert_eq!(tone.sample_rate(), 44100);
    assert_eq!(tone.channels(), 2);
    assert_eq!(tone.samples.len() % 2, 0);

    // Silence buffer
    let silence = AudioBuffer::silence(0.1);
    assert_eq!(silence.sample_rate(), 44100);
    assert_eq!(silence.channels(), 2);

    // After pipeline processing
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(0.5)));
    let mut processed = tone.clone();
    pipeline.process(&mut processed).unwrap();
    assert_eq!(processed.sample_rate(), 44100);
    assert_eq!(processed.channels(), 2);
    assert_eq!(processed.samples.len() % 2, 0);
}

#[test]
fn wav_file_loads_as_stereo_44100() {
    let tmp = std::env::temp_dir().join("omnivox_integ_format.wav");
    let samples = sine_i16(440.0, 44100, 1000);
    write_wav_file(&tmp, 44100, &samples);

    let loader = AudioFileLoader::new();
    let buf = loader.load(&tmp).expect("should load WAV");

    assert_eq!(buf.sample_rate(), 44100);
    assert_eq!(buf.channels(), 2);
    assert_eq!(buf.samples.len() % 2, 0);
    // Mono source -> stereo: L == R
    for i in 0..buf.frame_count() {
        assert_eq!(buf.left(i), buf.right(i), "mono->stereo mismatch at frame {}", i);
    }

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn resampled_wav_is_stereo_44100() {
    // Write a WAV at 22050Hz, loader should resample to 44100
    let tmp = std::env::temp_dir().join("omnivox_integ_resample.wav");
    let samples = sine_i16(440.0, 22050, 500);
    write_wav_file(&tmp, 22050, &samples);

    let loader = AudioFileLoader::new();
    let buf = loader.load(&tmp).expect("should load and resample WAV");

    assert_eq!(buf.sample_rate(), 44100);
    assert_eq!(buf.channels(), 2);
    assert_eq!(buf.samples.len() % 2, 0);
    // Resampled from 500 frames at 22050 -> ~1000 frames at 44100
    assert!(
        buf.frame_count() >= 990 && buf.frame_count() <= 1010,
        "expected ~1000 frames, got {}",
        buf.frame_count()
    );

    let _ = std::fs::remove_file(&tmp);
}

// ===========================================================================
// 6. Error handling tests
// ===========================================================================

#[test]
fn load_nonexistent_file_returns_file_not_found() {
    let loader = AudioFileLoader::new();
    let result = loader.load(Path::new("/nonexistent/path/audio.wav"));
    assert!(result.is_err());
    match result.unwrap_err() {
        omnivox_audio::AudioError::FileNotFound(msg) => {
            assert!(msg.contains("nonexistent"), "error should contain path: {}", msg);
        }
        other => panic!("expected FileNotFound, got: {:?}", other),
    }
}

#[test]
fn load_invalid_file_returns_decode_error() {
    let tmp = std::env::temp_dir().join("omnivox_integ_corrupt.wav");
    // Write garbage data that is not a valid audio file
    std::fs::write(&tmp, b"this is not a wav file at all").expect("write garbage file");

    let loader = AudioFileLoader::new();
    let result = loader.load(&tmp);

    let _ = std::fs::remove_file(&tmp);

    assert!(result.is_err());
    match result.unwrap_err() {
        omnivox_audio::AudioError::DecodeError(_) => {}
        other => panic!("expected DecodeError, got: {:?}", other),
    }
}

#[test]
fn empty_buffer_through_effects() {
    let mut buf = AudioBuffer::empty();

    // All effects should handle empty buffers gracefully
    let trimmer = SilenceTrimmer::new();
    trimmer.process(&mut buf).unwrap();
    assert!(buf.is_empty());

    let vol = VolumeAdjust::new(2.0);
    vol.process(&mut buf).unwrap();
    assert!(buf.is_empty());

    let router = ChannelRouter::new(ChannelMode::Left);
    router.process(&mut buf).unwrap();
    assert!(buf.is_empty());
}

#[test]
fn empty_buffer_through_pipeline() {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(SilenceTrimmer::new()));
    pipeline.push(Box::new(VolumeAdjust::new(0.5)));
    pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Right)));

    let mut buf = AudioBuffer::empty();
    pipeline.process(&mut buf).unwrap();
    assert!(buf.is_empty());
}

#[test]
fn zero_duration_tone_is_empty() {
    let buf = ToneGenerator::generate(440.0, 0, 1.0);
    assert!(buf.is_empty());
    assert_eq!(buf.frame_count(), 0);
}

// ===========================================================================
// Additional integration scenarios
// ===========================================================================

#[test]
fn tone_then_trim_preserves_content() {
    // A tone should have fade-in/fade-out but no silence beyond that.
    // Trimming with a low threshold should leave it mostly intact.
    let buf_original = ToneGenerator::generate(440.0, 100, 1.0);
    let original_frames = buf_original.frame_count();

    let mut buf = buf_original.clone();
    let trimmer = SilenceTrimmer::with_settings(0.001, 0.0);
    trimmer.process(&mut buf).unwrap();

    // The tone fades in/out linearly over 10ms = 441 frames.
    // The first and last few samples are extremely close to zero.
    // Trimmer may remove a handful of frames but the vast majority remain.
    assert!(
        buf.frame_count() as f32 / original_frames as f32 > 0.9,
        "trimmed {} of {} frames -- too much removed",
        original_frames - buf.frame_count(),
        original_frames
    );
}

#[test]
fn pipeline_with_loaded_wav_file() {
    let tmp = std::env::temp_dir().join("omnivox_integ_pipeline_wav.wav");
    let raw = sine_i16(440.0, 44100, 4410); // 0.1s of audio
    write_wav_file(&tmp, 44100, &raw);

    let loader = AudioFileLoader::new();
    let mut buf = loader.load(&tmp).expect("should load WAV");
    let _ = std::fs::remove_file(&tmp);

    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(0.5)));
    pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Both)));

    let original_peak = buf.peak_amplitude();
    pipeline.process(&mut buf).unwrap();

    // Volume halved
    assert!(
        buf.peak_amplitude() <= original_peak * 0.55,
        "peak {} should be roughly half of original {}",
        buf.peak_amplitude(),
        original_peak
    );
    // Still stereo 44100
    assert_eq!(buf.sample_rate(), 44100);
    assert_eq!(buf.channels(), 2);
}

#[test]
fn volume_clamps_output() {
    let vol = VolumeAdjust::new(5.0);
    let mut buf = AudioBuffer::new(vec![0.5, -0.5, 0.3, -0.3]);
    vol.process(&mut buf).unwrap();

    // 0.5 * 5.0 = 2.5 -> clamped to 1.0
    assert_eq!(buf.samples[0], 1.0);
    assert_eq!(buf.samples[1], -1.0);
    // 0.3 * 5.0 = 1.5 -> clamped to 1.0
    assert_eq!(buf.samples[2], 1.0);
    assert_eq!(buf.samples[3], -1.0);
}

#[test]
fn append_buffers_then_process() {
    let mut buf1 = AudioBuffer::silence(0.01); // 441 frames of silence
    let tone = ToneGenerator::generate(440.0, 10, 1.0);
    let tone_frames = tone.frame_count();
    buf1.append(&tone);

    let trimmer = SilenceTrimmer::with_settings(0.001, 0.0);
    trimmer.process(&mut buf1).unwrap();

    // Most of the silence should be removed; tone content preserved.
    // The tone has fade-in/out so a few frames near zero might also be trimmed.
    assert!(
        buf1.frame_count() >= tone_frames / 2,
        "expected most tone frames preserved, got {} of {}",
        buf1.frame_count(),
        tone_frames
    );
}

#[test]
fn cache_returns_same_content() {
    let tmp = std::env::temp_dir().join("omnivox_integ_cache.wav");
    let raw = sine_i16(440.0, 44100, 100);
    write_wav_file(&tmp, 44100, &raw);

    let loader = AudioFileLoader::with_cache();
    let buf1 = loader.load(&tmp).unwrap();
    let buf2 = loader.load(&tmp).unwrap();

    let _ = std::fs::remove_file(&tmp);

    assert_eq!(buf1.samples.len(), buf2.samples.len());
    for (a, b) in buf1.samples.iter().zip(buf2.samples.iter()) {
        assert_eq!(a, b);
    }
}
