//! Flite v2.2 adapter for the isolated Omnivox companion.

use std::collections::HashSet;
use std::env;
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use omnivox_flite_sys::{FliteVoice, FliteWave};
use omnivox_tts::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AudioOutputMode, Availability,
    CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor, EngineHealth,
    MarkerCapabilities, PhysicalVoiceId, TextRepertoire, VoiceDescriptor,
};
use omnivox_tts::helper_protocol::MAX_HELPER_SYNTHESIS_BYTES;
use omnivox_tts::{
    AudioBuffer, SynthesisRequest, SynthesisResult, TtsEngine, TtsError, VoiceInfo, VoiceQuality,
};

const ENGINE_ID: &str = "flite";
const BUILT_IN_VOICE_ID: &str = "cmu_us_slt";
const EXTERNAL_VOICES_ENV: &str = "OMNIVOX_FLITE_VOICES";
const MAX_EXTERNAL_VOICES: usize = 64;
const MAX_NATIVE_SAMPLES: usize = MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<i16>();

struct NativeVoice {
    pointer: *mut FliteVoice,
    id: String,
    name: String,
    owned: bool,
}

// SAFETY: Flite's process-global and voice state is accessed only while the
// owning engine's mutex is held.
unsafe impl Send for NativeVoice {}

struct Runtime {
    voices: Vec<NativeVoice>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        for voice in &mut self.voices {
            if voice.owned && !voice.pointer.is_null() {
                unsafe { omnivox_flite_sys::omnivox_flite_delete_voice(voice.pointer) };
                voice.pointer = ptr::null_mut();
            }
        }
    }
}

/// Serialized access to the built-in SLT voice and optional local `.flitevox`
/// voices. Native synthesis remains isolated in the helper process.
pub struct FliteTtsEngine {
    descriptor: EngineDescriptor,
    runtime: Mutex<Runtime>,
    cancellation: AtomicBool,
    speaking: AtomicBool,
}

impl FliteTtsEngine {
    pub fn from_environment() -> Result<Self, TtsError> {
        let (paths, mut warnings) = external_voice_paths();
        let mut engine = Self::new(paths, &mut warnings)?;
        if !warnings.is_empty() {
            engine.descriptor.health = EngineHealth::Degraded {
                reason: warnings.join("; "),
            };
        }
        Ok(engine)
    }

    fn new(paths: Vec<PathBuf>, warnings: &mut Vec<String>) -> Result<Self, TtsError> {
        let initialized = unsafe { omnivox_flite_sys::omnivox_flite_initialize() };
        if initialized != 0 {
            return Err(TtsError::SynthesisFailed(format!(
                "Flite initialization failed with status {initialized}"
            )));
        }

        let built_in = unsafe { omnivox_flite_sys::omnivox_flite_register_slt() };
        if built_in.is_null() {
            return Err(TtsError::NotAvailable);
        }
        let built_in_name = native_voice_name(built_in).map_err(TtsError::SynthesisFailed)?;
        let mut voices = vec![NativeVoice {
            pointer: built_in,
            id: BUILT_IN_VOICE_ID.to_owned(),
            name: built_in_name,
            owned: false,
        }];
        let mut external_names = HashSet::new();

        for path in paths.into_iter().take(MAX_EXTERNAL_VOICES) {
            let path = match validate_external_voice_path(&path) {
                Ok(path) => path,
                Err(reason) => {
                    warnings.push(reason);
                    continue;
                }
            };
            let Some(path_text) = path.to_str() else {
                warnings.push(format!(
                    "Flite voice path is not valid Unicode: {}",
                    path.display()
                ));
                continue;
            };
            let path_string = match CString::new(path_text) {
                Ok(path) => path,
                Err(_) => {
                    warnings.push(format!(
                        "Flite voice path contains a null byte: {}",
                        path.display()
                    ));
                    continue;
                }
            };
            let pointer =
                unsafe { omnivox_flite_sys::omnivox_flite_load_voice(path_string.as_ptr()) };
            if pointer.is_null() {
                warnings.push(format!("Flite could not load voice {}", path.display()));
                continue;
            }
            let name = match native_voice_name(pointer) {
                Ok(name) => name,
                Err(reason) => {
                    unsafe { omnivox_flite_sys::omnivox_flite_delete_voice(pointer) };
                    warnings.push(format!("{}: {reason}", path.display()));
                    continue;
                }
            };
            if !external_names.insert(name.clone()) {
                unsafe { omnivox_flite_sys::omnivox_flite_delete_voice(pointer) };
                warnings.push(format!(
                    "Flite voice {} duplicates external voice name {name}",
                    path.display()
                ));
                continue;
            }
            voices.push(NativeVoice {
                pointer,
                id: format!("flitevox:{name}"),
                name,
                owned: true,
            });
        }

        let descriptor = descriptor(&voices, warnings);
        Ok(Self {
            descriptor,
            runtime: Mutex::new(Runtime { voices }),
            cancellation: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
        })
    }

    fn runtime(&self) -> Result<MutexGuard<'_, Runtime>, TtsError> {
        self.runtime.lock().map_err(|error| {
            TtsError::SynthesisFailed(format!("Flite state lock is poisoned: {error}"))
        })
    }
}

impl TtsEngine for FliteTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let voice_id = request.voice_id_for_engine(ENGINE_ID)?;
        let runtime = self.runtime()?;
        let voice = runtime
            .voices
            .iter()
            .find(|voice| voice.id == voice_id || voice.name == voice_id)
            .ok_or_else(|| TtsError::VoiceNotFound(voice_id.to_owned()))?;
        let actual_voice = Some(PhysicalVoiceId::new(ENGINE_ID, voice.id.clone()));
        if request.text.is_empty() {
            return Ok(SynthesisResult::audio(
                ENGINE_ID,
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        let text = CString::new(request.text.as_str()).map_err(|_| {
            TtsError::InvalidParameter("Flite text contains a null byte".to_owned())
        })?;
        self.cancellation.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        let wave = unsafe {
            omnivox_flite_sys::omnivox_flite_synthesize(
                voice.pointer,
                text.as_ptr(),
                map_rate_to_duration(request.settings.rate),
                request.settings.pitch.clamp(0.5, 2.0),
            )
        };
        if wave.is_null() {
            return Err(TtsError::SynthesisFailed(
                "Flite did not return a waveform".to_owned(),
            ));
        }
        let wave = WaveGuard(wave);
        if self.cancellation.load(Ordering::Acquire) {
            return Err(TtsError::SynthesisFailed(
                "Flite synthesis was cancelled".to_owned(),
            ));
        }

        let sample_rate = positive_u32(
            unsafe { omnivox_flite_sys::omnivox_flite_wave_sample_rate(wave.0) },
            "sample rate",
        )?;
        let frame_count = positive_usize(
            unsafe { omnivox_flite_sys::omnivox_flite_wave_sample_count(wave.0) },
            "sample count",
        )?;
        let channels =
            u16::try_from(unsafe { omnivox_flite_sys::omnivox_flite_wave_channel_count(wave.0) })
                .ok()
                .filter(|channels| matches!(channels, 1 | 2))
                .ok_or_else(|| {
                    TtsError::SynthesisFailed("Flite returned invalid channels".to_owned())
                })?;
        let sample_count = frame_count
            .checked_mul(usize::from(channels))
            .filter(|count| *count <= MAX_NATIVE_SAMPLES)
            .ok_or_else(|| {
                TtsError::SynthesisFailed("Flite PCM exceeds the helper limit".to_owned())
            })?;
        let samples_pointer = unsafe { omnivox_flite_sys::omnivox_flite_wave_samples(wave.0) };
        if samples_pointer.is_null() {
            return Err(TtsError::SynthesisFailed(
                "Flite returned null PCM".to_owned(),
            ));
        }
        let native_samples = unsafe { std::slice::from_raw_parts(samples_pointer, sample_count) };
        let volume = request.settings.volume.clamp(0.0, 1.0);
        let samples = native_samples
            .iter()
            .map(|sample| (f32::from(*sample) * volume).round() as i16)
            .collect::<Vec<_>>();
        let audio = AudioBuffer::try_from_interleaved_i16(&samples, sample_rate, channels)
            .map_err(|error| {
                TtsError::SynthesisFailed(format!("could not canonicalize Flite PCM: {error}"))
            })?;
        let mut result = SynthesisResult::audio(ENGINE_ID, actual_voice, audio);
        result.degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&capabilities().acss)
            .omitted;
        Ok(result)
    }

    fn stop(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Acquire)
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.descriptor
            .voices
            .iter()
            .map(|voice| VoiceInfo {
                identifier: voice.id.voice_id.clone(),
                name: voice.display_name.clone(),
                language: voice.language.clone().unwrap_or_default(),
                quality: voice.quality,
            })
            .collect()
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|voice| voice.identifier == identifier || voice.name == identifier)
    }
}

struct SpeakingGuard<'a>(&'a AtomicBool);

impl Drop for SpeakingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct WaveGuard(*mut FliteWave);

impl Drop for WaveGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { omnivox_flite_sys::omnivox_flite_delete_wave(self.0) };
            self.0 = ptr::null_mut();
        }
    }
}

fn capabilities() -> EngineCapabilities {
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
        language_switching: false,
        text_repertoire: TextRepertoire::Unknown,
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

fn descriptor(voices: &[NativeVoice], warnings: &[String]) -> EngineDescriptor {
    let voices = voices
        .iter()
        .map(|voice| VoiceDescriptor {
            id: PhysicalVoiceId::new(ENGINE_ID, voice.id.clone()),
            display_name: voice.name.clone(),
            language: Some("en-US".to_owned()),
            gender: None,
            quality: VoiceQuality::Compact,
            availability: Availability::Available,
        })
        .collect::<Vec<_>>();
    EngineDescriptor {
        id: ENGINE_ID.to_owned(),
        display_name: "Flite".to_owned(),
        version: Some(format!(
            "{} ({})",
            omnivox_flite_sys::FLITE_VERSION,
            &omnivox_flite_sys::FLITE_COMMIT[..12]
        )),
        availability: Availability::Available,
        health: if warnings.is_empty() {
            EngineHealth::Healthy
        } else {
            EngineHealth::Degraded {
                reason: warnings.join("; "),
            }
        },
        capabilities: capabilities(),
        default_voice_id: voices.first().map(|voice| voice.id.voice_id.clone()),
        voices,
    }
}

fn external_voice_paths() -> (Vec<PathBuf>, Vec<String>) {
    let Some(value) = env::var_os(EXTERNAL_VOICES_ENV) else {
        return (Vec::new(), Vec::new());
    };
    let paths = env::split_paths(&value).collect::<Vec<_>>();
    let mut warnings = Vec::new();
    if paths.len() > MAX_EXTERNAL_VOICES {
        warnings.push(format!(
            "{EXTERNAL_VOICES_ENV} contains more than {MAX_EXTERNAL_VOICES} paths; extras were ignored"
        ));
    }
    (paths, warnings)
}

fn validate_external_voice_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "Flite voice path must be absolute: {}",
            path.display()
        ));
    }
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("flitevox"))
    {
        return Err(format!(
            "Flite voice path must end in .flitevox: {}",
            path.display()
        ));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("could not resolve Flite voice {}: {error}", path.display()))?;
    if !canonical.is_file() {
        return Err(format!(
            "Flite voice is not a regular file: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn native_voice_name(pointer: *mut FliteVoice) -> Result<String, String> {
    let name = unsafe { omnivox_flite_sys::omnivox_flite_voice_name(pointer) };
    if name.is_null() {
        return Err("Flite voice has no name".to_owned());
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_str()
        .map_err(|_| "Flite voice name is not UTF-8".to_owned())?;
    if name.is_empty() {
        return Err("Flite voice name is empty".to_owned());
    }
    Ok(name.to_owned())
}

fn map_rate_to_duration(rate: f32) -> f32 {
    let rate = rate.clamp(0.0, 2.0);
    if rate >= 0.5 {
        (1.5 - rate).max(0.1)
    } else {
        1.0 + (0.5 - rate) * 2.0
    }
}

fn positive_u32(value: i32, name: &str) -> Result<u32, TtsError> {
    u32::try_from(value)
        .ok()
        .filter(|value| *value > 0 && *value <= 384_000)
        .ok_or_else(|| TtsError::SynthesisFailed(format!("Flite returned invalid {name}")))
}

fn positive_usize(value: i32, name: &str) -> Result<usize, TtsError> {
    usize::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| TtsError::SynthesisFailed(format!("Flite returned invalid {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::TtsSettings;

    #[test]
    fn rate_mapping_preserves_normal_and_bounds_extremes() {
        assert_eq!(map_rate_to_duration(0.0), 2.0);
        assert_eq!(map_rate_to_duration(0.5), 1.0);
        assert_eq!(map_rate_to_duration(1.0), 0.5);
        assert_eq!(map_rate_to_duration(2.0), 0.1);
    }

    #[test]
    fn bundled_slt_engine_describes_and_synthesizes() {
        let mut warnings = Vec::new();
        let engine = FliteTtsEngine::new(Vec::new(), &mut warnings).unwrap();
        let descriptor = engine.descriptor();
        assert!(warnings.is_empty());
        assert_eq!(
            descriptor.default_voice_id.as_deref(),
            Some(BUILT_IN_VOICE_ID)
        );
        assert_eq!(descriptor.voices.len(), 1);

        let request = SynthesisRequest::new(
            "The compact SLT voice is ready.",
            TtsSettings {
                voice: BUILT_IN_VOICE_ID.to_owned(),
                ..TtsSettings::default()
            },
        );
        let result = engine.synthesize(&request).unwrap();
        assert_eq!(result.engine_id, ENGINE_ID);
        assert_eq!(
            result
                .actual_voice
                .as_ref()
                .map(|voice| voice.voice_id.as_str()),
            Some(BUILT_IN_VOICE_ID)
        );
        assert!(!result.audio.is_empty());
    }
}
