//! Windows TTS Engine using WinRT SpeechSynthesizer
//!
//! Uses the modern Windows.Media.SpeechSynthesis API for high-quality
//! voices (OneCore / neural). Audio is synthesized to a WAV stream,
//! parsed, and converted to the standard AudioBuffer format
//! (stereo f32 @ 44100Hz).

use crate::contracts::{
    AcssCapabilities, AudioOutputMode, CancellationSupport, ConcurrencyModel, EngineCapabilities,
    MarkerCapabilities,
};
#[cfg(not(target_os = "windows"))]
use crate::contracts::{Availability, EngineDescriptor, EngineHealth};

fn windows_capabilities() -> EngineCapabilities {
    EngineCapabilities {
        acss: AcssCapabilities {
            rate: true,
            average_pitch: true,
            volume: true,
            ..AcssCapabilities::default()
        },
        audio_output: AudioOutputMode::BufferedPcm,
        cancellation: CancellationSupport::PlaybackOnly,
        concurrency: ConcurrencyModel::Serialized,
        markers: MarkerCapabilities::default(),
        language_switching: true,
        native_extensions: Vec::new(),
    }
}

#[cfg(target_os = "windows")]
mod impl_windows {
    use super::windows_capabilities;
    use crate::contracts::{Availability, EngineDescriptor, EngineHealth, VoiceDescriptor};
    use crate::{AudioBuffer, TtsEngine, TtsError, TtsSettings, VoiceInfo, VoiceQuality};
    use std::sync::Mutex;
    use tracing::{debug, info, warn};
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    use windows::Storage::Streams::{DataReader, InputStreamOptions};

    struct SynthState {
        synth: SpeechSynthesizer,
    }

    // SAFETY: All WinRT calls are serialized through the Mutex
    unsafe impl Send for SynthState {}
    unsafe impl Sync for SynthState {}

    /// Windows WinRT TTS engine using SpeechSynthesizer
    pub struct WindowsTtsEngine {
        state: Mutex<SynthState>,
    }

    unsafe impl Send for WindowsTtsEngine {}
    unsafe impl Sync for WindowsTtsEngine {}

    impl WindowsTtsEngine {
        /// Create a new Windows WinRT TTS engine.
        pub fn new() -> Result<Self, TtsError> {
            info!("Initializing Windows WinRT TTS engine");

            let synth = SpeechSynthesizer::new().map_err(|e| {
                TtsError::SynthesisFailed(format!("Failed to create SpeechSynthesizer: {}", e))
            })?;

            info!("Windows WinRT TTS engine initialized");

            Ok(Self {
                state: Mutex::new(SynthState { synth }),
            })
        }

        /// Map TtsSettings rate (0.0..1.0, 0.5=normal) to WinRT speaking rate.
        /// WinRT: 0.5 = half speed, 1.0 = normal, 6.0 = max.
        pub(crate) fn map_rate(rate: f32) -> f64 {
            let rate = rate.clamp(0.0, 1.0);
            if rate <= 0.5 {
                // 0.0 -> 0.5, 0.5 -> 1.0
                0.5 + rate as f64
            } else {
                // 0.5 -> 1.0, 1.0 -> 6.0
                let t = (rate - 0.5) / 0.5;
                1.0 + t as f64 * 5.0
            }
        }

        /// Map TtsSettings volume (0.0..1.0) to WinRT volume (0.0..1.0). Direct.
        pub(crate) fn map_volume(volume: f32) -> f64 {
            volume.clamp(0.0, 1.0) as f64
        }

        /// Map TtsSettings pitch (0.5..2.0, 1.0=normal) to WinRT pitch (0.5..2.0). Direct.
        pub(crate) fn map_pitch(pitch: f32) -> f64 {
            pitch.clamp(0.5, 2.0) as f64
        }

        /// Try to find and set a voice matching the settings voice identifier.
        fn try_set_voice(synth: &SpeechSynthesizer, voice_id: &str) -> Result<(), TtsError> {
            let all_voices = SpeechSynthesizer::AllVoices().map_err(|error| {
                TtsError::SynthesisFailed(format!("Could not enumerate WinRT voices: {error}"))
            })?;
            let count = all_voices.Size().map_err(|error| {
                TtsError::SynthesisFailed(format!("Could not count WinRT voices: {error}"))
            })?;
            for i in 0..count {
                if let Ok(voice) = all_voices.GetAt(i) {
                    let id = voice.Id().map(|s| s.to_string_lossy()).unwrap_or_default();
                    let name = voice
                        .DisplayName()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default();
                    let lang = voice
                        .Language()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default();

                    if id == voice_id
                        || name == voice_id
                        || lang == voice_id
                        || format!("winrt:{}", id) == voice_id
                    {
                        synth.SetVoice(&voice).map_err(|error| {
                            TtsError::SynthesisFailed(format!(
                                "Could not select WinRT voice {voice_id}: {error}"
                            ))
                        })?;
                        return Ok(());
                    }
                }
            }
            Err(TtsError::VoiceNotFound(voice_id.to_owned()))
        }
    }

    impl TtsEngine for WindowsTtsEngine {
        fn descriptor(&self) -> EngineDescriptor {
            let voices = self
                .available_voices()
                .into_iter()
                .map(|voice| VoiceDescriptor::from_voice_info("winrt", voice))
                .collect();
            let default_voice_id = SpeechSynthesizer::DefaultVoice()
                .ok()
                .and_then(|voice| voice.Id().ok())
                .map(|id| format!("winrt:{}", id.to_string_lossy()));

            EngineDescriptor {
                id: "winrt".to_owned(),
                display_name: "Windows WinRT Speech Synthesis".to_owned(),
                version: None,
                availability: Availability::Available,
                health: EngineHealth::Healthy,
                capabilities: windows_capabilities(),
                voices,
                default_voice_id,
            }
        }

        fn synthesize(
            &self,
            text: &str,
            settings: &TtsSettings,
        ) -> Result<AudioBuffer, TtsError> {
            if text.is_empty() {
                return Ok(AudioBuffer::empty());
            }

            let guard = self.state.lock().map_err(|e| {
                TtsError::SynthesisFailed(format!("Lock poisoned: {}", e))
            })?;

            debug!(
                "WinRT synthesizing: {} (rate: {}, pitch: {}, volume: {})",
                text, settings.rate, settings.pitch, settings.volume
            );

            let synth = &guard.synth;

            // Set voice if specified
            Self::try_set_voice(synth, &settings.voice)?;

            // Set synthesis options (rate, pitch, volume)
            if let Ok(options) = synth.Options() {
                let _ = options.SetSpeakingRate(Self::map_rate(settings.rate));
                let _ = options.SetAudioVolume(Self::map_volume(settings.volume));
                let _ = options.SetAudioPitch(Self::map_pitch(settings.pitch));
            }

            // Synthesize text to stream (blocking)
            let htext = windows::core::HSTRING::from(text);
            let stream = synth
                .SynthesizeTextToStreamAsync(&htext)
                .map_err(|e| {
                    TtsError::SynthesisFailed(format!(
                        "SynthesizeTextToStreamAsync failed: {}",
                        e
                    ))
                })?
                .get()
                .map_err(|e| {
                    TtsError::SynthesisFailed(format!("Synthesis async failed: {}", e))
                })?;

            // Read the stream size
            let size = stream.Size().map_err(|e| {
                TtsError::SynthesisFailed(format!("Stream Size failed: {}", e))
            })? as u32;

            if size == 0 {
                debug!("WinRT produced no audio");
                return Ok(AudioBuffer::empty());
            }

            // Read bytes from the stream
            let input_stream = stream.GetInputStreamAt(0).map_err(|e| {
                TtsError::SynthesisFailed(format!("GetInputStreamAt failed: {}", e))
            })?;

            let reader = DataReader::CreateDataReader(&input_stream).map_err(|e| {
                TtsError::SynthesisFailed(format!("CreateDataReader failed: {}", e))
            })?;
            let _ = reader.SetInputStreamOptions(InputStreamOptions::ReadAhead);

            let bytes_loaded = reader
                .LoadAsync(size)
                .map_err(|e| TtsError::SynthesisFailed(format!("LoadAsync failed: {}", e)))?
                .get()
                .map_err(|e| {
                    TtsError::SynthesisFailed(format!("Load async get failed: {}", e))
                })?;

            if bytes_loaded == 0 {
                debug!("WinRT loaded 0 bytes");
                return Ok(AudioBuffer::empty());
            }

            let mut raw_bytes = vec![0u8; bytes_loaded as usize];
            reader.ReadBytes(&mut raw_bytes).map_err(|e| {
                TtsError::SynthesisFailed(format!("ReadBytes failed: {}", e))
            })?;

            // Parse WAV header to extract format and PCM data offset
            let (sample_rate, channels, bits_per_sample, data_start) =
                parse_wav_header(&raw_bytes)?;

            let pcm_bytes = &raw_bytes[data_start..];

            if bits_per_sample == 16 {
                let i16_samples: Vec<i16> = pcm_bytes
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                debug!(
                    "WinRT produced {} i16 samples at {}Hz ({} ch)",
                    i16_samples.len(),
                    sample_rate,
                    channels
                );

                let buffer = AudioBuffer::from_i16(&i16_samples, sample_rate, channels);
                Ok(buffer.to_standard_format())
            } else {
                Err(TtsError::SynthesisFailed(format!(
                    "Unsupported bits per sample: {}",
                    bits_per_sample
                )))
            }
        }

        fn stop(&self) {
            debug!("WinRT: stop requested");
            // Synthesis is synchronous (blocking .get()), so stop is a no-op.
            // The caller stops playback via AudioStreams::stop_all().
        }

        fn is_speaking(&self) -> bool {
            false // Synthesis is blocking, playback is handled by AudioStreams
        }

        fn available_voices(&self) -> Vec<VoiceInfo> {
            let voice_list = match SpeechSynthesizer::AllVoices() {
                Ok(v) => v,
                Err(e) => {
                    warn!("AllVoices failed: {}", e);
                    return Vec::new();
                }
            };

            let count = match voice_list.Size() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };

            let mut voices = Vec::with_capacity(count as usize);
            for i in 0..count {
                if let Ok(voice) = voice_list.GetAt(i) {
                    let id = voice
                        .Id()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default();
                    let name = voice
                        .DisplayName()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default();
                    let lang = voice
                        .Language()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default();

                    voices.push(VoiceInfo {
                        identifier: format!("winrt:{}", id),
                        name,
                        language: lang,
                        quality: VoiceQuality::Enhanced,
                    });
                }
            }

            debug!("WinRT found {} voices", voices.len());
            voices
        }

        fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
            let search = identifier.strip_prefix("winrt:").unwrap_or(identifier);
            self.available_voices().into_iter().find(|v| {
                v.identifier == identifier
                    || v.identifier == format!("winrt:{}", search)
                    || v.name == search
                    || v.language == search
            })
        }
    }

    /// Parse a WAV header to extract sample rate, channels, bits per sample,
    /// and the byte offset where PCM data begins.
    pub(crate) fn parse_wav_header(data: &[u8]) -> Result<(u32, u16, u16, usize), TtsError> {
        if data.len() < 44 {
            return Err(TtsError::SynthesisFailed("WAV data too short".to_string()));
        }

        // Verify RIFF/WAVE header
        if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err(TtsError::SynthesisFailed(
                "Not a valid WAV file".to_string(),
            ));
        }

        let mut pos = 12;
        let mut sample_rate = 0u32;
        let mut channels = 0u16;
        let mut bits_per_sample = 0u16;

        while pos + 8 <= data.len() {
            let chunk_id = &data[pos..pos + 4];
            let chunk_size = u32::from_le_bytes([
                data[pos + 4],
                data[pos + 5],
                data[pos + 6],
                data[pos + 7],
            ]) as usize;

            if chunk_id == b"fmt " && chunk_size >= 16 {
                channels =
                    u16::from_le_bytes([data[pos + 10], data[pos + 11]]);
                sample_rate = u32::from_le_bytes([
                    data[pos + 12],
                    data[pos + 13],
                    data[pos + 14],
                    data[pos + 15],
                ]);
                bits_per_sample =
                    u16::from_le_bytes([data[pos + 22], data[pos + 23]]);
            } else if chunk_id == b"data" {
                return Ok((sample_rate, channels, bits_per_sample, pos + 8));
            }

            pos += 8 + chunk_size;
            // WAV chunks are word-aligned
            if !chunk_size.is_multiple_of(2) {
                pos += 1;
            }
        }

        Err(TtsError::SynthesisFailed(
            "WAV data chunk not found".to_string(),
        ))
    }
}

#[cfg(target_os = "windows")]
pub use impl_windows::WindowsTtsEngine;

// Stub implementation for non-Windows platforms
#[cfg(not(target_os = "windows"))]
pub struct WindowsTtsEngine;

#[cfg(not(target_os = "windows"))]
impl WindowsTtsEngine {
    pub fn new() -> Result<Self, crate::TtsError> {
        Err(crate::TtsError::NotAvailable)
    }
}

#[cfg(not(target_os = "windows"))]
impl crate::TtsEngine for WindowsTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            id: "winrt".to_owned(),
            display_name: "Windows WinRT Speech Synthesis".to_owned(),
            version: None,
            availability: Availability::Unavailable {
                reason: "WinRT is only available on Windows".to_owned(),
            },
            health: EngineHealth::Failed {
                reason: "unsupported platform".to_owned(),
            },
            capabilities: windows_capabilities(),
            voices: Vec::new(),
            default_voice_id: None,
        }
    }

    fn synthesize(
        &self,
        _text: &str,
        _settings: &crate::TtsSettings,
    ) -> Result<crate::AudioBuffer, crate::TtsError> {
        Err(crate::TtsError::NotAvailable)
    }

    fn stop(&self) {}

    fn is_speaking(&self) -> bool {
        false
    }

    fn available_voices(&self) -> Vec<crate::VoiceInfo> {
        vec![]
    }

    fn voice_info(&self, _identifier: &str) -> Option<crate::VoiceInfo> {
        None
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod stub_tests {
    use super::WindowsTtsEngine;
    use crate::TtsEngine;

    #[test]
    fn windows_stub_reports_unavailable() {
        let descriptor = WindowsTtsEngine.descriptor();

        assert_eq!(descriptor.id, "winrt");
        assert!(!descriptor.can_synthesize());
        assert!(descriptor.voices.is_empty());
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::impl_windows::WindowsTtsEngine;

    #[test]
    fn test_rate_mapping() {
        // 0.0 -> 0.5 (half speed)
        assert!((WindowsTtsEngine::map_rate(0.0) - 0.5).abs() < 0.01);
        // 0.5 -> 1.0 (normal)
        assert!((WindowsTtsEngine::map_rate(0.5) - 1.0).abs() < 0.01);
        // 1.0 -> 6.0 (max)
        assert!((WindowsTtsEngine::map_rate(1.0) - 6.0).abs() < 0.01);
    }

    #[test]
    fn test_volume_mapping() {
        assert!((WindowsTtsEngine::map_volume(0.0) - 0.0).abs() < 0.01);
        assert!((WindowsTtsEngine::map_volume(0.5) - 0.5).abs() < 0.01);
        assert!((WindowsTtsEngine::map_volume(1.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_pitch_mapping() {
        assert!((WindowsTtsEngine::map_pitch(0.5) - 0.5).abs() < 0.01);
        assert!((WindowsTtsEngine::map_pitch(1.0) - 1.0).abs() < 0.01);
        assert!((WindowsTtsEngine::map_pitch(2.0) - 2.0).abs() < 0.01);
        // Clamping
        assert!((WindowsTtsEngine::map_pitch(0.1) - 0.5).abs() < 0.01);
        assert!((WindowsTtsEngine::map_pitch(3.0) - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_wav_header_parsing() {
        use super::impl_windows::parse_wav_header;
        // Minimal valid WAV: RIFF header + fmt chunk + data chunk
        let mut wav = Vec::new();
        // RIFF header
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&36u32.to_le_bytes()); // file size - 8
        wav.extend_from_slice(b"WAVE");
        // fmt chunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        wav.extend_from_slice(&1u16.to_le_bytes()); // 1 channel
        wav.extend_from_slice(&16000u32.to_le_bytes()); // sample rate
        wav.extend_from_slice(&32000u32.to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        // data chunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes()); // data size
        wav.extend_from_slice(&[0u8; 4]); // 2 samples of silence

        let (sr, ch, bps, offset) = parse_wav_header(&wav).unwrap();
        assert_eq!(sr, 16000);
        assert_eq!(ch, 1);
        assert_eq!(bps, 16);
        assert_eq!(offset, 44);
    }
}
