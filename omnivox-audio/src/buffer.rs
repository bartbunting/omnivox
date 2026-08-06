//! Audio Buffer
//!
//! The canonical audio buffer format for omnivox: stereo f32 at 44100Hz.
//! All audio sources produce this format, and all consumers expect it.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::AudioError;

/// Standard sample rate for all omnivox audio
pub const SAMPLE_RATE: u32 = 44100;

/// Standard channel count (stereo)
pub const CHANNELS: u16 = 2;

/// Audio buffer containing interleaved stereo f32 PCM samples at 44100Hz.
///
/// This is the universal format used throughout the omnivox audio pipeline.
/// All sources (TTS, tone generator, file loader) produce AudioBuffers,
/// and all consumers (effects, output) operate on them.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Interleaved stereo PCM samples (f32, range -1.0 to 1.0).
    /// Layout: [L0, R0, L1, R1, L2, R2, ...]
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    /// Create a new AudioBuffer from interleaved stereo samples.
    ///
    /// Panics if the number of samples is odd (not a multiple of 2).
    pub fn new(samples: Vec<f32>) -> Self {
        assert!(
            samples.len().is_multiple_of(CHANNELS as usize),
            "Sample count must be even for stereo interleaved data"
        );
        Self { samples }
    }

    /// Convert interleaved mono or stereo PCM into the canonical format.
    pub fn try_from_interleaved_f32(
        samples: Vec<f32>,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, AudioError> {
        validate_source_format(samples.len(), sample_rate, channels)?;
        if samples.is_empty() {
            return Ok(Self::empty());
        }

        let samples = if sample_rate == SAMPLE_RATE {
            samples
        } else {
            resample_interleaved(samples, sample_rate, channels)?
        };
        if channels == CHANNELS {
            return Ok(Self::new(samples));
        }

        let mut stereo = Vec::with_capacity(samples.len() * CHANNELS as usize);
        for sample in samples {
            stereo.push(sample);
            stereo.push(sample);
        }
        Ok(Self::new(stereo))
    }

    /// Convert signed 16-bit interleaved mono or stereo PCM into the canonical format.
    pub fn try_from_interleaved_i16(
        samples: &[i16],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, AudioError> {
        let samples = samples
            .iter()
            .map(|sample| f32::from(*sample) / 32768.0)
            .collect();
        Self::try_from_interleaved_f32(samples, sample_rate, channels)
    }

    /// Create an empty AudioBuffer.
    pub fn empty() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    /// Create a silent AudioBuffer of the given duration in seconds.
    pub fn silence(duration_secs: f32) -> Self {
        let frame_count = (duration_secs * SAMPLE_RATE as f32) as usize;
        let sample_count = frame_count * CHANNELS as usize;
        Self {
            samples: vec![0.0; sample_count],
        }
    }

    /// Get the sample rate (always 44100).
    pub fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    /// Get the channel count (always 2).
    pub fn channels(&self) -> u16 {
        CHANNELS
    }

    /// Get duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        self.frame_count() as f32 / SAMPLE_RATE as f32
    }

    /// Get duration in seconds.
    pub fn duration(&self) -> f32 {
        self.duration_secs()
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Get number of frames (each frame is one sample per channel).
    pub fn frame_count(&self) -> usize {
        self.samples.len() / CHANNELS as usize
    }

    /// Get the left channel sample at the given frame index.
    pub fn left(&self, frame: usize) -> f32 {
        self.samples[frame * 2]
    }

    /// Get the right channel sample at the given frame index.
    pub fn right(&self, frame: usize) -> f32 {
        self.samples[frame * 2 + 1]
    }

    /// Set the left channel sample at the given frame index.
    pub fn set_left(&mut self, frame: usize, value: f32) {
        self.samples[frame * 2] = value;
    }

    /// Set the right channel sample at the given frame index.
    pub fn set_right(&mut self, frame: usize, value: f32) {
        self.samples[frame * 2 + 1] = value;
    }

    /// Append another AudioBuffer to this one.
    pub fn append(&mut self, other: &AudioBuffer) {
        self.samples.extend_from_slice(&other.samples);
    }

    /// Clamp all samples to the range [-1.0, 1.0].
    pub fn clamp(&mut self) {
        for sample in &mut self.samples {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    /// Get the peak absolute amplitude across all samples.
    pub fn peak_amplitude(&self) -> f32 {
        self.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
    }
}

fn validate_source_format(
    sample_count: usize,
    sample_rate: u32,
    channels: u16,
) -> Result<(), AudioError> {
    if sample_rate == 0 {
        return Err(AudioError::InvalidFormat(
            "sample rate must be greater than zero".to_owned(),
        ));
    }
    if channels != 1 && channels != CHANNELS {
        return Err(AudioError::InvalidFormat(format!(
            "only mono or stereo PCM is supported, received {channels} channels"
        )));
    }
    if !sample_count.is_multiple_of(channels as usize) {
        return Err(AudioError::InvalidFormat(format!(
            "sample count {sample_count} is not aligned to {channels} channels"
        )));
    }
    Ok(())
}

fn resample_interleaved(
    samples: Vec<f32>,
    source_rate: u32,
    channels: u16,
) -> Result<Vec<f32>, AudioError> {
    let channel_count = channels as usize;
    let frame_count = samples.len() / channel_count;
    if frame_count < 256 {
        return Ok(linear_resample_interleaved(
            &samples,
            source_rate,
            channel_count,
        ));
    }
    let parameters = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(
        SAMPLE_RATE as f64 / source_rate as f64,
        2.0,
        parameters,
        frame_count,
        channel_count,
    )
    .map_err(|error| AudioError::InvalidFormat(format!("could not create resampler: {error}")))?;

    let mut channel_data = vec![Vec::with_capacity(frame_count); channel_count];
    for (index, sample) in samples.into_iter().enumerate() {
        channel_data[index % channel_count].push(sample);
    }
    let output_channels = resampler
        .process(&channel_data, None)
        .map_err(|error| AudioError::InvalidFormat(format!("could not resample PCM: {error}")))?;
    let output_frames = output_channels.first().map_or(0, Vec::len);
    let mut interleaved = Vec::with_capacity(output_frames * channel_count);
    for frame in 0..output_frames {
        for channel in &output_channels {
            interleaved.push(channel[frame]);
        }
    }
    Ok(interleaved)
}

fn linear_resample_interleaved(
    samples: &[f32],
    source_rate: u32,
    channel_count: usize,
) -> Vec<f32> {
    let source_frames = samples.len() / channel_count;
    let target_frames = ((source_frames as u128 * u128::from(SAMPLE_RATE)
        + u128::from(source_rate / 2))
        / u128::from(source_rate)) as usize;
    let mut output = Vec::with_capacity(target_frames * channel_count);
    for target_frame in 0..target_frames {
        let source_position = target_frame as f64 * source_rate as f64 / SAMPLE_RATE as f64;
        let left_frame = (source_position.floor() as usize).min(source_frames - 1);
        let right_frame = (left_frame + 1).min(source_frames - 1);
        let fraction = (source_position - left_frame as f64) as f32;
        for channel in 0..channel_count {
            let left = samples[left_frame * channel_count + channel];
            let right = samples[right_frame * channel_count + channel];
            output.push(left + (right - left) * fraction);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer() {
        let buf = AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2]);
        assert_eq!(buf.frame_count(), 2);
        assert_eq!(buf.samples.len(), 4);
        assert_eq!(buf.sample_rate(), SAMPLE_RATE);
        assert_eq!(buf.channels(), CHANNELS);
    }

    #[test]
    #[should_panic(expected = "Sample count must be even")]
    fn test_new_buffer_odd_samples_panics() {
        AudioBuffer::new(vec![0.1, -0.1, 0.2]);
    }

    #[test]
    fn test_empty_buffer() {
        let buf = AudioBuffer::empty();
        assert!(buf.is_empty());
        assert_eq!(buf.frame_count(), 0);
        assert_eq!(buf.duration_secs(), 0.0);
    }

    #[test]
    fn test_silence() {
        let buf = AudioBuffer::silence(1.0);
        assert_eq!(buf.frame_count(), SAMPLE_RATE as usize);
        assert_eq!(buf.samples.len(), SAMPLE_RATE as usize * 2);
        assert!(buf.samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_duration() {
        let buf = AudioBuffer::silence(0.5);
        let duration = buf.duration_secs();
        assert!((duration - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_left_right_access() {
        let buf = AudioBuffer::new(vec![0.5, -0.5, 0.3, -0.3]);
        assert_eq!(buf.left(0), 0.5);
        assert_eq!(buf.right(0), -0.5);
        assert_eq!(buf.left(1), 0.3);
        assert_eq!(buf.right(1), -0.3);
    }

    #[test]
    fn test_set_left_right() {
        let mut buf = AudioBuffer::new(vec![0.0, 0.0, 0.0, 0.0]);
        buf.set_left(0, 0.7);
        buf.set_right(0, -0.7);
        buf.set_left(1, 0.9);
        buf.set_right(1, -0.9);
        assert_eq!(buf.left(0), 0.7);
        assert_eq!(buf.right(0), -0.7);
        assert_eq!(buf.left(1), 0.9);
        assert_eq!(buf.right(1), -0.9);
    }

    #[test]
    fn test_append() {
        let mut buf1 = AudioBuffer::new(vec![0.1, 0.2]);
        let buf2 = AudioBuffer::new(vec![0.3, 0.4]);
        buf1.append(&buf2);
        assert_eq!(buf1.frame_count(), 2);
        assert_eq!(buf1.samples, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn test_clamp() {
        let mut buf = AudioBuffer::new(vec![1.5, -1.5, 0.5, -0.5]);
        buf.clamp();
        assert_eq!(buf.samples, vec![1.0, -1.0, 0.5, -0.5]);
    }

    #[test]
    fn test_peak_amplitude() {
        let buf = AudioBuffer::new(vec![0.3, -0.7, 0.5, -0.1]);
        assert_eq!(buf.peak_amplitude(), 0.7);
    }

    #[test]
    fn test_peak_amplitude_empty() {
        let buf = AudioBuffer::empty();
        assert_eq!(buf.peak_amplitude(), 0.0);
    }

    #[test]
    fn converts_native_i16_mono_to_canonical_stereo() {
        let buffer =
            AudioBuffer::try_from_interleaved_i16(&[0, 16384, -16384, 32767], SAMPLE_RATE, 1)
                .unwrap();

        assert_eq!(buffer.frame_count(), 4);
        assert_eq!(buffer.samples[0..4], [0.0, 0.0, 0.5, 0.5]);
        assert!((buffer.samples[4] + 0.5).abs() < 0.001);
        assert!((buffer.samples[6] - (32767.0 / 32768.0)).abs() < 0.001);
    }

    #[test]
    fn converts_and_resamples_native_mono() {
        let samples = (0..4410).map(|index| (index as f32 * 0.1).sin()).collect();
        let buffer = AudioBuffer::try_from_interleaved_f32(samples, 22_050, 1).unwrap();

        assert!(buffer.frame_count() >= 8000);
        assert!(buffer.frame_count() <= 9200);
        assert_eq!(buffer.samples.len() % CHANNELS as usize, 0);
    }

    #[test]
    fn rejects_invalid_native_formats() {
        assert!(AudioBuffer::try_from_interleaved_f32(vec![0.0], 0, 1).is_err());
        assert!(AudioBuffer::try_from_interleaved_f32(vec![0.0; 3], SAMPLE_RATE, 2).is_err());
        assert!(AudioBuffer::try_from_interleaved_f32(vec![0.0; 3], SAMPLE_RATE, 3).is_err());
    }

    #[test]
    fn tiny_native_buffers_do_not_disappear_during_resampling() {
        let buffer = AudioBuffer::try_from_interleaved_i16(
            &[-32_768, 0, 16_384, 32_767],
            22_050,
            1,
        )
        .unwrap();

        assert_eq!(buffer.frame_count(), 8);
        assert_eq!(buffer.samples.len(), 16);
    }
}
