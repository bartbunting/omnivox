//! espeak-ng TTS Engine
//!
//! Cross-platform TTS backend using espeak-ng. Always compiled in as the
//! guaranteed fallback engine that works on all platforms.

use crate::{AudioBuffer, TtsEngine, TtsError, TtsSettings, VoiceInfo, VoiceQuality};
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

/// Thread-local storage for collecting audio samples during synthesis callback
static SYNTH_BUFFER: Mutex<Option<Vec<i16>>> = Mutex::new(None);

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
    _events: *mut espeak_rs_sys::espeak_EVENT,
) -> c_int {
    if wav.is_null() || sample_count <= 0 {
        return 0;
    }

    let samples =
        std::slice::from_raw_parts(wav as *const i16, sample_count as usize);

    if let Ok(mut buf) = SYNTH_BUFFER.lock() {
        if let Some(ref mut buffer) = *buf {
            buffer.extend_from_slice(samples);
        }
    }

    // Return 0 to continue synthesis, 1 to abort
    0
}

/// Data path discovered at build time by build.rs
const ESPEAK_DATA_DIR: &str = env!("ESPEAK_NG_DATA_DIR");

impl EspeakTtsEngine {
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
        let state = ESPEAK_LOCK.get_or_try_init(|| -> Result<Mutex<EspeakState>, TtsError> {
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
        }).map_err(|e| TtsError::SynthesisFailed(format!("Failed to init espeak-ng: {}", e)))?;

        // Verify it's initialized
        let guard = state.lock().map_err(|e| {
            TtsError::SynthesisFailed(format!("espeak-ng lock poisoned: {}", e))
        })?;
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
}

impl TtsEngine for EspeakTtsEngine {
    fn synthesize(&self, text: &str, settings: &TtsSettings) -> Result<AudioBuffer, TtsError> {
        if text.is_empty() {
            return Ok(AudioBuffer::empty());
        }

        let state = ESPEAK_LOCK
            .get()
            .ok_or(TtsError::NotAvailable)?;
        let state_guard = state.lock().map_err(|e| {
            TtsError::SynthesisFailed(format!("espeak-ng lock poisoned: {}", e))
        })?;

        let sample_rate = state_guard.sample_rate;

        debug!(
            "espeak-ng synthesizing: {} (rate: {}, pitch: {}, volume: {})",
            text, settings.rate, settings.pitch, settings.volume
        );

        unsafe {
            // Set voice
            let voice_cstr = CString::new(settings.voice.as_str()).map_err(|_| {
                TtsError::InvalidParameter("Invalid voice name".to_string())
            })?;
            espeak_rs_sys::espeak_SetVoiceByName(voice_cstr.as_ptr());

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

            // Prepare the synthesis buffer
            {
                let mut buf = SYNTH_BUFFER.lock().map_err(|e| {
                    TtsError::SynthesisFailed(format!("Buffer lock poisoned: {}", e))
                })?;
                *buf = Some(Vec::new());
            }

            // Synthesize
            let text_cstr = CString::new(text).map_err(|_| {
                TtsError::SynthesisFailed("Text contains null bytes".to_string())
            })?;
            let text_len = text_cstr.as_bytes_with_nul().len();

            let result = espeak_rs_sys::espeak_Synth(
                text_cstr.as_ptr() as *const c_void,
                text_len,
                0,     // start position
                0,     // POS_CHARACTER
                0,     // end position (0 = all)
                0x1000, // espeakCHARS_UTF8
                std::ptr::null_mut(), // unique identifier
                std::ptr::null_mut(), // user data
            );

            if result != espeak_rs_sys::espeak_ERROR_EE_OK {
                return Err(TtsError::SynthesisFailed(format!(
                    "espeak_Synth failed with error: {}",
                    result
                )));
            }

            // Wait for synthesis to complete
            espeak_rs_sys::espeak_Synchronize();
        }

        // Extract the collected audio samples
        let i16_samples = {
            let mut buf = SYNTH_BUFFER.lock().map_err(|e| {
                TtsError::SynthesisFailed(format!("Buffer lock poisoned: {}", e))
            })?;
            buf.take().unwrap_or_default()
        };

        if i16_samples.is_empty() {
            debug!("espeak-ng produced no audio");
            return Ok(AudioBuffer::empty());
        }

        debug!(
            "espeak-ng produced {} i16 samples at {}Hz (mono)",
            i16_samples.len(),
            sample_rate
        );

        // Convert i16 mono to f32, then to standard format (stereo @ 44100Hz)
        let buffer = AudioBuffer::from_i16(&i16_samples, sample_rate, 1);
        Ok(buffer.to_standard_format())
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
        self.available_voices()
            .into_iter()
            .find(|v| {
                v.identifier == identifier
                    || v.identifier == format!("espeak:{}", search)
                    || v.name == search
                    || v.language == search
            })
    }
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
    fn test_volume_mapping() {
        assert_eq!(EspeakTtsEngine::map_volume(0.0), 0);
        assert_eq!(EspeakTtsEngine::map_volume(0.5), 100);
        assert_eq!(EspeakTtsEngine::map_volume(1.0), 200);
    }

    #[test]
    fn test_espeak_initialization() {
        // This test verifies that espeak-ng can be initialized
        let engine = EspeakTtsEngine::new();
        assert!(engine.is_ok(), "Failed to initialize espeak-ng: {:?}", engine.err());
    }

    #[test]
    fn test_espeak_synthesize_produces_audio() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let settings = TtsSettings::default();

        let buffer = engine
            .synthesize("hello world", &settings)
            .expect("Synthesis failed");

        assert!(!buffer.is_empty(), "Buffer should not be empty");
        assert_eq!(buffer.sample_rate, crate::STANDARD_SAMPLE_RATE);
        assert_eq!(buffer.channels, crate::STANDARD_CHANNELS);
        assert!(buffer.duration() > 0.0, "Duration should be positive");
    }

    #[test]
    fn test_espeak_empty_text() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let settings = TtsSettings::default();

        let buffer = engine
            .synthesize("", &settings)
            .expect("Synthesis should succeed for empty text");

        assert!(buffer.is_empty());
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

        let buffer = engine
            .synthesize("test", &settings)
            .expect("Synthesis with custom settings failed");

        assert!(!buffer.is_empty());
    }
}
