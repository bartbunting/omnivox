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
//! - Legacy model path: `OMNIVOX_PIPER_MODEL` or the helper's `--model` option
//! - Managed library: helper `--voice-library`, with lazy model/speaker selection
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
use crate::helper_protocol::MAX_HELPER_SYNTHESIS_BYTES;
use crate::{
    AudioBuffer, SynthesisRequest, SynthesisResult, SynthesisStreamCompletion, SynthesisStreamSink,
    SynthesisStreamStart, TtsEngine, TtsError, VoiceInfo, VoiceQuality,
};
use omnivox_audio::ProgressivePcmCanonicalizer;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tracing::{debug, warn};

mod library;
use library::{ModelSpec, ResidentModel, VoiceBinding};

/// espeak-ng data path discovered at build time by omnivox-piper-sys/build.rs.
/// Exposed as a pub const in the sys crate so we can reference it here.
use omnivox_piper_sys::PIPER_ESPEAK_DATA_DIR;

const MAX_NATIVE_SAMPLES: usize = MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<f32>();
const STREAMING_INPUT_FRAMES: usize = 512;

/// Piper helper adapter with serialized, single-model native residency.
/// Legacy construction eagerly loads its one model; library discovery is lazy.
pub struct PiperTtsEngine {
    state: Mutex<Option<ResidentModel>>,
    // Separate metadata lock: discovery never waits for native model loading.
    failed_models: Mutex<HashMap<usize, String>>,
    models: Vec<ModelSpec>,
    voices: Vec<VoiceBinding>,
    cancel_requested: AtomicBool,
    speaking: AtomicBool,
}

impl PiperTtsEngine {
    fn capabilities() -> EngineCapabilities {
        EngineCapabilities {
            acss: AcssCapabilities {
                rate: true,
                ..AcssCapabilities::default()
            },
            audio_output: AudioOutputMode::StreamingPcm,
            cancellation: CancellationSupport::PlaybackOnly,
            concurrency: ConcurrencyModel::Serialized,
            markers: MarkerCapabilities::default(),
            language_switching: false,
            text_repertoire: crate::contracts::TextRepertoire::Unicode,
            post_synthesis_dimensions: crate::contracts::buffered_post_synthesis_dimensions(),
            native_extensions: Vec::new(),
        }
    }

    /// Load the legacy single-model configuration, preserving its physical ID.
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self, TtsError> {
        let model_path = model_path.as_ref().to_path_buf();
        let config = find_config_path(&model_path).ok_or_else(|| {
            TtsError::VoiceNotFound(format!(
                "Piper configuration missing beside {}",
                model_path.display()
            ))
        })?;
        let name = model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("piper")
            .to_owned();
        let language = extract_language_from_name(&name);
        let spec = ModelSpec {
            model: model_path,
            config,
            speakers: vec![0],
            expected_assets: None,
        };
        let native = spec.load()?;
        Ok(Self {
            state: Mutex::new(Some(ResidentModel {
                model_index: 0,
                native,
            })),
            failed_models: Mutex::new(HashMap::new()),
            models: vec![spec],
            voices: vec![VoiceBinding {
                info: VoiceInfo {
                    identifier: format!("piper:{name}"),
                    name,
                    language: language.clone(),
                    quality: VoiceQuality::Enhanced,
                },
                language: Some(language),
                model_index: 0,
                speaker_index: 0,
            }],
            cancel_requested: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
        })
    }

    /// Consume an already verified library generation without loading models.
    /// Native loading rechecks asset hashes; the manager owns provenance validation.
    /// Only this generation's enabled Piper bindings can reach native loading.
    pub fn from_library(library: &crate::voice_library::RuntimeLibrary) -> Result<Self, TtsError> {
        let piper = library.document().piper.as_ref().ok_or_else(|| {
            TtsError::InvalidParameter("library has no managed Piper configuration".to_owned())
        })?;
        let mut models = Vec::new();
        let mut voices = Vec::new();
        for (model_index, model) in piper.models.iter().enumerate() {
            models.push(ModelSpec {
                model: PathBuf::from(&model.model.path),
                config: PathBuf::from(&model.config.path),
                speakers: model.voices.iter().map(|v| v.speaker_index).collect(),
                expected_assets: Some((model.model.clone(), model.config.clone())),
            });
            for voice in &model.voices {
                voices.push(VoiceBinding {
                    info: VoiceInfo {
                        identifier: voice.physical_id.clone(),
                        name: voice.display_name.clone(),
                        language: voice.language.clone().unwrap_or_else(|| "und".to_owned()),
                        quality: VoiceQuality::Enhanced,
                    },
                    language: voice.language.clone(),
                    model_index,
                    speaker_index: voice.speaker_index,
                });
            }
        }
        Ok(Self {
            state: Mutex::new(None),
            failed_models: Mutex::new(HashMap::new()),
            models,
            voices,
            cancel_requested: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
        })
    }

    fn binding(&self, request: &SynthesisRequest) -> Result<&VoiceBinding, TtsError> {
        let id = request.voice_id_for_engine("piper")?;
        let voice = self
            .voices
            .iter()
            .find(|v| v.info.identifier == id)
            .ok_or_else(|| {
                TtsError::VoiceNotFound("Piper voice is not enabled in this helper".to_owned())
            })?;
        if let Some(reason) = self
            .failed_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&voice.model_index)
        {
            return Err(TtsError::VoiceNotFound(reason.clone()));
        }
        Ok(voice)
    }

    fn prepare_model(
        &self,
        state: &mut Option<ResidentModel>,
        voice: &VoiceBinding,
        request: &SynthesisRequest,
    ) -> Result<*mut omnivox_piper_sys::piper_synthesizer, TtsError> {
        // Recheck after taking the native lock: another request may have failed
        // this model while we waited. Never clear a host-owned cancellation token.
        self.binding(request)?;
        self.cancel_requested.store(false, Ordering::Release);
        self.check_cancelled(request)?;
        if state
            .as_ref()
            .is_none_or(|resident| resident.model_index != voice.model_index)
        {
            drop(state.take());
            self.check_cancelled(request)?;
            match self.models[voice.model_index].load() {
                Ok(native) => {
                    *state = Some(ResidentModel {
                        model_index: voice.model_index,
                        native,
                    })
                }
                Err(error) => {
                    let reason = error.to_string();
                    self.failed_models
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(voice.model_index, reason);
                    return Err(error);
                }
            }
        }
        self.check_cancelled(request)?;
        Ok(state
            .as_ref()
            .expect("selected native model is resident")
            .native
            .0)
    }

    fn check_cancelled(&self, request: &SynthesisRequest) -> Result<(), TtsError> {
        if synthesis_cancelled(self, request) {
            Err(TtsError::SynthesisFailed(
                "Piper synthesis was cancelled".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    /// Create from the `OMNIVOX_PIPER_MODEL` environment variable.
    pub fn from_env() -> Result<Self, TtsError> {
        let model = std::env::var("OMNIVOX_PIPER_MODEL").map_err(|_| TtsError::NotAvailable)?;
        if model.is_empty() {
            return Err(TtsError::NotAvailable);
        }
        Self::new(&model)
    }

    /// Map the host rate to Piper's inverse duration scale.
    fn map_rate_to_length_scale(rate: f32) -> f32 {
        // Measured reference and saturation policy: docs/RATE-CALIBRATION.md.
        const CALIBRATION: &[(f32, f32)] = &[
            (0.0, 2.000_000),
            (0.1, 1.688_762),
            (0.2, 1.268_878),
            (0.3, 0.964_370),
            (0.4, 0.682_429),
            (0.5, 0.522_731),
            (0.6, 0.344_740),
            (0.7, 0.165_929),
            (0.8, 0.100_000),
        ];
        crate::rate_calibration::interpolate(rate, CALIBRATION)
    }
}

impl TtsEngine for PiperTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        let failed = self.failed_models.lock().unwrap_or_else(|e| e.into_inner());
        let voices: Vec<VoiceDescriptor> = self
            .voices
            .iter()
            .map(|binding| {
                let mut voice = VoiceDescriptor::from_voice_info("piper", binding.info.clone());
                voice.language = binding.language.clone();
                if let Some(reason) = failed.get(&binding.model_index) {
                    voice.availability = Availability::Unavailable {
                        reason: reason.clone(),
                    };
                }
                voice
            })
            .collect();
        let default_voice_id = self
            .voices
            .iter()
            .find(|voice| !failed.contains_key(&voice.model_index))
            .map(|voice| voice.info.identifier.clone());
        let version = piper_version();

        EngineDescriptor {
            id: "piper".to_owned(),
            display_name: "Piper".to_owned(),
            version: (version != "unknown").then_some(version),
            availability: if default_voice_id.is_some() {
                Availability::Available
            } else {
                Availability::Unavailable {
                    reason: "No eligible Piper voices in this helper".to_owned(),
                }
            },
            health: EngineHealth::Healthy,
            capabilities: Self::capabilities(),
            voices,
            espeak_variants: Vec::new(),
            default_voice_id,
        }
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let text = request.text.as_str();
        let settings = &request.settings;
        let voice = self.binding(request)?;
        let actual_voice = Some(PhysicalVoiceId::new("piper", voice.info.identifier.clone()));
        if request
            .cancellation
            .as_ref()
            .is_some_and(crate::SynthesisCancellationToken::is_cancelled)
        {
            return Err(TtsError::SynthesisFailed(
                "Piper synthesis was cancelled".to_owned(),
            ));
        }
        if text.is_empty() {
            return Ok(SynthesisResult::audio(
                "piper",
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        let mut state = self.state.lock().map_err(|error| {
            TtsError::SynthesisFailed(format!("Piper state lock poisoned: {error}"))
        })?;
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        let ptr = self.prepare_model(&mut state, voice, request)?;

        let length_scale = Self::map_rate_to_length_scale(settings.rate);

        debug!(
            "piper synthesizing: {} chars (length_scale={:.2})",
            text.len(),
            length_scale
        );

        let text_cstr = CString::new(text)
            .map_err(|_| TtsError::SynthesisFailed("Text contains null bytes".to_string()))?;

        let mut options = unsafe { omnivox_piper_sys::piper_default_synthesize_options(ptr) };
        options.speaker_id = voice.speaker_index as i32;
        options.length_scale = length_scale;
        let start =
            unsafe { omnivox_piper_sys::piper_synthesize_start(ptr, text_cstr.as_ptr(), &options) };
        if start != omnivox_piper_sys::PIPER_OK as i32 {
            return Err(TtsError::SynthesisFailed(format!(
                "libpiper could not start synthesis (status {start})"
            )));
        }

        let mut samples = Vec::new();
        let mut sample_rate = None;
        loop {
            if synthesis_cancelled(self, request) {
                return Err(TtsError::SynthesisFailed(
                    "Piper synthesis was cancelled".to_owned(),
                ));
            }
            let mut chunk: omnivox_piper_sys::piper_audio_chunk = unsafe { std::mem::zeroed() };
            let status = unsafe { omnivox_piper_sys::piper_synthesize_next(ptr, &mut chunk) };
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
                if chunk_samples.len() > MAX_NATIVE_SAMPLES.saturating_sub(samples.len()) {
                    return Err(TtsError::SynthesisFailed(
                        "Piper PCM exceeded the helper synthesis limit".to_owned(),
                    ));
                }
                samples.try_reserve(chunk_samples.len()).map_err(|_| {
                    TtsError::SynthesisFailed("could not allocate the Piper PCM buffer".to_owned())
                })?;
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
        let mut result = SynthesisResult::audio("piper", actual_voice, buffer);
        result.degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&Self::capabilities().acss)
            .omitted;
        Ok(result)
    }

    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        let text = request.text.as_str();
        let voice = self.binding(request)?;
        let actual_voice = Some(PhysicalVoiceId::new("piper", voice.info.identifier.clone()));
        if request
            .cancellation
            .as_ref()
            .is_some_and(crate::SynthesisCancellationToken::is_cancelled)
        {
            return Err(TtsError::SynthesisFailed(
                "Piper synthesis was cancelled".to_owned(),
            ));
        }
        let degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&Self::capabilities().acss)
            .omitted;
        sink.start(SynthesisStreamStart {
            engine_id: "piper".to_owned(),
            actual_voice,
            degraded_acss,
        })?;
        if text.is_empty() {
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }

        let mut state = self.state.lock().map_err(|error| {
            TtsError::SynthesisFailed(format!("Piper state lock poisoned: {error}"))
        })?;
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        let ptr = self.prepare_model(&mut state, voice, request)?;
        let text_cstr = CString::new(text)
            .map_err(|_| TtsError::SynthesisFailed("Text contains null bytes".to_owned()))?;
        let mut options = unsafe { omnivox_piper_sys::piper_default_synthesize_options(ptr) };
        options.speaker_id = voice.speaker_index as i32;
        options.length_scale = Self::map_rate_to_length_scale(request.settings.rate);
        let start =
            unsafe { omnivox_piper_sys::piper_synthesize_start(ptr, text_cstr.as_ptr(), &options) };
        if start != omnivox_piper_sys::PIPER_OK as i32 {
            return Err(TtsError::SynthesisFailed(format!(
                "libpiper could not start synthesis (status {start})"
            )));
        }

        let mut sample_rate = None;
        let mut native_samples = 0_usize;
        let mut canonicalizer = None;
        loop {
            if synthesis_cancelled(self, request) {
                return Err(TtsError::SynthesisFailed(
                    "Piper synthesis was cancelled".to_owned(),
                ));
            }
            let mut chunk: omnivox_piper_sys::piper_audio_chunk = unsafe { std::mem::zeroed() };
            let status = unsafe { omnivox_piper_sys::piper_synthesize_next(ptr, &mut chunk) };
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
                if chunk.num_samples > MAX_NATIVE_SAMPLES.saturating_sub(native_samples) {
                    return Err(TtsError::SynthesisFailed(
                        "Piper PCM exceeded the helper synthesis limit".to_owned(),
                    ));
                }
                native_samples += chunk.num_samples;
                let converter = if let Some(converter) = canonicalizer.as_mut() {
                    converter
                } else {
                    canonicalizer.insert(ProgressivePcmCanonicalizer::new(chunk_rate, 1).map_err(
                        |error| {
                            TtsError::SynthesisFailed(format!(
                                "could not initialize progressive Piper PCM conversion: {error}"
                            ))
                        },
                    )?)
                };
                let chunk_samples =
                    unsafe { std::slice::from_raw_parts(chunk.samples, chunk.num_samples) };
                for input in chunk_samples.chunks(STREAMING_INPUT_FRAMES) {
                    let windows = converter.push_interleaved_f32(input).map_err(|error| {
                        TtsError::SynthesisFailed(format!(
                            "could not canonicalize progressive Piper PCM: {error}"
                        ))
                    })?;
                    emit_audio_windows(sink, windows)?;
                    if synthesis_cancelled(self, request) {
                        return Err(TtsError::SynthesisFailed(
                            "Piper synthesis was cancelled".to_owned(),
                        ));
                    }
                }
            }
            if status == omnivox_piper_sys::PIPER_DONE as i32 || chunk.is_last {
                break;
            }
        }

        let Some(mut canonicalizer) = canonicalizer else {
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        };
        let windows = canonicalizer.finish().map_err(|error| {
            TtsError::SynthesisFailed(format!(
                "could not finish progressive Piper PCM conversion: {error}"
            ))
        })?;
        emit_audio_windows(sink, windows)?;
        Ok(SynthesisStreamCompletion {
            frame_count: canonicalizer.output_frames(),
        })
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
        let failed = self.failed_models.lock().unwrap_or_else(|e| e.into_inner());
        self.voices
            .iter()
            .filter(|voice| !failed.contains_key(&voice.model_index))
            .map(|voice| voice.info.clone())
            .collect()
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|v| v.identifier == identifier || v.name == identifier)
    }
}

fn synthesis_cancelled(engine: &PiperTtsEngine, request: &SynthesisRequest) -> bool {
    engine.cancel_requested.load(Ordering::Acquire)
        || request
            .cancellation
            .as_ref()
            .is_some_and(crate::SynthesisCancellationToken::is_cancelled)
}

fn emit_audio_windows(
    sink: &mut dyn SynthesisStreamSink,
    windows: Vec<AudioBuffer>,
) -> Result<(), TtsError> {
    for window in windows {
        if !window.is_empty() {
            sink.audio(window)?;
        }
    }
    Ok(())
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
mod library_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_mapping() {
        assert!((PiperTtsEngine::map_rate_to_length_scale(0.0) - 2.0).abs() < 0.001);
        assert!((PiperTtsEngine::map_rate_to_length_scale(0.5) - 0.522_731).abs() < 0.001);
        assert!((PiperTtsEngine::map_rate_to_length_scale(0.8) - 0.1).abs() < 0.001);
        assert!((PiperTtsEngine::map_rate_to_length_scale(2.0) - 0.1).abs() < 0.001);
        assert!((PiperTtsEngine::map_rate_to_length_scale(-1.0) - 2.0).abs() < 0.001);
        let mapped: Vec<_> = (0..=20)
            .map(|point| PiperTtsEngine::map_rate_to_length_scale(point as f32 / 10.0))
            .collect();
        assert!(mapped.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn piper_advertises_progressive_pcm() {
        assert_eq!(
            PiperTtsEngine::capabilities().audio_output,
            AudioOutputMode::StreamingPcm
        );
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
