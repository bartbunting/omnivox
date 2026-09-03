//! espeak-ng TTS Engine
//!
//! Cross-platform TTS backend using espeak-ng. Always compiled in as the
//! guaranteed fallback engine that works on all platforms.

use crate::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AnchorSupport, AudioOutputMode,
    Availability, CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor,
    EngineHealth, MarkerCapabilities, PhysicalVoiceId, VoiceDescriptor,
};
use crate::helper_protocol::{MAX_HELPER_MARKERS, MAX_HELPER_SYNTHESIS_BYTES};
use crate::rate_calibration::interpolate;
#[cfg(test)]
use crate::TtsSettings;
use crate::{
    AnchorResolution, AudioBuffer, ResolvedAnchor, SynthesisCancellationToken, SynthesisMarker,
    SynthesisMarkerKind, SynthesisRequest, SynthesisResult, SynthesisStreamCompletion,
    SynthesisStreamSink, SynthesisStreamStart, TtsEngine, TtsError, VoiceInfo, VoiceQuality,
};
use omnivox_audio::ProgressivePcmCanonicalizer;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::raw::{c_int, c_short, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Mutex;
use tracing::{debug, info, warn};

/// Global espeak-ng initialization guard.
/// espeak-ng uses global state internally, so we need a mutex to serialize access.
static ESPEAK_LOCK: OnceCell<Mutex<EspeakState>> = OnceCell::new();

const VOICE_CACHE_SCHEMA_VERSION: u32 = 1;
const VOICE_CACHE_FILE_NAME: &str = "omnivox-espeak-voices-v1.json";
const VOICE_CACHE_IDENTITY_FILE_NAME: &str = "omnivox-espeak-data.sha256";
const MAX_VOICE_CACHE_BYTES: u64 = 1024 * 1024;
const MAX_CACHED_VOICES: usize = 4096;
const MAX_CACHED_IDENTIFIER_BYTES: usize = 4096;
const MAX_CACHED_NAME_BYTES: usize = 1024;
const MAX_CACHED_LANGUAGE_BYTES: usize = 128;
static VOICE_CACHE_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Internal state for espeak-ng synthesis
struct EspeakState {
    sample_rate: u32,
    initialized: bool,
    data_parent: Option<PathBuf>,
}

/// Defensive limit for the native callback's zero-terminated event array.
const MAX_CALLBACK_EVENTS: usize = 65_536;
const MAX_NATIVE_SAMPLES: usize = MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<i16>();
const STREAM_CHANNEL_CAPACITY: usize = 4;

/// Audio and synchronization events collected for the one serialized synthesis.
static SYNTH_CAPTURE: Mutex<Option<EspeakSynthesisCapture>> = Mutex::new(None);

#[derive(Default)]
struct EspeakSynthesisCapture {
    samples: Vec<i16>,
    markers: Vec<EspeakNativeMarker>,
    cancellation: Option<SynthesisCancellationToken>,
    stream_sender: Option<SyncSender<EspeakStreamEvent>>,
    native_samples: usize,
    marker_count: usize,
    expected_anchor_count: usize,
    reported_anchors: HashSet<usize>,
    failure: Option<String>,
}

enum EspeakStreamEvent {
    Started {
        actual_voice: Option<PhysicalVoiceId>,
        sample_rate: u32,
        degraded_acss: Vec<crate::contracts::AcssDimension>,
    },
    Markers(Vec<EspeakNativeMarker>),
    Audio(Vec<i16>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EspeakNativeMarkerKind {
    Word,
    Sentence,
    Anchor(usize),
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

struct EspeakInput {
    text: CString,
    flags: u32,
    /// For SSML input, map each generated Unicode character boundary back to
    /// the corresponding UTF-8 byte boundary in the caller's plain text.
    source_boundaries: Option<Vec<u32>>,
}

impl EspeakInput {
    fn for_request(request: &SynthesisRequest) -> Result<Self, TtsError> {
        if request.anchors.is_empty() {
            return Ok(Self {
                text: CString::new(request.text.as_str()).map_err(|_| {
                    TtsError::SynthesisFailed("Text contains null bytes".to_owned())
                })?,
                flags: espeak_rs_sys::espeakENDPAUSE,
                source_boundaries: None,
            });
        }

        let mut generated = String::new();
        let mut source_boundaries = vec![0_u32];
        append_generated_text(&mut generated, &mut source_boundaries, "<speak>", 0, 0);

        let mut anchors = request.anchors.iter().enumerate().collect::<Vec<_>>();
        anchors.sort_by_key(|(index, anchor)| (anchor.text_offset, *index));
        let mut next_anchor = 0;
        for (source_start, character) in request.text.char_indices() {
            while anchors
                .get(next_anchor)
                .is_some_and(|(_, anchor)| anchor.text_offset as usize == source_start)
            {
                let (index, anchor) = anchors[next_anchor];
                append_espeak_anchor(
                    &mut generated,
                    &mut source_boundaries,
                    index,
                    anchor.text_offset,
                );
                next_anchor += 1;
            }
            let source_end = source_start + character.len_utf8();
            let escaped = match character {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\'' => "&apos;",
                _ => {
                    generated.push(character);
                    source_boundaries.push(source_end as u32);
                    continue;
                }
            };
            append_generated_text(
                &mut generated,
                &mut source_boundaries,
                escaped,
                source_start as u32,
                source_end as u32,
            );
        }
        while let Some((index, anchor)) = anchors.get(next_anchor).copied() {
            append_espeak_anchor(
                &mut generated,
                &mut source_boundaries,
                index,
                anchor.text_offset,
            );
            next_anchor += 1;
        }
        let source_end = request.text.len() as u32;
        append_generated_text(
            &mut generated,
            &mut source_boundaries,
            "</speak>",
            source_end,
            source_end,
        );
        Ok(Self {
            text: CString::new(generated).map_err(|_| {
                TtsError::SynthesisFailed("Generated eSpeak SSML contains a null byte".to_owned())
            })?,
            flags: espeak_rs_sys::espeakENDPAUSE | espeak_rs_sys::espeakSSML,
            source_boundaries: Some(source_boundaries),
        })
    }

    fn source_range(
        &self,
        plain_text: &str,
        one_based_start: usize,
        character_length: usize,
    ) -> Option<(u32, u32)> {
        if let Some(boundaries) = &self.source_boundaries {
            let start_character = one_based_start.checked_sub(1)?;
            let end_character = start_character.checked_add(character_length)?;
            let start = *boundaries.get(start_character)?;
            let end = *boundaries.get(end_character)?;
            return Some((start, end.saturating_sub(start)));
        }
        utf8_range_for_characters(plain_text, one_based_start, character_length)
    }

    fn character_boundary_count(&self, plain_text: &str) -> usize {
        self.source_boundaries.as_ref().map_or_else(
            || plain_text.chars().count() + 1,
            |boundaries| boundaries.len(),
        )
    }
}

fn append_espeak_anchor(
    generated: &mut String,
    source_boundaries: &mut Vec<u32>,
    index: usize,
    source_offset: u32,
) {
    append_generated_text(
        generated,
        source_boundaries,
        &format!("<mark name=\"ovx{index}\"/>"),
        source_offset,
        source_offset,
    );
}

fn append_generated_text(
    generated: &mut String,
    source_boundaries: &mut Vec<u32>,
    text: &str,
    source_start: u32,
    source_end: u32,
) {
    let character_count = text.chars().count();
    for (index, character) in text.chars().enumerate() {
        generated.push(character);
        source_boundaries.push(if index + 1 == character_count {
            source_end
        } else {
            source_start
        });
    }
}

/// espeak-ng TTS engine
pub struct EspeakTtsEngine {
    descriptor: EngineDescriptor,
}

struct EspeakDataLocation {
    parent: PathBuf,
    native_path: CString,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EspeakVoiceCache {
    schema_version: u32,
    data_identity: String,
    engine_version: String,
    voices: Vec<CachedEspeakVoice>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedEspeakVoice {
    identifier: String,
    name: String,
    language: String,
    quality: VoiceQuality,
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
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut slot = SYNTH_CAPTURE
            .lock()
            .map_err(|error| format!("eSpeak capture lock is poisoned: {error}"))?;
        let capture = slot
            .as_mut()
            .ok_or_else(|| "eSpeak invoked its callback without an active capture".to_owned())?;
        capture.consume(wav, sample_count, events)
    }));
    match result {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            record_callback_failure(error);
            1
        }
        Err(_) => {
            record_callback_failure("eSpeak callback processing panicked".to_owned());
            1
        }
    }
}

impl EspeakSynthesisCapture {
    unsafe fn consume(
        &mut self,
        wav: *mut c_short,
        sample_count: c_int,
        events: *mut espeak_rs_sys::espeak_EVENT,
    ) -> Result<c_int, String> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(SynthesisCancellationToken::is_cancelled)
        {
            return Ok(1);
        }
        let markers = unsafe { native_markers_from_callback(events)? };
        if markers.len() > MAX_HELPER_MARKERS.saturating_sub(self.marker_count) {
            return Err("eSpeak returned too many synchronization markers".to_owned());
        }
        self.marker_count += markers.len();
        for marker in &markers {
            if let EspeakNativeMarkerKind::Anchor(index) = marker.kind {
                if index >= self.expected_anchor_count {
                    return Err("eSpeak returned an unknown requested anchor".to_owned());
                }
                if !self.reported_anchors.insert(index) {
                    return Err("eSpeak returned a requested anchor more than once".to_owned());
                }
            }
        }

        let sample_count = usize::try_from(sample_count)
            .map_err(|_| "eSpeak returned a negative PCM sample count".to_owned())?;
        if sample_count > 0 && wav.is_null() {
            return Err("eSpeak returned a null PCM buffer".to_owned());
        }
        if sample_count > MAX_NATIVE_SAMPLES.saturating_sub(self.native_samples) {
            return Err("eSpeak PCM exceeded the synthesis limit".to_owned());
        }
        self.native_samples += sample_count;

        if let Some(sender) = &self.stream_sender {
            // eSpeak events describe the accompanying PCM block. Publish them
            // first so no marker can arrive behind already-visible audio.
            if !markers.is_empty() {
                sender
                    .send(EspeakStreamEvent::Markers(markers))
                    .map_err(|_| "eSpeak streaming marker consumer disconnected".to_owned())?;
            }
            if sample_count > 0 {
                let samples = unsafe { std::slice::from_raw_parts(wav, sample_count) };
                let mut owned = Vec::new();
                owned
                    .try_reserve(sample_count)
                    .map_err(|_| "could not allocate the eSpeak PCM window".to_owned())?;
                owned.extend_from_slice(samples);
                sender
                    .send(EspeakStreamEvent::Audio(owned))
                    .map_err(|_| "eSpeak streaming audio consumer disconnected".to_owned())?;
            }
        } else {
            self.markers
                .try_reserve(markers.len())
                .map_err(|_| "could not allocate the eSpeak synchronization markers".to_owned())?;
            self.markers.extend(markers);
            if sample_count > 0 {
                self.samples
                    .try_reserve(sample_count)
                    .map_err(|_| "could not allocate the eSpeak PCM buffer".to_owned())?;
                let samples = unsafe { std::slice::from_raw_parts(wav, sample_count) };
                self.samples.extend_from_slice(samples);
            }
        }
        Ok(0)
    }
}

unsafe fn native_markers_from_callback(
    events: *mut espeak_rs_sys::espeak_EVENT,
) -> Result<Vec<EspeakNativeMarker>, String> {
    if events.is_null() {
        return Ok(Vec::new());
    }
    let mut markers = Vec::new();
    for index in 0..MAX_CALLBACK_EVENTS {
        let event = unsafe { &*events.add(index) };
        if event.type_ == espeak_rs_sys::espeak_EVENT_TYPE_espeakEVENT_LIST_TERMINATED {
            return Ok(markers);
        }
        let kind = match event.type_ {
            espeak_rs_sys::espeak_EVENT_TYPE_espeakEVENT_WORD => EspeakNativeMarkerKind::Word,
            espeak_rs_sys::espeak_EVENT_TYPE_espeakEVENT_SENTENCE => {
                EspeakNativeMarkerKind::Sentence
            }
            espeak_rs_sys::espeak_EVENT_TYPE_espeakEVENT_MARK => {
                let name = unsafe { event.id.name };
                if name.is_null() {
                    return Err("eSpeak returned a mark without a name".to_owned());
                }
                let name = unsafe { CStr::from_ptr(name) }
                    .to_str()
                    .map_err(|_| "eSpeak returned a non-UTF-8 mark name".to_owned())?;
                let Some(index) = name
                    .strip_prefix("ovx")
                    .and_then(|value| value.parse::<usize>().ok())
                else {
                    return Err("eSpeak returned an unknown mark name".to_owned());
                };
                EspeakNativeMarkerKind::Anchor(index)
            }
            _ => continue,
        };
        if event.text_position <= 0 || event.audio_position < 0 {
            continue;
        }
        markers.push(EspeakNativeMarker {
            kind,
            text_position: event.text_position as usize,
            text_length: event.length.max(0) as usize,
            audio_position_ms: event.audio_position as u64,
        });
    }
    Err(format!(
        "eSpeak callback event list exceeded {MAX_CALLBACK_EVENTS} entries"
    ))
}

fn record_callback_failure(error: String) {
    if let Ok(mut slot) = SYNTH_CAPTURE.lock() {
        if let Some(capture) = slot.as_mut() {
            capture.failure = Some(error);
        }
    }
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
            audio_output: AudioOutputMode::StreamingPcm,
            cancellation: CancellationSupport::SynthesisAndPlayback,
            concurrency: ConcurrencyModel::Serialized,
            markers: MarkerCapabilities {
                word: true,
                sentence: true,
                requested_anchors: AnchorSupport::Exact,
                ..MarkerCapabilities::default()
            },
            language_switching: true,
            text_repertoire: crate::contracts::TextRepertoire::Unicode,
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
    /// Checks (in order): ESPEAK_NG_DATA env var, next to executable, build-time path, system paths.
    /// Returns the parent directory (espeak-ng appends "espeak-ng-data" itself).
    fn data_location(parent: PathBuf) -> Option<EspeakDataLocation> {
        let native_path = CString::new(parent.to_string_lossy().as_bytes()).ok()?;
        Some(EspeakDataLocation {
            parent,
            native_path,
        })
    }

    fn find_data_path() -> Option<EspeakDataLocation> {
        // 1. Runtime environment variable
        if let Ok(dir) = std::env::var("ESPEAK_NG_DATA") {
            if !dir.is_empty()
                && std::path::Path::new(&dir)
                    .join("espeak-ng-data")
                    .join("phontab")
                    .exists()
            {
                debug!("Using espeak-ng data from ESPEAK_NG_DATA env: {}", dir);
                return Self::data_location(PathBuf::from(dir));
            }
        }

        // 2. Next to the executable. Release archives and supported local
        // builds stage the data here, so a relocated binary stays paired with
        // the data generated by its own dependency build.
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let data_check = exe_dir.join("espeak-ng-data").join("phontab");
                if data_check.exists() {
                    debug!("Using espeak-ng data next to executable");
                    return Self::data_location(exe_dir.to_path_buf());
                }
            }
        }

        // 3. Stable Cargo profile path embedded by build.rs. This supports
        // cargo-run binaries after the post-build staging step has completed.
        if !ESPEAK_DATA_DIR.is_empty()
            && std::path::Path::new(ESPEAK_DATA_DIR)
                .join("espeak-ng-data")
                .join("phontab")
                .exists()
        {
            debug!("Using espeak-ng data from build path: {}", ESPEAK_DATA_DIR);
            return Self::data_location(PathBuf::from(ESPEAK_DATA_DIR));
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
                return Self::data_location(PathBuf::from(candidate));
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
                        Some(location) => location.native_path.as_ptr(),
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
                    data_parent: data_path.map(|location| location.parent),
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
        let data_parent = guard.data_parent.clone();

        drop(guard);

        Ok(Self {
            descriptor: Self::discover_descriptor(data_parent.as_deref()),
        })
    }

    /// Map the host rate to eSpeak NG's 80-through-450 words-per-minute control.
    fn map_rate(rate: f32) -> c_int {
        // Measured reference and saturation policy: docs/RATE-CALIBRATION.md.
        const CALIBRATION: &[(f32, f32)] = &[
            (0.0, 80.000_000),
            (0.1, 100.906_27),
            (0.2, 128.206_04),
            (0.3, 156.756_2),
            (0.4, 198.985_5),
            (0.5, 249.314_9),
            (0.6, 318.873_4),
            (0.7, 389.706_24),
            (0.8, 450.000_000),
        ];
        interpolate(rate, CALIBRATION).round() as c_int
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

    fn reported_voice_id(requested_voice_id: &str, backend_identifier: &str) -> String {
        let reported = format!("espeak:{backend_identifier}");
        if reported.eq_ignore_ascii_case(requested_voice_id) {
            requested_voice_id.to_owned()
        } else {
            reported
        }
    }

    fn markers_from_native(
        text: &str,
        input: &EspeakInput,
        native_markers: &[EspeakNativeMarker],
        sample_rate: u32,
        frame_count: u64,
    ) -> Vec<SynthesisMarker> {
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
                        .unwrap_or(input.character_boundary_count(text))
                        .saturating_sub(native.text_position),
                    EspeakNativeMarkerKind::Anchor(_) => return None,
                };
                let (text_start, text_length) =
                    input.source_range(text, native.text_position, character_length)?;
                let frame_offset = native
                    .audio_position_ms
                    .saturating_mul(u64::from(sample_rate))
                    .saturating_add(500)
                    / 1_000;
                Some(SynthesisMarker {
                    kind: match native.kind {
                        EspeakNativeMarkerKind::Word => SynthesisMarkerKind::Word,
                        EspeakNativeMarkerKind::Sentence => SynthesisMarkerKind::Sentence,
                        EspeakNativeMarkerKind::Anchor(_) => unreachable!(),
                    },
                    frame_offset: frame_offset.min(frame_count),
                    text_start: Some(text_start),
                    text_length: Some(text_length),
                    value: None,
                })
            })
            .collect()
    }

    fn anchors_from_native(
        request: &SynthesisRequest,
        native_markers: &[EspeakNativeMarker],
        sample_rate: u32,
        frame_count: u64,
    ) -> Vec<ResolvedAnchor> {
        native_markers
            .iter()
            .filter_map(|native| {
                let EspeakNativeMarkerKind::Anchor(index) = native.kind else {
                    return None;
                };
                let requested = request.anchors.get(index)?;
                let frame_offset = native
                    .audio_position_ms
                    .saturating_mul(u64::from(sample_rate))
                    .saturating_add(500)
                    / 1_000;
                Some(ResolvedAnchor {
                    id: requested.id.clone(),
                    frame_offset: Some(frame_offset.min(frame_count)),
                    resolution: AnchorResolution::Exact,
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

    fn discover_descriptor(data_parent: Option<&Path>) -> EngineDescriptor {
        let (version, runtime_default) = Self::runtime_metadata();
        let voices = Self::available_voices_with_cache(data_parent, version.as_deref())
            .into_iter()
            .map(|voice| VoiceDescriptor::from_voice_info("espeak", voice))
            .collect::<Vec<_>>();
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

    fn available_voices_with_cache(
        data_parent: Option<&Path>,
        engine_version: Option<&str>,
    ) -> Vec<VoiceInfo> {
        if let (Some(data_parent), Some(version)) = (data_parent, engine_version) {
            match Self::load_cached_voices(data_parent, version) {
                Ok(Some(voices)) => return voices,
                Ok(None) => {}
                Err(error) => warn!(
                    cache = %data_parent.join(VOICE_CACHE_FILE_NAME).display(),
                    %error,
                    "Ignoring invalid eSpeak voice cache"
                ),
            }
        }

        let voices = Self::discover_available_voices();
        if !voices.is_empty() {
            if let (Some(data_parent), Some(version)) = (data_parent, engine_version) {
                if let Err(error) = Self::store_cached_voices(data_parent, version, &voices) {
                    warn!(
                        cache = %data_parent.join(VOICE_CACHE_FILE_NAME).display(),
                        %error,
                        "Could not store eSpeak voice cache"
                    );
                }
            }
        }
        voices
    }

    fn cache_location(data_parent: &Path) -> Option<(PathBuf, String)> {
        let identity = data_parent.file_name()?.to_str()?;
        if identity.len() != 64
            || !identity
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        let identity_file = File::open(data_parent.join(VOICE_CACHE_IDENTITY_FILE_NAME)).ok()?;
        let mut encoded_identity = Vec::with_capacity(65);
        identity_file
            .take(66)
            .read_to_end(&mut encoded_identity)
            .ok()?;
        if encoded_identity != format!("{identity}\n").as_bytes() {
            return None;
        }
        Some((data_parent.join(VOICE_CACHE_FILE_NAME), identity.to_owned()))
    }

    fn load_cached_voices(
        data_parent: &Path,
        engine_version: &str,
    ) -> Result<Option<Vec<VoiceInfo>>, String> {
        let Some((cache_path, data_identity)) = Self::cache_location(data_parent) else {
            return Ok(None);
        };
        let file = match File::open(&cache_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_VOICE_CACHE_BYTES {
            return Err(format!(
                "cache must be a regular file no larger than {MAX_VOICE_CACHE_BYTES} bytes"
            ));
        }
        let mut encoded = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_VOICE_CACHE_BYTES + 1)
            .read_to_end(&mut encoded)
            .map_err(|error| error.to_string())?;
        if encoded.len() as u64 > MAX_VOICE_CACHE_BYTES {
            return Err(format!(
                "cache exceeds the {MAX_VOICE_CACHE_BYTES}-byte limit"
            ));
        }
        let cache: EspeakVoiceCache =
            serde_json::from_slice(&encoded).map_err(|error| error.to_string())?;
        Self::validate_voice_cache(&cache, &data_identity, engine_version)?;
        let voices = cache
            .voices
            .into_iter()
            .map(|voice| VoiceInfo {
                identifier: voice.identifier,
                name: voice.name,
                language: voice.language,
                quality: voice.quality,
            })
            .collect::<Vec<_>>();
        info!(
            cache = %cache_path.display(),
            voice_count = voices.len(),
            "Loaded cached eSpeak voice inventory"
        );
        Ok(Some(voices))
    }

    fn validate_voice_cache(
        cache: &EspeakVoiceCache,
        data_identity: &str,
        engine_version: &str,
    ) -> Result<(), String> {
        if cache.schema_version != VOICE_CACHE_SCHEMA_VERSION {
            return Err(format!("unsupported cache schema {}", cache.schema_version));
        }
        if cache.data_identity != data_identity {
            return Err("cache data identity does not match its directory".to_owned());
        }
        if cache.engine_version != engine_version {
            return Err("cache eSpeak version does not match the loaded runtime".to_owned());
        }
        if cache.voices.is_empty() || cache.voices.len() > MAX_CACHED_VOICES {
            return Err(format!(
                "cache must contain between 1 and {MAX_CACHED_VOICES} voices"
            ));
        }
        for voice in &cache.voices {
            if !voice.identifier.starts_with("espeak:")
                || voice.identifier.len() <= "espeak:".len()
                || voice.identifier.len() > MAX_CACHED_IDENTIFIER_BYTES
                || voice.identifier.contains('\0')
                || voice.name.is_empty()
                || voice.name.len() > MAX_CACHED_NAME_BYTES
                || voice.name.contains('\0')
                || voice.language.len() > MAX_CACHED_LANGUAGE_BYTES
                || voice.language.contains('\0')
            {
                return Err("cache contains an invalid voice record".to_owned());
            }
        }
        Ok(())
    }

    fn store_cached_voices(
        data_parent: &Path,
        engine_version: &str,
        voices: &[VoiceInfo],
    ) -> Result<(), String> {
        let Some((cache_path, data_identity)) = Self::cache_location(data_parent) else {
            return Ok(());
        };
        let cache = EspeakVoiceCache {
            schema_version: VOICE_CACHE_SCHEMA_VERSION,
            data_identity,
            engine_version: engine_version.to_owned(),
            voices: voices
                .iter()
                .map(|voice| CachedEspeakVoice {
                    identifier: voice.identifier.clone(),
                    name: voice.name.clone(),
                    language: voice.language.clone(),
                    quality: voice.quality,
                })
                .collect(),
        };
        Self::validate_voice_cache(&cache, cache.data_identity.as_str(), engine_version)?;
        let encoded = serde_json::to_vec(&cache).map_err(|error| error.to_string())?;
        if encoded.len() as u64 > MAX_VOICE_CACHE_BYTES {
            return Err(format!(
                "encoded cache exceeds the {MAX_VOICE_CACHE_BYTES}-byte limit"
            ));
        }

        let sequence = VOICE_CACHE_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_path = data_parent.join(format!(
            ".{VOICE_CACHE_FILE_NAME}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let write_result = (|| -> Result<(), String> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)
                .map_err(|error| error.to_string())?;
            file.write_all(&encoded)
                .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            drop(file);
            if cache_path.exists() {
                fs::remove_file(&cache_path).map_err(|error| error.to_string())?;
            }
            fs::rename(&temporary_path, &cache_path).map_err(|error| error.to_string())?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result?;
        info!(
            cache = %cache_path.display(),
            voice_count = voices.len(),
            "Stored eSpeak voice inventory cache"
        );
        Ok(())
    }

    fn discover_available_voices() -> Vec<VoiceInfo> {
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
                    // The languages field is a string with priority byte prefix.
                    let lang_ptr = voice.languages.offset(1);
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

    fn produce_stream(
        &self,
        request: &SynthesisRequest,
        input: &EspeakInput,
        sender: SyncSender<EspeakStreamEvent>,
    ) -> Result<(), TtsError> {
        let voice_id = request.voice_id_for_engine("espeak")?.to_owned();
        let state = ESPEAK_LOCK.get().ok_or(TtsError::NotAvailable)?;
        let state_guard = state.lock().map_err(|error| {
            TtsError::SynthesisFailed(format!("espeak-ng lock poisoned: {error}"))
        })?;
        let sample_rate = state_guard.sample_rate;

        unsafe {
            let voice_name = Self::backend_voice_name(&voice_id);
            let voice_cstr = CString::new(voice_name)
                .map_err(|_| TtsError::InvalidParameter("Invalid voice name".to_owned()))?;
            let voice_result = espeak_rs_sys::espeak_SetVoiceByName(voice_cstr.as_ptr());
            if voice_result != espeak_rs_sys::espeak_ERROR_EE_OK {
                return Err(TtsError::VoiceNotFound(voice_id));
            }
            let current_voice = espeak_rs_sys::espeak_GetCurrentVoice();
            let actual_voice = if current_voice.is_null() {
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
                        Self::reported_voice_id(
                            &voice_id,
                            &CStr::from_ptr(identifier).to_string_lossy(),
                        ),
                    )
                })
            };
            espeak_rs_sys::espeak_SetParameter(
                espeak_rs_sys::espeak_PARAMETER_espeakRATE,
                Self::map_rate(request.settings.rate),
                0,
            );
            espeak_rs_sys::espeak_SetParameter(
                espeak_rs_sys::espeak_PARAMETER_espeakPITCH,
                Self::map_pitch(request.settings.pitch),
                0,
            );
            espeak_rs_sys::espeak_SetParameter(
                espeak_rs_sys::espeak_PARAMETER_espeakVOLUME,
                Self::map_volume(request.settings.volume),
                0,
            );

            sender
                .send(EspeakStreamEvent::Started {
                    actual_voice,
                    sample_rate,
                    degraded_acss: request
                        .normalized_acss
                        .clone()
                        .degrade_for(&Self::capabilities().acss)
                        .omitted,
                })
                .map_err(|_| {
                    TtsError::SynthesisFailed(
                        "eSpeak streaming consumer disconnected before synthesis".to_owned(),
                    )
                })?;

            let text_len = input.text.as_bytes_with_nul().len();
            {
                let mut capture = SYNTH_CAPTURE.lock().map_err(|error| {
                    TtsError::SynthesisFailed(format!("Capture lock poisoned: {error}"))
                })?;
                *capture = Some(EspeakSynthesisCapture {
                    cancellation: request.cancellation.clone(),
                    stream_sender: Some(sender),
                    expected_anchor_count: request.anchors.len(),
                    ..EspeakSynthesisCapture::default()
                });
            }
            let result = espeak_rs_sys::espeak_Synth(
                input.text.as_ptr() as *const c_void,
                text_len,
                0,
                0,
                0,
                input.flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if result != espeak_rs_sys::espeak_ERROR_EE_OK {
                clear_synthesis_capture();
                return Err(TtsError::SynthesisFailed(format!(
                    "espeak_Synth failed with error: {result}"
                )));
            }
            espeak_rs_sys::espeak_Synchronize();
        }

        let capture = take_synthesis_capture()?;
        if capture
            .cancellation
            .as_ref()
            .is_some_and(SynthesisCancellationToken::is_cancelled)
        {
            return Err(TtsError::SynthesisFailed(
                "espeak synthesis was cancelled".to_owned(),
            ));
        }
        if let Some(failure) = capture.failure {
            return Err(TtsError::SynthesisFailed(failure));
        }
        if capture.reported_anchors.len() != capture.expected_anchor_count {
            return Err(TtsError::SynthesisFailed(format!(
                "eSpeak reported {} of {} requested anchors",
                capture.reported_anchors.len(),
                capture.expected_anchor_count
            )));
        }
        Ok(())
    }
}

impl TtsEngine for EspeakTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let text = request.text.as_str();
        let settings = &request.settings;
        if request
            .cancellation
            .as_ref()
            .is_some_and(SynthesisCancellationToken::is_cancelled)
        {
            return Err(TtsError::SynthesisFailed(
                "espeak synthesis was cancelled".to_owned(),
            ));
        }
        if text.is_empty() {
            let mut result = SynthesisResult::audio("espeak", None, AudioBuffer::empty());
            result.anchors = request
                .anchors
                .iter()
                .map(|anchor| ResolvedAnchor {
                    id: anchor.id.clone(),
                    frame_offset: Some(0),
                    resolution: AnchorResolution::Exact,
                })
                .collect();
            return Ok(result);
        }
        let input = EspeakInput::for_request(request)?;

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
                        Self::reported_voice_id(
                            &voice_id,
                            &CStr::from_ptr(identifier).to_string_lossy(),
                        ),
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

            let text_len = input.text.as_bytes_with_nul().len();

            // Prepare the synthesis capture before eSpeak starts invoking the callback.
            {
                let mut capture = SYNTH_CAPTURE.lock().map_err(|e| {
                    TtsError::SynthesisFailed(format!("Capture lock poisoned: {}", e))
                })?;
                *capture = Some(EspeakSynthesisCapture {
                    cancellation: request.cancellation.clone(),
                    expected_anchor_count: request.anchors.len(),
                    ..EspeakSynthesisCapture::default()
                });
            }

            // Synthesize
            let result = espeak_rs_sys::espeak_Synth(
                input.text.as_ptr() as *const c_void,
                text_len,
                0, // start position
                0, // POS_CHARACTER
                0, // end position (0 = all)
                input.flags,
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

        if capture
            .cancellation
            .as_ref()
            .is_some_and(SynthesisCancellationToken::is_cancelled)
        {
            return Err(TtsError::SynthesisFailed(
                "espeak synthesis was cancelled".to_owned(),
            ));
        }
        if let Some(failure) = capture.failure {
            return Err(TtsError::SynthesisFailed(failure));
        }
        if capture.reported_anchors.len() != capture.expected_anchor_count {
            return Err(TtsError::SynthesisFailed(format!(
                "eSpeak reported {} of {} requested anchors",
                capture.reported_anchors.len(),
                capture.expected_anchor_count
            )));
        }

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
            &input,
            &capture.markers,
            sample_rate,
            capture.samples.len() as u64,
        );
        let anchors = Self::anchors_from_native(
            request,
            &capture.markers,
            sample_rate,
            capture.samples.len() as u64,
        );
        let mut result = SynthesisResult::from_native_i16(
            "espeak",
            actual_voice,
            &capture.samples,
            sample_rate,
            1,
            markers,
            anchors,
        )?;
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
        if request
            .cancellation
            .as_ref()
            .is_some_and(SynthesisCancellationToken::is_cancelled)
        {
            return Err(TtsError::SynthesisFailed(
                "espeak synthesis was cancelled".to_owned(),
            ));
        }
        if request.text.is_empty() {
            sink.start(SynthesisStreamStart {
                engine_id: "espeak".to_owned(),
                actual_voice: None,
                degraded_acss: request
                    .normalized_acss
                    .clone()
                    .degrade_for(&Self::capabilities().acss)
                    .omitted,
            })?;
            if !request.anchors.is_empty() {
                sink.markers(
                    Vec::new(),
                    request
                        .anchors
                        .iter()
                        .map(|anchor| ResolvedAnchor {
                            id: anchor.id.clone(),
                            frame_offset: Some(0),
                            resolution: AnchorResolution::Exact,
                        })
                        .collect(),
                )?;
            }
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }
        let input = EspeakInput::for_request(request)?;

        std::thread::scope(|scope| {
            let (sender, receiver) = sync_channel(STREAM_CHANNEL_CAPACITY);
            let producer = scope.spawn(|| self.produce_stream(request, &input, sender));
            let mut canonicalizer = None;
            let mut stream_error = None;
            let mut started = false;

            while let Ok(event) = receiver.recv() {
                let result = match event {
                    EspeakStreamEvent::Started {
                        actual_voice,
                        sample_rate,
                        degraded_acss,
                    } => {
                        if started {
                            Err(TtsError::SynthesisFailed(
                                "eSpeak started one synthesis twice".to_owned(),
                            ))
                        } else {
                            started = true;
                            match ProgressivePcmCanonicalizer::new(sample_rate, 1) {
                                Ok(converter) => {
                                    canonicalizer = Some(converter);
                                    sink.start(SynthesisStreamStart {
                                        engine_id: "espeak".to_owned(),
                                        actual_voice,
                                        degraded_acss,
                                    })
                                }
                                Err(error) => Err(TtsError::SynthesisFailed(format!(
                                    "could not initialize progressive eSpeak PCM conversion: {error}"
                                ))),
                            }
                        }
                    }
                    EspeakStreamEvent::Markers(native) => {
                        if canonicalizer.is_none() {
                            stream_error = Some(TtsError::SynthesisFailed(
                                "eSpeak emitted markers before stream metadata".to_owned(),
                            ));
                            break;
                        }
                        let (markers, anchors) =
                            progressive_timing_from_native(&request.text, &input, request, &native);
                        if markers.is_empty() && anchors.is_empty() {
                            Ok(())
                        } else {
                            sink.markers(markers, anchors)
                        }
                    }
                    EspeakStreamEvent::Audio(samples) => {
                        let Some(converter) = canonicalizer.as_mut() else {
                            stream_error = Some(TtsError::SynthesisFailed(
                                "eSpeak emitted audio before stream metadata".to_owned(),
                            ));
                            break;
                        };
                        converter
                            .push_interleaved_i16(&samples)
                            .map_err(|error| {
                                TtsError::SynthesisFailed(format!(
                                    "could not canonicalize progressive eSpeak PCM: {error}"
                                ))
                            })
                            .and_then(|windows| emit_espeak_audio_windows(sink, windows))
                    }
                };
                if let Err(error) = result {
                    stream_error = Some(error);
                    break;
                }
            }

            drop(receiver);
            if stream_error.is_some() {
                self.stop();
            }
            let producer_result = producer.join().map_err(|_| {
                TtsError::SynthesisFailed("eSpeak streaming worker panicked".to_owned())
            })?;
            if let Some(error) = stream_error {
                return Err(error);
            }
            producer_result?;
            if !started {
                return Err(TtsError::SynthesisFailed(
                    "eSpeak ended before stream metadata".to_owned(),
                ));
            }
            let converter = canonicalizer
                .as_mut()
                .expect("started eSpeak stream has a PCM converter");
            let windows = converter.finish().map_err(|error| {
                TtsError::SynthesisFailed(format!(
                    "could not finish progressive eSpeak PCM conversion: {error}"
                ))
            })?;
            emit_espeak_audio_windows(sink, windows)?;
            Ok(SynthesisStreamCompletion {
                frame_count: converter.output_frames(),
            })
        })
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
        let search = identifier.strip_prefix("espeak:").unwrap_or(identifier);
        self.available_voices().into_iter().find(|v| {
            v.identifier == identifier
                || v.identifier == format!("espeak:{}", search)
                || v.name == search
                || v.language == search
        })
    }
}

fn clear_synthesis_capture() {
    if let Ok(mut capture) = SYNTH_CAPTURE.lock() {
        capture.take();
    }
}

fn take_synthesis_capture() -> Result<EspeakSynthesisCapture, TtsError> {
    let mut capture = SYNTH_CAPTURE
        .lock()
        .map_err(|error| TtsError::SynthesisFailed(format!("Capture lock poisoned: {error}")))?;
    capture.take().ok_or_else(|| {
        TtsError::SynthesisFailed("eSpeak ended without an active synthesis capture".to_owned())
    })
}

fn progressive_timing_from_native(
    text: &str,
    input: &EspeakInput,
    request: &SynthesisRequest,
    native_markers: &[EspeakNativeMarker],
) -> (Vec<SynthesisMarker>, Vec<ResolvedAnchor>) {
    let mut markers = native_markers
        .iter()
        .filter_map(|native| {
            let character_length = match native.kind {
                EspeakNativeMarkerKind::Word => native.text_length,
                // eSpeak does not provide the sentence length until a future
                // sentence event. A zero-length range truthfully preserves the
                // known boundary without delaying the marker behind its PCM.
                EspeakNativeMarkerKind::Sentence => 0,
                EspeakNativeMarkerKind::Anchor(_) => return None,
            };
            let (text_start, text_length) =
                input.source_range(text, native.text_position, character_length)?;
            let frame_offset = native
                .audio_position_ms
                .saturating_mul(u64::from(crate::STANDARD_SAMPLE_RATE))
                .saturating_add(500)
                / 1_000;
            Some(SynthesisMarker {
                kind: match native.kind {
                    EspeakNativeMarkerKind::Word => SynthesisMarkerKind::Word,
                    EspeakNativeMarkerKind::Sentence => SynthesisMarkerKind::Sentence,
                    EspeakNativeMarkerKind::Anchor(_) => unreachable!(),
                },
                frame_offset,
                text_start: Some(text_start),
                text_length: Some(text_length),
                value: None,
            })
        })
        .collect::<Vec<_>>();
    markers.sort_by_key(|marker| marker.frame_offset);
    let mut anchors = native_markers
        .iter()
        .filter_map(|native| {
            let EspeakNativeMarkerKind::Anchor(index) = native.kind else {
                return None;
            };
            let requested = request.anchors.get(index)?;
            let frame_offset = native
                .audio_position_ms
                .saturating_mul(u64::from(crate::STANDARD_SAMPLE_RATE))
                .saturating_add(500)
                / 1_000;
            Some(ResolvedAnchor {
                id: requested.id.clone(),
                frame_offset: Some(frame_offset),
                resolution: AnchorResolution::Exact,
            })
        })
        .collect::<Vec<_>>();
    anchors.sort_by_key(|anchor| anchor.frame_offset);
    (markers, anchors)
}

fn emit_espeak_audio_windows(
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

/// Convert eSpeak's one-based Unicode character range to Omnivox's UTF-8 byte range.
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

    #[derive(Default)]
    struct RecordingStreamSink {
        started: bool,
        audio_windows: usize,
        frames: u64,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<crate::ResolvedAnchor>,
    }

    impl SynthesisStreamSink for RecordingStreamSink {
        fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
            assert!(!self.started);
            assert_eq!(start.engine_id, "espeak");
            self.started = true;
            Ok(())
        }

        fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
            assert!(self.started);
            assert!(!audio.is_empty());
            self.audio_windows += 1;
            self.frames += audio.frame_count() as u64;
            Ok(())
        }

        fn markers(
            &mut self,
            markers: Vec<SynthesisMarker>,
            anchors: Vec<crate::ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            assert!(self.started);
            assert!(!markers.is_empty() || !anchors.is_empty());
            assert!(markers
                .iter()
                .all(|marker| marker.frame_offset >= self.frames));
            assert!(anchors.iter().all(|anchor| anchor
                .frame_offset
                .is_some_and(|frame| frame >= self.frames)));
            self.markers.extend(markers);
            self.anchors.extend(anchors);
            Ok(())
        }
    }

    struct CacheTestDirectory(PathBuf);

    impl CacheTestDirectory {
        fn new(label: &str) -> Self {
            let sequence = VOICE_CACHE_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "omnivox-espeak-cache-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn data_parent(&self) -> PathBuf {
            let path = self.0.join("a".repeat(64));
            fs::create_dir(&path).unwrap();
            fs::write(
                path.join(VOICE_CACHE_IDENTITY_FILE_NAME),
                format!("{}\n", "a".repeat(64)),
            )
            .unwrap();
            path
        }
    }

    impl Drop for CacheTestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn cached_voice() -> VoiceInfo {
        VoiceInfo {
            identifier: r"espeak:gmw\en-US".to_owned(),
            name: "English (America)".to_owned(),
            language: "en-us".to_owned(),
            quality: VoiceQuality::Compact,
        }
    }

    #[test]
    fn voice_cache_requires_a_lowercase_sha256_data_directory() {
        let directory = CacheTestDirectory::new("identity");
        let data_parent = directory.data_parent();
        assert!(EspeakTtsEngine::cache_location(&data_parent).is_some());

        fs::remove_file(data_parent.join(VOICE_CACHE_IDENTITY_FILE_NAME)).unwrap();
        assert!(EspeakTtsEngine::cache_location(&data_parent).is_none());

        let invalid_directory = CacheTestDirectory::new("invalid-identity");
        let invalid = invalid_directory.0.join("A".repeat(64));
        fs::create_dir(&invalid).unwrap();
        fs::write(
            invalid.join(VOICE_CACHE_IDENTITY_FILE_NAME),
            format!("{}\n", "A".repeat(64)),
        )
        .unwrap();
        assert!(EspeakTtsEngine::cache_location(&invalid).is_none());
    }

    #[test]
    fn cached_voice_inventory_round_trips_without_native_discovery() {
        let directory = CacheTestDirectory::new("round-trip");
        let data_parent = directory.data_parent();
        let expected = vec![cached_voice()];
        EspeakTtsEngine::store_cached_voices(&data_parent, "1.52.0", &expected).unwrap();

        let loaded = EspeakTtsEngine::available_voices_with_cache(
            Some(data_parent.as_path()),
            Some("1.52.0"),
        );

        assert_eq!(loaded, expected);
    }

    #[test]
    fn voice_cache_rejects_version_mismatch_and_invalid_records() {
        let directory = CacheTestDirectory::new("validation");
        let data_parent = directory.data_parent();
        EspeakTtsEngine::store_cached_voices(&data_parent, "1.52.0", &[cached_voice()]).unwrap();

        let version_error =
            EspeakTtsEngine::load_cached_voices(&data_parent, "1.53.0").unwrap_err();
        assert!(version_error.contains("version"));

        let cache_path = data_parent.join(VOICE_CACHE_FILE_NAME);
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&cache_path)
            .unwrap()
            .set_len(MAX_VOICE_CACHE_BYTES + 1)
            .unwrap();
        let size_error = EspeakTtsEngine::load_cached_voices(&data_parent, "1.52.0").unwrap_err();
        assert!(size_error.contains("no larger"));

        let invalid = EspeakVoiceCache {
            schema_version: VOICE_CACHE_SCHEMA_VERSION,
            data_identity: "a".repeat(64),
            engine_version: "1.52.0".to_owned(),
            voices: vec![CachedEspeakVoice {
                identifier: "other:voice".to_owned(),
                name: "Other".to_owned(),
                language: "en".to_owned(),
                quality: VoiceQuality::Compact,
            }],
        };
        let validation_error =
            EspeakTtsEngine::validate_voice_cache(&invalid, &"a".repeat(64), "1.52.0").unwrap_err();
        assert!(validation_error.contains("invalid voice"));
    }

    #[test]
    fn test_rate_mapping() {
        assert_eq!(EspeakTtsEngine::map_rate(0.0), 80);
        assert_eq!(EspeakTtsEngine::map_rate(0.5), 249);
        assert_eq!(EspeakTtsEngine::map_rate(0.8), 450);
        assert_eq!(EspeakTtsEngine::map_rate(-1.0), 80);
        assert_eq!(EspeakTtsEngine::map_rate(2.0), 450);

        let mapped: Vec<_> = (0..=20)
            .map(|point| EspeakTtsEngine::map_rate(point as f32 / 10.0))
            .collect();
        assert!(mapped.windows(2).all(|pair| pair[0] <= pair[1]));
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
        assert_eq!(
            descriptor.capabilities.audio_output,
            AudioOutputMode::StreamingPcm
        );
        assert!(descriptor.capabilities.markers.word);
        assert!(descriptor.capabilities.markers.sentence);
        assert_eq!(
            descriptor.capabilities.markers.requested_anchors,
            AnchorSupport::Exact
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
    fn reported_voice_preserves_inventory_case_after_backend_normalization() {
        assert_eq!(
            EspeakTtsEngine::reported_voice_id(r"espeak:gmw\en-US", r"gmw\en-us"),
            r"espeak:gmw\en-US"
        );
        assert_eq!(
            EspeakTtsEngine::reported_voice_id("en", r"gmw\en-us"),
            r"espeak:gmw\en-us"
        );
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
    fn espeak_streams_ordered_markers_and_audio_windows() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let request = SynthesisRequest::new(
            "First sentence has words. A second sentence follows.",
            TtsSettings::default(),
        );
        let mut sink = RecordingStreamSink::default();

        let completion = engine
            .synthesize_stream(&request, &mut sink)
            .expect("Streaming synthesis failed");

        assert!(sink.started);
        assert!(sink.audio_windows > 1);
        assert!(sink.anchors.is_empty());
        assert_eq!(completion.frame_count, sink.frames);
        assert!(sink
            .markers
            .iter()
            .any(|marker| marker.kind == SynthesisMarkerKind::Word));
        assert!(sink
            .markers
            .iter()
            .any(|marker| marker.kind == SynthesisMarkerKind::Sentence));
    }

    #[test]
    fn anchored_espeak_streams_native_marks_before_their_audio() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let request = SynthesisRequest::new("hello world", TtsSettings::default())
            .with_anchors(vec![
                crate::RequestedAnchor::new("start", 0, crate::AnchorAffinity::Before),
                crate::RequestedAnchor::new("middle", 6, crate::AnchorAffinity::After),
                crate::RequestedAnchor::new("end", 11, crate::AnchorAffinity::After),
            ])
            .unwrap();
        let mut sink = RecordingStreamSink::default();

        let completion = engine
            .synthesize_stream(&request, &mut sink)
            .expect("Anchored synthesis failed");

        assert_eq!(completion.frame_count, sink.frames);
        assert!(sink.audio_windows > 1);
        assert_eq!(sink.anchors.len(), 3);
        assert_eq!(sink.anchors[0].id, "start");
        assert_eq!(sink.anchors[1].id, "middle");
        assert_eq!(sink.anchors[2].id, "end");
        assert!(sink
            .anchors
            .iter()
            .all(|anchor| anchor.resolution == AnchorResolution::Exact));
    }

    #[test]
    fn native_marks_preserve_espeak_audio_and_plain_text_ranges() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let text = "héllo world";
        let plain = engine
            .synthesize(&SynthesisRequest::new(text, TtsSettings::default()))
            .expect("Plain synthesis failed");
        let request = SynthesisRequest::new(text, TtsSettings::default())
            .with_anchors(vec![crate::RequestedAnchor::new(
                "world",
                7,
                crate::AnchorAffinity::After,
            )])
            .unwrap();
        let marked = engine
            .synthesize(&request)
            .expect("Marked synthesis failed");

        marked.validate(&request).unwrap();
        let sample_count_delta = marked
            .audio
            .samples
            .len()
            .abs_diff(plain.audio.samples.len());
        assert!(sample_count_delta * 100 < plain.audio.samples.len() * 2);
        assert_eq!(marked.anchors.len(), 1);
        assert!(marked
            .anchors
            .iter()
            .all(|anchor| anchor.resolution == AnchorResolution::Exact));
        let word_ranges = marked
            .markers
            .iter()
            .filter(|marker| marker.kind == SynthesisMarkerKind::Word)
            .map(|marker| (marker.text_start, marker.text_length))
            .collect::<Vec<_>>();
        assert!(word_ranges.contains(&(Some(0), Some(6))));
        assert!(word_ranges.contains(&(Some(7), Some(5))));
    }

    #[test]
    fn espeak_rejects_an_already_cancelled_request() {
        let engine = EspeakTtsEngine::new().expect("Failed to init espeak-ng");
        let cancellation = SynthesisCancellationToken::new();
        cancellation.cancel();
        let request = SynthesisRequest::new("obsolete", TtsSettings::default())
            .with_cancellation(cancellation);

        assert!(matches!(
            engine.synthesize(&request),
            Err(TtsError::SynthesisFailed(message)) if message.contains("cancelled")
        ));
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
            let result = engine
                .synthesize(&request)
                .expect("stress synthesis failed");

            result
                .validate(&request)
                .expect("stress result was invalid");
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

        let request = SynthesisRequest::new(text, TtsSettings::default());
        let input = EspeakInput::for_request(&request).unwrap();
        let markers = EspeakTtsEngine::markers_from_native(text, &input, &native, 22_050, 44_100);

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
