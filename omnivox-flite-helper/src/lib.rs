//! Flite v2.2 adapter for the isolated Omnivox companion.

use std::collections::HashSet;
use std::env;
use std::ffi::{c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use omnivox_audio::ProgressivePcmCanonicalizer;
use omnivox_flite_sys::{FliteSynthesis, FliteVoice, FliteWordMarker};
use omnivox_tts::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AnchorSupport, AudioOutputMode,
    Availability, CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor,
    EngineHealth, MarkerCapabilities, PhysicalVoiceId, TextRepertoire, VoiceDescriptor,
};
use omnivox_tts::helper_protocol::{MAX_HELPER_MARKERS, MAX_HELPER_SYNTHESIS_BYTES};
use omnivox_tts::rate_calibration::interpolate;
use omnivox_tts::{
    AnchorAffinity, AnchorResolution, AudioBuffer, RequestedAnchor, ResolvedAnchor,
    SynthesisCancellationToken, SynthesisMarker, SynthesisMarkerKind, SynthesisRequest,
    SynthesisResult, SynthesisStreamCompletion, SynthesisStreamSink, SynthesisStreamStart,
    TtsEngine, TtsError, VoiceInfo, VoiceQuality, STANDARD_SAMPLE_RATE,
};

const ENGINE_ID: &str = "flite";
const BUILT_IN_VOICE_ID: &str = "cmu_us_slt";
const EXTERNAL_VOICES_ENV: &str = "OMNIVOX_FLITE_VOICES";
const MAX_EXTERNAL_VOICES: usize = 64;
const MAX_NATIVE_SAMPLES: usize = MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<i16>();
static FLITE_GLOBAL_STATE: Mutex<()> = Mutex::new(());

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
        let _global = FLITE_GLOBAL_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _global = lock_flite_global()?;
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

fn lock_flite_global() -> Result<MutexGuard<'static, ()>, TtsError> {
    FLITE_GLOBAL_STATE.lock().map_err(|error| {
        TtsError::SynthesisFailed(format!("Flite global state lock is poisoned: {error}"))
    })
}

impl TtsEngine for FliteTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let voice_id = request.voice_id_for_engine(ENGINE_ID)?;
        let _global = lock_flite_global()?;
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
        let synthesis = unsafe {
            omnivox_flite_sys::omnivox_flite_synthesize(
                voice.pointer,
                text.as_ptr(),
                map_rate_to_duration(request.settings.rate),
                request.settings.pitch.clamp(0.5, 2.0),
            )
        };
        if synthesis.is_null() {
            return Err(TtsError::SynthesisFailed(
                "Flite did not return a synthesis result".to_owned(),
            ));
        }
        let synthesis = SynthesisGuard(synthesis);
        if self.cancellation.load(Ordering::Acquire) {
            return Err(TtsError::SynthesisFailed(
                "Flite synthesis was cancelled".to_owned(),
            ));
        }

        let sample_rate = positive_u32(
            unsafe { omnivox_flite_sys::omnivox_flite_synthesis_sample_rate(synthesis.0) },
            "sample rate",
        )?;
        let frame_count = positive_usize(
            unsafe { omnivox_flite_sys::omnivox_flite_synthesis_sample_count(synthesis.0) },
            "sample count",
        )?;
        let channels = u16::try_from(unsafe {
            omnivox_flite_sys::omnivox_flite_synthesis_channel_count(synthesis.0)
        })
        .ok()
        .filter(|channels| matches!(channels, 1 | 2))
        .ok_or_else(|| TtsError::SynthesisFailed("Flite returned invalid channels".to_owned()))?;
        let sample_count = frame_count
            .checked_mul(usize::from(channels))
            .filter(|count| *count <= MAX_NATIVE_SAMPLES)
            .ok_or_else(|| {
                TtsError::SynthesisFailed("Flite PCM exceeds the helper limit".to_owned())
            })?;
        let samples_pointer =
            unsafe { omnivox_flite_sys::omnivox_flite_synthesis_samples(synthesis.0) };
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
        let markers = native_word_markers(
            synthesis.0,
            &text,
            &request.text,
            sample_rate,
            frame_count as u64,
            audio.frame_count() as u64,
        )?;
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
        let voice_id = request.voice_id_for_engine(ENGINE_ID)?;
        let _global = lock_flite_global()?;
        let runtime = self.runtime()?;
        let voice = runtime
            .voices
            .iter()
            .find(|voice| voice.id == voice_id || voice.name == voice_id)
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
            let anchors = resolve_word_anchors(&request.anchors, &[]);
            if !anchors.is_empty() {
                sink.markers(Vec::new(), anchors)?;
            }
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }

        let text = CString::new(request.text.as_str()).map_err(|_| {
            TtsError::InvalidParameter("Flite text contains a null byte".to_owned())
        })?;
        self.cancellation.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        let mut capture = FliteStreamCapture {
            sink,
            text: &request.text,
            anchors: &request.anchors,
            cancellation: &self.cancellation,
            request_cancellation: request.cancellation.as_ref(),
            volume: request.settings.volume.clamp(0.0, 1.0),
            canonicalizer: None,
            sample_rate: None,
            channels: None,
            native_samples: 0,
            timing_emitted: false,
            saw_last: false,
            failure: None,
        };
        let synthesis = unsafe {
            omnivox_flite_sys::omnivox_flite_synthesize_stream(
                voice.pointer,
                text.as_ptr(),
                map_rate_to_duration(request.settings.rate),
                request.settings.pitch.clamp(0.5, 2.0),
                MAX_HELPER_MARKERS as c_int,
                consume_flite_stream,
                std::ptr::from_mut(&mut capture).cast(),
            )
        };
        let synthesis = (!synthesis.is_null()).then(|| SynthesisGuard(synthesis));

        if let Some(failure) = capture.failure.take() {
            return Err(failure);
        }
        if capture.cancelled() {
            return Err(TtsError::SynthesisFailed(
                "Flite synthesis was cancelled".to_owned(),
            ));
        }
        if synthesis.is_none() {
            return Err(TtsError::SynthesisFailed(
                "Flite did not return a streaming synthesis result".to_owned(),
            ));
        }
        if !capture.timing_emitted || !capture.saw_last || capture.native_samples == 0 {
            return Err(TtsError::SynthesisFailed(
                "Flite did not complete its PCM stream".to_owned(),
            ));
        }
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

struct SpeakingGuard<'a>(&'a AtomicBool);

impl Drop for SpeakingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct SynthesisGuard(*mut FliteSynthesis);

impl Drop for SynthesisGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { omnivox_flite_sys::omnivox_flite_delete_synthesis(self.0) };
            self.0 = ptr::null_mut();
        }
    }
}

struct FliteStreamCapture<'a> {
    sink: &'a mut dyn SynthesisStreamSink,
    text: &'a str,
    anchors: &'a [RequestedAnchor],
    cancellation: &'a AtomicBool,
    request_cancellation: Option<&'a SynthesisCancellationToken>,
    volume: f32,
    canonicalizer: Option<ProgressivePcmCanonicalizer>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    native_samples: usize,
    timing_emitted: bool,
    saw_last: bool,
    failure: Option<TtsError>,
}

struct NativeStreamChunk {
    samples: *const i16,
    sample_count: c_int,
    sample_rate: c_int,
    channel_count: c_int,
    last: c_int,
    markers: *const FliteWordMarker,
    marker_count: c_int,
}

impl FliteStreamCapture<'_> {
    fn cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
            || self
                .request_cancellation
                .is_some_and(SynthesisCancellationToken::is_cancelled)
    }

    fn consume(&mut self, chunk: NativeStreamChunk) -> Result<c_int, TtsError> {
        if self.cancelled() {
            return Ok(0);
        }
        let sample_rate = positive_u32(chunk.sample_rate, "stream sample rate")?;
        let channels = u16::try_from(chunk.channel_count)
            .ok()
            .filter(|channels| matches!(channels, 1 | 2))
            .ok_or_else(|| {
                TtsError::SynthesisFailed("Flite returned invalid stream channels".to_owned())
            })?;
        if self
            .sample_rate
            .replace(sample_rate)
            .is_some_and(|previous| previous != sample_rate)
            || self
                .channels
                .replace(channels)
                .is_some_and(|previous| previous != channels)
        {
            return Err(TtsError::SynthesisFailed(
                "Flite changed PCM format within one stream".to_owned(),
            ));
        }
        if self.canonicalizer.is_none() {
            self.canonicalizer = Some(
                ProgressivePcmCanonicalizer::new(sample_rate, channels).map_err(|error| {
                    TtsError::SynthesisFailed(format!(
                        "could not initialize progressive Flite PCM conversion: {error}"
                    ))
                })?,
            );
        }

        if !self.timing_emitted {
            let marker_count = usize::try_from(chunk.marker_count).map_err(|_| {
                TtsError::SynthesisFailed(
                    "Flite could not produce its streaming word markers".to_owned(),
                )
            })?;
            if marker_count > MAX_HELPER_MARKERS {
                return Err(TtsError::SynthesisFailed(
                    "Flite returned too many streaming word markers".to_owned(),
                ));
            }
            if marker_count != 0 && chunk.markers.is_null() {
                return Err(TtsError::SynthesisFailed(
                    "Flite returned null streaming word markers".to_owned(),
                ));
            }
            let native_markers = if marker_count == 0 {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(chunk.markers, marker_count) }
            };
            let markers = progressive_word_markers(
                native_markers,
                self.text,
                self.canonicalizer
                    .as_ref()
                    .expect("Flite canonicalizer was initialized"),
            )?;
            let anchors = resolve_word_anchors(self.anchors, &markers);
            if !markers.is_empty() || !anchors.is_empty() {
                self.sink.markers(markers, anchors)?;
            }
            self.timing_emitted = true;
        } else if chunk.marker_count != 0 {
            return Err(TtsError::SynthesisFailed(
                "Flite returned its streaming word markers more than once".to_owned(),
            ));
        }

        let sample_count = usize::try_from(chunk.sample_count).map_err(|_| {
            TtsError::SynthesisFailed("Flite returned a negative PCM chunk size".to_owned())
        })?;
        if !sample_count.is_multiple_of(usize::from(channels)) {
            return Err(TtsError::SynthesisFailed(
                "Flite returned a channel-misaligned PCM chunk".to_owned(),
            ));
        }
        if sample_count > MAX_NATIVE_SAMPLES.saturating_sub(self.native_samples) {
            return Err(TtsError::SynthesisFailed(
                "Flite PCM exceeds the helper limit".to_owned(),
            ));
        }
        if sample_count != 0 && chunk.samples.is_null() {
            return Err(TtsError::SynthesisFailed(
                "Flite returned null streaming PCM".to_owned(),
            ));
        }
        self.native_samples += sample_count;
        if sample_count != 0 {
            let native = unsafe { std::slice::from_raw_parts(chunk.samples, sample_count) };
            let converted = native
                .iter()
                .map(|sample| (f32::from(*sample) * self.volume).round() as i16)
                .collect::<Vec<_>>();
            let windows = self
                .canonicalizer
                .as_mut()
                .expect("Flite canonicalizer was initialized")
                .push_interleaved_i16(&converted)
                .map_err(|error| {
                    TtsError::SynthesisFailed(format!(
                        "could not canonicalize progressive Flite PCM: {error}"
                    ))
                })?;
            self.emit(windows)?;
        }
        self.saw_last |= chunk.last != 0;
        Ok(i32::from(!self.cancelled()))
    }

    fn finish(&mut self) -> Result<u64, TtsError> {
        let canonicalizer = self
            .canonicalizer
            .as_mut()
            .ok_or_else(|| TtsError::SynthesisFailed("Flite returned no PCM format".to_owned()))?;
        let windows = canonicalizer.finish().map_err(|error| {
            TtsError::SynthesisFailed(format!(
                "could not finish progressive Flite PCM conversion: {error}"
            ))
        })?;
        self.emit(windows)?;
        Ok(self
            .canonicalizer
            .as_ref()
            .expect("Flite canonicalizer remains available")
            .output_frames())
    }

    fn emit(&mut self, windows: Vec<AudioBuffer>) -> Result<(), TtsError> {
        for window in windows {
            if window.is_empty() {
                continue;
            }
            self.sink.audio(window)?;
            if self.cancelled() {
                return Err(TtsError::SynthesisFailed(
                    "Flite synthesis was cancelled".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

unsafe extern "C" fn consume_flite_stream(
    samples: *const i16,
    sample_count: c_int,
    sample_rate: c_int,
    channel_count: c_int,
    last: c_int,
    markers: *const FliteWordMarker,
    marker_count: c_int,
    user_data: *mut c_void,
) -> c_int {
    if user_data.is_null() {
        return 0;
    }
    let capture = unsafe { &mut *user_data.cast::<FliteStreamCapture<'_>>() };
    match catch_unwind(AssertUnwindSafe(|| {
        capture.consume(NativeStreamChunk {
            samples,
            sample_count,
            sample_rate,
            channel_count,
            last,
            markers,
            marker_count,
        })
    })) {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            capture.failure = Some(error);
            0
        }
        Err(_) => {
            capture.failure = Some(TtsError::SynthesisFailed(
                "Flite streaming callback panicked".to_owned(),
            ));
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
            requested_anchors: AnchorSupport::WordBoundary,
            ..MarkerCapabilities::default()
        },
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
    // Measured reference and saturation policy: docs/RATE-CALIBRATION.md.
    const CALIBRATION: &[(f32, f32)] = &[
        (0.0, 2.000_000),
        (0.1, 1.666_442),
        (0.2, 1.314_194),
        (0.3, 1.079_852),
        (0.4, 0.850_347),
        (0.5, 0.690_226),
        (0.6, 0.550_129),
        (0.7, 0.445_190),
        (0.8, 0.334_819),
        (0.9, 0.254_232),
        (1.0, 0.107_539),
        (1.2, 0.100_000),
    ];
    interpolate(rate, CALIBRATION)
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

fn native_word_markers(
    synthesis: *mut FliteSynthesis,
    native_text: &CString,
    request_text: &str,
    sample_rate: u32,
    native_frame_count: u64,
    canonical_frame_count: u64,
) -> Result<Vec<SynthesisMarker>, TtsError> {
    let mut native_markers = vec![FliteWordMarker::default(); MAX_HELPER_MARKERS];
    let count = unsafe {
        omnivox_flite_sys::omnivox_flite_synthesis_word_markers(
            synthesis,
            native_text.as_ptr(),
            native_markers.as_mut_ptr(),
            MAX_HELPER_MARKERS as i32,
        )
    };
    let count = usize::try_from(count).map_err(|_| {
        TtsError::SynthesisFailed(format!(
            "Flite returned more than {MAX_HELPER_MARKERS} word markers"
        ))
    })?;
    native_markers.truncate(count);

    let mut markers = Vec::with_capacity(count);
    for marker in native_markers {
        let frame_offset = u64::try_from(marker.frame_offset).map_err(|_| {
            TtsError::SynthesisFailed("Flite returned a negative word frame".to_owned())
        })?;
        if frame_offset > native_frame_count {
            return Err(TtsError::SynthesisFailed(
                "Flite returned a word frame outside its PCM".to_owned(),
            ));
        }
        let text_start = u32::try_from(marker.text_start).map_err(|_| {
            TtsError::SynthesisFailed("Flite returned a negative word offset".to_owned())
        })?;
        let text_length = u32::try_from(marker.text_length).map_err(|_| {
            TtsError::SynthesisFailed("Flite returned a negative word length".to_owned())
        })?;
        let start = text_start as usize;
        let end = start
            .checked_add(text_length as usize)
            .filter(|end| *end <= request_text.len())
            .ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "Flite returned a word range outside its source text".to_owned(),
                )
            })?;
        if !request_text.is_char_boundary(start) || !request_text.is_char_boundary(end) {
            return Err(TtsError::SynthesisFailed(
                "Flite returned a word range outside UTF-8 boundaries".to_owned(),
            ));
        }
        markers.push(SynthesisMarker {
            kind: SynthesisMarkerKind::Word,
            frame_offset: scale_frame(frame_offset, sample_rate, canonical_frame_count),
            text_start: Some(text_start),
            text_length: Some(text_length),
            value: None,
        });
    }
    markers.sort_by_key(|marker| marker.frame_offset);
    Ok(markers)
}

fn progressive_word_markers(
    native_markers: &[FliteWordMarker],
    request_text: &str,
    canonicalizer: &ProgressivePcmCanonicalizer,
) -> Result<Vec<SynthesisMarker>, TtsError> {
    let mut markers = Vec::with_capacity(native_markers.len());
    for marker in native_markers {
        let native_frame = u64::try_from(marker.frame_offset).map_err(|_| {
            TtsError::SynthesisFailed("Flite returned a negative word frame".to_owned())
        })?;
        let text_start = u32::try_from(marker.text_start).map_err(|_| {
            TtsError::SynthesisFailed("Flite returned a negative word offset".to_owned())
        })?;
        let text_length = u32::try_from(marker.text_length).map_err(|_| {
            TtsError::SynthesisFailed("Flite returned a negative word length".to_owned())
        })?;
        let start = text_start as usize;
        let end = start
            .checked_add(text_length as usize)
            .filter(|end| *end <= request_text.len())
            .ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "Flite returned a word range outside its source text".to_owned(),
                )
            })?;
        if !request_text.is_char_boundary(start) || !request_text.is_char_boundary(end) {
            return Err(TtsError::SynthesisFailed(
                "Flite returned a word range outside UTF-8 boundaries".to_owned(),
            ));
        }
        let frame_offset = canonicalizer
            .canonical_frame_offset(native_frame)
            .map_err(|error| {
                TtsError::SynthesisFailed(format!(
                    "could not map a Flite word marker into canonical PCM: {error}"
                ))
            })?;
        markers.push(SynthesisMarker {
            kind: SynthesisMarkerKind::Word,
            frame_offset,
            text_start: Some(text_start),
            text_length: Some(text_length),
            value: None,
        });
    }
    markers.sort_by_key(|marker| marker.frame_offset);
    Ok(markers)
}

fn resolve_word_anchors(
    requested: &[RequestedAnchor],
    markers: &[SynthesisMarker],
) -> Vec<ResolvedAnchor> {
    requested
        .iter()
        .map(|requested| {
            let candidate = match requested.affinity {
                AnchorAffinity::Before => markers
                    .iter()
                    .filter(|marker| marker.kind == SynthesisMarkerKind::Word)
                    .filter_map(|marker| {
                        marker.text_start.map(|start| (start, marker.frame_offset))
                    })
                    .filter(|(start, _)| *start >= requested.text_offset)
                    .min_by_key(|(start, _)| *start),
                AnchorAffinity::After => markers
                    .iter()
                    .filter(|marker| marker.kind == SynthesisMarkerKind::Word)
                    .filter_map(|marker| {
                        marker.text_start.map(|start| (start, marker.frame_offset))
                    })
                    .filter(|(start, _)| *start <= requested.text_offset)
                    .max_by_key(|(start, _)| *start),
            };
            candidate.map_or_else(
                || ResolvedAnchor {
                    id: requested.id.clone(),
                    frame_offset: None,
                    resolution: AnchorResolution::Omitted,
                },
                |(_, frame_offset)| ResolvedAnchor {
                    id: requested.id.clone(),
                    frame_offset: Some(frame_offset),
                    resolution: AnchorResolution::WordBoundary,
                },
            )
        })
        .collect()
}

fn scale_frame(frame: u64, source_rate: u32, target_frame_count: u64) -> u64 {
    frame
        .saturating_mul(u64::from(STANDARD_SAMPLE_RATE))
        .saturating_add(u64::from(source_rate) / 2)
        .checked_div(u64::from(source_rate))
        .unwrap_or_default()
        .min(target_frame_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::{AnchorAffinity, AnchorResolution, RequestedAnchor, TtsSettings};

    #[derive(Default)]
    struct RecordingStreamSink {
        events: Vec<&'static str>,
        audio_windows: usize,
        frames: u64,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<ResolvedAnchor>,
    }

    impl SynthesisStreamSink for RecordingStreamSink {
        fn start(&mut self, _start: SynthesisStreamStart) -> Result<(), TtsError> {
            self.events.push("start");
            Ok(())
        }

        fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
            assert!(!audio.is_empty());
            self.events.push("audio");
            self.audio_windows += 1;
            self.frames += audio.frame_count() as u64;
            Ok(())
        }

        fn markers(
            &mut self,
            markers: Vec<SynthesisMarker>,
            anchors: Vec<ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            assert!(!markers.is_empty() || !anchors.is_empty());
            assert!(markers
                .iter()
                .all(|marker| marker.frame_offset >= self.frames));
            assert!(anchors
                .iter()
                .all(|anchor| anchor.frame_offset.is_none_or(|frame| frame >= self.frames)));
            self.events.push("markers");
            self.markers.extend(markers);
            self.anchors.extend(anchors);
            Ok(())
        }
    }

    struct CancellingStreamSink {
        cancellation: SynthesisCancellationToken,
        audio_calls: usize,
    }

    #[derive(Default)]
    struct RejectingStreamSink {
        started: bool,
        audio_calls: usize,
    }

    impl SynthesisStreamSink for RejectingStreamSink {
        fn start(&mut self, _start: SynthesisStreamStart) -> Result<(), TtsError> {
            self.started = true;
            Ok(())
        }

        fn audio(&mut self, _audio: AudioBuffer) -> Result<(), TtsError> {
            self.audio_calls += 1;
            Err(TtsError::SynthesisFailed(
                "test stream sink rejected PCM".to_owned(),
            ))
        }

        fn markers(
            &mut self,
            _markers: Vec<SynthesisMarker>,
            _anchors: Vec<ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            Ok(())
        }
    }

    impl SynthesisStreamSink for CancellingStreamSink {
        fn start(&mut self, _start: SynthesisStreamStart) -> Result<(), TtsError> {
            Ok(())
        }

        fn audio(&mut self, _audio: AudioBuffer) -> Result<(), TtsError> {
            self.audio_calls += 1;
            self.cancellation.cancel();
            Ok(())
        }

        fn markers(
            &mut self,
            _markers: Vec<SynthesisMarker>,
            _anchors: Vec<ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            Ok(())
        }
    }

    #[test]
    fn rate_mapping_is_calibrated_and_bounds_extremes() {
        assert_eq!(map_rate_to_duration(0.0), 2.0);
        assert!((map_rate_to_duration(0.5) - 0.690_226).abs() < 0.000_001);
        assert!((map_rate_to_duration(1.0) - 0.107_539).abs() < 0.000_001);
        assert_eq!(map_rate_to_duration(2.0), 0.1);
        let mapped: Vec<_> = (0..=20)
            .map(|point| map_rate_to_duration(point as f32 / 10.0))
            .collect();
        assert!(mapped.windows(2).all(|pair| pair[0] >= pair[1]));
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
        assert!(descriptor.capabilities.markers.word);
        assert_eq!(
            descriptor.capabilities.markers.requested_anchors,
            AnchorSupport::WordBoundary
        );

        let text = "The compact SLT voice is ready.";
        let request = SynthesisRequest::new(
            text,
            TtsSettings {
                voice: BUILT_IN_VOICE_ID.to_owned(),
                ..TtsSettings::default()
            },
        );
        let mut result = engine.synthesize(&request).unwrap();
        assert_eq!(result.engine_id, ENGINE_ID);
        assert_eq!(
            result
                .actual_voice
                .as_ref()
                .map(|voice| voice.voice_id.as_str()),
            Some(BUILT_IN_VOICE_ID)
        );
        assert!(!result.audio.is_empty());
        let ranges = result
            .markers
            .iter()
            .map(|marker| {
                let start = marker.text_start.unwrap() as usize;
                let end = start + marker.text_length.unwrap() as usize;
                (&text[start..end], marker.frame_offset)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            ranges.iter().map(|(word, _)| *word).collect::<Vec<_>>(),
            ["The", "compact", "SLT", "voice", "is", "ready"]
        );
        assert!(ranges.windows(2).all(|pair| pair[0].1 <= pair[1].1));
        assert!(ranges
            .iter()
            .all(|(_, frame)| *frame <= result.audio.frame_count() as u64));

        let anchored_request = request
            .with_anchors(vec![RequestedAnchor::new(
                "compact",
                4,
                AnchorAffinity::Before,
            )])
            .unwrap();
        result.resolve_anchors(&anchored_request, AnchorSupport::WordBoundary);
        assert_eq!(result.anchors.len(), 1);
        assert_eq!(result.anchors[0].id, "compact");
        assert_eq!(result.anchors[0].resolution, AnchorResolution::WordBoundary);
        assert_eq!(result.anchors[0].frame_offset, Some(ranges[1].1));
    }

    #[test]
    fn bundled_slt_streams_word_markers_and_anchors_before_pcm() {
        let mut warnings = Vec::new();
        let engine = FliteTtsEngine::new(Vec::new(), &mut warnings).unwrap();
        let request = SynthesisRequest::new(
            "The compact SLT voice is ready.",
            TtsSettings {
                voice: BUILT_IN_VOICE_ID.to_owned(),
                ..TtsSettings::default()
            },
        )
        .with_anchors(vec![
            RequestedAnchor::new("compact", 4, AnchorAffinity::Before),
            RequestedAnchor::new("after-compact", 10, AnchorAffinity::After),
        ])
        .unwrap();
        let mut sink = RecordingStreamSink::default();

        let completion = engine.synthesize_stream(&request, &mut sink).unwrap();

        assert_eq!(completion.frame_count, sink.frames);
        assert!(completion.frame_count > 0);
        assert!(sink.audio_windows > 1);
        assert_eq!(sink.events.first(), Some(&"start"));
        assert_eq!(sink.events.get(1), Some(&"markers"));
        assert_eq!(sink.markers.len(), 6);
        assert_eq!(sink.anchors.len(), 2);
        assert!(sink
            .anchors
            .iter()
            .all(|anchor| anchor.resolution == AnchorResolution::WordBoundary));
        assert_eq!(
            sink.anchors[0].frame_offset,
            sink.markers[1].frame_offset.into()
        );
        assert_eq!(
            sink.anchors[1].frame_offset,
            sink.markers[1].frame_offset.into()
        );
        assert!(sink
            .markers
            .iter()
            .all(|marker| marker.frame_offset <= completion.frame_count));
        assert!(sink.anchors.iter().all(|anchor| anchor
            .frame_offset
            .is_some_and(|frame| frame <= completion.frame_count)));
    }

    #[test]
    fn progressive_flite_stops_after_request_cancellation() {
        let mut warnings = Vec::new();
        let engine = FliteTtsEngine::new(Vec::new(), &mut warnings).unwrap();
        let cancellation = SynthesisCancellationToken::new();
        let request = SynthesisRequest::new(
            "This deliberately longer sentence gives cancellation a callback window.",
            TtsSettings {
                voice: BUILT_IN_VOICE_ID.to_owned(),
                ..TtsSettings::default()
            },
        )
        .with_cancellation(cancellation.clone());
        let mut sink = CancellingStreamSink {
            cancellation,
            audio_calls: 0,
        };

        let error = engine.synthesize_stream(&request, &mut sink).unwrap_err();

        assert!(error.to_string().contains("cancelled"));
        assert_eq!(sink.audio_calls, 1);
        assert!(!engine.is_speaking());
    }

    #[test]
    fn progressive_flite_propagates_sink_backpressure() {
        let mut warnings = Vec::new();
        let engine = FliteTtsEngine::new(Vec::new(), &mut warnings).unwrap();
        let request = SynthesisRequest::new(
            "Flite must stop when the output queue rejects a window.",
            TtsSettings {
                voice: BUILT_IN_VOICE_ID.to_owned(),
                ..TtsSettings::default()
            },
        );
        let mut sink = RejectingStreamSink::default();

        let error = engine.synthesize_stream(&request, &mut sink).unwrap_err();

        assert!(error.to_string().contains("test stream sink rejected PCM"));
        assert!(sink.started);
        assert_eq!(sink.audio_calls, 1);
        assert!(!engine.is_speaking());
    }
}
