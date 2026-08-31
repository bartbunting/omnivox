//! Piper Neural TTS Engine
//!
//! Cross-platform TTS backend using maintained libpiper
//! (https://github.com/OHF-Voice/piper1-gpl), a fast neural text-to-speech
//! system powered by ONNX Runtime and espeak-ng for phonemization.
//!
//! Piper models are per-voice `.onnx` files paired with a `.onnx.json` config.
//! Review model licences before downloading; each upstream model has its own
//! `MODEL_CARD`.
//!
//! # Configuration
//!
//! - Model path: `OMNIVOX_PIPER_MODEL` env var (required when using this engine)
//! - espeak data path: `OMNIVOX_PIPER_ESPEAK_DATA` overrides auto-discovery
//!
//! # Thread Safety
//!
//! The opaque `piper_synthesizer` is accessed through a `Mutex`, so synthesis
//! calls are serialized. This adapter runs in `omnivox-piper-helper`, not the
//! main speech server. `stop()` is observed between libpiper audio chunks; the
//! host still retires the helper if a native chunk does not return promptly.

use crate::contracts::{
    AcssCapabilities, AudioOutputMode, Availability, CancellationSupport, ConcurrencyModel,
    EngineCapabilities, EngineDescriptor, EngineHealth, MarkerCapabilities, PhysicalVoiceId,
    VoiceDescriptor,
};
use crate::{
    AudioBuffer, SynthesisRequest, SynthesisResult, TtsEngine, TtsError, VoiceInfo, VoiceQuality,
};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tracing::{debug, info, warn};

/// espeak-ng data path discovered at build time by omnivox-piper-sys/build.rs.
/// Exposed as a pub const in the sys crate so we can reference it here.
use omnivox_piper_sys::PIPER_ESPEAK_DATA_DIR;

/// Piper neural TTS engine.
///
/// Wraps libpiper's C API via `omnivox_piper_sys`. A single voice model is
/// loaded at construction time; voice switching requires creating a new engine.
pub struct PiperTtsEngine {
    /// Raw libpiper synthesizer pointer protected by a mutex.
    // SAFETY: libpiper synthesizers are not thread-safe on their own, so all
    // native calls are serialized. The pointer is non-null after construction.
    state: Mutex<*mut omnivox_piper_sys::piper_synthesizer>,
    cancel_requested: AtomicBool,
    speaking: AtomicBool,
    /// Display name derived from the model filename.
    voice_name: String,
    /// The model path, kept for voice listing (language extraction etc.).
    #[allow(dead_code)]
    model_path: PathBuf,
}

// SAFETY: All native synthesizer accesses are serialized through the Mutex.
unsafe impl Send for PiperTtsEngine {}
unsafe impl Sync for PiperTtsEngine {}

impl Drop for PiperTtsEngine {
    fn drop(&mut self) {
        if let Ok(ptr) = self.state.lock() {
            unsafe { omnivox_piper_sys::piper_free(*ptr) };
        }
    }
}

impl PiperTtsEngine {
    fn capabilities() -> EngineCapabilities {
        EngineCapabilities {
            acss: AcssCapabilities {
                rate: true,
                ..AcssCapabilities::default()
            },
            audio_output: AudioOutputMode::BufferedPcm,
            cancellation: CancellationSupport::PlaybackOnly,
            concurrency: ConcurrencyModel::Serialized,
            markers: MarkerCapabilities::default(),
            language_switching: false,
            text_repertoire: crate::contracts::TextRepertoire::Unicode,
            post_synthesis_dimensions: crate::contracts::buffered_post_synthesis_dimensions(),
            native_extensions: Vec::new(),
        }
    }

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
            espeak_data.display()
        );

        let espeak_cstr = CString::new(espeak_data.to_string_lossy().as_bytes())
            .map_err(|_| TtsError::InvalidParameter("Invalid espeak data path".to_string()))?;

        let model_cstr = CString::new(model_path.to_string_lossy().as_ref())
            .map_err(|_| TtsError::InvalidParameter("Invalid model path".to_string()))?;
        let config_cstr = CString::new(config_path.to_string_lossy().as_ref())
            .map_err(|_| TtsError::InvalidParameter("Invalid config path".to_string()))?;

        let create_options = omnivox_piper_sys::piper_create_options {
            struct_size: std::mem::size_of::<omnivox_piper_sys::piper_create_options>(),
            model_path: model_cstr.as_ptr(),
            config_path: config_cstr.as_ptr(),
            espeak_data_path: espeak_cstr.as_ptr(),
        };
        let state_ptr = unsafe { omnivox_piper_sys::piper_create_with_options(&create_options) };
        if state_ptr.is_null() {
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
            cancel_requested: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
            voice_name,
            model_path,
        })
    }

    /// Create from the `OMNIVOX_PIPER_MODEL` environment variable.
    pub fn from_env() -> Result<Self, TtsError> {
        let model = std::env::var("OMNIVOX_PIPER_MODEL").map_err(|_| TtsError::NotAvailable)?;
        if model.is_empty() {
            return Err(TtsError::NotAvailable);
        }
        Self::new(&model)
    }

    /// Map TtsSettings.rate (0.0..2.0, 0.5=normal) to piper length_scale.
    ///
    /// Rate convention: 0.0=slowest, 0.5=normal, 1.0=fast (2x), 2.0=very fast (~10x).
    /// piper length_scale: 1.0=normal, <1.0=faster, >1.0=slower (inverse of rate).
    ///
    /// Mapping:
    ///   rate 0.0 -> length_scale 2.0  (slowest)
    ///   rate 0.5 -> length_scale 1.0  (normal)
    ///   rate 1.0 -> length_scale 0.5  (2x speed)
    ///   rate 1.5 -> length_scale 0.1  (clamped floor, ~10x speed)
    ///   rate 2.0 -> length_scale 0.1  (same floor)
    fn map_rate_to_length_scale(rate: f32) -> f32 {
        let rate = rate.clamp(0.0, 2.0);
        if rate >= 0.5 {
            // Linear: 0.5 -> 1.0 (normal), 1.0 -> 0.5 (2x fast), 1.5+ -> 0.1 floor
            (1.5 - rate).max(0.1)
        } else {
            // 0.0 -> 2.0 (slowest), 0.5 -> 1.0 (normal)
            1.0 + (0.5 - rate) * 2.0
        }
    }
}

impl TtsEngine for PiperTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        let voices: Vec<VoiceDescriptor> = self
            .available_voices()
            .into_iter()
            .map(|voice| VoiceDescriptor::from_voice_info("piper", voice))
            .collect();
        let default_voice_id = voices.first().map(|voice| voice.id.voice_id.clone());
        let version = piper_version();

        EngineDescriptor {
            id: "piper".to_owned(),
            display_name: "Piper".to_owned(),
            version: (version != "unknown").then_some(version),
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
        request.voice_id_for_engine("piper")?;
        let actual_voice = Some(PhysicalVoiceId::new(
            "piper",
            format!("piper:{}", self.voice_name),
        ));
        if text.is_empty() {
            return Ok(SynthesisResult::audio(
                "piper",
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        let ptr = self
            .state
            .lock()
            .map_err(|e| TtsError::SynthesisFailed(format!("piper state lock poisoned: {}", e)))?;
        self.cancel_requested.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);

        let length_scale = Self::map_rate_to_length_scale(settings.rate);

        debug!(
            "piper synthesizing: {} chars (length_scale={:.2})",
            text.len(),
            length_scale
        );

        let text_cstr = CString::new(text)
            .map_err(|_| TtsError::SynthesisFailed("Text contains null bytes".to_string()))?;

        let mut options = unsafe { omnivox_piper_sys::piper_default_synthesize_options(*ptr) };
        options.length_scale = length_scale;
        let start = unsafe {
            omnivox_piper_sys::piper_synthesize_start(*ptr, text_cstr.as_ptr(), &options)
        };
        if start != omnivox_piper_sys::PIPER_OK as i32 {
            return Err(TtsError::SynthesisFailed(format!(
                "libpiper could not start synthesis (status {start})"
            )));
        }

        let mut samples = Vec::new();
        let mut sample_rate = None;
        loop {
            if self.cancel_requested.load(Ordering::Acquire) {
                return Err(TtsError::SynthesisFailed(
                    "Piper synthesis was cancelled".to_owned(),
                ));
            }
            let mut chunk: omnivox_piper_sys::piper_audio_chunk = unsafe { std::mem::zeroed() };
            let status = unsafe { omnivox_piper_sys::piper_synthesize_next(*ptr, &mut chunk) };
            if status != omnivox_piper_sys::PIPER_OK as i32
                && status != omnivox_piper_sys::PIPER_DONE as i32
            {
                return Err(TtsError::SynthesisFailed(format!(
                    "libpiper synthesis failed (status {status})"
                )));
            }
            if chunk.sample_rate <= 0 {
                return Err(TtsError::SynthesisFailed(
                    "libpiper returned an invalid sample rate".to_owned(),
                ));
            }
            let chunk_rate = chunk.sample_rate as u32;
            if sample_rate
                .replace(chunk_rate)
                .is_some_and(|rate| rate != chunk_rate)
            {
                return Err(TtsError::SynthesisFailed(
                    "libpiper changed sample rate within one utterance".to_owned(),
                ));
            }
            if chunk.num_samples > 0 {
                if chunk.samples.is_null() {
                    return Err(TtsError::SynthesisFailed(
                        "libpiper returned a null audio chunk".to_owned(),
                    ));
                }
                let chunk_samples =
                    unsafe { std::slice::from_raw_parts(chunk.samples, chunk.num_samples) };
                samples.extend_from_slice(chunk_samples);
            }
            if status == omnivox_piper_sys::PIPER_DONE as i32 || chunk.is_last {
                break;
            }
        }

        if samples.is_empty() {
            debug!("piper produced no audio");
            return Ok(SynthesisResult::audio(
                "piper",
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        debug!(
            "piper produced {} samples at {}Hz (mono f32)",
            samples.len(),
            sample_rate.unwrap_or_default()
        );

        let buffer = AudioBuffer::try_from_interleaved_f32(
            samples,
            sample_rate.expect("non-empty Piper audio has a sample rate"),
            1,
        )
        .map_err(|error| {
            TtsError::SynthesisFailed(format!("could not canonicalize Piper PCM: {error}"))
        })?;
        Ok(SynthesisResult::audio("piper", actual_voice, buffer))
    }

    fn stop(&self) {
        // libpiper has no native stop call. The synthesis loop observes this
        // between sentence chunks; the host retires the helper when a current
        // native inference call does not return within its cancellation grace.
        self.cancel_requested.store(true, Ordering::Release);
        debug!("piper: stop requested");
    }

    fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Acquire)
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

/// Find the exact espeak-ng data directory for piper's phonemizer.
///
/// Search order:
/// 1. `OMNIVOX_PIPER_ESPEAK_DATA` env var
/// 2. Data adjacent to the helper executable
/// 3. `ESPEAK_NG_DATA` env var (shared with the espeak TTS backend)
/// 4. Build-time path captured by omnivox-piper-sys/build.rs
/// 5. Well-known system paths
fn find_espeak_data() -> Option<PathBuf> {
    // 1. Piper-specific override
    if let Ok(dir) = std::env::var("OMNIVOX_PIPER_ESPEAK_DATA") {
        if !dir.is_empty() {
            if let Some(path) = normalize_espeak_data(Path::new(&dir)) {
                debug!("Using espeak data from OMNIVOX_PIPER_ESPEAK_DATA: {}", dir);
                return Some(path);
            }
        }
    }

    // 2. Companion data staged beside omnivox-piper-helper. Prefer this over
    // the main server's shared ESPEAK_NG_DATA so the two builds cannot select
    // one another's generated data after installation.
    if let Ok(executable) = std::env::current_exe() {
        if let Some(path) = adjacent_espeak_data(&executable) {
            debug!("Using Piper eSpeak data next to helper executable");
            return Some(path);
        }
    }

    // 3. Shared espeak env var
    if let Ok(dir) = std::env::var("ESPEAK_NG_DATA") {
        if !dir.is_empty() {
            if let Some(path) = normalize_espeak_data(Path::new(&dir)) {
                debug!("Using espeak data from ESPEAK_NG_DATA: {}", dir);
                return Some(path);
            }
        }
    }

    // 4. Build-time path from the maintained libpiper install.
    if !PIPER_ESPEAK_DATA_DIR.is_empty() {
        if let Some(path) = normalize_espeak_data(Path::new(PIPER_ESPEAK_DATA_DIR)) {
            debug!(
                "Using espeak data from build path: {}",
                PIPER_ESPEAK_DATA_DIR
            );
            return Some(path);
        }
    }

    // 5. System paths
    let candidates = [
        "/opt/homebrew/share",
        "/usr/local/share",
        "/usr/share",
        "/usr/lib/espeak-ng",
        "/usr/local/lib/espeak-ng",
    ];
    for candidate in &candidates {
        if let Some(path) = normalize_espeak_data(Path::new(candidate)) {
            debug!("Found espeak data at: {}", candidate);
            return Some(path);
        }
    }

    warn!("Could not find espeak-ng data directory for piper");
    None
}

/// Accept either the exact data directory or its traditional parent.
fn normalize_espeak_data(path: &Path) -> Option<PathBuf> {
    if path.join("phontab").is_file() {
        return Some(path.to_path_buf());
    }
    let nested = path.join("espeak-ng-data");
    nested.join("phontab").is_file().then_some(nested)
}

fn adjacent_espeak_data(executable: &Path) -> Option<PathBuf> {
    normalize_espeak_data(executable.parent()?)
}

struct SpeakingGuard<'a>(&'a AtomicBool);

impl Drop for SpeakingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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
        // rate 1.0 (fast) -> length_scale 0.5 (2x speed)
        assert!((PiperTtsEngine::map_rate_to_length_scale(1.0) - 0.5).abs() < 0.001);
        // rate 1.5 (very fast) -> length_scale 0.1 (floor)
        assert!((PiperTtsEngine::map_rate_to_length_scale(1.5) - 0.1).abs() < 0.001);
        // rate 2.0 -> floor
        assert!((PiperTtsEngine::map_rate_to_length_scale(2.0) - 0.1).abs() < 0.001);
        // Clamping
        assert!((PiperTtsEngine::map_rate_to_length_scale(-1.0) - 2.0).abs() < 0.001);
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
    fn test_espeak_data_normalization_missing() {
        assert!(normalize_espeak_data(Path::new("/nonexistent/path")).is_none());
    }

    #[test]
    fn test_espeak_data_normalizes_parent_and_exact_directory() {
        let root = std::env::temp_dir().join(format!(
            "omnivox-piper-espeak-data-test-{}",
            std::process::id()
        ));
        let data = root.join("espeak-ng-data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("phontab"), b"test").unwrap();

        assert_eq!(normalize_espeak_data(&root), Some(data.clone()));
        assert_eq!(normalize_espeak_data(&data), Some(data));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_espeak_data_is_found_beside_companion_helper() {
        let root = std::env::temp_dir().join(format!(
            "omnivox-piper-adjacent-data-test-{}",
            std::process::id()
        ));
        let data = root.join("piper/espeak-ng-data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("phontab"), b"test").unwrap();

        assert_eq!(
            adjacent_espeak_data(&root.join("piper/omnivox-piper-helper")),
            Some(data)
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
