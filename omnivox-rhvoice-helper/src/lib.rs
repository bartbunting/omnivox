//! Dynamically loaded RHVoice adapter for the isolated Omnivox helper.

use std::collections::BTreeSet;
use std::ffi::{CStr, CString, OsString};
use std::os::raw::{c_char, c_double, c_int, c_short, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use libloading::Library;
use omnivox_audio::ProgressivePcmCanonicalizer;
use omnivox_tts::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AnchorSupport, AudioOutputMode,
    Availability, CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor,
    EngineHealth, MarkerCapabilities, PhysicalVoiceId, TextRepertoire, VoiceDescriptor,
    VoiceGender,
};
use omnivox_tts::helper_protocol::{MAX_HELPER_MARKERS, MAX_HELPER_SYNTHESIS_BYTES};
use omnivox_tts::rate_calibration::interpolate;
use omnivox_tts::{
    AudioBuffer, SynthesisCancellationToken, SynthesisMarker, SynthesisMarkerKind,
    SynthesisRequest, SynthesisResult, SynthesisStreamCompletion, SynthesisStreamSink,
    SynthesisStreamStart, TtsEngine, TtsError, VoiceInfo, VoiceQuality, STANDARD_SAMPLE_RATE,
};
use thiserror::Error;

const ENGINE_ID: &str = "rhvoice";
const ENV_LIBRARY: &str = "OMNIVOX_RHVOICE_LIBRARY";
const ENV_DATA: &str = "OMNIVOX_RHVOICE_DATA";
const ENV_CONFIG: &str = "OMNIVOX_RHVOICE_CONFIG";
const ENV_RESOURCES: &str = "OMNIVOX_RHVOICE_RESOURCES";
const MINIMUM_SUPPORTED_MINOR: u32 = 14;
const MAX_NATIVE_SAMPLES: usize = MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<i16>();

type NativeEngine = *mut c_void;
type NativeMessage = *mut c_void;

type SetSampleRateCallback = unsafe extern "C" fn(c_int, *mut c_void) -> c_int;
type PlaySpeechCallback = unsafe extern "C" fn(*const c_short, c_uint, *mut c_void) -> c_int;
type ProcessMarkCallback = unsafe extern "C" fn(*const c_char, *mut c_void) -> c_int;
type TextEventCallback = unsafe extern "C" fn(c_uint, c_uint, *mut c_void) -> c_int;
type PlayAudioCallback = unsafe extern "C" fn(*const c_char, *mut c_void) -> c_int;
type DoneCallback = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeCallbacks {
    set_sample_rate: Option<SetSampleRateCallback>,
    play_speech: Option<PlaySpeechCallback>,
    process_mark: Option<ProcessMarkCallback>,
    word_starts: Option<TextEventCallback>,
    word_ends: Option<TextEventCallback>,
    sentence_starts: Option<TextEventCallback>,
    sentence_ends: Option<TextEventCallback>,
    play_audio: Option<PlayAudioCallback>,
    done: Option<DoneCallback>,
}

#[repr(C)]
struct NativeInitParams {
    data_path: *const c_char,
    config_path: *const c_char,
    resource_paths: *const *const c_char,
    callbacks: NativeCallbacks,
    options: c_uint,
}

#[repr(C)]
struct NativeVoiceInfo {
    language: *const c_char,
    name: *const c_char,
    gender: c_int,
    country: *const c_char,
}

#[repr(C)]
struct NativeSynthParams {
    voice_profile: *const c_char,
    absolute_rate: c_double,
    absolute_pitch: c_double,
    absolute_volume: c_double,
    relative_rate: c_double,
    relative_pitch: c_double,
    relative_volume: c_double,
    punctuation_mode: c_int,
    punctuation_list: *const c_char,
    capitals_mode: c_int,
    flags: c_int,
}

type GetVersionFn = unsafe extern "C" fn() -> *const c_char;
type NewEngineFn = unsafe extern "C" fn(*const NativeInitParams) -> NativeEngine;
type DeleteEngineFn = unsafe extern "C" fn(NativeEngine);
type GetVoiceCountFn = unsafe extern "C" fn(NativeEngine) -> c_uint;
type GetVoicesFn = unsafe extern "C" fn(NativeEngine) -> *const NativeVoiceInfo;
type NewMessageFn = unsafe extern "C" fn(
    NativeEngine,
    *const c_char,
    c_uint,
    c_int,
    *const NativeSynthParams,
    *mut c_void,
) -> NativeMessage;
type DeleteMessageFn = unsafe extern "C" fn(NativeMessage);
type SpeakFn = unsafe extern "C" fn(NativeMessage) -> c_int;

#[derive(Clone, Copy)]
struct RhVoiceApi {
    get_version: GetVersionFn,
    new_engine: NewEngineFn,
    delete_engine: DeleteEngineFn,
    get_voice_count: GetVoiceCountFn,
    get_voices: GetVoicesFn,
    new_message: NewMessageFn,
    delete_message: DeleteMessageFn,
    speak: SpeakFn,
}

impl RhVoiceApi {
    unsafe fn load(library: &Library) -> Result<Self, RhVoiceError> {
        Ok(Self {
            get_version: load_symbol(library, b"RHVoice_get_version\0")?,
            new_engine: load_symbol(library, b"RHVoice_new_tts_engine\0")?,
            delete_engine: load_symbol(library, b"RHVoice_delete_tts_engine\0")?,
            get_voice_count: load_symbol(library, b"RHVoice_get_number_of_voices\0")?,
            get_voices: load_symbol(library, b"RHVoice_get_voices\0")?,
            new_message: load_symbol(library, b"RHVoice_new_message\0")?,
            delete_message: load_symbol(library, b"RHVoice_delete_message\0")?,
            speak: load_symbol(library, b"RHVoice_speak\0")?,
        })
    }
}

unsafe fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, RhVoiceError> {
    library
        .get::<T>(name)
        .map(|symbol| *symbol)
        .map_err(|error| {
            RhVoiceError::Runtime(format!(
                "required RHVoice symbol {} is missing: {error}",
                String::from_utf8_lossy(name).trim_end_matches('\0')
            ))
        })
}

#[derive(Debug, Error)]
enum RhVoiceError {
    #[error("{0}")]
    Configuration(String),

    #[error("{0}")]
    Runtime(String),
}

#[derive(Debug, Default)]
struct RuntimeConfig {
    library: Option<PathBuf>,
    data: Option<PathBuf>,
    config: Option<PathBuf>,
    resources: Vec<PathBuf>,
}

impl RuntimeConfig {
    fn from_environment() -> Result<Self, RhVoiceError> {
        Ok(Self {
            library: absolute_file_from_env(ENV_LIBRARY)?,
            data: absolute_directory_from_env(ENV_DATA)?,
            config: absolute_directory_from_env(ENV_CONFIG)?,
            resources: absolute_directories_from_env(ENV_RESOURCES)?,
        })
    }
}

#[derive(Clone)]
struct VoiceRecord {
    id: String,
    name: String,
    language: String,
    gender: Option<VoiceGender>,
    profile: CString,
}

struct LoadedRuntime {
    api: RhVoiceApi,
    engine: NativeEngine,
    voices: Vec<VoiceRecord>,
    _library: Option<Library>,
}

// SAFETY: All calls through the native engine handle are serialized by the
// owning `Mutex`. Cancellation touches only an independent atomic read by the
// native callbacks.
unsafe impl Send for LoadedRuntime {}

impl Drop for LoadedRuntime {
    fn drop(&mut self) {
        if !self.engine.is_null() {
            unsafe { (self.api.delete_engine)(self.engine) };
            self.engine = ptr::null_mut();
        }
    }
}

/// RHVoice engine state. Missing or rejected runtimes remain representable so
/// the helper can negotiate and return a bounded `not_available` diagnostic.
pub struct RhVoiceTtsEngine {
    descriptor: EngineDescriptor,
    runtime: Option<Mutex<LoadedRuntime>>,
    cancellation: Arc<AtomicBool>,
    speaking: AtomicBool,
}

impl RhVoiceTtsEngine {
    pub fn from_environment() -> Self {
        match RuntimeConfig::from_environment().and_then(Self::load) {
            Ok(engine) => engine,
            Err(error) => Self::unavailable(error.to_string()),
        }
    }

    fn load(mut config: RuntimeConfig) -> Result<Self, RhVoiceError> {
        let library_path = match config.library.take() {
            Some(path) => path,
            None => discover_library().ok_or_else(|| {
                RhVoiceError::Runtime(format!(
                    "RHVoice native library was not found in documented installation locations; set {ENV_LIBRARY} to its absolute path"
                ))
            })?,
        };
        let library = unsafe { open_library(&library_path) }.map_err(|error| {
            RhVoiceError::Runtime(format!(
                "could not load RHVoice library {}: {error}",
                library_path.display()
            ))
        })?;
        let api = unsafe { RhVoiceApi::load(&library) }?;
        let version = native_string(unsafe { (api.get_version)() }, "runtime version")?;
        validate_runtime_version(&version)?;

        let data = optional_path_cstring(config.data.as_deref(), ENV_DATA)?;
        let config_path = optional_path_cstring(config.config.as_deref(), ENV_CONFIG)?;
        let resources = config
            .resources
            .iter()
            .map(|path| path_cstring(path, ENV_RESOURCES))
            .collect::<Result<Vec<_>, _>>()?;
        let mut resource_pointers = resources
            .iter()
            .map(|path| path.as_ptr())
            .collect::<Vec<_>>();
        resource_pointers.push(ptr::null());
        let init = NativeInitParams {
            data_path: data.as_ref().map_or(ptr::null(), |path| path.as_ptr()),
            config_path: config_path
                .as_ref()
                .map_or(ptr::null(), |path| path.as_ptr()),
            resource_paths: if resources.is_empty() {
                ptr::null()
            } else {
                resource_pointers.as_ptr()
            },
            callbacks: native_callbacks(),
            options: 0,
        };
        let native_engine = unsafe { (api.new_engine)(&init) };
        if native_engine.is_null() {
            return Err(RhVoiceError::Runtime(
                "RHVoice rejected its data, configuration, or voice resources".to_owned(),
            ));
        }

        let mut runtime = LoadedRuntime {
            api,
            engine: native_engine,
            voices: Vec::new(),
            _library: Some(library),
        };
        runtime.voices = discover_voices(&runtime)?;
        if runtime.voices.is_empty() {
            return Err(RhVoiceError::Runtime(
                "RHVoice loaded successfully but found no installed voices".to_owned(),
            ));
        }

        let descriptor = available_descriptor(&runtime.voices, version);
        Ok(Self {
            descriptor,
            runtime: Some(Mutex::new(runtime)),
            cancellation: Arc::new(AtomicBool::new(false)),
            speaking: AtomicBool::new(false),
        })
    }

    fn unavailable(reason: String) -> Self {
        Self {
            descriptor: EngineDescriptor {
                id: ENGINE_ID.to_owned(),
                display_name: "RHVoice".to_owned(),
                version: None,
                availability: Availability::Unavailable { reason },
                health: EngineHealth::Healthy,
                capabilities: capabilities(),
                voices: Vec::new(),
                default_voice_id: None,
            },
            runtime: None,
            cancellation: Arc::new(AtomicBool::new(false)),
            speaking: AtomicBool::new(false),
        }
    }

    fn runtime(&self) -> Result<MutexGuard<'_, LoadedRuntime>, TtsError> {
        self.runtime
            .as_ref()
            .ok_or(TtsError::NotAvailable)?
            .lock()
            .map_err(|error| {
                TtsError::SynthesisFailed(format!("RHVoice state lock is poisoned: {error}"))
            })
    }
}

impl TtsEngine for RhVoiceTtsEngine {
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
            .cloned()
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
            TtsError::InvalidParameter("RHVoice text contains a null byte".to_owned())
        })?;
        let text_length = c_uint::try_from(request.text.len())
            .map_err(|_| TtsError::InvalidParameter("RHVoice text is too large".to_owned()))?;
        let parameters = NativeSynthParams {
            voice_profile: voice.profile.as_ptr(),
            absolute_rate: map_rate(request.settings.rate),
            absolute_pitch: map_pitch(request.settings.pitch),
            absolute_volume: map_volume(request.settings.volume),
            relative_rate: 1.0,
            relative_pitch: 1.0,
            relative_volume: 1.0,
            punctuation_mode: 0,
            punctuation_list: ptr::null(),
            capitals_mode: 0,
            flags: 0,
        };

        self.cancellation.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        let mut capture = Box::new(SynthesisCapture::buffered(
            &self.cancellation,
            request.cancellation.as_ref(),
        ));
        let message = unsafe {
            (runtime.api.new_message)(
                runtime.engine,
                text.as_ptr(),
                text_length,
                0,
                &parameters,
                std::ptr::from_mut(&mut *capture).cast(),
            )
        };
        if message.is_null() {
            return Err(TtsError::SynthesisFailed(
                "RHVoice rejected the text, voice, or synthesis parameters".to_owned(),
            ));
        }
        let message = MessageGuard {
            api: runtime.api,
            message,
        };
        let status = unsafe { (runtime.api.speak)(message.message) };
        drop(message);

        if capture.cancelled() {
            return Err(TtsError::SynthesisFailed(
                "RHVoice synthesis was cancelled".to_owned(),
            ));
        }
        if let Some(failure) = capture.take_failure() {
            return Err(failure);
        }
        if status == 0 {
            return Err(TtsError::SynthesisFailed(
                "RHVoice synthesis did not complete".to_owned(),
            ));
        }
        let SynthesisCapture::Buffered(capture) = *capture else {
            unreachable!("buffered RHVoice synthesis used a streaming capture")
        };
        if capture.samples.is_empty() {
            let mut result = SynthesisResult::audio(ENGINE_ID, actual_voice, AudioBuffer::empty());
            result.degraded_acss = request
                .normalized_acss
                .clone()
                .degrade_for(&capabilities().acss)
                .omitted;
            return Ok(result);
        }
        let sample_rate = capture.sample_rate.ok_or_else(|| {
            TtsError::SynthesisFailed("RHVoice returned audio without a sample rate".to_owned())
        })?;
        let markers = markers_from_native(
            &request.text,
            &capture.markers,
            sample_rate,
            capture.samples.len() as u64,
        );
        let audio = AudioBuffer::try_from_interleaved_i16(&capture.samples, sample_rate, 1)
            .map_err(|error| {
                TtsError::SynthesisFailed(format!("could not canonicalize RHVoice PCM: {error}"))
            })?;
        let mut result = SynthesisResult::new(ENGINE_ID, actual_voice, audio, markers);
        result.degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&capabilities().acss)
            .omitted;
        Ok(result)
    }

    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        // RHVoice reports word starts only as synthesis advances. An `After`
        // anchor can therefore require a later word to resolve an earlier
        // boundary. Retain the established buffered path for anchored calls.
        if !request.anchors.is_empty() {
            let mut result = self.synthesize(request)?;
            result.resolve_anchors(request, AnchorSupport::WordBoundary);
            result.validate(request)?;
            let frame_count = result.audio.frame_count() as u64;
            sink.start(SynthesisStreamStart {
                engine_id: result.engine_id,
                actual_voice: result.actual_voice,
                degraded_acss: result.degraded_acss,
            })?;
            // The progressive contract requires metadata to precede any PCM
            // that passes its frame. A buffered fallback knows every marker,
            // so publish the complete batch before its one audio window.
            if !result.markers.is_empty() || !result.anchors.is_empty() {
                sink.markers(result.markers, result.anchors)?;
            }
            if !result.audio.is_empty() {
                sink.audio(result.audio)?;
            }
            return Ok(SynthesisStreamCompletion { frame_count });
        }

        let voice_id = request.voice_id_for_engine(ENGINE_ID)?;
        let runtime = self.runtime()?;
        let voice = runtime
            .voices
            .iter()
            .find(|voice| voice.id == voice_id || voice.name == voice_id)
            .cloned()
            .ok_or_else(|| TtsError::VoiceNotFound(voice_id.to_owned()))?;
        let actual_voice = Some(PhysicalVoiceId::new(ENGINE_ID, voice.id.clone()));
        let degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&capabilities().acss)
            .omitted;
        sink.start(SynthesisStreamStart {
            engine_id: ENGINE_ID.to_owned(),
            actual_voice,
            degraded_acss,
        })?;
        if request.text.is_empty() {
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }

        let text = CString::new(request.text.as_str()).map_err(|_| {
            TtsError::InvalidParameter("RHVoice text contains a null byte".to_owned())
        })?;
        let text_length = c_uint::try_from(request.text.len())
            .map_err(|_| TtsError::InvalidParameter("RHVoice text is too large".to_owned()))?;
        let parameters = NativeSynthParams {
            voice_profile: voice.profile.as_ptr(),
            absolute_rate: map_rate(request.settings.rate),
            absolute_pitch: map_pitch(request.settings.pitch),
            absolute_volume: map_volume(request.settings.volume),
            relative_rate: 1.0,
            relative_pitch: 1.0,
            relative_volume: 1.0,
            punctuation_mode: 0,
            punctuation_list: ptr::null(),
            capitals_mode: 0,
            flags: 0,
        };

        self.cancellation.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        let mut capture = Box::new(SynthesisCapture::streaming(
            sink,
            &request.text,
            &self.cancellation,
            request.cancellation.as_ref(),
        ));
        let message = unsafe {
            (runtime.api.new_message)(
                runtime.engine,
                text.as_ptr(),
                text_length,
                0,
                &parameters,
                std::ptr::from_mut(&mut *capture).cast(),
            )
        };
        if message.is_null() {
            return Err(TtsError::SynthesisFailed(
                "RHVoice rejected the text, voice, or synthesis parameters".to_owned(),
            ));
        }
        let message = MessageGuard {
            api: runtime.api,
            message,
        };
        let status = unsafe { (runtime.api.speak)(message.message) };
        drop(message);

        if capture.cancelled() {
            return Err(TtsError::SynthesisFailed(
                "RHVoice synthesis was cancelled".to_owned(),
            ));
        }
        if let Some(failure) = capture.take_failure() {
            return Err(failure);
        }
        if status == 0 {
            return Err(TtsError::SynthesisFailed(
                "RHVoice synthesis did not complete".to_owned(),
            ));
        }
        let SynthesisCapture::Streaming(mut capture) = *capture else {
            unreachable!("streaming RHVoice synthesis used a buffered capture")
        };
        let frame_count = capture.finish()?;
        Ok(SynthesisStreamCompletion { frame_count })
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

struct MessageGuard {
    api: RhVoiceApi,
    message: NativeMessage,
}

impl Drop for MessageGuard {
    fn drop(&mut self) {
        if !self.message.is_null() {
            unsafe { (self.api.delete_message)(self.message) };
            self.message = ptr::null_mut();
        }
    }
}

struct SpeakingGuard<'a>(&'a AtomicBool);

impl Drop for SpeakingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct BufferedCapture<'a> {
    samples: Vec<i16>,
    sample_rate: Option<u32>,
    markers: Vec<NativeMarker>,
    cancellation: &'a AtomicBool,
    request_cancellation: Option<&'a SynthesisCancellationToken>,
    failure: Option<TtsError>,
}

struct StreamingCapture<'a> {
    sink: &'a mut dyn SynthesisStreamSink,
    text: &'a str,
    cancellation: &'a AtomicBool,
    request_cancellation: Option<&'a SynthesisCancellationToken>,
    canonicalizer: Option<Box<ProgressivePcmCanonicalizer>>,
    sample_rate: Option<u32>,
    native_samples: usize,
    marker_count: usize,
    pending_markers: Vec<NativeMarker>,
    failure: Option<TtsError>,
}

enum SynthesisCapture<'a> {
    Buffered(BufferedCapture<'a>),
    Streaming(StreamingCapture<'a>),
}

impl<'a> SynthesisCapture<'a> {
    fn buffered(
        cancellation: &'a AtomicBool,
        request_cancellation: Option<&'a SynthesisCancellationToken>,
    ) -> Self {
        Self::Buffered(BufferedCapture {
            samples: Vec::new(),
            sample_rate: None,
            markers: Vec::new(),
            cancellation,
            request_cancellation,
            failure: None,
        })
    }

    fn streaming(
        sink: &'a mut dyn SynthesisStreamSink,
        text: &'a str,
        cancellation: &'a AtomicBool,
        request_cancellation: Option<&'a SynthesisCancellationToken>,
    ) -> Self {
        Self::Streaming(StreamingCapture {
            sink,
            text,
            cancellation,
            request_cancellation,
            canonicalizer: None,
            sample_rate: None,
            native_samples: 0,
            marker_count: 0,
            pending_markers: Vec::new(),
            failure: None,
        })
    }

    fn cancelled(&self) -> bool {
        let (cancellation, request_cancellation) = match self {
            Self::Buffered(capture) => (capture.cancellation, capture.request_cancellation),
            Self::Streaming(capture) => (capture.cancellation, capture.request_cancellation),
        };
        cancellation.load(Ordering::Acquire)
            || request_cancellation.is_some_and(SynthesisCancellationToken::is_cancelled)
    }

    fn failed(&self) -> bool {
        match self {
            Self::Buffered(capture) => capture.failure.is_some(),
            Self::Streaming(capture) => capture.failure.is_some(),
        }
    }

    fn should_continue(&self) -> bool {
        !self.failed() && !self.cancelled()
    }

    fn take_failure(&mut self) -> Option<TtsError> {
        match self {
            Self::Buffered(capture) => capture.failure.take(),
            Self::Streaming(capture) => capture.failure.take(),
        }
    }

    fn fail(&mut self, error: TtsError) {
        match self {
            Self::Buffered(capture) => capture.failure = Some(error),
            Self::Streaming(capture) => capture.failure = Some(error),
        }
    }

    fn set_sample_rate(&mut self, sample_rate: u32) -> Result<c_int, TtsError> {
        match self {
            Self::Buffered(capture) => {
                if capture
                    .sample_rate
                    .replace(sample_rate)
                    .is_some_and(|previous| previous != sample_rate)
                {
                    return Err(synthesis_failure(
                        "RHVoice changed sample rate within one utterance",
                    ));
                }
            }
            Self::Streaming(capture) => capture.set_sample_rate(sample_rate)?,
        }
        Ok(i32::from(self.should_continue()))
    }

    fn play_speech(&mut self, samples: &[i16]) -> Result<c_int, TtsError> {
        match self {
            Self::Buffered(capture) => {
                if samples.len() > MAX_NATIVE_SAMPLES.saturating_sub(capture.samples.len()) {
                    return Err(synthesis_failure(
                        "RHVoice PCM exceeded the helper synthesis limit",
                    ));
                }
                capture
                    .samples
                    .try_reserve(samples.len())
                    .map_err(|_| synthesis_failure("could not allocate the RHVoice PCM buffer"))?;
                capture.samples.extend_from_slice(samples);
            }
            Self::Streaming(capture) => capture.play_speech(samples)?,
        }
        Ok(i32::from(self.should_continue()))
    }

    fn record_marker(
        &mut self,
        kind: SynthesisMarkerKind,
        start: c_uint,
        length: c_uint,
    ) -> Result<c_int, TtsError> {
        match self {
            Self::Buffered(capture) => {
                if capture.markers.len() >= MAX_HELPER_MARKERS {
                    return Err(synthesis_failure(
                        "RHVoice returned too many synchronization markers",
                    ));
                }
                capture.markers.push(NativeMarker {
                    kind,
                    native_frame: capture.samples.len() as u64,
                    text_start: start,
                    text_length: length,
                });
            }
            Self::Streaming(capture) => capture.record_marker(kind, start, length)?,
        }
        Ok(i32::from(self.should_continue()))
    }
}

impl StreamingCapture<'_> {
    fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), TtsError> {
        if self
            .sample_rate
            .replace(sample_rate)
            .is_some_and(|previous| previous != sample_rate)
        {
            return Err(synthesis_failure(
                "RHVoice changed sample rate within one utterance",
            ));
        }
        if self.canonicalizer.is_none() {
            self.canonicalizer = Some(Box::new(
                ProgressivePcmCanonicalizer::new(sample_rate, 1).map_err(|error| {
                    synthesis_failure(format!(
                        "could not initialize progressive RHVoice PCM conversion: {error}"
                    ))
                })?,
            ));
            let pending = std::mem::take(&mut self.pending_markers);
            self.emit_native_markers(pending)?;
        }
        Ok(())
    }

    fn play_speech(&mut self, samples: &[i16]) -> Result<(), TtsError> {
        if samples.len() > MAX_NATIVE_SAMPLES.saturating_sub(self.native_samples) {
            return Err(synthesis_failure(
                "RHVoice PCM exceeded the helper synthesis limit",
            ));
        }
        self.native_samples += samples.len();
        let canonicalizer = self
            .canonicalizer
            .as_mut()
            .ok_or_else(|| synthesis_failure("RHVoice returned audio without a sample rate"))?;
        let windows = canonicalizer
            .push_interleaved_i16(samples)
            .map_err(|error| {
                synthesis_failure(format!(
                    "could not canonicalize progressive RHVoice PCM: {error}"
                ))
            })?;
        self.emit(windows)
    }

    fn record_marker(
        &mut self,
        kind: SynthesisMarkerKind,
        text_start: u32,
        text_length: u32,
    ) -> Result<(), TtsError> {
        if self.marker_count >= MAX_HELPER_MARKERS {
            return Err(synthesis_failure(
                "RHVoice returned too many synchronization markers",
            ));
        }
        self.marker_count += 1;
        let marker = NativeMarker {
            kind,
            native_frame: self.native_samples as u64,
            text_start,
            text_length,
        };
        if self.canonicalizer.is_none() {
            self.pending_markers.push(marker);
            return Ok(());
        }
        self.emit_native_markers(vec![marker])
    }

    fn emit_native_markers(&mut self, markers: Vec<NativeMarker>) -> Result<(), TtsError> {
        let canonicalizer = self
            .canonicalizer
            .as_ref()
            .expect("RHVoice marker emission requires a sample rate");
        let markers = markers
            .into_iter()
            .filter_map(|marker| {
                let start = marker.text_start as usize;
                let length = marker.text_length as usize;
                let end = start.checked_add(length)?;
                if end > self.text.len()
                    || !self.text.is_char_boundary(start)
                    || !self.text.is_char_boundary(end)
                {
                    return None;
                }
                let frame_offset = canonicalizer
                    .canonical_frame_offset(marker.native_frame)
                    .ok()?;
                Some(SynthesisMarker {
                    kind: marker.kind,
                    frame_offset,
                    text_start: Some(marker.text_start),
                    text_length: Some(marker.text_length),
                    value: None,
                })
            })
            .collect::<Vec<_>>();
        if !markers.is_empty() {
            self.sink.markers(markers, Vec::new())?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<u64, TtsError> {
        if self.native_samples == 0 {
            return Ok(0);
        }
        let canonicalizer = self
            .canonicalizer
            .as_mut()
            .ok_or_else(|| synthesis_failure("RHVoice returned audio without a sample rate"))?;
        let windows = canonicalizer.finish().map_err(|error| {
            synthesis_failure(format!(
                "could not finish progressive RHVoice PCM conversion: {error}"
            ))
        })?;
        self.emit(windows)?;
        Ok(self
            .canonicalizer
            .as_ref()
            .expect("RHVoice canonicalizer remains available")
            .output_frames())
    }

    fn emit(&mut self, windows: Vec<AudioBuffer>) -> Result<(), TtsError> {
        for window in windows {
            if !window.is_empty() {
                self.sink.audio(window)?;
            }
        }
        Ok(())
    }
}

fn synthesis_failure(message: impl Into<String>) -> TtsError {
    TtsError::SynthesisFailed(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeMarker {
    kind: SynthesisMarkerKind,
    native_frame: u64,
    text_start: u32,
    text_length: u32,
}

fn native_callbacks() -> NativeCallbacks {
    NativeCallbacks {
        set_sample_rate: Some(set_sample_rate_callback),
        play_speech: Some(play_speech_callback),
        process_mark: None,
        word_starts: Some(word_starts_callback),
        word_ends: None,
        sentence_starts: Some(sentence_starts_callback),
        sentence_ends: None,
        play_audio: None,
        done: Some(done_callback),
    }
}

unsafe extern "C" fn set_sample_rate_callback(sample_rate: c_int, user_data: *mut c_void) -> c_int {
    with_capture(user_data, |capture| {
        let Ok(sample_rate) = u32::try_from(sample_rate) else {
            return Err(synthesis_failure("RHVoice returned an invalid sample rate"));
        };
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(synthesis_failure("RHVoice returned an invalid sample rate"));
        }
        capture.set_sample_rate(sample_rate)
    })
}

unsafe extern "C" fn play_speech_callback(
    samples: *const c_short,
    count: c_uint,
    user_data: *mut c_void,
) -> c_int {
    with_capture(user_data, |capture| {
        if !capture.should_continue() {
            return Ok(0);
        }
        let count = count as usize;
        if count == 0 {
            return Ok(1);
        }
        if samples.is_null() {
            return Err(synthesis_failure("RHVoice returned a null audio chunk"));
        }
        let samples = unsafe { std::slice::from_raw_parts(samples, count) };
        capture.play_speech(samples)
    })
}

unsafe extern "C" fn word_starts_callback(
    position: c_uint,
    length: c_uint,
    user_data: *mut c_void,
) -> c_int {
    with_capture(user_data, |capture| {
        capture.record_marker(SynthesisMarkerKind::Word, position, length)
    })
}

unsafe extern "C" fn sentence_starts_callback(
    position: c_uint,
    length: c_uint,
    user_data: *mut c_void,
) -> c_int {
    with_capture(user_data, |capture| {
        capture.record_marker(SynthesisMarkerKind::Sentence, position, length)
    })
}

unsafe extern "C" fn done_callback(_user_data: *mut c_void) {}

unsafe fn with_capture(
    user_data: *mut c_void,
    callback: impl FnOnce(&mut SynthesisCapture<'_>) -> Result<c_int, TtsError>,
) -> c_int {
    if user_data.is_null() {
        return 0;
    }
    let capture = unsafe { &mut *user_data.cast::<SynthesisCapture<'_>>() };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(capture))) {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            capture.fail(error);
            0
        }
        Err(_) => {
            capture.fail(synthesis_failure("RHVoice callback processing panicked"));
            0
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
        audio_output: AudioOutputMode::StreamingPcm,
        cancellation: CancellationSupport::SynthesisAndPlayback,
        concurrency: ConcurrencyModel::Serialized,
        markers: MarkerCapabilities {
            word: true,
            sentence: true,
            requested_anchors: AnchorSupport::WordBoundary,
            ..MarkerCapabilities::default()
        },
        language_switching: false,
        text_repertoire: TextRepertoire::Unicode,
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

fn available_descriptor(voices: &[VoiceRecord], version: String) -> EngineDescriptor {
    let voices = voices
        .iter()
        .map(|voice| VoiceDescriptor {
            id: PhysicalVoiceId::new(ENGINE_ID, voice.id.clone()),
            display_name: voice.name.clone(),
            language: (!voice.language.is_empty()).then(|| voice.language.clone()),
            gender: voice.gender,
            quality: VoiceQuality::Enhanced,
            availability: Availability::Available,
        })
        .collect::<Vec<_>>();
    EngineDescriptor {
        id: ENGINE_ID.to_owned(),
        display_name: "RHVoice".to_owned(),
        version: Some(version),
        availability: Availability::Available,
        health: EngineHealth::Healthy,
        capabilities: capabilities(),
        default_voice_id: voices.first().map(|voice| voice.id.voice_id.clone()),
        voices,
    }
}

fn discover_voices(runtime: &LoadedRuntime) -> Result<Vec<VoiceRecord>, RhVoiceError> {
    let count = unsafe { (runtime.api.get_voice_count)(runtime.engine) } as usize;
    if count > 4_096 {
        return Err(RhVoiceError::Runtime(format!(
            "RHVoice reported {count} voices, exceeding the 4096-voice limit"
        )));
    }
    let voices = unsafe { (runtime.api.get_voices)(runtime.engine) };
    if count != 0 && voices.is_null() {
        return Err(RhVoiceError::Runtime(
            "RHVoice returned a null voice inventory".to_owned(),
        ));
    }
    let mut result = Vec::with_capacity(count);
    let mut ids = BTreeSet::new();
    for index in 0..count {
        let voice = unsafe { &*voices.add(index) };
        let name = native_string(voice.name, "voice name")?;
        if name.is_empty() {
            return Err(RhVoiceError::Runtime(
                "RHVoice returned an empty voice name".to_owned(),
            ));
        }
        let id = format!("{ENGINE_ID}:{name}");
        if !ids.insert(id.clone()) {
            return Err(RhVoiceError::Runtime(format!(
                "RHVoice repeated voice name {name}"
            )));
        }
        let language = optional_native_string(voice.language, "voice language")?;
        let country = optional_native_string(voice.country, "voice country")?;
        result.push(VoiceRecord {
            id,
            profile: CString::new(name.as_str()).expect("a C string cannot contain an inner null"),
            name,
            language: language_tag(&language, &country),
            gender: match voice.gender {
                1 => Some(VoiceGender::Male),
                2 => Some(VoiceGender::Female),
                _ => None,
            },
        });
    }
    Ok(result)
}

fn native_string(pointer: *const c_char, field: &str) -> Result<String, RhVoiceError> {
    if pointer.is_null() {
        return Err(RhVoiceError::Runtime(format!(
            "RHVoice returned a null {field}"
        )));
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| RhVoiceError::Runtime(format!("RHVoice returned non-UTF-8 {field}")))
}

fn optional_native_string(pointer: *const c_char, field: &str) -> Result<String, RhVoiceError> {
    if pointer.is_null() {
        Ok(String::new())
    } else {
        native_string(pointer, field)
    }
}

fn validate_runtime_version(version: &str) -> Result<(), RhVoiceError> {
    let (major, minor) = parse_major_minor(version).ok_or_else(|| {
        RhVoiceError::Runtime(format!("RHVoice returned unrecognized version {version:?}"))
    })?;
    if major != 1 || minor < MINIMUM_SUPPORTED_MINOR {
        return Err(RhVoiceError::Runtime(format!(
            "RHVoice {version} is unsupported; install an ABI-compatible 1.{MINIMUM_SUPPORTED_MINOR} or later 1.x runtime"
        )));
    }
    Ok(())
}

fn parse_major_minor(version: &str) -> Option<(u32, u32)> {
    let mut components = version.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next()?.parse().ok()?;
    Some((major, minor))
}

fn language_tag(language: &str, country: &str) -> String {
    let language = language.replace('_', "-");
    let country = country.replace('_', "-");
    if language.is_empty() {
        country
    } else if country.is_empty()
        || language.eq_ignore_ascii_case(&country)
        || language
            .split('-')
            .any(|component| component.eq_ignore_ascii_case(&country))
    {
        language
    } else {
        format!("{language}-{country}")
    }
}

fn map_rate(rate: f32) -> f64 {
    // Measured reference and saturation policy: docs/RATE-CALIBRATION.md.
    const CALIBRATION: &[(f32, f32)] = &[
        (0.0, -1.000_000),
        (0.1, -0.798_056),
        (0.2, -0.472_908),
        (0.3, -0.140_148),
        (0.4, 0.176_818),
        (0.5, 0.446_861),
        (0.6, 0.823_423),
        (0.7, 1.000_000),
    ];
    f64::from(interpolate(rate, CALIBRATION))
}

fn map_pitch(pitch: f32) -> f64 {
    let pitch = if pitch.is_finite() { pitch } else { 1.0 }.clamp(0.5, 2.0);
    if pitch < 1.0 {
        f64::from((pitch - 1.0) / 0.5)
    } else {
        f64::from(pitch - 1.0)
    }
}

fn map_volume(volume: f32) -> f64 {
    f64::from(if volume.is_finite() { volume } else { 1.0 }.clamp(0.0, 1.0) - 1.0)
}

fn markers_from_native(
    text: &str,
    markers: &[NativeMarker],
    sample_rate: u32,
    native_frame_count: u64,
) -> Vec<SynthesisMarker> {
    let mut result = markers
        .iter()
        .filter_map(|marker| {
            let start = marker.text_start as usize;
            let length = marker.text_length as usize;
            let end = start.checked_add(length)?;
            if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return None;
            }
            Some(SynthesisMarker {
                kind: marker.kind,
                frame_offset: scale_frame(marker.native_frame.min(native_frame_count), sample_rate),
                text_start: Some(marker.text_start),
                text_length: Some(marker.text_length),
                value: None,
            })
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|marker| marker.frame_offset);
    result
}

fn scale_frame(frame: u64, source_rate: u32) -> u64 {
    frame
        .saturating_mul(u64::from(STANDARD_SAMPLE_RATE))
        .saturating_add(u64::from(source_rate) / 2)
        / u64::from(source_rate)
}

fn absolute_file_from_env(name: &str) -> Result<Option<PathBuf>, RhVoiceError> {
    let Some(value) = nonempty_env(name) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    validate_absolute(&path, name)?;
    if !path.is_file() {
        return Err(RhVoiceError::Configuration(format!(
            "{name} does not name a file: {}",
            path.display()
        )));
    }
    canonical_path(path, name).map(Some)
}

fn absolute_directory_from_env(name: &str) -> Result<Option<PathBuf>, RhVoiceError> {
    let Some(value) = nonempty_env(name) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    validate_absolute(&path, name)?;
    if !path.is_dir() {
        return Err(RhVoiceError::Configuration(format!(
            "{name} does not name a directory: {}",
            path.display()
        )));
    }
    canonical_path(path, name).map(Some)
}

fn absolute_directories_from_env(name: &str) -> Result<Vec<PathBuf>, RhVoiceError> {
    let Some(value) = nonempty_env(name) else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for path in std::env::split_paths(&value) {
        validate_absolute(&path, name)?;
        if !path.is_dir() {
            return Err(RhVoiceError::Configuration(format!(
                "{name} contains a path that is not a directory: {}",
                path.display()
            )));
        }
        paths.push(canonical_path(path, name)?);
    }
    Ok(paths)
}

fn nonempty_env(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

fn validate_absolute(path: &Path, name: &str) -> Result<(), RhVoiceError> {
    if !path.is_absolute() {
        return Err(RhVoiceError::Configuration(format!(
            "{name} must be an absolute path: {}",
            path.display()
        )));
    }
    Ok(())
}

fn canonical_path(path: PathBuf, name: &str) -> Result<PathBuf, RhVoiceError> {
    path.canonicalize().map_err(|error| {
        RhVoiceError::Configuration(format!(
            "could not resolve {name} path {}: {error}",
            path.display()
        ))
    })
}

fn optional_path_cstring(
    path: Option<&Path>,
    source: &str,
) -> Result<Option<CString>, RhVoiceError> {
    path.map(|path| path_cstring(path, source)).transpose()
}

fn path_cstring(path: &Path, source: &str) -> Result<CString, RhVoiceError> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| RhVoiceError::Configuration(format!("{source} path contains a null byte")))
}

fn discover_library() -> Option<PathBuf> {
    for directory in library_directories() {
        for name in library_names() {
            let candidate = directory.join(name);
            if candidate.is_file() {
                if let Ok(candidate) = candidate.canonicalize() {
                    return Some(candidate);
                }
            }
        }
        if let Some(candidate) = discover_versioned_library(&directory) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(target_os = "windows")]
unsafe fn open_library(path: &Path) -> Result<Library, libloading::Error> {
    use libloading::os::windows::{
        Library as WindowsLibrary, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };

    WindowsLibrary::load_with_flags(
        path,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
    )
    .map(Into::into)
}

#[cfg(not(target_os = "windows"))]
unsafe fn open_library(path: &Path) -> Result<Library, libloading::Error> {
    Library::new(path)
}

fn discover_versioned_library(directory: &Path) -> Option<PathBuf> {
    let mut candidates = std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if cfg!(target_os = "macos") {
                name.starts_with("libRHVoice.") && name.ends_with(".dylib")
            } else if cfg!(target_os = "linux") {
                name.starts_with("libRHVoice.so.")
            } else {
                false
            }
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .rev()
        .find_map(|path| path.is_file().then(|| path.canonicalize().ok()).flatten())
}

fn library_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["RHVoice.dll"]
    } else if cfg!(target_os = "macos") {
        &["libRHVoice.dylib", "libRHVoice.1.dylib"]
    } else {
        &["libRHVoice.so", "libRHVoice.so.5", "libRHVoice.so.1"]
    }
}

fn library_directories() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let multiarch = match std::env::consts::ARCH {
            "x86_64" => Some("x86_64-linux-gnu"),
            "aarch64" => Some("aarch64-linux-gnu"),
            "x86" => Some("i386-linux-gnu"),
            "arm" => Some("arm-linux-gnueabihf"),
            _ => None,
        };
        let mut directories = Vec::new();
        if let Some(multiarch) = multiarch {
            directories.push(PathBuf::from("/usr/lib").join(multiarch));
            directories.push(PathBuf::from("/lib").join(multiarch));
        }
        directories.extend(
            [
                "/usr/local/lib",
                "/usr/local/lib64",
                "/usr/lib",
                "/usr/lib64",
            ]
            .into_iter()
            .map(PathBuf::from),
        );
        directories
    }
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/opt/homebrew/lib"),
            PathBuf::from("/usr/local/lib"),
            PathBuf::from("/opt/local/lib"),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        Vec::new()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        events: Vec<&'static str>,
        frames: u64,
        markers: Vec<SynthesisMarker>,
    }

    impl SynthesisStreamSink for RecordingSink {
        fn start(&mut self, _start: SynthesisStreamStart) -> Result<(), TtsError> {
            self.events.push("start");
            Ok(())
        }

        fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
            self.events.push("audio");
            self.frames += audio.frame_count() as u64;
            Ok(())
        }

        fn markers(
            &mut self,
            markers: Vec<SynthesisMarker>,
            anchors: Vec<omnivox_tts::ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            assert!(anchors.is_empty());
            self.events.push("markers");
            self.markers.extend(markers);
            Ok(())
        }
    }

    #[test]
    fn parameter_mappings_preserve_calibrated_rate_and_other_endpoints() {
        assert_eq!(map_rate(0.0), -1.0);
        assert!((map_rate(0.5) - 0.446_861).abs() < 0.000_001);
        assert_eq!(map_rate(0.7), 1.0);
        assert_eq!(map_rate(2.0), 1.0);
        let mapped: Vec<_> = (0..=20)
            .map(|point| map_rate(point as f32 / 10.0))
            .collect();
        assert!(mapped.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(map_pitch(0.5), -1.0);
        assert_eq!(map_pitch(1.0), 0.0);
        assert_eq!(map_pitch(2.0), 1.0);
        assert_eq!(map_volume(0.0), -1.0);
        assert_eq!(map_volume(1.0), 0.0);
    }

    #[test]
    fn supported_runtime_versions_keep_the_stable_one_x_abi() {
        assert!(validate_runtime_version("1.14.0").is_ok());
        assert!(validate_runtime_version("1.18.4").is_ok());
        assert!(validate_runtime_version("1.99.0").is_ok());
        assert!(validate_runtime_version("1.13.9").is_err());
        assert!(validate_runtime_version("2.0.0").is_err());
        assert!(validate_runtime_version("development").is_err());
    }

    #[test]
    fn language_and_country_become_one_bcp47_style_tag() {
        assert_eq!(language_tag("en", "US"), "en-US");
        assert_eq!(language_tag("en_US", "US"), "en-US");
        assert_eq!(language_tag("eo", ""), "eo");
        assert_eq!(language_tag("", "GE"), "GE");
    }

    #[test]
    fn native_markers_are_validated_and_rescaled() {
        let markers = markers_from_native(
            "héllo world",
            &[
                NativeMarker {
                    kind: SynthesisMarkerKind::Word,
                    native_frame: 12_000,
                    text_start: 0,
                    text_length: 6,
                },
                NativeMarker {
                    kind: SynthesisMarkerKind::Word,
                    native_frame: 24_000,
                    text_start: 1,
                    text_length: 1,
                },
            ],
            24_000,
            24_000,
        );

        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].frame_offset, 22_050);
        assert_eq!(markers[0].text_start, Some(0));
        assert_eq!(markers[0].text_length, Some(6));
    }

    #[test]
    fn streaming_callbacks_emit_markers_before_progressive_audio() {
        let cancellation = AtomicBool::new(false);
        let mut sink = RecordingSink::default();
        let mut capture =
            SynthesisCapture::streaming(&mut sink, "hello world", &cancellation, None);
        let pointer = std::ptr::from_mut(&mut capture).cast();

        assert_eq!(unsafe { set_sample_rate_callback(24_000, pointer) }, 1);
        assert_eq!(unsafe { word_starts_callback(0, 5, pointer) }, 1);
        let samples = vec![1_000_i16; 1_024];
        assert_eq!(
            unsafe { play_speech_callback(samples.as_ptr(), samples.len() as u32, pointer) },
            1
        );
        let SynthesisCapture::Streaming(mut capture) = capture else {
            unreachable!()
        };
        let frame_count = capture.finish().unwrap();
        drop(capture);

        assert_eq!(frame_count, sink.frames);
        assert_eq!(sink.events.first(), Some(&"markers"));
        assert!(sink.events.contains(&"audio"));
        assert_eq!(sink.markers.len(), 1);
        assert_eq!(sink.markers[0].frame_offset, 0);
        assert_eq!(sink.markers[0].text_start, Some(0));
        assert_eq!(sink.markers[0].text_length, Some(5));
    }

    #[test]
    fn missing_runtime_still_has_a_protocol_usable_descriptor() {
        let engine = RhVoiceTtsEngine::unavailable("runtime missing".to_owned());
        let descriptor = engine.descriptor();

        assert_eq!(descriptor.id, ENGINE_ID);
        assert!(!descriptor.can_synthesize());
        assert!(descriptor.voices.is_empty());
        assert_eq!(descriptor.default_voice_id, None);
        assert_eq!(
            descriptor.capabilities.audio_output,
            AudioOutputMode::StreamingPcm
        );
        assert!(descriptor.capabilities.markers.word);
        assert!(descriptor.capabilities.markers.sentence);
    }
}
