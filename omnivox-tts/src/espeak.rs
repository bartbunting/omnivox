//! espeak-ng TTS Engine
//!
//! Cross-platform TTS backend using espeak-ng. Always compiled in as the
//! guaranteed fallback engine that works on all platforms.

use crate::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AnchorSupport, AudioOutputMode,
    Availability, CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor,
    EngineHealth, MarkerCapabilities, PhysicalVoiceId, VoiceDescriptor,
};
#[cfg(test)]
use crate::TtsSettings;
use crate::{
    AudioBuffer, SynthesisMarker, SynthesisMarkerKind, SynthesisRequest, SynthesisResult,
    TtsEngine, TtsError, VoiceInfo, VoiceQuality,
};
use once_cell::sync::OnceCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_short, c_void};
use std::sync::Mutex;
use tracing::{debug, info};

/// Global espeak-ng initialization guard.
/// espeak-ng uses global state internally, so we need a mutex to serialize access.
static ESPEAK_LOCK: OnceCell<Mutex<EspeakState>> = OnceCell::new();

/// Internal state for espeak-ng synthesis
struct EspeakState {
    sample_rate: u32,
    initialized: bool,
}

/// Defensive limit for the native callback's zero-terminated event array.
const MAX_CALLBACK_EVENTS: usize = 65_536;

/// Audio and synchronization events collected for the one serialized synthesis.
static SYNTH_CAPTURE: Mutex<Option<EspeakSynthesisCapture>> = Mutex::new(None);

#[derive(Default)]
struct EspeakSynthesisCapture {
    samples: Vec<i16>,
    markers: Vec<EspeakNativeMarker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EspeakNativeMarkerKind {
    Word,
    Sentence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EspeakNativeMarker {
    kind: EspeakNativeMarkerKind,
    /// One-based Unicode character position supplied by eSpeak.
    text_position: usize,
    /// Unicode character count; eSpeak supplies this only for words.
    text_length: usize,
    audio_position_ms: u64,
}

/// espeak-ng TTS engine
pub struct EspeakTtsEngine {
    _private: (),
}

// SAFETY: All espeak-ng access is serialized through the ESPEAK_LOCK mutex
unsafe impl Send for EspeakTtsEngine {}
unsafe impl Sync for EspeakTtsEngine {}

/// Callback function for espeak-ng synthesis.
/// Called by espeak-ng with chunks of i16 PCM audio data.
unsafe extern "C" fn synth_callback(
    wav: *mut c_short,
    sample_count: c_int,
    events: *mut espeak_rs_sys::espeak_EVENT,
) -> c_int {
    if let Ok(mut slot) = SYNTH_CAPTURE.lock() {
        if let Some(capture) = slot.as_mut() {
            if !wav.is_null() && sample_count > 0 {
                let samples = std::slice::from_raw_parts(wav as *const i16, sample_count as usize);
                capture.samples.extend_from_slice(samples);
            }

            if !events.is_null() {
                for index in 0..MAX_CALLBACK_EVENTS {
                    let event = &*events.add(index);
                    if event.type_ == espeak_rs_sys::espeak_EVENT_TYPE_espeakEVENT_LIST_TERMINATED {
                        break;
                    }
                    let kind = match event.type_ {
                        espeak_rs_sys::espeak_EVENT_TYPE_espeakEVENT_WORD => {
                            EspeakNativeMarkerKind::Word
                        }
                        espeak_rs_sys::espeak_EVENT_TYPE_espeakEVENT_SENTENCE => {
                            EspeakNativeMarkerKind::Sentence
                        }
                        _ => continue,
                    };
                    if event.text_position <= 0 || event.audio_position < 0 {
                        continue;
                    }
                    capture.markers.push(EspeakNativeMarker {
                        kind,
                        text_position: event.text_position as usize,
                        text_length: event.length.max(0) as usize,
                        audio_position_ms: event.audio_position as u64,
                    });
                }
            }
        }
    }

    // Return 0 to continue synthesis, 1 to abort
    0
}

/// Data path discovered at build time by build.rs
const ESPEAK_DATA_DIR: &str = env!("ESPEAK_NG_DATA_DIR");

impl EspeakTtsEngine {
    fn capabilities() -> EngineCapabilities {
        EngineCapabilities {
            acss: AcssCapabilities {
                rate: true,
                average_pitch: true,
                volume: true,
                ..AcssCapabilities::default()
            },
            audio_output: AudioOutputMode::BufferedPcm,
            cancellation: CancellationSupport::SynthesisAndPlayback,
            concurrency: ConcurrencyModel::Serialized,
            markers: MarkerCapabilities {
                word: true,
                sentence: true,
                requested_anchors: AnchorSupport::WordBoundary,
                ..MarkerCapabilities::default()
            },
            language_switching: true,
            post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
            native_extensions: Vec::new(),
        }
    }

    fn runtime_metadata() -> (Option<String>, Option<String>) {
        let Some(state) = ESPEAK_LOCK.get() else {
            return (None, None);
        };
        let Ok(_guard) = state.lock() else {
            return (None, None);
        };

        unsafe {
            let version_ptr = espeak_rs_sys::espeak_Info(std::ptr::null_mut());
            let version = (!version_ptr.is_null())
                .then(|| CStr::from_ptr(version_ptr).to_string_lossy().into_owned());

            let voice_ptr = espeak_rs_sys::espeak_GetCurrentVoice();
            let default_voice_id = if voice_ptr.is_null() {
                None
            } else {
                let voice = &*voice_ptr;
                let id_ptr = if voice.identifier.is_null() {
                    voice.name
                } else {
                    voice.identifier
                };
                (!id_ptr.is_null())
                    .then(|| format!("espeak:{}", CStr::from_ptr(id_ptr).to_string_lossy()))
            };

            (version, default_voice_id)
        }
    }

    /// Find the espeak-ng data directory.
    /// Checks (in order): ESPEAK_NG_DATA env var, build-time path, next to executable, system paths.
    /// Returns the parent directory (espeak-ng appends "espeak-ng-data" itself).
    fn find_data_path() -> Option<CString> {
        // 1. Runtime environment variable
        if let Ok(dir) = std::env::var("ESPEAK_NG_DATA") {
            if !dir.is_empty()
                && std::path::Path::new(&dir)
                    .join("espeak-ng-data")
                    .join("phontab")
                    .exists()
            {
                debug!("Using espeak-ng data from ESPEAK_NG_DATA env: {}", dir);
                return CString::new(dir).ok();
            }
        }

        // 2. Build-time discovered path (from espeak-rs-sys OUT_DIR)
        if !ESPEAK_DATA_DIR.is_empty()
            && std::path::Path::new(ESPEAK_DATA_DIR)
                .join("espeak-ng-data")
                .join("phontab")
                .exists()
        {
            debug!("Using espeak-ng data from build path: {}", ESPEAK_DATA_DIR);
            return CString::new(ESPEAK_DATA_DIR).ok();
        }

        // 3. Next to the executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let data_check = exe_dir.join("espeak-ng-data").join("phontab");
                if data_check.exists() {
                    debug!("Using espeak-ng data next to executable");
                    return CString::new(exe_dir.to_string_lossy().as_ref()).ok();
                }
            }
        }

        // 4. System paths
        let candidates = [
            "/opt/homebrew/share",
            "/usr/local/share",
            "/usr/share",
            "/usr/lib/espeak-ng",
            "/usr/local/lib/espeak-ng",
        ];

        for candidate in &candidates {
            let data_dir = format!("{}/espeak-ng-data", candidate);
            if std::path::Path::new(&data_dir).exists() {
                debug!("Found espeak-ng data at: {}", data_dir);
                return CString::new(*candidate).ok();
            }
        }

        None
    }

    /// Create a new espeak-ng TTS engine.
    /// Initializes espeak-ng on first call; subsequent calls reuse the existing initialization.
    pub fn new() -> Result<Self, TtsError> {
        let state = ESPEAK_LOCK
            .get_or_try_init(|| -> Result<Mutex<EspeakState>, TtsError> {
                info!("Initializing espeak-ng TTS engine");

                let data_path = Self::find_data_path();

                let sample_rate = unsafe {
                    let path_ptr = match &data_path {
                        Some(p) => p.as_ptr(),
                        None => std::ptr::null(),
                    };

                    // Initialize espeak-ng with AUDIO_OUTPUT_RETRIEVAL mode (no direct playback)
                    // 0x8000 = espeakINITIALIZE_DONT_EXIT: prevents espeak from calling exit()
                    let rate = espeak_rs_sys::espeak_Initialize(
                        espeak_rs_sys::espeak_AUDIO_OUTPUT_AUDIO_OUTPUT_RETRIEVAL,
                        0,        // default buffer length
                        path_ptr, // data path (or null for default)
                        0x8000,   // espeakINITIALIZE_DONT_EXIT
                    );

                    if rate <= 0 {
                        return Err(TtsError::SynthesisFailed(
                            "espeak_Initialize failed (is espeak-ng-data installed?)".to_string(),
                        ));
                    }

                    // Set the synthesis callback
                    espeak_rs_sys::espeak_SetSynthCallback(Some(synth_callback));

                    rate as u32
                };

                info!("espeak-ng initialized with sample rate: {}Hz", sample_rate);

                Ok(Mutex::new(EspeakState {
                    sample_rate,
                    initialized: true,
                }))
            })
            .map_err(|e| TtsError::SynthesisFailed(format!("Failed to init espeak-ng: {}", e)))?;

        // Verify it's initialized
        let guard = state
            .lock()
            .map_err(|e| TtsError::SynthesisFailed(format!("espeak-ng lock poisoned: {}", e)))?;
        if !guard.initialized {
            return Err(TtsError::NotAvailable);
        }

        Ok(Self { _private: () })
    }

    /// Map TtsSettings rate (0.0..1.0, 0.5=normal) to espeak-ng rate (80..450, 175=normal)
    fn map_rate(rate: f32) -> c_int {
        // Map 0.0..1.0 to 80..450 with 0.5 -> 175
        let rate = rate.clamp(0.0, 1.0);
        if rate <= 0.5 {
            // 0.0 -> 80, 0.5 -> 175
            let t = rate / 0.5;
            (80.0 + t * 95.0) as c_int
        } else {
            // 0.5 -> 175, 1.0 -> 450
            let t = (rate - 0.5) / 0.5;
            (175.0 + t * 275.0) as c_int
        }
    }

    /// Map TtsSettings pitch (0.5..2.0, 1.0=normal) to espeak-ng pitch (0..99, 50=normal)
    fn map_pitch(pitch: f32) -> c_int {
        // Map 0.5..2.0 to 0..99 with 1.0 -> 50
        let pitch = pitch.clamp(0.5, 2.0);
        let t = (pitch - 0.5) / 1.5;
        (t * 99.0) as c_int
    }

    /// Map TtsSettings volume (0.0..1.0) to espeak-ng volume (0..200, 100=normal)
    fn map_volume(volume: f32) -> c_int {
        let volume = volume.clamp(0.0, 1.0);
        (volume * 200.0) as c_int
    }

    fn backend_voice_name(voice_id: &str) -> &str {
        voice_id.strip_prefix("espeak:").unwrap_or(voice_id)
    }

    fn markers_from_native(
        text: &str,
        native_markers: &[EspeakNativeMarker],
        sample_rate: u32,
        frame_count: u64,
    ) -> Vec<SynthesisMarker> {
        let character_boundaries = text
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let sentence_positions = native_markers
            .iter()
            .filter(|marker| marker.kind == EspeakNativeMarkerKind::Sentence)
            .map(|marker| marker.text_position)
            .collect::<Vec<_>>();
        native_markers
            .iter()
            .filter_map(|native| {
                let character_length = match native.kind {
                    EspeakNativeMarkerKind::Word => native.text_length,
                    EspeakNativeMarkerKind::Sentence => sentence_positions
                        .iter()
                        .copied()
                        .filter(|position| *position > native.text_position)
                        .min()
                        .unwrap_or(character_boundaries.len())
                        .saturating_sub(native.text_position),
                };
                let (text_start, text_length) = utf8_range_for_character_boundaries(
                    &character_boundaries,
                    native.text_position,
                    character_length,
                )?;
                let frame_offset = native
                    .audio_position_ms
                    .saturating_mul(u64::from(sample_rate))
                    .saturating_add(500)
                    / 1_000;
                Some(SynthesisMarker {
                    kind: match native.kind {
                        EspeakNativeMarkerKind::Word => SynthesisMarkerKind::Word,
                        EspeakNativeMarkerKind::Sentence => SynthesisMarkerKind::Sentence,
                    },
                    frame_offset: frame_offset.min(frame_count),
                    text_start: Some(text_start),
                    text_length: Some(text_length),
                    value: None,
                })
            })
            .collect()
    }

    fn default_voice_id(
        voices: &[VoiceDescriptor],
        runtime_default: Option<String>,
    ) -> Option<String> {
        voices
            .iter()
            .find(|voice| voice.language.as_deref() == Some("en-us"))
            .or_else(|| {
                runtime_default.as_deref().and_then(|runtime_default| {
                    voices
                        .iter()
                        .find(|voice| voice.id.voice_id == runtime_default)
                })
            })
            .or_else(|| voices.first())
            .map(|voice| voice.id.voice_id.clone())
    }
}

impl TtsEngine for EspeakTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        let voices = self
            .available_voices()
            .into_iter()
            .map(|voice| VoiceDescriptor::from_voice_info("espeak", voice))
            .collect::<Vec<_>>();
        let (version, runtime_default) = Self::runtime_metadata();
        let default_voice_id = Self::default_voice_id(&voices, runtime_default);

        EngineDescriptor {
            id: "espeak".to_owned(),
            display_name: "eSpeak NG".to_owned(),
            version,
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: Self::capabilities(),
            voices,
            default_voice_id,
        }
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let text = request.text.as_str();
        let settings = &request.settings;
        if text.is_empty() {
            return Ok(SynthesisResult::audio("espeak", None, AudioBuffer::empty()));
        }

        let voice_id = request.voice_id_for_engine("espeak")?.to_owned();

        let state = ESPEAK_LOCK.get().ok_or(TtsError::NotAvailable)?;
        let state_guard = state
            .lock()
            .map_err(|e| TtsError::SynthesisFailed(format!("espeak-ng lock poisoned: {}", e)))?;

        let sample_rate = state_guard.sample_rate;

        debug!(
            "espeak-ng synthesizing: {} (rate: {}, pitch: {}, volume: {})",
            text, settings.rate, settings.pitch, settings.volume
        );

        let actual_voice;
        unsafe {
            // Set voice
            let voice_name = Self::backend_voice_name(&voice_id);
            let voice_cstr = CString::new(voice_name)
                .map_err(|_| TtsError::InvalidParameter("Invalid voice name".to_string()))?;
            let voice_result = espeak_rs_sys::espeak_SetVoiceByName(voice_cstr.as_ptr());
            if voice_result != espeak_rs_sys::espeak_ERROR_EE_OK {
                return Err(TtsError::VoiceNotFound(voice_id));
            }
            let current_voice = espeak_rs_sys::espeak_GetCurrentVoice();
            actual_voice = if current_voice.is_null() {
                None
            } else {
                let current_voice = &*current_voice;
                let identifier = if current_voice.identifier.is_null() {
                    current_voice.name
                } else {
                    current_voice.identifier
                };
                (!identifier.is_null()).then(|| {
                    PhysicalVoiceId::new(
                        "espeak",
                        format!("espeak:{}", CStr::from_ptr(identifier).to_string_lossy()),
                    )
                })
            };

            // Set parameters
            espeak_rs_sys::espeak_SetParameter(
                espeak_rs_sys::espeak_PARAMETER_espeakRATE,
                Self::map_rate(settings.rate),
                0, // absolute
            );
            espeak_rs_sys::espeak_SetParameter(
                espeak_rs_sys::espeak_PARAMETER_espeakPITCH,
                Self::map_pitch(settings.pitch),
                0,
            );
            espeak_rs_sys::espeak_SetParameter(
                espeak_rs_sys::espeak_PARAMETER_espeakVOLUME,
                Self::map_volume(settings.volume),
                0,
            );

            let text_cstr = CString::new(text)
                .map_err(|_| TtsError::SynthesisFailed("Text contains null bytes".to_string()))?;
            let text_len = text_cstr.as_bytes_with_nul().len();

            // Prepare the synthesis capture before eSpeak starts invoking the callback.
            {
                let mut capture = SYNTH_CAPTURE.lock().map_err(|e| {
                    TtsError::SynthesisFailed(format!("Capture lock poisoned: {}", e))
                })?;
                *capture = Some(EspeakSynthesisCapture::default());
            }

            // Synthesize
            let result = espeak_rs_sys::espeak_Synth(
                text_cstr.as_ptr() as *const c_void,
                text_len,
                0,                    // start position
                0,                    // POS_CHARACTER
                0,                    // end position (0 = all)
                0x1000,               // espeakCHARS_UTF8
                std::ptr::null_mut(), // unique identifier
                std::ptr::null_mut(), // user data
            );

            if result != espeak_rs_sys::espeak_ERROR_EE_OK {
                if let Ok(mut capture) = SYNTH_CAPTURE.lock() {
                    capture.take();
                }
                return Err(TtsError::SynthesisFailed(format!(
                    "espeak_Synth failed with error: {}",
                    result
                )));
            }

            // Wait for synthesis to complete
            espeak_rs_sys::espeak_Synchronize();
        }

        // Extract the collected audio and native synchronization events.
        let capture = {
            let mut capture = SYNTH_CAPTURE
                .lock()
                .map_err(|e| TtsError::SynthesisFailed(format!("Capture lock poisoned: {}", e)))?;
            capture.take().unwrap_or_default()
        };

        if capture.samples.is_empty() {
            debug!("espeak-ng produced no audio");
            return Ok(SynthesisResult::audio(
                "espeak",
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        debug!(
            "espeak-ng produced {} i16 samples at {}Hz (mono)",
            capture.samples.len(),
            sample_rate
        );

        let markers = Self::markers_from_native(
            text,
            &capture.markers,
            sample_rate,
            capture.samples.len() as u64,
        );
        SynthesisResult::from_native_i16(
            "espeak",
            actual_voice,
            &capture.samples,
            sample_rate,
            1,
            markers,
            Vec::new(),
        )
    }

    fn stop(&self) {
        debug!("espeak-ng: stopping synthesis");
        // espeak_Cancel() is designed to be called from any thread to interrupt
        // ongoing synthesis. We must NOT acquire ESPEAK_LOCK here because
        // synthesize() holds it for the entire duration -- acquiring it in stop()
        // would deadlock when called from the reader thread while the worker is
        // synthesizing.
        unsafe {
            espeak_rs_sys::espeak_Cancel();
        }
    }

    fn is_speaking(&self) -> bool {
        if let Some(state) = ESPEAK_LOCK.get() {
            if let Ok(_guard) = state.lock() {
                return unsafe { espeak_rs_sys::espeak_IsPlaying() != 0 };
            }
        }
        false
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        let state = match ESPEAK_LOCK.get() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let _guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };

        let mut voices = Vec::new();

        unsafe {
            let voice_list = espeak_rs_sys::espeak_ListVoices(std::ptr::null_mut());
            if voice_list.is_null() {
                return voices;
            }

            let mut i = 0;
            loop {
                let voice_ptr = *voice_list.offset(i);
                if voice_ptr.is_null() {
                    break;
                }
                i += 1;

                let voice = &*voice_ptr;

                let name = if !voice.name.is_null() {
                    CStr::from_ptr(voice.name).to_string_lossy().to_string()
                } else {
                    continue;
                };

                let identifier = if !voice.identifier.is_null() {
                    CStr::from_ptr(voice.identifier)
                        .to_string_lossy()
                        .to_string()
                } else {
                    name.clone()
                };

                let language = if !voice.languages.is_null() {
                    // The languages field is a string with priority byte prefix
                    // Format: priority_byte + language_string, null-separated
                    let lang_ptr = voice.languages.offset(1); // skip priority byte
                    CStr::from_ptr(lang_ptr).to_string_lossy().to_string()
                } else {
                    String::new()
                };

                voices.push(VoiceInfo {
                    identifier: format!("espeak:{}", identifier),
                    name,
                    language,
                    quality: VoiceQuality::Compact,
                });
            }
        }

        debug!("espeak-ng found {} voices", voices.len());
        voices
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        let search = identifier.strip_prefix("espeak:").unwrap_or(identifier);
        self.available_voices().into_iter().find(|v| {
            v.identifier == identifier
                || v.identifier == format!("espeak:{}", search)
                || v.name == search
                || v.language == search
        })
    }
}

/// Convert eSpeak's one-based Unicode character range to Omnivox's UTF-8 byte range.
#[cfg(test)]
fn utf8_range_for_characters(
    text: &str,
    one_based_start: usize,
    character_length: usize,
) -> Option<(u32, u32)> {
    let character_boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    utf8_range_for_character_boundaries(&character_boundaries, one_based_start, character_length)
}

fn utf8_range_for_character_boundaries(
    character_boundaries: &[usize],
    one_based_start: usize,
    character_length: usize,
) -> Option<(u32, u32)> {
    let start_character = one_based_start.checked_sub(1)?;
    let end_character = start_character.checked_add(character_length)?;
    let start = *character_boundaries.get(start_character)?;
    let end = *character_boundaries.get(end_character)?;
    Some((u32::try_from(start).ok()?, u32::try_from(end - start).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_mapping() {
        // 0.0 -> 80
        assert_eq!(EspeakTtsEngine::map_rate(0.0), 80);
        // 0.5 -> 175
        assert_eq!(EspeakTtsEngine::map_rate(0.5), 175);
        // 1.0 -> 450
        assert_eq!(EspeakTtsEngine::map_rate(1.0), 450);
        // clamping
        assert_eq!(EspeakTtsEngine::map_rate(-1.0), 80);
        assert_eq!(EspeakTtsEngine::map_rate(2.0), 450);
    }

    #[test]
    fn test_pitch_mapping() {
        // 0.5 -> 0
        assert_eq!(EspeakTtsEngine::map_pitch(0.5), 0);
        // 1.0 -> 33 (approximately 50 * 0.5/1.5 * 2)
        let mid = EspeakTtsEngine::map_pitch(1.0);
        assert!(mid > 20 && mid < 45, "mid pitch {} should be ~33", mid);
        // 2.0 -> 99
        assert_eq!(EspeakTtsEngine::map_pitch(2.0), 99);
    }

    #[test]
    fn test_espeak_descriptor_is_self_consistent() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let descriptor = engine.descriptor();

        assert_eq!(descriptor.id, "espeak");
        assert!(descriptor.can_synthesize());
        assert!(descriptor.capabilities.acss.rate);
        assert!(descriptor.capabilities.acss.average_pitch);
        assert!(descriptor.capabilities.markers.word);
        assert!(descriptor.capabilities.markers.sentence);
        assert_eq!(
            descriptor.capabilities.markers.requested_anchors,
            AnchorSupport::WordBoundary
        );
        assert!(descriptor
            .voices
            .iter()
            .all(|voice| voice.id.engine_id == descriptor.id));
        let default_voice_id = descriptor.default_voice_id.unwrap();
        let default_voice = descriptor
            .voices
            .iter()
            .find(|voice| voice.id.voice_id == default_voice_id)
            .unwrap();
        assert_eq!(default_voice.language.as_deref(), Some("en-us"));
    }

    #[test]
    fn test_volume_mapping() {
        assert_eq!(EspeakTtsEngine::map_volume(0.0), 0);
        assert_eq!(EspeakTtsEngine::map_volume(0.5), 100);
        assert_eq!(EspeakTtsEngine::map_volume(1.0), 200);
    }

    #[test]
    fn test_backend_voice_name_accepts_structured_ids() {
        assert_eq!(
            EspeakTtsEngine::backend_voice_name(r"espeak:gmw\en-US"),
            r"gmw\en-US"
        );
        assert_eq!(EspeakTtsEngine::backend_voice_name("en"), "en");
    }

    #[test]
    fn test_espeak_initialization() {
        // This test verifies that espeak-ng can be initialized
        let engine = EspeakTtsEngine::new();
        assert!(
            engine.is_ok(),
            "Failed to initialize espeak-ng: {:?}",
            engine.err()
        );
    }

    #[test]
    fn test_espeak_synthesize_produces_audio() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let settings = TtsSettings::default();

        let result = engine
            .synthesize(&SynthesisRequest::new("hello world", settings))
            .expect("Synthesis failed");

        assert!(!result.audio.is_empty(), "Buffer should not be empty");
        assert_eq!(result.audio.sample_rate(), crate::STANDARD_SAMPLE_RATE);
        assert_eq!(result.audio.channels(), crate::STANDARD_CHANNELS);
        assert!(result.audio.duration() > 0.0, "Duration should be positive");
        assert_eq!(result.engine_id, "espeak");
        assert!(result.actual_voice.is_some());
        result
            .validate(&SynthesisRequest::new(
                "hello world",
                TtsSettings::default(),
            ))
            .unwrap();
        let words = result
            .markers
            .iter()
            .filter(|marker| marker.kind == SynthesisMarkerKind::Word)
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 2);
        assert_eq!(
            (words[0].text_start, words[0].text_length),
            (Some(0), Some(5))
        );
        assert_eq!(
            (words[1].text_start, words[1].text_length),
            (Some(6), Some(5))
        );
        assert!(words[0].frame_offset <= words[1].frame_offset);
        assert!(result
            .markers
            .iter()
            .any(|marker| marker.kind == SynthesisMarkerKind::Sentence));
    }

    #[test]
    #[ignore = "long-session synthesis stress test"]
    fn stress_repeated_synthesis_session() {
        const ITERATIONS: usize = 100;
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let texts = [
            "First sentence has several words. Second sentence checks markers!",
            "Unicode café and naïve words work here. Another sentence follows?",
            "A short clause, followed by another clause; then the sentence ends.",
        ];
        let mut total_frames = 0_usize;
        for iteration in 0..ITERATIONS {
            let settings = TtsSettings {
                rate: [0.35, 0.5, 0.7][iteration % 3],
                pitch: [0.8, 1.0, 1.2][iteration % 3],
                volume: [0.35, 0.65, 1.0][iteration % 3],
                ..TtsSettings::default()
            };
            let request = SynthesisRequest::new(texts[iteration % texts.len()], settings);
            let result = engine.synthesize(&request).expect("stress synthesis failed");

            result.validate(&request).expect("stress result was invalid");
            assert!(!result.audio.is_empty());
            assert_eq!(result.audio.sample_rate(), crate::STANDARD_SAMPLE_RATE);
            assert_eq!(result.audio.channels(), crate::STANDARD_CHANNELS);
            assert!(result
                .markers
                .iter()
                .any(|marker| marker.kind == SynthesisMarkerKind::Word));
            assert!(result
                .markers
                .iter()
                .any(|marker| marker.kind == SynthesisMarkerKind::Sentence));
            total_frames += result.audio.frame_count();
        }
        assert!(total_frames > ITERATIONS * crate::STANDARD_SAMPLE_RATE as usize);
    }

    #[test]
    fn native_character_ranges_become_utf8_byte_ranges() {
        assert_eq!(utf8_range_for_characters("héllo 世界", 1, 5), Some((0, 6)));
        assert_eq!(utf8_range_for_characters("héllo 世界", 7, 2), Some((7, 6)));
        assert_eq!(utf8_range_for_characters("héllo 世界", 9, 0), Some((13, 0)));
        assert_eq!(utf8_range_for_characters("hello", 0, 1), None);
        assert_eq!(utf8_range_for_characters("hello", 6, 1), None);
    }

    #[test]
    fn native_markers_include_unicode_words_and_sentence_spans() {
        let text = "Héllo world. Next sentence.";
        let native = [
            EspeakNativeMarker {
                kind: EspeakNativeMarkerKind::Sentence,
                text_position: 1,
                text_length: 0,
                audio_position_ms: 0,
            },
            EspeakNativeMarker {
                kind: EspeakNativeMarkerKind::Word,
                text_position: 1,
                text_length: 5,
                audio_position_ms: 10,
            },
            EspeakNativeMarker {
                kind: EspeakNativeMarkerKind::Sentence,
                text_position: 14,
                text_length: 0,
                audio_position_ms: 800,
            },
        ];

        let markers = EspeakTtsEngine::markers_from_native(text, &native, 22_050, 44_100);

        assert_eq!(
            (markers[0].text_start, markers[0].text_length),
            (Some(0), Some(14))
        );
        assert_eq!(
            (markers[1].text_start, markers[1].text_length),
            (Some(0), Some(6))
        );
        assert_eq!(
            (markers[2].text_start, markers[2].text_length),
            (Some(14), Some(14))
        );
        assert_eq!(markers[2].frame_offset, 17_640);
    }

    #[test]
    fn test_espeak_empty_text() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let settings = TtsSettings::default();

        let result = engine
            .synthesize(&SynthesisRequest::new("", settings))
            .expect("Synthesis should succeed for empty text");

        assert!(result.audio.is_empty());
    }

    #[test]
    fn test_espeak_missing_voice_is_reported() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let settings = TtsSettings {
            voice: "omnivox-missing-voice".to_owned(),
            ..TtsSettings::default()
        };

        let error = engine
            .synthesize(&SynthesisRequest::new("test", settings))
            .unwrap_err();

        assert!(matches!(error, TtsError::VoiceNotFound(_)));
    }

    #[test]
    fn test_espeak_voice_listing() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let voices = engine.available_voices();

        assert!(!voices.is_empty(), "Should have at least one voice");

        // Check that English is available
        let has_english = voices.iter().any(|v| v.language.starts_with("en"));
        assert!(has_english, "Should have an English voice");
    }

    #[test]
    fn test_espeak_settings() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");

        // Test with various settings
        let settings = TtsSettings {
            voice: "en".to_string(),
            rate: 0.3,
            pitch: 1.5,
            volume: 0.8,
        };

        let result = engine
            .synthesize(&SynthesisRequest::new("test", settings))
            .expect("Synthesis with custom settings failed");

        assert!(!result.audio.is_empty());
    }
}
