//! TGSpeechBox adapter for the isolated, source-built Omnivox companion.

use std::collections::HashSet;
use std::ffi::{c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use omnivox_tts::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AudioOutputMode, Availability,
    CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor, EngineHealth,
    MarkerCapabilities, PhysicalVoiceId, TextRepertoire, VoiceDescriptor, VoiceGender,
};
use omnivox_tts::helper_protocol::MAX_HELPER_SYNTHESIS_BYTES;
use omnivox_tts::{
    AudioBuffer, SynthesisCancellationToken, SynthesisRequest, SynthesisResult,
    SynthesisStreamCompletion, SynthesisStreamSink, SynthesisStreamStart, TtsEngine, TtsError,
    VoiceInfo, VoiceQuality,
};

const ENGINE_ID: &str = "tgspeechbox";
const SAMPLE_RATE_ENVIRONMENT_VARIABLE: &str = "OMNIVOX_TGSPEECHBOX_SAMPLE_RATE";
const DEFAULT_SAMPLE_RATE: u32 = 44_100;
const LOWER_SAMPLE_RATE: u32 = 22_050;
const MAX_NATIVE_SAMPLES: usize = MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<i16>();
const PCM_CHUNK_SAMPLES: usize = 4_096;

#[derive(Debug, Clone)]
struct ProfileDefinition {
    id: String,
    native_name: String,
    display_name: String,
    gender: Option<VoiceGender>,
}

#[derive(Debug, Clone)]
struct VoiceSelection {
    id: String,
    language: String,
    profile: ProfileDefinition,
}

struct NativeRuntime {
    handle: NonNull<c_void>,
    espeak_initialized: bool,
    espeak_language: Option<String>,
}

// SAFETY: the pointer is owned by this value and all access is serialized by
// TgSpeechBoxTtsEngine::runtime. TGSpeechBox and eSpeak stay in this helper.
unsafe impl Send for NativeRuntime {}

impl Drop for NativeRuntime {
    fn drop(&mut self) {
        if self.espeak_initialized {
            unsafe {
                espeak_rs_sys::espeak_Terminate();
            }
        }
        unsafe {
            omnivox_tgspeechbox_sys::omnivox_tgspeechbox_destroy(self.handle.as_ptr());
        }
    }
}

/// Serialized TGSpeechBox DSP/frontend plus eSpeak-ng text-to-IPA state.
pub struct TgSpeechBoxTtsEngine {
    descriptor: EngineDescriptor,
    selections: Vec<VoiceSelection>,
    sample_rate: u32,
    runtime: Mutex<NativeRuntime>,
    cancellation: AtomicBool,
    speaking: AtomicBool,
}

impl TgSpeechBoxTtsEngine {
    pub fn from_environment() -> Result<Self, TtsError> {
        let sample_rate = configured_sample_rate()?;
        let pack_root = find_pack_root()?;
        let espeak_parent = find_espeak_data_parent()?;
        Self::new_with_sample_rate(&pack_root, &espeak_parent, sample_rate)
    }

    pub fn new(pack_root: &Path, espeak_data_parent: &Path) -> Result<Self, TtsError> {
        Self::new_with_sample_rate(pack_root, espeak_data_parent, DEFAULT_SAMPLE_RATE)
    }

    fn new_with_sample_rate(
        pack_root: &Path,
        espeak_data_parent: &Path,
        sample_rate: u32,
    ) -> Result<Self, TtsError> {
        validate_pack_root(pack_root)?;
        validate_espeak_data_parent(espeak_data_parent)?;
        let pack_path = path_to_c_string(pack_root, "TGSpeechBox pack root")?;
        let data_path = path_to_c_string(espeak_data_parent, "eSpeak-ng data parent")?;

        let handle = NonNull::new(unsafe {
            omnivox_tgspeechbox_sys::omnivox_tgspeechbox_create(
                pack_path.as_ptr(),
                sample_rate as i32,
            )
        })
        .ok_or_else(|| {
            TtsError::SynthesisFailed(format!("could not initialize TGSpeechBox: {}", unsafe {
                native_string(
                    omnivox_tgspeechbox_sys::omnivox_tgspeechbox_create_error(),
                    "unknown native initialization error",
                )
            }))
        })?;

        let dsp_version = unsafe { omnivox_tgspeechbox_sys::omnivox_tgspeechbox_dsp_version() };
        let frontend_version =
            unsafe { omnivox_tgspeechbox_sys::omnivox_tgspeechbox_frontend_abi_version() };
        if dsp_version != omnivox_tgspeechbox_sys::TGSPEECHBOX_DSP_VERSION
            || frontend_version != omnivox_tgspeechbox_sys::TGSPEECHBOX_FRONTEND_ABI_VERSION
        {
            unsafe {
                omnivox_tgspeechbox_sys::omnivox_tgspeechbox_destroy(handle.as_ptr());
            }
            return Err(TtsError::SynthesisFailed(format!(
                "unsupported TGSpeechBox ABI: DSP {dsp_version}, frontend {frontend_version}"
            )));
        }

        let rate = unsafe {
            espeak_rs_sys::espeak_Initialize(
                espeak_rs_sys::espeak_AUDIO_OUTPUT_AUDIO_OUTPUT_SYNCHRONOUS,
                0,
                data_path.as_ptr(),
                espeak_rs_sys::espeakINITIALIZE_DONT_EXIT as i32,
            )
        };
        if rate <= 0 {
            unsafe {
                omnivox_tgspeechbox_sys::omnivox_tgspeechbox_destroy(handle.as_ptr());
            }
            return Err(TtsError::SynthesisFailed(
                "eSpeak-ng could not initialize TGSpeechBox phonemization data".to_owned(),
            ));
        }

        let languages = match available_languages(handle) {
            Ok(languages) => filter_espeak_languages(languages),
            Err(error) => {
                unsafe {
                    espeak_rs_sys::espeak_Terminate();
                    omnivox_tgspeechbox_sys::omnivox_tgspeechbox_destroy(handle.as_ptr());
                }
                return Err(error);
            }
        };
        if languages.is_empty() {
            unsafe {
                espeak_rs_sys::espeak_Terminate();
                omnivox_tgspeechbox_sys::omnivox_tgspeechbox_destroy(handle.as_ptr());
            }
            return Err(TtsError::SynthesisFailed(
                "TGSpeechBox and eSpeak-ng have no common languages".to_owned(),
            ));
        }

        let profiles = match available_profiles(handle, &languages[0]) {
            Ok(profiles) => profiles,
            Err(error) => {
                unsafe {
                    espeak_rs_sys::espeak_Terminate();
                    omnivox_tgspeechbox_sys::omnivox_tgspeechbox_destroy(handle.as_ptr());
                }
                return Err(error);
            }
        };
        let selections = build_selections(&languages, &profiles);
        let default_voice_id = selections
            .iter()
            .find(|selection| selection.language == "en-us" && selection.profile.id == "adam")
            .or_else(|| {
                selections
                    .iter()
                    .find(|selection| selection.profile.id == "adam")
            })
            .or_else(|| selections.first())
            .map(|selection| selection.id.clone())
            .ok_or_else(|| TtsError::SynthesisFailed("TGSpeechBox has no voices".to_owned()))?;
        let descriptor = descriptor(
            &selections,
            default_voice_id,
            dsp_version,
            frontend_version,
            sample_rate,
        );

        Ok(Self {
            descriptor,
            selections,
            sample_rate,
            runtime: Mutex::new(NativeRuntime {
                handle,
                espeak_initialized: true,
                espeak_language: None,
            }),
            cancellation: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
        })
    }

    fn runtime(&self) -> Result<MutexGuard<'_, NativeRuntime>, TtsError> {
        self.runtime.lock().map_err(|error| {
            TtsError::SynthesisFailed(format!("TGSpeechBox state lock is poisoned: {error}"))
        })
    }

    fn selection(&self, id: &str) -> Result<&VoiceSelection, TtsError> {
        self.selections
            .iter()
            .find(|selection| selection.id == id)
            .ok_or_else(|| TtsError::VoiceNotFound(id.to_owned()))
    }
}

impl TtsEngine for TgSpeechBoxTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let voice_id = request.voice_id_for_engine(ENGINE_ID)?;
        let selection = self.selection(voice_id)?.clone();
        let actual_voice = Some(PhysicalVoiceId::new(ENGINE_ID, selection.id.clone()));
        if request.text.is_empty() {
            return Ok(SynthesisResult::audio(
                ENGINE_ID,
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        let mut runtime = self.runtime()?;
        self.cancellation.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        ensure_not_cancelled(&self.cancellation, request.cancellation.as_ref())?;

        configure(&mut runtime, &selection)?;
        let source_text = CString::new(request.text.as_str()).map_err(|_| {
            TtsError::InvalidParameter("TGSpeechBox text contains a null byte".to_owned())
        })?;
        let prepared = prepare_text(&mut runtime, &source_text)?;
        let ipa = phonemize(&prepared)?;
        if ipa.as_bytes().is_empty() {
            return Ok(SynthesisResult::audio(
                ENGINE_ID,
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        let queued = unsafe {
            omnivox_tgspeechbox_sys::omnivox_tgspeechbox_begin(
                runtime.handle.as_ptr(),
                prepared.as_ptr(),
                ipa.as_ptr(),
                map_rate(request.settings.rate),
                map_pitch(request.settings.pitch),
                request.normalized_acss.pitch_range.unwrap_or(0.5) as f64,
                request.settings.volume.clamp(0.0, 1.0) as f64,
            )
        };
        if queued == 0 {
            return Err(last_native_error(
                &runtime,
                "TGSpeechBox rejected synthesis",
            ));
        }

        let mut samples = Vec::new();
        let mut chunk = [0i16; PCM_CHUNK_SAMPLES];
        loop {
            if is_cancelled(&self.cancellation, request.cancellation.as_ref()) {
                reset(&mut runtime);
                return Err(TtsError::SynthesisFailed(
                    "TGSpeechBox synthesis was cancelled".to_owned(),
                ));
            }
            let count = unsafe {
                omnivox_tgspeechbox_sys::omnivox_tgspeechbox_next(
                    runtime.handle.as_ptr(),
                    chunk.as_mut_ptr(),
                    chunk.len(),
                )
            };
            if count < 0 {
                reset(&mut runtime);
                return Err(last_native_error(
                    &runtime,
                    "TGSpeechBox failed while producing PCM",
                ));
            }
            let count = count as usize;
            if count == 0 {
                break;
            }
            if count > chunk.len() || count > MAX_NATIVE_SAMPLES.saturating_sub(samples.len()) {
                reset(&mut runtime);
                return Err(TtsError::SynthesisFailed(
                    "TGSpeechBox PCM exceeds the helper limit".to_owned(),
                ));
            }
            samples.extend_from_slice(&chunk[..count]);
            if count < chunk.len() {
                break;
            }
        }

        ensure_not_cancelled(&self.cancellation, request.cancellation.as_ref())?;
        let audio = AudioBuffer::try_from_interleaved_i16(&samples, self.sample_rate, 1).map_err(
            |error| {
                TtsError::SynthesisFailed(format!(
                    "could not canonicalize TGSpeechBox PCM: {error}"
                ))
            },
        )?;
        let mut result = SynthesisResult::audio(ENGINE_ID, actual_voice, audio);
        result.degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&capabilities(self.sample_rate).acss)
            .omitted;
        Ok(result)
    }

    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        // Preserve whole-utterance sinc resampling in the optional comparison
        // mode until a stateful converter can span arbitrary native pulls.
        if self.sample_rate != DEFAULT_SAMPLE_RATE {
            let result = self.synthesize(request)?;
            let frame_count = result.audio.frame_count() as u64;
            sink.start(SynthesisStreamStart {
                engine_id: result.engine_id,
                actual_voice: result.actual_voice,
                degraded_acss: result.degraded_acss,
            })?;
            if !result.audio.is_empty() {
                sink.audio(result.audio)?;
            }
            if !result.markers.is_empty() || !result.anchors.is_empty() {
                sink.markers(result.markers, result.anchors)?;
            }
            return Ok(SynthesisStreamCompletion { frame_count });
        }

        let voice_id = request.voice_id_for_engine(ENGINE_ID)?;
        let selection = self.selection(voice_id)?.clone();
        let actual_voice = Some(PhysicalVoiceId::new(ENGINE_ID, selection.id.clone()));
        let degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&capabilities(self.sample_rate).acss)
            .omitted;
        sink.start(SynthesisStreamStart {
            engine_id: ENGINE_ID.to_owned(),
            actual_voice,
            degraded_acss,
        })?;
        if request.text.is_empty() {
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }

        let mut runtime = self.runtime()?;
        self.cancellation.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        ensure_not_cancelled(&self.cancellation, request.cancellation.as_ref())?;

        configure(&mut runtime, &selection)?;
        let source_text = CString::new(request.text.as_str()).map_err(|_| {
            TtsError::InvalidParameter("TGSpeechBox text contains a null byte".to_owned())
        })?;
        let prepared = prepare_text(&mut runtime, &source_text)?;
        let ipa = phonemize(&prepared)?;
        if ipa.as_bytes().is_empty() {
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }

        let queued = unsafe {
            omnivox_tgspeechbox_sys::omnivox_tgspeechbox_begin(
                runtime.handle.as_ptr(),
                prepared.as_ptr(),
                ipa.as_ptr(),
                map_rate(request.settings.rate),
                map_pitch(request.settings.pitch),
                request.normalized_acss.pitch_range.unwrap_or(0.5) as f64,
                request.settings.volume.clamp(0.0, 1.0) as f64,
            )
        };
        if queued == 0 {
            return Err(last_native_error(
                &runtime,
                "TGSpeechBox rejected synthesis",
            ));
        }

        let mut emitted_samples = 0usize;
        let mut chunk = [0i16; PCM_CHUNK_SAMPLES];
        loop {
            if is_cancelled(&self.cancellation, request.cancellation.as_ref()) {
                reset(&mut runtime);
                return Err(TtsError::SynthesisFailed(
                    "TGSpeechBox synthesis was cancelled".to_owned(),
                ));
            }
            let count = unsafe {
                omnivox_tgspeechbox_sys::omnivox_tgspeechbox_next(
                    runtime.handle.as_ptr(),
                    chunk.as_mut_ptr(),
                    chunk.len(),
                )
            };
            if count < 0 {
                reset(&mut runtime);
                return Err(last_native_error(
                    &runtime,
                    "TGSpeechBox failed while producing PCM",
                ));
            }
            let count = count as usize;
            if count == 0 {
                break;
            }
            if count > chunk.len() || count > MAX_NATIVE_SAMPLES.saturating_sub(emitted_samples) {
                reset(&mut runtime);
                return Err(TtsError::SynthesisFailed(
                    "TGSpeechBox PCM exceeds the helper limit".to_owned(),
                ));
            }
            let audio = AudioBuffer::try_from_interleaved_i16(&chunk[..count], self.sample_rate, 1)
                .map_err(|error| {
                    TtsError::SynthesisFailed(format!(
                        "could not canonicalize TGSpeechBox PCM: {error}"
                    ))
                })?;
            sink.audio(audio)?;
            emitted_samples += count;
            if count < chunk.len() {
                break;
            }
        }

        ensure_not_cancelled(&self.cancellation, request.cancellation.as_ref())?;
        Ok(SynthesisStreamCompletion {
            frame_count: emitted_samples as u64,
        })
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

fn capabilities(sample_rate: u32) -> EngineCapabilities {
    EngineCapabilities {
        acss: AcssCapabilities {
            rate: true,
            average_pitch: true,
            pitch_range: true,
            volume: true,
            ..AcssCapabilities::default()
        },
        audio_output: if sample_rate == DEFAULT_SAMPLE_RATE {
            AudioOutputMode::StreamingPcm
        } else {
            AudioOutputMode::BufferedPcm
        },
        cancellation: CancellationSupport::SynthesisAndPlayback,
        concurrency: ConcurrencyModel::Serialized,
        markers: MarkerCapabilities::default(),
        language_switching: true,
        text_repertoire: TextRepertoire::Unicode,
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

fn descriptor(
    selections: &[VoiceSelection],
    default_voice_id: String,
    dsp_version: u32,
    frontend_version: i32,
    sample_rate: u32,
) -> EngineDescriptor {
    let voices = selections
        .iter()
        .map(|selection| VoiceDescriptor {
            id: PhysicalVoiceId::new(ENGINE_ID, selection.id.clone()),
            display_name: format!(
                "TGSpeechBox {} ({})",
                selection.profile.display_name, selection.language
            ),
            language: Some(selection.language.clone()),
            gender: selection.profile.gender,
            quality: VoiceQuality::Compact,
            availability: Availability::Available,
        })
        .collect();
    EngineDescriptor {
        id: ENGINE_ID.to_owned(),
        display_name: "TGSpeechBox".to_owned(),
        version: Some(format!(
            "{} (DSP {dsp_version}, frontend ABI {frontend_version}, native {sample_rate} Hz, {})",
            omnivox_tgspeechbox_sys::TGSPEECHBOX_RELEASE,
            &omnivox_tgspeechbox_sys::TGSPEECHBOX_COMMIT[..12]
        )),
        availability: Availability::Available,
        health: EngineHealth::Healthy,
        capabilities: capabilities(sample_rate),
        voices,
        default_voice_id: Some(default_voice_id),
    }
}

fn built_in_profiles() -> Vec<ProfileDefinition> {
    [
        ("adam", "Adam", Some(VoiceGender::Male)),
        ("benjamin", "Benjamin", Some(VoiceGender::Male)),
        ("caleb", "Caleb", Some(VoiceGender::Neutral)),
        ("david", "David", Some(VoiceGender::Male)),
        ("robert", "Robert", Some(VoiceGender::Male)),
    ]
    .into_iter()
    .map(|(id, name, gender)| ProfileDefinition {
        id: id.to_owned(),
        native_name: name.to_owned(),
        display_name: name.to_owned(),
        gender,
    })
    .collect()
}

fn available_profiles(
    handle: NonNull<c_void>,
    initial_language: &str,
) -> Result<Vec<ProfileDefinition>, TtsError> {
    let language = CString::new(initial_language).map_err(|_| {
        TtsError::InvalidParameter("TGSpeechBox language contains a null byte".to_owned())
    })?;
    let base_profile = CString::new("").expect("an empty profile has no null byte");
    if unsafe {
        omnivox_tgspeechbox_sys::omnivox_tgspeechbox_configure(
            handle.as_ptr(),
            language.as_ptr(),
            base_profile.as_ptr(),
        )
    } == 0
    {
        return Err(TtsError::SynthesisFailed(unsafe {
            native_string(
                omnivox_tgspeechbox_sys::omnivox_tgspeechbox_last_error(handle.as_ptr()),
                "TGSpeechBox could not load a language for profile discovery",
            )
        }));
    }

    let mut profiles = built_in_profiles();
    let mut ids = profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<HashSet<_>>();
    let names = unsafe {
        native_string(
            omnivox_tgspeechbox_sys::omnivox_tgspeechbox_profile_names(handle.as_ptr()),
            "",
        )
    };
    let mut discovered = Vec::new();
    for name in names.lines().map(str::trim).filter(|name| !name.is_empty()) {
        let id = profile_id(name);
        if id.is_empty() || !ids.insert(id.clone()) {
            continue;
        }
        discovered.push(ProfileDefinition {
            id,
            native_name: name.to_owned(),
            display_name: name.to_owned(),
            gender: name
                .eq_ignore_ascii_case("beth")
                .then_some(VoiceGender::Female),
        });
    }
    discovered.sort_by(|left, right| left.id.cmp(&right.id));
    profiles.extend(discovered);
    Ok(profiles)
}

fn profile_id(name: &str) -> String {
    name.chars()
        .filter_map(|character| {
            if character.is_ascii_alphanumeric() {
                Some(character.to_ascii_lowercase())
            } else if character == '-' || character == '_' || character.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

fn build_selections(languages: &[String], profiles: &[ProfileDefinition]) -> Vec<VoiceSelection> {
    let mut selections = Vec::with_capacity(languages.len().saturating_mul(profiles.len()));
    for language in languages {
        for profile in profiles {
            selections.push(VoiceSelection {
                id: format!("{language}/{}", profile.id),
                language: language.clone(),
                profile: profile.clone(),
            });
        }
    }
    selections
}

fn available_languages(handle: NonNull<c_void>) -> Result<Vec<String>, TtsError> {
    let pointer =
        unsafe { omnivox_tgspeechbox_sys::omnivox_tgspeechbox_languages(handle.as_ptr()) };
    if pointer.is_null() {
        return Err(TtsError::SynthesisFailed(
            "TGSpeechBox could not enumerate language packs".to_owned(),
        ));
    }
    let text = unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() };
    unsafe {
        omnivox_tgspeechbox_sys::omnivox_tgspeechbox_free_string(pointer);
    }
    let mut languages = text
        .lines()
        .map(str::trim)
        .filter(|language| !language.is_empty() && *language != "default")
        .map(str::to_owned)
        .collect::<Vec<_>>();
    languages.sort();
    languages.dedup();
    Ok(languages)
}

fn filter_espeak_languages(languages: Vec<String>) -> Vec<String> {
    languages
        .into_iter()
        .filter(|language| {
            let Ok(language) = CString::new(language.as_str()) else {
                return false;
            };
            (unsafe { espeak_rs_sys::espeak_SetVoiceByName(language.as_ptr()) })
                == espeak_rs_sys::espeak_ERROR_EE_OK
        })
        .collect()
}

fn configure(runtime: &mut NativeRuntime, selection: &VoiceSelection) -> Result<(), TtsError> {
    let language = CString::new(selection.language.as_str()).map_err(|_| {
        TtsError::InvalidParameter("TGSpeechBox language contains a null byte".to_owned())
    })?;
    let profile = CString::new(selection.profile.native_name.as_str()).map_err(|_| {
        TtsError::InvalidParameter("TGSpeechBox profile contains a null byte".to_owned())
    })?;
    if runtime.espeak_language.as_deref() != Some(selection.language.as_str()) {
        if unsafe { espeak_rs_sys::espeak_SetVoiceByName(language.as_ptr()) }
            != espeak_rs_sys::espeak_ERROR_EE_OK
        {
            return Err(TtsError::VoiceNotFound(selection.id.clone()));
        }
        runtime.espeak_language = Some(selection.language.clone());
    }
    if unsafe {
        omnivox_tgspeechbox_sys::omnivox_tgspeechbox_configure(
            runtime.handle.as_ptr(),
            language.as_ptr(),
            profile.as_ptr(),
        )
    } == 0
    {
        return Err(last_native_error(
            runtime,
            "TGSpeechBox could not configure the requested voice",
        ));
    }
    Ok(())
}

fn prepare_text(runtime: &mut NativeRuntime, source: &CString) -> Result<CString, TtsError> {
    let pointer = unsafe {
        omnivox_tgspeechbox_sys::omnivox_tgspeechbox_prepare_text(
            runtime.handle.as_ptr(),
            source.as_ptr(),
        )
    };
    if pointer.is_null() {
        return Ok(source.clone());
    }
    let prepared = unsafe { CStr::from_ptr(pointer).to_bytes().to_vec() };
    unsafe {
        omnivox_tgspeechbox_sys::omnivox_tgspeechbox_free_string(pointer);
    }
    CString::new(prepared).map_err(|_| {
        TtsError::SynthesisFailed("TGSpeechBox prepared text contains a null byte".to_owned())
    })
}

fn phonemize(text: &CString) -> Result<CString, TtsError> {
    let mut cursor = text.as_ptr().cast::<c_void>();
    let mut ipa = Vec::new();
    let mut calls = 0usize;
    while !cursor.is_null() && unsafe { *cursor.cast::<u8>() } != 0 {
        let before = cursor;
        let pointer = unsafe {
            espeak_rs_sys::espeak_TextToPhonemes(
                &mut cursor,
                espeak_rs_sys::espeakCHARS_UTF8 as i32,
                espeak_rs_sys::espeakPHONEMES_IPA as i32,
            )
        };
        if pointer.is_null() {
            return Err(TtsError::SynthesisFailed(
                "eSpeak-ng failed to phonemize TGSpeechBox text".to_owned(),
            ));
        }
        let clause = unsafe { CStr::from_ptr(pointer).to_bytes() };
        if !clause.is_empty() {
            if !ipa.is_empty() {
                ipa.push(b' ');
            }
            ipa.extend_from_slice(clause);
        }
        calls += 1;
        if cursor == before || calls > text.as_bytes().len().saturating_add(1) {
            return Err(TtsError::SynthesisFailed(
                "eSpeak-ng phonemization did not advance".to_owned(),
            ));
        }
    }
    CString::new(ipa).map_err(|_| {
        TtsError::SynthesisFailed("eSpeak-ng returned IPA containing a null byte".to_owned())
    })
}

fn map_rate(rate: f32) -> f64 {
    let rate = if rate.is_nan() {
        0.5
    } else {
        rate.clamp(0.0, 2.0)
    } as f64;
    if rate <= 1.0 {
        2.0f64.powf(rate * 2.0 - 1.0)
    } else {
        2.0f64.powf(rate)
    }
}

fn map_pitch(pitch: f32) -> f64 {
    let pitch = if pitch.is_nan() {
        1.0
    } else {
        pitch.clamp(0.5, 2.0)
    };
    f64::from(pitch) * 110.0
}

fn reset(runtime: &mut NativeRuntime) {
    unsafe {
        omnivox_tgspeechbox_sys::omnivox_tgspeechbox_reset(runtime.handle.as_ptr());
    }
}

fn is_cancelled(engine: &AtomicBool, request: Option<&SynthesisCancellationToken>) -> bool {
    engine.load(Ordering::Acquire) || request.is_some_and(SynthesisCancellationToken::is_cancelled)
}

fn ensure_not_cancelled(
    engine: &AtomicBool,
    request: Option<&SynthesisCancellationToken>,
) -> Result<(), TtsError> {
    if is_cancelled(engine, request) {
        Err(TtsError::SynthesisFailed(
            "TGSpeechBox synthesis was cancelled".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn last_native_error(runtime: &NativeRuntime, fallback: &str) -> TtsError {
    let error = unsafe {
        native_string(
            omnivox_tgspeechbox_sys::omnivox_tgspeechbox_last_error(runtime.handle.as_ptr()),
            fallback,
        )
    };
    TtsError::SynthesisFailed(if error.is_empty() {
        fallback.to_owned()
    } else {
        error
    })
}

unsafe fn native_string(pointer: *const std::ffi::c_char, fallback: &str) -> String {
    if pointer.is_null() {
        fallback.to_owned()
    } else {
        unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() }
    }
}

fn path_to_c_string(path: &Path, label: &str) -> Result<CString, TtsError> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| TtsError::InvalidParameter(format!("{label} contains a null byte")))
}

fn validate_pack_root(path: &Path) -> Result<(), TtsError> {
    if path.is_absolute()
        && path.join("packs/phonemes.yaml").is_file()
        && path.join("packs/lang/default.yaml").is_file()
    {
        Ok(())
    } else {
        Err(TtsError::InvalidParameter(format!(
            "TGSpeechBox pack root is incomplete or not absolute: {}",
            path.display()
        )))
    }
}

fn validate_espeak_data_parent(path: &Path) -> Result<(), TtsError> {
    if path.is_absolute() && path.join("espeak-ng-data/phontab").is_file() {
        Ok(())
    } else {
        Err(TtsError::InvalidParameter(format!(
            "eSpeak-ng data parent is incomplete or not absolute: {}",
            path.display()
        )))
    }
}

fn configured_sample_rate() -> Result<u32, TtsError> {
    let value = std::env::var_os(SAMPLE_RATE_ENVIRONMENT_VARIABLE);
    let value = value
        .as_deref()
        .map(|value| {
            value.to_str().ok_or_else(|| {
                TtsError::InvalidParameter(format!(
                    "{SAMPLE_RATE_ENVIRONMENT_VARIABLE} must be valid Unicode"
                ))
            })
        })
        .transpose()?;
    parse_sample_rate(value)
}

fn parse_sample_rate(value: Option<&str>) -> Result<u32, TtsError> {
    match value {
        None | Some("") | Some("44100") => Ok(DEFAULT_SAMPLE_RATE),
        Some("22050") => Ok(LOWER_SAMPLE_RATE),
        Some(value) => Err(TtsError::InvalidParameter(format!(
            "{SAMPLE_RATE_ENVIRONMENT_VARIABLE} must be 22050 or 44100, not {value:?}"
        ))),
    }
}

fn configured_absolute_path(variable: &str) -> Result<Option<PathBuf>, TtsError> {
    let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(TtsError::InvalidParameter(format!(
            "{variable} must be an absolute path: {}",
            path.display()
        )));
    }
    Ok(Some(path))
}

fn helper_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn find_pack_root() -> Result<PathBuf, TtsError> {
    if let Some(path) = configured_absolute_path("OMNIVOX_TGSPEECHBOX_DATA")? {
        validate_pack_root(&path)?;
        return Ok(path);
    }
    helper_directory()
        .filter(|path| validate_pack_root(path).is_ok())
        .ok_or(TtsError::NotAvailable)
}

fn find_espeak_data_parent() -> Result<PathBuf, TtsError> {
    if let Some(path) = configured_absolute_path("ESPEAK_NG_DATA")? {
        validate_espeak_data_parent(&path)?;
        return Ok(path);
    }
    helper_directory()
        .filter(|path| validate_espeak_data_parent(path).is_ok())
        .or_else(|| {
            [
                PathBuf::from("/opt/homebrew/share"),
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
            .into_iter()
            .find(|path| validate_espeak_data_parent(path).is_ok())
        })
        .ok_or(TtsError::NotAvailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisional_rate_mapping_is_monotonic_and_has_expected_landmarks() {
        assert_eq!(map_rate(0.0), 0.5);
        assert_eq!(map_rate(0.5), 1.0);
        assert_eq!(map_rate(1.0), 2.0);
        assert_eq!(map_rate(2.0), 4.0);
        let mapped = (0..=20)
            .map(|point| map_rate(point as f32 / 10.0))
            .collect::<Vec<_>>();
        assert!(mapped.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn pitch_mapping_uses_the_tgspeechbox_normal_baseline() {
        assert_eq!(map_pitch(0.5), 55.0);
        assert_eq!(map_pitch(1.0), 110.0);
        assert_eq!(map_pitch(2.0), 220.0);
    }

    #[test]
    fn sample_rate_configuration_accepts_only_the_comparison_rates() {
        assert_eq!(parse_sample_rate(None).unwrap(), 44_100);
        assert_eq!(parse_sample_rate(Some("")).unwrap(), 44_100);
        assert_eq!(parse_sample_rate(Some("44100")).unwrap(), 44_100);
        assert_eq!(parse_sample_rate(Some("22050")).unwrap(), 22_050);
        assert!(matches!(
            parse_sample_rate(Some("48000")),
            Err(TtsError::InvalidParameter(message)) if message.contains("22050 or 44100")
        ));
    }

    #[test]
    fn only_native_canonical_rate_advertises_progressive_pcm() {
        assert_eq!(
            capabilities(DEFAULT_SAMPLE_RATE).audio_output,
            AudioOutputMode::StreamingPcm
        );
        assert_eq!(
            capabilities(LOWER_SAMPLE_RATE).audio_output,
            AudioOutputMode::BufferedPcm
        );
    }

    #[test]
    fn voice_inventory_crosses_each_language_with_each_profile() {
        let profiles = vec![
            built_in_profiles()[0].clone(),
            ProfileDefinition {
                id: "beth".to_owned(),
                native_name: "Beth".to_owned(),
                display_name: "Beth".to_owned(),
                gender: Some(VoiceGender::Female),
            },
        ];
        let selections = build_selections(&["en-us".to_owned(), "de".to_owned()], &profiles);
        assert_eq!(selections.len(), 4);
        assert!(selections.iter().any(|voice| voice.id == "de/beth"));
    }
}
