//! Windows TTS Engine using WinRT SpeechSynthesizer
//!
//! Uses the modern Windows.Media.SpeechSynthesis API for high-quality
//! voices (OneCore / neural). Audio is synthesized to a WAV stream,
//! parsed, and converted to the standard AudioBuffer format
//! (stereo f32 @ 44100Hz).

use crate::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AnchorSupport, AudioOutputMode,
    CancellationSupport, ConcurrencyModel, EngineCapabilities, MarkerCapabilities,
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
        markers: MarkerCapabilities {
            word: cfg!(target_os = "windows"),
            sentence: cfg!(target_os = "windows"),
            requested_anchors: if cfg!(target_os = "windows") {
                AnchorSupport::WordBoundary
            } else {
                AnchorSupport::None
            },
            ..MarkerCapabilities::default()
        },
        language_switching: true,
        text_repertoire: crate::contracts::TextRepertoire::Unicode,
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

#[cfg(any(target_os = "windows", test))]
const WINRT_TICKS_PER_SECOND: u128 = 10_000_000;

#[cfg(any(target_os = "windows", test))]
fn winrt_timestamp_to_frame_offset(duration: i64, sample_rate: u32, frame_count: u64) -> u64 {
    let ticks = u128::try_from(duration).unwrap_or_default();
    let frames = ticks
        .saturating_mul(u128::from(sample_rate))
        .saturating_add(WINRT_TICKS_PER_SECOND / 2)
        / WINRT_TICKS_PER_SECOND;
    u64::try_from(frames).unwrap_or(u64::MAX).min(frame_count)
}

#[cfg(any(target_os = "windows", test))]
fn utf16_inclusive_range_to_utf8(text: &str, start: i32, end_inclusive: i32) -> Option<(u32, u32)> {
    let start = usize::try_from(start).ok()?;
    let end_exclusive = usize::try_from(end_inclusive).ok()?.checked_add(1)?;
    if end_exclusive < start {
        return None;
    }

    let start_byte = utf16_offset_to_utf8(text, start)?;
    let end_byte = utf16_offset_to_utf8(text, end_exclusive)?;
    Some((
        u32::try_from(start_byte).ok()?,
        u32::try_from(end_byte.checked_sub(start_byte)?).ok()?,
    ))
}

#[cfg(any(target_os = "windows", test))]
fn utf16_offset_to_utf8(text: &str, target: usize) -> Option<usize> {
    let mut utf16_offset = 0usize;
    for (byte_offset, character) in text.char_indices() {
        if utf16_offset == target {
            return Some(byte_offset);
        }
        utf16_offset = utf16_offset.checked_add(character.len_utf16())?;
        if utf16_offset > target {
            return None;
        }
    }
    (utf16_offset == target).then_some(text.len())
}

#[cfg(target_os = "windows")]
mod impl_windows {
    use super::windows_capabilities;
    use crate::contracts::{
        Availability, EngineDescriptor, EngineHealth, PhysicalVoiceId, VoiceDescriptor,
    };
    use crate::{
        AudioBuffer, SynthesisMarker, SynthesisMarkerKind, SynthesisRequest, SynthesisResult,
        TtsEngine, TtsError, VoiceInfo, VoiceQuality,
    };
    use std::sync::Mutex;
    use tracing::{debug, info, warn};
    use windows::core::Interface;
    use windows::Media::Core::{SpeechCue, TimedMetadataKind};
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    use windows::Storage::Streams::{DataReader, InputStreamOptions};

    use super::{utf16_inclusive_range_to_utf8, winrt_timestamp_to_frame_offset};

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
        fn try_set_voice(
            synth: &SpeechSynthesizer,
            voice_id: &str,
        ) -> Result<PhysicalVoiceId, TtsError> {
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
                        return Ok(PhysicalVoiceId::new("winrt", format!("winrt:{id}")));
                    }
                }
            }
            Err(TtsError::VoiceNotFound(voice_id.to_owned()))
        }

        fn collect_timed_markers(
            stream: &windows::Media::SpeechSynthesis::SpeechSynthesisStream,
            text: &str,
            sample_rate: u32,
            frame_count: u64,
        ) -> Vec<SynthesisMarker> {
            let tracks = match stream.TimedMetadataTracks() {
                Ok(tracks) => tracks,
                Err(error) => {
                    debug!("WinRT timed metadata unavailable: {error}");
                    return Vec::new();
                }
            };
            let track_count = match tracks.Size() {
                Ok(count) => count,
                Err(error) => {
                    warn!("Could not count WinRT timed metadata tracks: {error}");
                    return Vec::new();
                }
            };
            let mut markers = Vec::new();

            for track_index in 0..track_count {
                let Ok(track) = tracks.GetAt(track_index) else {
                    continue;
                };
                if track.TimedMetadataKind().ok() != Some(TimedMetadataKind::Speech) {
                    continue;
                }
                let track_id = track
                    .Id()
                    .map(|id| id.to_string_lossy())
                    .unwrap_or_default();
                let kind = match track_id.as_str() {
                    "SpeechWord" => SynthesisMarkerKind::Word,
                    "SpeechSentence" => SynthesisMarkerKind::Sentence,
                    _ => continue,
                };
                let Ok(cues) = track.Cues() else {
                    continue;
                };
                let Ok(cue_count) = cues.Size() else {
                    continue;
                };

                for cue_index in 0..cue_count {
                    let Ok(cue) = cues.GetAt(cue_index) else {
                        continue;
                    };
                    let Ok(cue) = cue.cast::<SpeechCue>() else {
                        continue;
                    };
                    let Ok(start_time) = cue.StartTime() else {
                        continue;
                    };
                    let text_range = cue
                        .StartPositionInInput()
                        .and_then(|position| position.Value())
                        .and_then(|start| {
                            cue.EndPositionInInput()
                                .and_then(|position| position.Value())
                                .map(|end| (start, end))
                        })
                        .ok()
                        .and_then(|(start, end)| utf16_inclusive_range_to_utf8(text, start, end));
                    let value = cue
                        .Text()
                        .ok()
                        .map(|value| value.to_string_lossy())
                        .filter(|value| !value.is_empty());

                    markers.push(SynthesisMarker {
                        kind,
                        frame_offset: winrt_timestamp_to_frame_offset(
                            start_time.Duration,
                            sample_rate,
                            frame_count,
                        ),
                        text_start: text_range.map(|range| range.0),
                        text_length: text_range.map(|range| range.1),
                        value,
                    });
                }
            }

            markers.sort_by_key(|marker| marker.frame_offset);
            markers
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

        fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            let text = request.text.as_str();
            let settings = &request.settings;
            if text.is_empty() {
                return Ok(SynthesisResult::audio("winrt", None, AudioBuffer::empty()));
            }

            let guard = self
                .state
                .lock()
                .map_err(|e| TtsError::SynthesisFailed(format!("Lock poisoned: {}", e)))?;

            debug!(
                "WinRT synthesizing: {} (rate: {}, pitch: {}, volume: {})",
                text, settings.rate, settings.pitch, settings.volume
            );

            let synth = &guard.synth;

            // Set voice if specified
            let voice_id = request.voice_id_for_engine("winrt")?;
            let actual_voice = Self::try_set_voice(synth, voice_id)?;

            // Requested native controls must either be applied or fail clearly;
            // marker tracks remain optional metadata and may degrade independently.
            let options = synth.Options().map_err(|error| {
                TtsError::SynthesisFailed(format!(
                    "Could not access WinRT synthesis options: {error}"
                ))
            })?;
            options
                .SetSpeakingRate(Self::map_rate(settings.rate))
                .map_err(|error| {
                    TtsError::SynthesisFailed(format!("Could not set WinRT speaking rate: {error}"))
                })?;
            options
                .SetAudioVolume(Self::map_volume(settings.volume))
                .map_err(|error| {
                    TtsError::SynthesisFailed(format!("Could not set WinRT audio volume: {error}"))
                })?;
            options
                .SetAudioPitch(Self::map_pitch(settings.pitch))
                .map_err(|error| {
                    TtsError::SynthesisFailed(format!("Could not set WinRT audio pitch: {error}"))
                })?;
            if let Err(error) = options.SetIncludeWordBoundaryMetadata(true) {
                debug!("WinRT word boundary metadata unavailable: {error}");
            }
            if let Err(error) = options.SetIncludeSentenceBoundaryMetadata(true) {
                debug!("WinRT sentence boundary metadata unavailable: {error}");
            }

            // Synthesize text to stream (blocking)
            let htext = windows::core::HSTRING::from(text);
            let stream = synth
                .SynthesizeTextToStreamAsync(&htext)
                .map_err(|e| {
                    TtsError::SynthesisFailed(format!("SynthesizeTextToStreamAsync failed: {}", e))
                })?
                .get()
                .map_err(|e| TtsError::SynthesisFailed(format!("Synthesis async failed: {}", e)))?;

            // Read the stream size
            let size = stream
                .Size()
                .map_err(|e| TtsError::SynthesisFailed(format!("Stream Size failed: {}", e)))?
                as u32;

            if size == 0 {
                debug!("WinRT produced no audio");
                return Ok(SynthesisResult::audio(
                    "winrt",
                    Some(actual_voice),
                    AudioBuffer::empty(),
                ));
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
                .map_err(|e| TtsError::SynthesisFailed(format!("Load async get failed: {}", e)))?;

            if bytes_loaded == 0 {
                debug!("WinRT loaded 0 bytes");
                return Ok(SynthesisResult::audio(
                    "winrt",
                    Some(actual_voice),
                    AudioBuffer::empty(),
                ));
            }

            let mut raw_bytes = vec![0u8; bytes_loaded as usize];
            reader
                .ReadBytes(&mut raw_bytes)
                .map_err(|e| TtsError::SynthesisFailed(format!("ReadBytes failed: {}", e)))?;

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

                let native_frame_count = i16_samples.len() as u64 / u64::from(channels);
                let markers = Self::collect_timed_markers(
                    &stream,
                    text,
                    sample_rate,
                    native_frame_count,
                );
                SynthesisResult::from_native_i16(
                    "winrt",
                    Some(actual_voice),
                    &i16_samples,
                    sample_rate,
                    channels,
                    markers,
                    Vec::new(),
                )
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
                    let id = voice.Id().map(|s| s.to_string_lossy()).unwrap_or_default();
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
            let chunk_size =
                u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                    as usize;

            if chunk_id == b"fmt " && chunk_size >= 16 {
                channels = u16::from_le_bytes([data[pos + 10], data[pos + 11]]);
                sample_rate = u32::from_le_bytes([
                    data[pos + 12],
                    data[pos + 13],
                    data[pos + 14],
                    data[pos + 15],
                ]);
                bits_per_sample = u16::from_le_bytes([data[pos + 22], data[pos + 23]]);
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

#[cfg(test)]
mod marker_conversion_tests {
    use super::{utf16_inclusive_range_to_utf8, winrt_timestamp_to_frame_offset};

    #[test]
    fn converts_winrt_ticks_to_bounded_audio_frames() {
        assert_eq!(
            winrt_timestamp_to_frame_offset(5_000_000, 44_100, 50_000),
            22_050
        );
        assert_eq!(winrt_timestamp_to_frame_offset(-1, 44_100, 50_000), 0);
        assert_eq!(
            winrt_timestamp_to_frame_offset(20_000_000, 44_100, 50_000),
            50_000
        );
    }

    #[test]
    fn converts_inclusive_utf16_ranges_to_utf8_bytes() {
        let text = "a😀 café";

        assert_eq!(utf16_inclusive_range_to_utf8(text, 1, 2), Some((1, 4)));
        assert_eq!(utf16_inclusive_range_to_utf8(text, 4, 7), Some((6, 5)));
    }

    #[test]
    fn rejects_invalid_or_split_utf16_ranges() {
        let text = "a😀";

        assert_eq!(utf16_inclusive_range_to_utf8(text, 1, 1), None);
        assert_eq!(utf16_inclusive_range_to_utf8(text, 2, 1), None);
        assert_eq!(utf16_inclusive_range_to_utf8(text, 4, 4), None);
    }
}

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
        _request: &crate::SynthesisRequest,
    ) -> Result<crate::SynthesisResult, crate::TtsError> {
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
