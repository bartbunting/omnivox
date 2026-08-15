//! Audio File Loader
//!
//! Loads OGG Vorbis and WAV files into the common AudioBuffer format
//! using rodio decoders. Handles resampling, channel conversion,
//! and format conversion automatically.

use crate::buffer::{AudioBuffer, CHANNELS, SAMPLE_RATE};
use crate::AudioError;
use rodio::Source;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Mutex;

/// Maximum encoded resource size accepted by the common loader (16 MiB).
pub const MAX_AUDIO_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum decoded duration retained for one resource.
pub const MAX_AUDIO_DURATION_SECS: usize = 30;
/// Maximum number of decoded resources retained by the shared cache.
pub const MAX_AUDIO_CACHE_ENTRIES: usize = 128;
/// Maximum canonical PCM sample storage retained by the shared cache (64 MiB).
pub const MAX_AUDIO_CACHE_SAMPLES: usize = 64 * 1024 * 1024 / std::mem::size_of::<f32>();

#[derive(Debug, Clone)]
struct CachedAudio {
    buffer: AudioBuffer,
    last_used: u64,
}

#[derive(Debug, Default)]
struct AudioCache {
    entries: HashMap<String, CachedAudio>,
    total_samples: usize,
    access_clock: u64,
}

impl AudioCache {
    fn get(&mut self, key: &str) -> Option<AudioBuffer> {
        let entry = self.entries.get_mut(key)?;
        self.access_clock = self.access_clock.wrapping_add(1);
        entry.last_used = self.access_clock;
        Some(entry.buffer.clone())
    }

    fn insert(&mut self, key: String, buffer: AudioBuffer) {
        self.access_clock = self.access_clock.wrapping_add(1);
        if let Some(previous) = self.entries.remove(&key) {
            self.total_samples = self
                .total_samples
                .saturating_sub(previous.buffer.samples.len());
        }
        self.total_samples = self.total_samples.saturating_add(buffer.samples.len());
        self.entries.insert(
            key,
            CachedAudio {
                buffer,
                last_used: self.access_clock,
            },
        );
        while self.entries.len() > MAX_AUDIO_CACHE_ENTRIES
            || self.total_samples > MAX_AUDIO_CACHE_SAMPLES
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.total_samples = self
                    .total_samples
                    .saturating_sub(removed.buffer.samples.len());
            }
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.total_samples = 0;
    }
}

/// Loads audio files (OGG, WAV) into AudioBuffers.
pub struct AudioFileLoader {
    cache: Mutex<AudioCache>,
    cache_enabled: bool,
}

impl AudioFileLoader {
    /// Create a new AudioFileLoader without caching.
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(AudioCache::default()),
            cache_enabled: false,
        }
    }

    /// Create a new AudioFileLoader with an LRU-style cache.
    pub fn with_cache() -> Self {
        Self {
            cache: Mutex::new(AudioCache::default()),
            cache_enabled: true,
        }
    }

    /// Load an audio file and return it as a stereo f32 44100Hz AudioBuffer.
    ///
    /// Supports OGG Vorbis and WAV formats. Automatically handles:
    /// - Resampling to 44100Hz if the source has a different sample rate
    /// - Mono to stereo conversion (duplicate channels)
    /// - Integer to f32 conversion
    pub fn load(&self, path: &Path) -> Result<AudioBuffer, AudioError> {
        let path_key = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string();

        // Check cache first
        if self.cache_enabled {
            if let Ok(mut cache) = self.cache.lock() {
                if let Some(buffer) = cache.get(&path_key) {
                    return Ok(buffer);
                }
            }
        }

        let buffer = self.load_uncached(path)?;

        // Store in cache
        if self.cache_enabled {
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(path_key, buffer.clone());
            }
        }

        Ok(buffer)
    }

    /// Load without checking or updating the cache.
    fn load_uncached(&self, path: &Path) -> Result<AudioBuffer, AudioError> {
        let metadata = std::fs::metadata(path).map_err(|error| {
            AudioError::FileNotFound(format!("{}: {error}", path.to_string_lossy()))
        })?;
        if metadata.len() > MAX_AUDIO_FILE_BYTES {
            return Err(AudioError::DecodeError(format!(
                "{} is {} bytes; maximum resource size is {MAX_AUDIO_FILE_BYTES}",
                path.to_string_lossy(),
                metadata.len()
            )));
        }

        let file = File::open(path)
            .map_err(|e| AudioError::FileNotFound(format!("{}: {}", path.to_string_lossy(), e)))?;
        let reader = BufReader::new(file);

        let decoder = rodio::Decoder::new(reader)
            .map_err(|e| AudioError::DecodeError(format!("{}: {}", path.to_string_lossy(), e)))?;

        let source_channels = decoder.channels();
        let source_sample_rate = decoder.sample_rate();
        if source_channels == 0 || source_sample_rate == 0 {
            return Err(AudioError::DecodeError(format!(
                "{} reports an invalid audio format",
                path.to_string_lossy()
            )));
        }

        let max_raw_samples = (source_sample_rate as usize)
            .checked_mul(MAX_AUDIO_DURATION_SECS)
            .and_then(|frames| frames.checked_mul(source_channels as usize))
            .ok_or_else(|| {
                AudioError::DecodeError(format!(
                    "{} reports an unbounded audio format",
                    path.to_string_lossy()
                ))
            })?;
        let raw_samples: Vec<f32> = decoder
            .convert_samples::<f32>()
            .take(max_raw_samples.saturating_add(1))
            .collect();
        if raw_samples.len() > max_raw_samples {
            return Err(AudioError::DecodeError(format!(
                "{} exceeds the {MAX_AUDIO_DURATION_SECS}-second decoded duration limit",
                path.to_string_lossy()
            )));
        }

        if raw_samples.is_empty() {
            return Ok(AudioBuffer::empty());
        }

        // Convert to stereo if mono
        let stereo_samples = if source_channels == 1 {
            mono_to_stereo(&raw_samples)
        } else if source_channels == 2 {
            raw_samples
        } else {
            // For multi-channel, take first two channels
            downmix_to_stereo(&raw_samples, source_channels)
        };

        // Resample if needed
        let final_samples = if source_sample_rate != SAMPLE_RATE {
            resample_linear(&stereo_samples, source_sample_rate, SAMPLE_RATE)
        } else {
            stereo_samples
        };

        Ok(AudioBuffer::new(final_samples))
    }

    /// Clear the audio file cache.
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    /// Get the number of cached entries.
    pub fn cache_size(&self) -> usize {
        self.cache
            .lock()
            .map(|cache| cache.entries.len())
            .unwrap_or(0)
    }
}

impl Default for AudioFileLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert mono samples to stereo by duplicating each sample.
fn mono_to_stereo(mono: &[f32]) -> Vec<f32> {
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for &sample in mono {
        stereo.push(sample);
        stereo.push(sample);
    }
    stereo
}

/// Downmix multi-channel audio to stereo by taking the first two channels.
fn downmix_to_stereo(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels as usize;
    let frame_count = samples.len() / ch;
    let mut stereo = Vec::with_capacity(frame_count * 2);
    for frame in 0..frame_count {
        let offset = frame * ch;
        stereo.push(samples[offset]); // left
        stereo.push(if ch >= 2 {
            samples[offset + 1]
        } else {
            samples[offset]
        }); // right
    }
    stereo
}

/// Linear interpolation resampling for stereo audio.
fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    let from_frames = samples.len() / CHANNELS as usize;
    let to_frames = (from_frames as f64 * to_rate as f64 / from_rate as f64).round() as usize;

    if to_frames == 0 {
        return Vec::new();
    }

    let mut output = Vec::with_capacity(to_frames * CHANNELS as usize);
    let ratio = from_rate as f64 / to_rate as f64;

    for i in 0..to_frames {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos.floor() as usize;
        let frac = (src_pos - src_idx as f64) as f32;

        let next_idx = (src_idx + 1).min(from_frames - 1);

        // Left channel
        let l0 = samples[src_idx * 2];
        let l1 = samples[next_idx * 2];
        output.push(l0 + (l1 - l0) * frac);

        // Right channel
        let r0 = samples[src_idx * 2 + 1];
        let r1 = samples[next_idx * 2 + 1];
        output.push(r0 + (r1 - r0) * frac);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_mono_to_stereo() {
        let mono = vec![0.5, -0.3, 0.1];
        let stereo = mono_to_stereo(&mono);
        assert_eq!(stereo, vec![0.5, 0.5, -0.3, -0.3, 0.1, 0.1]);
    }

    #[test]
    fn test_downmix_to_stereo() {
        // 4-channel audio: [L, R, C, LFE] per frame
        let samples = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let stereo = downmix_to_stereo(&samples, 4);
        // Should take just L and R from each frame
        assert_eq!(stereo, vec![0.1, 0.2, 0.5, 0.6]);
    }

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let resampled = resample_linear(&samples, 44100, 44100);
        assert_eq!(resampled.len(), samples.len());
    }

    #[test]
    fn test_resample_upsample() {
        // 2 frames at 22050 Hz -> ~4 frames at 44100 Hz
        let samples = vec![0.0, 0.0, 1.0, 1.0];
        let resampled = resample_linear(&samples, 22050, 44100);
        assert_eq!(resampled.len() / 2, 4); // 4 frames
    }

    #[test]
    fn test_resample_downsample() {
        // 4 frames at 44100 Hz -> ~2 frames at 22050 Hz
        let samples = vec![0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 0.5, 0.5];
        let resampled = resample_linear(&samples, 44100, 22050);
        assert_eq!(resampled.len() / 2, 2); // 2 frames
    }

    #[test]
    fn test_load_nonexistent_file() {
        let loader = AudioFileLoader::new();
        let result = loader.load(Path::new("/nonexistent/file.wav"));
        assert!(result.is_err());
        match result.unwrap_err() {
            AudioError::FileNotFound(_) => {}
            other => panic!("expected FileNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_cache_operations() {
        let loader = AudioFileLoader::with_cache();
        assert_eq!(loader.cache_size(), 0);
        loader.clear_cache();
        assert_eq!(loader.cache_size(), 0);
    }

    #[test]
    fn cache_evicts_the_least_recently_used_entry() {
        let mut cache = AudioCache::default();
        for index in 0..MAX_AUDIO_CACHE_ENTRIES {
            cache.insert(
                format!("resource-{index}"),
                AudioBuffer::new(vec![index as f32, index as f32]),
            );
        }
        assert!(cache.get("resource-0").is_some());

        cache.insert("new-resource".to_owned(), AudioBuffer::new(vec![0.0, 0.0]));

        assert_eq!(cache.entries.len(), MAX_AUDIO_CACHE_ENTRIES);
        assert!(cache.entries.contains_key("resource-0"));
        assert!(!cache.entries.contains_key("resource-1"));
        assert!(cache.entries.contains_key("new-resource"));
    }

    #[test]
    fn test_loader_default() {
        let loader = AudioFileLoader::default();
        assert_eq!(loader.cache_size(), 0);
    }

    #[test]
    fn encoded_resource_size_is_bounded_before_decode() {
        let path = std::env::temp_dir().join(format!(
            "omnivox-oversized-resource-{}.wav",
            std::process::id()
        ));
        let file = File::create(&path).unwrap();
        file.set_len(MAX_AUDIO_FILE_BYTES + 1).unwrap();

        let error = AudioFileLoader::new().load(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);

        assert!(matches!(error, AudioError::DecodeError(_)));
        assert!(error.to_string().contains("maximum resource size"));
    }

    #[test]
    fn test_load_generated_wav() {
        // Generate a minimal WAV file in memory and write to a temp file
        let tmp_dir = std::env::temp_dir();
        let wav_path = tmp_dir.join("omnivox_test.wav");

        // Create a simple WAV file: 44100 Hz, mono, 16-bit, 100 samples
        let num_samples: u32 = 100;
        let sample_rate: u32 = 44100;
        let bits_per_sample: u16 = 16;
        let num_channels: u16 = 1;
        let byte_rate: u32 = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
        let block_align: u16 = num_channels * bits_per_sample / 8;
        let data_size: u32 = num_samples * block_align as u32;

        let mut wav_data: Vec<u8> = Vec::new();
        // RIFF header
        wav_data.extend_from_slice(b"RIFF");
        wav_data.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav_data.extend_from_slice(b"WAVE");
        // fmt chunk
        wav_data.extend_from_slice(b"fmt ");
        wav_data.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        wav_data.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        wav_data.extend_from_slice(&num_channels.to_le_bytes());
        wav_data.extend_from_slice(&sample_rate.to_le_bytes());
        wav_data.extend_from_slice(&byte_rate.to_le_bytes());
        wav_data.extend_from_slice(&block_align.to_le_bytes());
        wav_data.extend_from_slice(&bits_per_sample.to_le_bytes());
        // data chunk
        wav_data.extend_from_slice(b"data");
        wav_data.extend_from_slice(&data_size.to_le_bytes());
        // Generate a simple sine wave
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            let sample_i16 = (sample * 32767.0) as i16;
            wav_data.extend_from_slice(&sample_i16.to_le_bytes());
        }

        std::fs::write(&wav_path, &wav_data).expect("failed to write test WAV");

        let loader = AudioFileLoader::new();
        let result = loader.load(&wav_path);

        // Clean up
        let _ = std::fs::remove_file(&wav_path);

        let buffer = result.expect("should load WAV file");
        // Source is mono 44100Hz, so output should be stereo 44100Hz
        assert_eq!(buffer.channels(), 2);
        assert_eq!(buffer.sample_rate(), 44100);
        // Mono -> stereo: frame count should match original sample count
        assert_eq!(buffer.frame_count(), num_samples as usize);
        // Each frame should have identical L and R (mono duplication)
        for i in 0..buffer.frame_count() {
            assert_eq!(
                buffer.left(i),
                buffer.right(i),
                "mono->stereo: L and R should match at frame {}",
                i
            );
        }
    }

    #[test]
    fn test_cache_hit() {
        let tmp_dir = std::env::temp_dir();
        let wav_path = tmp_dir.join("omnivox_cache_test.wav");

        // Create minimal WAV
        let num_samples: u32 = 10;
        let sample_rate: u32 = 44100;
        let mut wav_data: Vec<u8> = Vec::new();
        wav_data.extend_from_slice(b"RIFF");
        wav_data.extend_from_slice(&(36 + num_samples * 2).to_le_bytes());
        wav_data.extend_from_slice(b"WAVE");
        wav_data.extend_from_slice(b"fmt ");
        wav_data.extend_from_slice(&16u32.to_le_bytes());
        wav_data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav_data.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav_data.extend_from_slice(&sample_rate.to_le_bytes());
        wav_data.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        wav_data.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav_data.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        wav_data.extend_from_slice(b"data");
        wav_data.extend_from_slice(&(num_samples * 2).to_le_bytes());
        for _ in 0..num_samples {
            wav_data.extend_from_slice(&0i16.to_le_bytes());
        }

        std::fs::write(&wav_path, &wav_data).expect("failed to write test WAV");

        let loader = AudioFileLoader::with_cache();
        assert_eq!(loader.cache_size(), 0);

        let _ = loader.load(&wav_path).unwrap();
        assert_eq!(loader.cache_size(), 1);

        // Second load should hit cache
        let _ = loader.load(&wav_path).unwrap();
        assert_eq!(loader.cache_size(), 1);

        let _ = std::fs::remove_file(&wav_path);
    }
}
