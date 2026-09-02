//! Stateful conversion of bounded native PCM windows into Omnivox audio.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::buffer::{CHANNELS, SAMPLE_RATE};
use crate::{AudioBuffer, AudioError};

/// Native input frames retained between progressive conversion calls.
const INPUT_WINDOW_FRAMES: usize = 512;
const SINC_LENGTH: usize = 256;

/// Stateful, bounded conversion from native mono/stereo PCM to Omnivox's
/// 44.1 kHz stereo playback format.
///
/// Input callback boundaries are deliberately invisible to the resampler. A
/// fixed native-frame window feeds one continuous sinc filter, and at most one
/// incomplete input window is retained between calls.
pub struct ProgressivePcmCanonicalizer {
    source_sample_rate: u32,
    source_channels: u16,
    resampler: Option<SincFixedIn<f32>>,
    pending_channels: Vec<Vec<f32>>,
    output_delay_remaining: usize,
    source_frames: u64,
    output_frames: u64,
    finished: bool,
}

impl ProgressivePcmCanonicalizer {
    /// Create a converter for one invariant native PCM format.
    pub fn new(source_sample_rate: u32, source_channels: u16) -> Result<Self, AudioError> {
        validate_format(source_sample_rate, source_channels)?;
        let resampler = if source_sample_rate == SAMPLE_RATE {
            None
        } else {
            Some(
                SincFixedIn::<f32>::new(
                    SAMPLE_RATE as f64 / source_sample_rate as f64,
                    2.0,
                    sinc_parameters(),
                    INPUT_WINDOW_FRAMES,
                    source_channels as usize,
                )
                .map_err(|error| {
                    AudioError::InvalidFormat(format!(
                        "could not create progressive resampler: {error}"
                    ))
                })?,
            )
        };
        let output_delay_remaining = resampler.as_ref().map_or(0, Resampler::output_delay);
        Ok(Self {
            source_sample_rate,
            source_channels,
            resampler,
            pending_channels: vec![Vec::new(); source_channels as usize],
            output_delay_remaining,
            source_frames: 0,
            output_frames: 0,
            finished: false,
        })
    }

    /// Convert another non-final interleaved floating-point PCM window.
    pub fn push_interleaved_f32(
        &mut self,
        samples: &[f32],
    ) -> Result<Vec<AudioBuffer>, AudioError> {
        self.ensure_active()?;
        if samples.is_empty() {
            return Ok(Vec::new());
        }
        let channel_count = self.source_channels as usize;
        if !samples.len().is_multiple_of(channel_count) {
            return Err(AudioError::InvalidFormat(format!(
                "progressive sample count {} is not aligned to {} channels",
                samples.len(),
                self.source_channels
            )));
        }
        let frame_count = samples.len() / channel_count;
        self.source_frames = self
            .source_frames
            .checked_add(frame_count as u64)
            .ok_or_else(|| AudioError::InvalidFormat("native frame count overflowed".to_owned()))?;

        if self.resampler.is_none() {
            let buffer = AudioBuffer::try_from_interleaved_f32(
                samples.to_vec(),
                self.source_sample_rate,
                self.source_channels,
            )?;
            self.output_frames = self
                .output_frames
                .checked_add(buffer.frame_count() as u64)
                .ok_or_else(|| {
                    AudioError::InvalidFormat("canonical frame count overflowed".to_owned())
                })?;
            return Ok(vec![buffer]);
        }

        for frame in samples.chunks_exact(channel_count) {
            for (channel, sample) in self.pending_channels.iter_mut().zip(frame) {
                channel.push(*sample);
            }
        }
        self.process_complete_windows()
    }

    /// Convert another non-final interleaved signed 16-bit PCM window.
    pub fn push_interleaved_i16(
        &mut self,
        samples: &[i16],
    ) -> Result<Vec<AudioBuffer>, AudioError> {
        let samples = samples
            .iter()
            .map(|sample| f32::from(*sample) / 32768.0)
            .collect::<Vec<_>>();
        self.push_interleaved_f32(&samples)
    }

    /// Flush the filter and return the final canonical windows.
    pub fn finish(&mut self) -> Result<Vec<AudioBuffer>, AudioError> {
        self.ensure_active()?;
        self.finished = true;
        if self.resampler.is_none() || self.source_frames == 0 {
            return Ok(Vec::new());
        }

        let target_frames = self.canonical_frame_offset(self.source_frames)?;
        if self.output_frames > target_frames {
            return Err(AudioError::InvalidFormat(format!(
                "progressive resampler emitted {} frames for a {target_frames}-frame result",
                self.output_frames
            )));
        }

        let mut windows = Vec::new();
        if !self.pending_channels[0].is_empty() {
            let output = self.process_partial_input()?;
            self.collect_output(output, Some(target_frames), &mut windows)?;
            for channel in &mut self.pending_channels {
                channel.clear();
            }
        }

        // One zero-padded input window is normally sufficient to release the
        // sinc delay. Keep the loop bounded defensively for extreme accepted
        // sample-rate ratios.
        for _ in 0..4 {
            if self.output_frames >= target_frames {
                break;
            }
            let output = self.process_filter_tail()?;
            let before = self.output_frames;
            self.collect_output(output, Some(target_frames), &mut windows)?;
            if self.output_frames == before {
                break;
            }
        }
        if self.output_frames != target_frames {
            return Err(AudioError::InvalidFormat(format!(
                "progressive resampler completed with {} of {target_frames} frames",
                self.output_frames
            )));
        }
        Ok(windows)
    }

    /// Map a native frame boundary into the canonical playback clock.
    pub fn canonical_frame_offset(&self, source_frame: u64) -> Result<u64, AudioError> {
        let numerator = u128::from(source_frame)
            .checked_mul(u128::from(SAMPLE_RATE))
            .and_then(|value| value.checked_add(u128::from(self.source_sample_rate / 2)))
            .ok_or_else(|| AudioError::InvalidFormat("frame conversion overflowed".to_owned()))?;
        u64::try_from(numerator / u128::from(self.source_sample_rate))
            .map_err(|_| AudioError::InvalidFormat("frame conversion overflowed".to_owned()))
    }

    /// Number of native frames accepted so far.
    pub fn source_frames(&self) -> u64 {
        self.source_frames
    }

    /// Number of canonical frames emitted so far.
    pub fn output_frames(&self) -> u64 {
        self.output_frames
    }

    fn process_complete_windows(&mut self) -> Result<Vec<AudioBuffer>, AudioError> {
        let mut windows = Vec::new();
        let mut consumed = 0;
        while self.pending_channels[0].len() - consumed >= INPUT_WINDOW_FRAMES {
            let input = self
                .pending_channels
                .iter()
                .map(|channel| &channel[consumed..consumed + INPUT_WINDOW_FRAMES])
                .collect::<Vec<_>>();
            let output = self
                .resampler
                .as_mut()
                .expect("non-native sample rate has a resampler")
                .process(&input, None)
                .map_err(resampling_error)?;
            consumed += INPUT_WINDOW_FRAMES;
            self.collect_output(output, None, &mut windows)?;
        }
        if consumed > 0 {
            for channel in &mut self.pending_channels {
                channel.drain(..consumed);
            }
        }
        Ok(windows)
    }

    fn process_partial_input(&mut self) -> Result<Vec<Vec<f32>>, AudioError> {
        let input = self
            .pending_channels
            .iter()
            .map(Vec::as_slice)
            .collect::<Vec<_>>();
        self.resampler
            .as_mut()
            .expect("non-native sample rate has a resampler")
            .process_partial(Some(&input), None)
            .map_err(resampling_error)
    }

    fn process_filter_tail(&mut self) -> Result<Vec<Vec<f32>>, AudioError> {
        self.resampler
            .as_mut()
            .expect("non-native sample rate has a resampler")
            .process_partial::<&[f32]>(None, None)
            .map_err(resampling_error)
    }

    fn collect_output(
        &mut self,
        output: Vec<Vec<f32>>,
        frame_limit: Option<u64>,
        windows: &mut Vec<AudioBuffer>,
    ) -> Result<(), AudioError> {
        let Some(first) = output.first() else {
            return Err(AudioError::InvalidFormat(
                "progressive resampler returned no channels".to_owned(),
            ));
        };
        if output.len() != self.source_channels as usize
            || output.iter().any(|channel| channel.len() != first.len())
        {
            return Err(AudioError::InvalidFormat(
                "progressive resampler returned inconsistent channels".to_owned(),
            ));
        }

        let skip = self.output_delay_remaining.min(first.len());
        self.output_delay_remaining -= skip;
        let available = first.len() - skip;
        let permitted = frame_limit.map_or(available, |limit| {
            usize::try_from(limit.saturating_sub(self.output_frames))
                .unwrap_or(usize::MAX)
                .min(available)
        });
        if permitted == 0 {
            return Ok(());
        }

        let mut samples = Vec::with_capacity(permitted * CHANNELS as usize);
        let end = skip + permitted;
        if self.source_channels == 1 {
            for sample in &output[0][skip..end] {
                samples.push(*sample);
                samples.push(*sample);
            }
        } else {
            for (left, right) in output[0][skip..end].iter().zip(&output[1][skip..end]) {
                samples.push(*left);
                samples.push(*right);
            }
        }
        let buffer = AudioBuffer::new(samples);
        self.output_frames = self
            .output_frames
            .checked_add(buffer.frame_count() as u64)
            .ok_or_else(|| {
                AudioError::InvalidFormat("canonical frame count overflowed".to_owned())
            })?;
        windows.push(buffer);
        Ok(())
    }

    fn ensure_active(&self) -> Result<(), AudioError> {
        if self.finished {
            Err(AudioError::InvalidFormat(
                "progressive PCM converter is already finished".to_owned(),
            ))
        } else {
            Ok(())
        }
    }
}

fn validate_format(sample_rate: u32, channels: u16) -> Result<(), AudioError> {
    if sample_rate == 0 {
        return Err(AudioError::InvalidFormat(
            "sample rate must be greater than zero".to_owned(),
        ));
    }
    if !matches!(channels, 1 | CHANNELS) {
        return Err(AudioError::InvalidFormat(format!(
            "only mono or stereo progressive PCM is supported, received {channels} channels"
        )));
    }
    Ok(())
}

fn sinc_parameters() -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: SINC_LENGTH,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    }
}

fn resampling_error(error: rubato::ResampleError) -> AudioError {
    AudioError::InvalidFormat(format!("could not resample progressive PCM: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(
        samples: &[f32],
        sample_rate: u32,
        channels: u16,
        chunk_frames: &[usize],
    ) -> Vec<f32> {
        let mut converter = ProgressivePcmCanonicalizer::new(sample_rate, channels).unwrap();
        let mut output = Vec::new();
        let mut frame = 0;
        let total_frames = samples.len() / channels as usize;
        let mut chunk_index = 0;
        while frame < total_frames {
            let frames = chunk_frames[chunk_index % chunk_frames.len()].min(total_frames - frame);
            let start = frame * channels as usize;
            let end = (frame + frames) * channels as usize;
            for window in converter
                .push_interleaved_f32(&samples[start..end])
                .unwrap()
            {
                output.extend(window.samples);
            }
            frame += frames;
            chunk_index += 1;
        }
        for window in converter.finish().unwrap() {
            output.extend(window.samples);
        }
        output
    }

    #[test]
    fn conversion_is_independent_of_native_callback_boundaries() {
        let samples = (0..4_137)
            .map(|frame| {
                let time = frame as f32 / 11_025.0;
                (2.0 * std::f32::consts::PI * 1_237.0 * time).sin() * 0.8
            })
            .collect::<Vec<_>>();
        let contiguous = collect(&samples, 11_025, 1, &[samples.len()]);
        let fragmented = collect(&samples, 11_025, 1, &[1, 7, 511, 3, 997, 64]);
        assert_eq!(fragmented, contiguous);
        assert_eq!(contiguous.len() / 2, samples.len() * 4);
    }

    #[test]
    fn completion_has_the_exact_scaled_frame_count() {
        for sample_rate in [
            8_000, 10_000, 11_025, 16_000, 22_050, 44_100, 48_000, 384_000,
        ] {
            for frames in [0, 1, 17, 511, 512, 513, 1_337] {
                let samples = vec![0.25; frames];
                let output = collect(&samples, sample_rate, 1, &[37]);
                let expected = ((frames as u128 * u128::from(SAMPLE_RATE)
                    + u128::from(sample_rate / 2))
                    / u128::from(sample_rate)) as usize;
                assert_eq!(
                    output.len() / 2,
                    expected,
                    "{frames} frames at {sample_rate} Hz"
                );
            }
        }
    }

    #[test]
    fn canonical_pcm_passes_through_and_expands_mono() {
        let stereo = [0.1, -0.1, 0.25, -0.25];
        assert_eq!(collect(&stereo, SAMPLE_RATE, 2, &[1]), stereo);
        assert_eq!(
            collect(&[0.1, 0.25], SAMPLE_RATE, 1, &[1]),
            [0.1, 0.1, 0.25, 0.25]
        );
    }

    #[test]
    fn sinc_conversion_preserves_more_high_frequency_energy_than_linear() {
        let frames = 11_025;
        let source = (0..frames)
            .map(|frame| {
                let phase = 2.0 * std::f32::consts::PI * 4_500.0 * frame as f32 / 11_025.0;
                phase.sin() * 0.5
            })
            .collect::<Vec<_>>();
        let sinc = collect(&source, 11_025, 1, &[512]);
        let mut linear = Vec::with_capacity(frames * 4 * 2);
        for frame in 0..frames {
            let left = source[frame];
            let right = source[(frame + 1).min(frames - 1)];
            for phase in 0..4 {
                let sample = left + (right - left) * phase as f32 / 4.0;
                linear.extend([sample, sample]);
            }
        }
        let interior = 4_096..sinc.len() - 4_096;
        let rms = |samples: &[f32]| {
            (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32)
                .sqrt()
        };
        assert!(rms(&sinc[interior.clone()]) > rms(&linear[interior]) * 1.25);
    }

    #[test]
    fn rejects_misaligned_or_post_completion_input() {
        let mut converter = ProgressivePcmCanonicalizer::new(22_050, 2).unwrap();
        assert!(converter.push_interleaved_f32(&[0.0]).is_err());
        converter.finish().unwrap();
        assert!(converter.push_interleaved_f32(&[0.0, 0.0]).is_err());
        assert!(converter.finish().is_err());
    }
}
