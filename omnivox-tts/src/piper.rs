//! Piper Neural TTS Engine
//!
//! Cross-platform TTS backend using piper (https://github.com/rhasspy/piper),
//! a fast neural text-to-speech system powered by ONNX Runtime and
//! espeak-ng for phonemization.
//!
//! Piper models are per-voice `.onnx` files paired with a `.onnx.json` config.
//! Download models from: https://github.com/rhasspy/piper/blob/master/VOICES.md
//!
//! # Configuration
//!
//! - Model path: `OMNIVOX_PIPER_MODEL` env var (required when using this engine)
//! - espeak data path: `OMNIVOX_PIPER_ESPEAK_DATA` overrides auto-discovery
//!
//! # Thread Safety
//!
//! `PiperState*` is a C++ object accessed through a `Mutex`. All synthesis
//! calls are serialized. `stop()` is a no-op because piper synthesis is
//! synchronous (cancellation requires killing the ongoing call, which the
//! bridge does not currently support).

use crate::{AudioBuffer, TtsEngine, TtsError, TtsSettings, VoiceInfo, VoiceQuality};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::{debug, info, warn};

/// espeak-ng data path discovered at build time by omnivox-piper-sys/build.rs.
/// Exposed as a pub const in the sys crate so we can reference it here.
use omnivox_piper_sys::PIPER_ESPEAK_DATA_DIR;

/// Piper neural TTS engine.
///
/// Wraps piper's C++ API via `omnivox_piper_sys`. A single voice model is
/// loaded at construction time; voice switching requires creating a new engine.
pub struct PiperTtsEngine {
    /// Raw pointer to piper's C++ state object, protected by a mutex.
    // SAFETY: PiperState is not thread-safe on its own, so we serialize all
    // accesses through this mutex. The pointer is non-null after construction.
    state: Mutex<*mut omnivox_piper_sys::PiperState>,
    /// Display name derived from the model filename.
    voice_name: String,
    /// The model path, kept for voice listing (language extraction etc.).
    #[allow(dead_code)]
    model_path: PathBuf,
}

// SAFETY: All accesses to PiperState* are serialized through the Mutex.
unsafe impl Send for PiperTtsEngine {}
unsafe impl Sync for PiperTtsEngine {}

impl Drop for PiperTtsEngine {
    fn drop(&mut self) {
        if let Ok(ptr) = self.state.lock() {
            unsafe { omnivox_piper_sys::piper_destroy(*ptr) };
        }
    }
}

impl PiperTtsEngine {
    /// Create a new piper TTS engine loading the given `.onnx` model file.
    ///
    /// The JSON config is expected alongside the model as either
    /// `<model>.onnx.json` or `<model>.json`.
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self, TtsError> {
        let model_path = model_path.as_ref().to_path_buf();

        if !model_path.exists() {
            return Err(TtsError::VoiceNotFound(format!(
                "Piper model not found: {}",
                model_path.display()
            )));
        }

        let config_path = find_config_path(&model_path).ok_or_else(|| {
            TtsError::VoiceNotFound(format!(
                "Piper config (.onnx.json or .json) not found alongside {}",
                model_path.display()
            ))
        })?;

        let espeak_data = find_espeak_data().ok_or_else(|| {
            TtsError::SynthesisFailed(
                "Cannot find espeak-ng data directory for piper phonemizer. \
                 Set OMNIVOX_PIPER_ESPEAK_DATA to the directory containing espeak-ng-data/."
                    .to_string(),
            )
        })?;

        debug!(
            "Initializing piper: model={} config={} espeak={}",
            model_path.display(),
            config_path.display(),
            espeak_data
        );

        let espeak_cstr = CString::new(espeak_data.as_str())
            .map_err(|_| TtsError::InvalidParameter("Invalid espeak data path".to_string()))?;

        let state_ptr =
            unsafe { omnivox_piper_sys::piper_init(espeak_cstr.as_ptr()) };
        if state_ptr.is_null() {
            return Err(TtsError::SynthesisFailed(
                "piper_init failed (check espeak-ng data path)".to_string(),
            ));
        }

        let model_cstr =
            CString::new(model_path.to_string_lossy().as_ref())
                .map_err(|_| TtsError::InvalidParameter("Invalid model path".to_string()))?;
        let config_cstr =
            CString::new(config_path.to_string_lossy().as_ref())
                .map_err(|_| TtsError::InvalidParameter("Invalid config path".to_string()))?;

        let ret = unsafe {
            omnivox_piper_sys::piper_load_voice(
                state_ptr,
                model_cstr.as_ptr(),
                config_cstr.as_ptr(),
            )
        };
        if ret != 0 {
            unsafe { omnivox_piper_sys::piper_destroy(state_ptr) };
            return Err(TtsError::VoiceNotFound(format!(
                "Failed to load piper voice from {}",
                model_path.display()
            )));
        }

        let voice_name = model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("piper")
            .to_string();

        info!("Piper TTS engine ready: {}", voice_name);

        Ok(Self {
            state: Mutex::new(state_ptr),
            voice_name,
            model_path,
        })
    }

    /// Create from the `OMNIVOX_PIPER_MODEL` environment variable.
    pub fn from_env() -> Result<Self, TtsError> {
        let model = std::env::var("OMNIVOX_PIPER_MODEL").map_err(|_| {
            TtsError::NotAvailable
        })?;
        if model.is_empty() {
            return Err(TtsError::NotAvailable);
        }
        Self::new(&model)
    }

    /// Map TtsSettings.rate (0.0..1.0, 0.5=normal) to piper length_scale.
    ///
    /// Rate convention (matches espeak/macOS): 0.0=slow, 0.5=normal, 1.0=fast.
    /// piper length_scale: 1.0=normal, <1.0=faster, >1.0=slower (inverse of rate).
    /// Mapping: 0.0 -> 2.0 (slowest), 0.5 -> 1.0 (normal), 1.0 -> 0.5 (fastest).
    fn map_rate_to_length_scale(rate: f32) -> f32 {
        let rate = rate.clamp(0.0, 1.0);
        if rate >= 0.5 {
            // 0.5 -> 1.0 (normal), 1.0 -> 0.5 (fastest)
            1.5 - rate
        } else {
            // 0.0 -> 2.0 (slowest), 0.5 -> 1.0 (normal)
            1.0 + (0.5 - rate) * 2.0
        }
    }
}

impl TtsEngine for PiperTtsEngine {
    fn synthesize(&self, text: &str, settings: &TtsSettings) -> Result<AudioBuffer, TtsError> {
        if text.is_empty() {
            return Ok(AudioBuffer::empty());
        }

        let ptr = self.state.lock().map_err(|e| {
            TtsError::SynthesisFailed(format!("piper state lock poisoned: {}", e))
        })?;

        let length_scale = Self::map_rate_to_length_scale(settings.rate);
        // piper noise parameters use standard defaults
        let noise_scale: f32 = 0.667;
        let noise_w: f32 = 0.8;

        debug!(
            "piper synthesizing: {} chars (length_scale={:.2})",
            text.len(),
            length_scale
        );

        let text_cstr = CString::new(text)
            .map_err(|_| TtsError::SynthesisFailed("Text contains null bytes".to_string()))?;

        let mut num_samples: u32 = 0;
        let mut sample_rate: u32 = 0;

        let audio_ptr = unsafe {
            omnivox_piper_sys::piper_synthesize(
                *ptr,
                text_cstr.as_ptr(),
                length_scale,
                noise_scale,
                noise_w,
                &mut num_samples,
                &mut sample_rate,
            )
        };

        if audio_ptr.is_null() || num_samples == 0 {
            debug!("piper produced no audio");
            return Ok(AudioBuffer::empty());
        }

        let i16_samples =
            unsafe { std::slice::from_raw_parts(audio_ptr, num_samples as usize).to_vec() };

        unsafe { omnivox_piper_sys::piper_free_audio(audio_ptr) };

        debug!(
            "piper produced {} samples at {}Hz (mono i16)",
            num_samples, sample_rate
        );

        let buffer = AudioBuffer::from_i16(&i16_samples, sample_rate, 1);
        Ok(buffer.to_standard_format())
    }

    fn stop(&self) {
        // Piper synthesis is synchronous; there is no cancel mechanism in the
        // current bridge. The synthesis worker will complete the current chunk
        // and the generation counter will prevent it from playing the result.
        debug!("piper: stop requested (no-op — synthesis is synchronous)");
    }

    fn is_speaking(&self) -> bool {
        false
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        vec![VoiceInfo {
            identifier: format!("piper:{}", self.voice_name),
            name: self.voice_name.clone(),
            language: extract_language_from_name(&self.voice_name),
            quality: VoiceQuality::Enhanced,
        }]
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|v| v.identifier == identifier || v.name == identifier)
    }
}

/// Locate the companion .json config for a piper .onnx model file.
///
/// Piper uses two naming conventions:
/// - `en_US-lessac-medium.onnx` + `en_US-lessac-medium.onnx.json`
/// - `model.onnx` + `model.json`
fn find_config_path(model_path: &Path) -> Option<PathBuf> {
    // Preferred: <name>.onnx.json
    let with_onnx_json = {
        let mut p = model_path.as_os_str().to_owned();
        p.push(".json");
        PathBuf::from(p)
    };
    if with_onnx_json.exists() {
        return Some(with_onnx_json);
    }

    // Fallback: <name>.json (replaces .onnx extension)
    let with_json = model_path.with_extension("json");
    if with_json.exists() {
        return Some(with_json);
    }

    None
}

/// Find the espeak-ng data parent directory for piper's phonemizer.
///
/// Search order:
/// 1. `OMNIVOX_PIPER_ESPEAK_DATA` env var
/// 2. `ESPEAK_NG_DATA` env var (shared with the espeak TTS backend)
/// 3. Build-time path captured by omnivox-piper-sys/build.rs
/// 4. Well-known system paths
fn find_espeak_data() -> Option<String> {
    // 1. Piper-specific override
    if let Ok(dir) = std::env::var("OMNIVOX_PIPER_ESPEAK_DATA") {
        if !dir.is_empty() && espeak_data_valid(&dir) {
            debug!("Using espeak data from OMNIVOX_PIPER_ESPEAK_DATA: {}", dir);
            return Some(dir);
        }
    }

    // 2. Shared espeak env var
    if let Ok(dir) = std::env::var("ESPEAK_NG_DATA") {
        if !dir.is_empty() && espeak_data_valid(&dir) {
            debug!("Using espeak data from ESPEAK_NG_DATA: {}", dir);
            return Some(dir);
        }
    }

    // 3. Build-time path (piper-phonemize bundled data)
    if !PIPER_ESPEAK_DATA_DIR.is_empty() && espeak_data_valid(PIPER_ESPEAK_DATA_DIR) {
        debug!(
            "Using espeak data from build path: {}",
            PIPER_ESPEAK_DATA_DIR
        );
        return Some(PIPER_ESPEAK_DATA_DIR.to_string());
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
        if espeak_data_valid(candidate) {
            debug!("Found espeak data at: {}", candidate);
            return Some(candidate.to_string());
        }
    }

    warn!("Could not find espeak-ng data directory for piper");
    None
}

/// Check that `dir/espeak-ng-data/phontab` exists (valid espeak-ng data parent).
fn espeak_data_valid(dir: &str) -> bool {
    std::path::Path::new(dir)
        .join("espeak-ng-data")
        .join("phontab")
        .exists()
}

/// Try to extract a BCP-47 language tag from the piper model filename.
///
/// Piper model names follow the pattern `<lang>-<voice>-<quality>`, e.g.
/// `en_US-lessac-medium` → "en-US".
fn extract_language_from_name(name: &str) -> String {
    // Split on '-', check if the first part looks like a language code
    if let Some(lang_part) = name.split('-').next() {
        // Convert underscore separator to hyphen: en_US -> en-US
        let normalized = lang_part.replace('_', "-");
        if normalized.len() >= 2 {
            return normalized;
        }
    }
    String::from("en")
}

/// Return the piper library version string, if available.
pub fn piper_version() -> String {
    let ptr = unsafe { omnivox_piper_sys::piper_version() };
    if ptr.is_null() {
        return String::from("unknown");
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_mapping() {
        // rate 0.0 (slow) -> length_scale 2.0 (slowest/longest)
        assert!((PiperTtsEngine::map_rate_to_length_scale(0.0) - 2.0).abs() < 0.001);
        // rate 0.5 (normal) -> length_scale 1.0 (normal)
        assert!((PiperTtsEngine::map_rate_to_length_scale(0.5) - 1.0).abs() < 0.001);
        // rate 1.0 (fast) -> length_scale 0.5 (fastest/shortest)
        assert!((PiperTtsEngine::map_rate_to_length_scale(1.0) - 0.5).abs() < 0.001);
        // Clamping
        assert!((PiperTtsEngine::map_rate_to_length_scale(-1.0) - 2.0).abs() < 0.001);
        assert!((PiperTtsEngine::map_rate_to_length_scale(2.0) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_find_config_path_missing() {
        let p = Path::new("/nonexistent/model.onnx");
        assert!(find_config_path(p).is_none());
    }

    #[test]
    fn test_extract_language_from_name() {
        assert_eq!(extract_language_from_name("en_US-lessac-medium"), "en-US");
        assert_eq!(extract_language_from_name("de_DE-thorsten-low"), "de-DE");
        assert_eq!(extract_language_from_name("fr-upmc-medium"), "fr");
        assert_eq!(extract_language_from_name("en"), "en");
    }

    #[test]
    fn test_espeak_data_valid_false() {
        assert!(!espeak_data_valid("/nonexistent/path"));
    }
}
