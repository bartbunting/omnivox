//! Engine-neutral helper-protocol host for isolated Omnivox TTS adapters.

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use omnivox_tts::contracts::{
    AudioOutputMode, CancellationSupport, ConcurrencyModel, EngineDescriptor, NormalizedAcss,
    PhysicalVoiceId,
};
use omnivox_tts::helper_protocol::{
    read_frame, write_frame, HelperAudioFormat, HelperErrorCode, HelperMarker, HelperMarkerKind,
    HelperPcmChunk, HelperProtocolError, HelperRequest, HelperRequestBody, HelperResponse,
    HelperResponseBody, HelperSampleFormat, HelperSynthesisSettings, HELPER_PROTOCOL_VERSION,
    MAX_HELPER_AUDIO_CHUNK_BYTES, MAX_HELPER_MARKERS, MAX_HELPER_SYNTHESIS_BYTES,
    SUPPORTED_HELPER_PROTOCOL_VERSIONS,
};
use omnivox_tts::{
    SynthesisMarkerKind, SynthesisRequest, SynthesisResult, TtsEngine, TtsError, TtsSettings,
    STANDARD_CHANNELS, STANDARD_SAMPLE_RATE,
};
use thiserror::Error;

const MAX_HELPER_MESSAGE_BYTES: usize = 16 * 1024;

#[derive(Debug, Error)]
pub enum HelperServerError {
    #[error(transparent)]
    Protocol(#[from] HelperProtocolError),

    #[error("could not start helper synthesis thread: {0}")]
    Thread(#[from] io::Error),

    #[error("invalid helper engine descriptor: {0}")]
    Descriptor(String),
}

/// Serve the helper protocol over standard input and output.
pub fn run_stdio(
    engine: Arc<dyn TtsEngine>,
    helper_name: impl Into<String>,
    helper_version: impl Into<String>,
) -> Result<(), HelperServerError> {
    run_helper(
        BufReader::new(io::stdin()),
        BufWriter::new(io::stdout()),
        engine,
        helper_name,
        helper_version,
    )
}

/// Serve one helper session over caller-owned streams.
pub fn run_helper<R, W>(
    mut reader: R,
    writer: W,
    engine: Arc<dyn TtsEngine>,
    helper_name: impl Into<String>,
    helper_version: impl Into<String>,
) -> Result<(), HelperServerError>
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let runtime = Arc::new(HelperRuntime::new(
        engine,
        writer,
        helper_name.into(),
        helper_version.into(),
    )?);
    loop {
        let request = match read_frame(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) => {
                runtime.send_error(
                    None,
                    HELPER_PROTOCOL_VERSION,
                    protocol_error_code(&error),
                    &error.to_string(),
                    false,
                )?;
                return Err(error.into());
            }
        };
        if runtime.handle(request)? == HandleOutcome::Shutdown {
            break;
        }
    }
    runtime.cancel_active();
    Ok(())
}

#[derive(Debug, Clone)]
struct ActiveSynthesis {
    request_id: u64,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct RuntimeState {
    protocol_version: Option<u16>,
    active: Option<ActiveSynthesis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandleOutcome {
    Continue,
    Shutdown,
}

struct HelperRuntime<W> {
    engine: Arc<dyn TtsEngine>,
    descriptor: EngineDescriptor,
    helper_name: String,
    helper_version: String,
    writer: Arc<Mutex<W>>,
    state: Mutex<RuntimeState>,
}

impl<W> HelperRuntime<W>
where
    W: Write + Send + 'static,
{
    fn new(
        engine: Arc<dyn TtsEngine>,
        writer: W,
        helper_name: String,
        helper_version: String,
    ) -> Result<Self, HelperServerError> {
        let mut descriptor = engine.descriptor();
        if descriptor.id.is_empty() {
            return Err(HelperServerError::Descriptor(
                "engine must have a non-empty ID".to_owned(),
            ));
        }
        if descriptor.can_synthesize()
            && descriptor.capabilities.audio_output != AudioOutputMode::BufferedPcm
        {
            return Err(HelperServerError::Descriptor(
                "engine must return buffered PCM".to_owned(),
            ));
        }
        if descriptor.can_synthesize()
            && descriptor.default_voice_id.as_ref().is_none_or(|default| {
                !descriptor
                    .voices
                    .iter()
                    .any(|voice| voice.id.voice_id == *default)
            })
        {
            return Err(HelperServerError::Descriptor(
                "engine must advertise a valid default voice".to_owned(),
            ));
        }

        // The process boundary supplies cancellation: the host first requests
        // cancellation and then forcibly retires a helper that does not finish.
        descriptor.capabilities.cancellation = CancellationSupport::SynthesisAndPlayback;
        descriptor.capabilities.concurrency = ConcurrencyModel::Serialized;

        Ok(Self {
            engine,
            descriptor,
            helper_name,
            helper_version,
            writer: Arc::new(Mutex::new(writer)),
            state: Mutex::new(RuntimeState::default()),
        })
    }

    fn handle(
        self: &Arc<Self>,
        request: HelperRequest,
    ) -> Result<HandleOutcome, HelperServerError> {
        let request_id = request.request_id;
        let request_version = request.protocol_version;
        if let Err(error) = request.validate() {
            self.send_protocol_error(request_id, request_version, error)?;
            return Ok(HandleOutcome::Continue);
        }

        if let HelperRequestBody::Hello {
            supported_protocol_versions,
        } = &request.body
        {
            return self.handle_hello(
                request_id,
                request_version,
                supported_protocol_versions.clone(),
            );
        }

        let protocol_version = match self.state.lock().unwrap().protocol_version {
            Some(version) if version == request_version => version,
            Some(_) => {
                self.send_error(
                    Some(request_id),
                    request_version,
                    HelperErrorCode::UnsupportedVersion,
                    "request does not use the negotiated protocol version",
                    false,
                )?;
                return Ok(HandleOutcome::Continue);
            }
            None => {
                self.send_error(
                    Some(request_id),
                    request_version,
                    HelperErrorCode::InvalidRequest,
                    "hello must negotiate the helper protocol first",
                    false,
                )?;
                return Ok(HandleOutcome::Continue);
            }
        };

        let result = match request.body {
            HelperRequestBody::Describe => {
                if !self.descriptor.can_synthesize() {
                    self.send_error(
                        Some(request_id),
                        protocol_version,
                        HelperErrorCode::NotAvailable,
                        unavailable_reason(&self.descriptor),
                        true,
                    )?;
                    return Ok(HandleOutcome::Continue);
                }
                self.send(
                    Some(request_id),
                    protocol_version,
                    HelperResponseBody::Descriptor {
                        descriptor: self.descriptor.clone(),
                    },
                )?;
                Ok(HandleOutcome::Continue)
            }
            HelperRequestBody::Ping => {
                self.send(Some(request_id), protocol_version, HelperResponseBody::Pong)?;
                Ok(HandleOutcome::Continue)
            }
            HelperRequestBody::Shutdown => {
                self.cancel_active();
                self.send(
                    Some(request_id),
                    protocol_version,
                    HelperResponseBody::ShuttingDown,
                )?;
                Ok(HandleOutcome::Shutdown)
            }
            HelperRequestBody::Cancel { target_request_id } => {
                self.handle_cancel(request_id, target_request_id, protocol_version)
            }
            HelperRequestBody::Synthesize {
                text,
                settings,
                anchors,
            } => self.handle_synthesis(
                request_id,
                protocol_version,
                text,
                settings,
                anchors.unwrap_or_default(),
            ),
            HelperRequestBody::Hello { .. } => unreachable!("hello was handled above"),
        };

        match result {
            Ok(outcome) => Ok(outcome),
            Err(fault) => {
                self.send_error(
                    Some(request_id),
                    protocol_version,
                    fault.code,
                    &fault.message,
                    fault.retryable,
                )?;
                Ok(HandleOutcome::Continue)
            }
        }
    }

    fn handle_hello(
        &self,
        request_id: u64,
        request_version: u16,
        offered: Vec<u16>,
    ) -> Result<HandleOutcome, HelperServerError> {
        let selected = offered
            .iter()
            .copied()
            .filter(|version| SUPPORTED_HELPER_PROTOCOL_VERSIONS.contains(version))
            .max();
        let mut state = self.state.lock().unwrap();
        if state.protocol_version.is_some() {
            drop(state);
            self.send_error(
                Some(request_id),
                request_version,
                HelperErrorCode::InvalidRequest,
                "helper protocol is already negotiated",
                false,
            )?;
            return Ok(HandleOutcome::Continue);
        }
        let Some(selected) = selected else {
            drop(state);
            self.send_error(
                Some(request_id),
                request_version,
                HelperErrorCode::UnsupportedVersion,
                "no supported helper protocol version was offered",
                false,
            )?;
            return Ok(HandleOutcome::Continue);
        };
        state.protocol_version = Some(selected);
        drop(state);
        self.send(
            Some(request_id),
            selected,
            HelperResponseBody::Hello {
                selected_protocol_version: selected,
                helper_name: self.helper_name.clone(),
                helper_version: self.helper_version.clone(),
            },
        )?;
        Ok(HandleOutcome::Continue)
    }

    fn handle_cancel(
        &self,
        request_id: u64,
        target_request_id: u64,
        protocol_version: u16,
    ) -> Result<HandleOutcome, RemoteFault> {
        let state = self.state.lock().unwrap();
        let active = state
            .active
            .as_ref()
            .filter(|active| active.request_id == target_request_id)
            .ok_or_else(|| {
                RemoteFault::new(
                    HelperErrorCode::InvalidRequest,
                    format!("synthesis request {target_request_id} is not active"),
                    false,
                )
            })?;
        active.cancelled.store(true, Ordering::Release);
        self.engine.stop();
        // Retain the state lock through the acknowledgement. The synthesis
        // worker therefore cannot publish a conflicting successful terminal
        // response after cancellation has been accepted.
        self.send(
            Some(request_id),
            protocol_version,
            HelperResponseBody::CancelAccepted { target_request_id },
        )
        .map_err(RemoteFault::internal)?;
        drop(state);
        Ok(HandleOutcome::Continue)
    }

    fn handle_synthesis(
        self: &Arc<Self>,
        request_id: u64,
        protocol_version: u16,
        text: String,
        settings: HelperSynthesisSettings,
        anchors: Vec<omnivox_tts::RequestedAnchor>,
    ) -> Result<HandleOutcome, RemoteFault> {
        if !self.descriptor.can_synthesize() {
            return Err(RemoteFault::new(
                HelperErrorCode::NotAvailable,
                unavailable_reason(&self.descriptor),
                true,
            ));
        }
        let voice_id = settings
            .voice_id
            .clone()
            .or_else(|| self.descriptor.default_voice_id.clone())
            .ok_or_else(|| {
                RemoteFault::new(
                    HelperErrorCode::VoiceNotFound,
                    "helper has no default voice",
                    false,
                )
            })?;
        if !self
            .descriptor
            .voices
            .iter()
            .any(|voice| voice.id.voice_id == voice_id)
        {
            return Err(RemoteFault::new(
                HelperErrorCode::VoiceNotFound,
                format!("voice was not found: {voice_id}"),
                false,
            ));
        }

        let mut request = SynthesisRequest::new(
            text,
            TtsSettings {
                voice: voice_id.clone(),
                rate: settings.rate,
                pitch: settings.pitch,
                volume: settings.volume,
            },
        );
        request.requested_voice = Some(PhysicalVoiceId::new(
            self.descriptor.id.clone(),
            voice_id.clone(),
        ));
        request.normalized_acss = NormalizedAcss {
            rate: None,
            average_pitch: None,
            pitch_range: settings.pitch_range,
            stress: settings.stress,
            richness: settings.richness,
            volume: None,
        };
        request.anchors = anchors;

        let active = ActiveSynthesis {
            request_id,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        {
            let mut state = self.state.lock().unwrap();
            if state.active.is_some() {
                return Err(RemoteFault::new(
                    HelperErrorCode::Busy,
                    "the helper engine permits one active synthesis",
                    true,
                ));
            }
            state.active = Some(active.clone());
        }

        if let Err(error) = self.send(
            Some(request_id),
            protocol_version,
            HelperResponseBody::SynthesisStarted {
                format: HelperAudioFormat {
                    sample_rate: STANDARD_SAMPLE_RATE,
                    channels: STANDARD_CHANNELS,
                    sample_format: HelperSampleFormat::PcmS16Le,
                },
                actual_voice_id: voice_id,
            },
        ) {
            self.clear_active(request_id);
            return Err(RemoteFault::internal(error));
        }

        let runtime = Arc::clone(self);
        let spawn = thread::Builder::new()
            .name("omnivox-helper-native".to_owned())
            .spawn(move || runtime.synthesis_worker(protocol_version, active, request));
        if let Err(error) = spawn {
            self.clear_active(request_id);
            return Err(RemoteFault::internal(error));
        }
        Ok(HandleOutcome::Continue)
    }

    fn synthesis_worker(
        self: Arc<Self>,
        protocol_version: u16,
        active: ActiveSynthesis,
        request: SynthesisRequest,
    ) {
        let synthesis = panic::catch_unwind(AssertUnwindSafe(|| self.engine.synthesize(&request)));
        let terminal = if active.cancelled.load(Ordering::Acquire) {
            HelperResponseBody::SynthesisCancelled
        } else {
            match synthesis {
                Ok(Ok(result)) => {
                    match self.emit_result(protocol_version, &active, &request, result) {
                        Ok(Some(frame_count)) => {
                            HelperResponseBody::SynthesisCompleted { frame_count }
                        }
                        Ok(None) => HelperResponseBody::SynthesisCancelled,
                        Err(error) => error_response_body(map_tts_error(error)),
                    }
                }
                Ok(Err(error)) => error_response_body(map_tts_error(error)),
                Err(_) => error_response_body(RemoteFault::new(
                    HelperErrorCode::Internal,
                    "helper synthesis panicked",
                    false,
                )),
            }
        };
        let _ = self.finish_synthesis(protocol_version, &active, terminal);
    }

    fn emit_result(
        &self,
        protocol_version: u16,
        active: &ActiveSynthesis,
        request: &SynthesisRequest,
        mut result: SynthesisResult,
    ) -> Result<Option<u64>, TtsError> {
        result.resolve_anchors(
            request,
            self.descriptor.capabilities.markers.requested_anchors,
        );
        result.validate(request)?;
        let sample_bytes = result
            .audio
            .samples
            .len()
            .checked_mul(std::mem::size_of::<i16>())
            .ok_or_else(|| TtsError::SynthesisFailed("helper PCM size overflowed".to_owned()))?;
        if sample_bytes > MAX_HELPER_SYNTHESIS_BYTES {
            return Err(TtsError::SynthesisFailed(format!(
                "PCM exceeds the {MAX_HELPER_SYNTHESIS_BYTES}-byte helper limit"
            )));
        }

        let samples_per_chunk = MAX_HELPER_AUDIO_CHUNK_BYTES / std::mem::size_of::<i16>();
        for (sequence, samples) in result.audio.samples.chunks(samples_per_chunk).enumerate() {
            if active.cancelled.load(Ordering::Acquire) {
                return Ok(None);
            }
            let mut bytes = Vec::with_capacity(samples.len() * std::mem::size_of::<i16>());
            for sample in samples {
                bytes.extend_from_slice(&f32_to_i16(*sample).to_le_bytes());
            }
            let chunk = HelperPcmChunk::from_bytes(sequence as u32, &bytes)
                .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
            self.send(
                Some(active.request_id),
                protocol_version,
                HelperResponseBody::AudioChunk { chunk },
            )
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        }

        let markers = helper_markers(&result)?;
        if !markers.is_empty() {
            self.send(
                Some(active.request_id),
                protocol_version,
                HelperResponseBody::Markers { markers },
            )
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        }
        if active.cancelled.load(Ordering::Acquire) {
            return Ok(None);
        }
        Ok(Some(result.audio.frame_count() as u64))
    }

    fn clear_active(&self, request_id: u64) {
        let mut state = self.state.lock().unwrap();
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.request_id == request_id)
        {
            state.active = None;
        }
    }

    fn finish_synthesis(
        &self,
        protocol_version: u16,
        active: &ActiveSynthesis,
        completed_terminal: HelperResponseBody,
    ) -> Result<(), HelperServerError> {
        let mut state = self.state.lock().unwrap();
        if !state
            .active
            .as_ref()
            .is_some_and(|current| current.request_id == active.request_id)
        {
            return Ok(());
        }
        let terminal = if active.cancelled.load(Ordering::Acquire) {
            HelperResponseBody::SynthesisCancelled
        } else {
            completed_terminal
        };
        let result = self.send(Some(active.request_id), protocol_version, terminal);
        state.active = None;
        result
    }

    fn cancel_active(&self) {
        let active = self.state.lock().unwrap().active.clone();
        if let Some(active) = active {
            active.cancelled.store(true, Ordering::Release);
            self.engine.stop();
        }
    }

    fn send_protocol_error(
        &self,
        request_id: u64,
        request_version: u16,
        error: HelperProtocolError,
    ) -> Result<(), HelperServerError> {
        let code = protocol_error_code(&error);
        let version = if SUPPORTED_HELPER_PROTOCOL_VERSIONS.contains(&request_version) {
            request_version
        } else {
            HELPER_PROTOCOL_VERSION
        };
        self.send_error(Some(request_id), version, code, &error.to_string(), false)
    }

    fn send_error(
        &self,
        request_id: Option<u64>,
        protocol_version: u16,
        code: HelperErrorCode,
        message: &str,
        retryable: bool,
    ) -> Result<(), HelperServerError> {
        self.send(
            request_id,
            protocol_version,
            HelperResponseBody::Error {
                code,
                message: bounded_message(message),
                retryable,
            },
        )
    }

    fn send(
        &self,
        request_id: Option<u64>,
        protocol_version: u16,
        body: HelperResponseBody,
    ) -> Result<(), HelperServerError> {
        let response = HelperResponse {
            protocol_version,
            request_id,
            body,
        };
        response.validate()?;
        write_frame(&mut *self.writer.lock().unwrap(), &response)?;
        Ok(())
    }
}

#[derive(Debug)]
struct RemoteFault {
    code: HelperErrorCode,
    message: String,
    retryable: bool,
}

impl RemoteFault {
    fn new(code: HelperErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(HelperErrorCode::Internal, error.to_string(), false)
    }
}

fn error_response_body(fault: RemoteFault) -> HelperResponseBody {
    HelperResponseBody::Error {
        code: fault.code,
        message: bounded_message(&fault.message),
        retryable: fault.retryable,
    }
}

fn unavailable_reason(descriptor: &EngineDescriptor) -> &str {
    match &descriptor.availability {
        omnivox_tts::contracts::Availability::Unavailable { reason } => reason,
        omnivox_tts::contracts::Availability::Available => match &descriptor.health {
            omnivox_tts::contracts::EngineHealth::Failed { reason } => reason,
            _ => "the helper engine is not available",
        },
    }
}

fn map_tts_error(error: TtsError) -> RemoteFault {
    match error {
        TtsError::VoiceNotFound(message) => {
            RemoteFault::new(HelperErrorCode::VoiceNotFound, message, false)
        }
        TtsError::NotAvailable => RemoteFault::new(
            HelperErrorCode::NotAvailable,
            "the helper engine is not available",
            true,
        ),
        TtsError::InvalidParameter(message) => {
            RemoteFault::new(HelperErrorCode::InvalidParameter, message, false)
        }
        TtsError::SynthesisFailed(message) => {
            RemoteFault::new(HelperErrorCode::SynthesisFailed, message, true)
        }
    }
}

fn protocol_error_code(error: &HelperProtocolError) -> HelperErrorCode {
    match error {
        HelperProtocolError::UnsupportedVersion(_) => HelperErrorCode::UnsupportedVersion,
        HelperProtocolError::FrameTooLarge
        | HelperProtocolError::TextTooLarge
        | HelperProtocolError::AudioChunkTooLarge => HelperErrorCode::PayloadTooLarge,
        _ => HelperErrorCode::InvalidRequest,
    }
}

fn helper_markers(result: &SynthesisResult) -> Result<Vec<HelperMarker>, TtsError> {
    let mut markers = result
        .markers
        .iter()
        .map(|marker| HelperMarker {
            kind: match marker.kind {
                SynthesisMarkerKind::Word => HelperMarkerKind::Word,
                SynthesisMarkerKind::Sentence => HelperMarkerKind::Sentence,
                SynthesisMarkerKind::Phoneme => HelperMarkerKind::Phoneme,
                SynthesisMarkerKind::NativeIndex => HelperMarkerKind::NativeIndex,
            },
            frame_offset: marker.frame_offset,
            text_start: marker.text_start,
            text_length: marker.text_length,
            value: marker.value.clone(),
        })
        .collect::<Vec<_>>();
    markers.extend(
        result
            .anchors
            .iter()
            .filter(|anchor| anchor.resolution == omnivox_tts::AnchorResolution::Exact)
            .map(|anchor| HelperMarker {
                kind: HelperMarkerKind::RequestedAnchor,
                frame_offset: anchor.frame_offset.expect("an exact anchor has a frame"),
                text_start: None,
                text_length: None,
                value: Some(anchor.id.clone()),
            }),
    );
    if markers.len() > MAX_HELPER_MARKERS {
        return Err(TtsError::SynthesisFailed(format!(
            "the engine returned more than {MAX_HELPER_MARKERS} helper markers"
        )));
    }
    markers.sort_by_key(|marker| marker.frame_offset);
    Ok(markers)
}

fn f32_to_i16(sample: f32) -> i16 {
    let sample = sample.clamp(-1.0, 1.0);
    if sample <= -1.0 {
        i16::MIN
    } else {
        (sample * f32::from(i16::MAX)).round() as i16
    }
}

fn bounded_message(message: &str) -> String {
    let message = if message.is_empty() {
        "helper error"
    } else {
        message
    };
    if message.len() <= MAX_HELPER_MESSAGE_BYTES {
        return message.to_owned();
    }
    let mut end = MAX_HELPER_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Condvar;
    use std::time::{Duration, Instant};

    use omnivox_tts::contracts::{
        AcssCapabilities, Availability, EngineCapabilities, EngineHealth, MarkerCapabilities,
        TextRepertoire, VoiceDescriptor,
    };
    use omnivox_tts::{AudioBuffer, VoiceInfo, VoiceQuality};

    use super::*;

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SharedWriter {
        fn responses(&self) -> Vec<HelperResponse> {
            let bytes = self.0.lock().unwrap().clone();
            let mut reader = BufReader::new(Cursor::new(bytes));
            let mut responses = Vec::new();
            while let Some(response) = read_frame(&mut reader).unwrap() {
                responses.push(response);
            }
            responses
        }

        fn wait_for(&self, predicate: impl Fn(&HelperResponseBody) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if self
                    .responses()
                    .iter()
                    .any(|response| predicate(&response.body))
                {
                    return;
                }
                assert!(Instant::now() < deadline, "helper response did not arrive");
                thread::yield_now();
            }
        }
    }

    fn descriptor() -> EngineDescriptor {
        let voice = VoiceInfo {
            identifier: "mock:test".to_owned(),
            name: "Test".to_owned(),
            language: "en-US".to_owned(),
            quality: VoiceQuality::Enhanced,
        };
        EngineDescriptor {
            id: "mock".to_owned(),
            display_name: "Test helper engine".to_owned(),
            version: Some("test".to_owned()),
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: EngineCapabilities {
                acss: AcssCapabilities {
                    rate: true,
                    ..AcssCapabilities::default()
                },
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::PlaybackOnly,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities::default(),
                language_switching: false,
                text_repertoire: TextRepertoire::Unicode,
                post_synthesis_dimensions: Vec::new(),
                native_extensions: Vec::new(),
            },
            voices: vec![VoiceDescriptor::from_voice_info("mock", voice)],
            default_voice_id: Some("mock:test".to_owned()),
        }
    }

    struct ImmediateEngine;

    impl TtsEngine for ImmediateEngine {
        fn descriptor(&self) -> EngineDescriptor {
            descriptor()
        }

        fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            Ok(SynthesisResult::audio(
                "mock",
                request.requested_voice.clone(),
                AudioBuffer::new(vec![0.5, -0.5, 1.0, -1.0]),
            ))
        }

        fn stop(&self) {}

        fn is_speaking(&self) -> bool {
            false
        }

        fn available_voices(&self) -> Vec<VoiceInfo> {
            Vec::new()
        }

        fn voice_info(&self, _identifier: &str) -> Option<VoiceInfo> {
            None
        }
    }

    struct UnavailableEngine;

    impl TtsEngine for UnavailableEngine {
        fn descriptor(&self) -> EngineDescriptor {
            let mut descriptor = descriptor();
            descriptor.availability = Availability::Unavailable {
                reason: "test runtime is missing".to_owned(),
            };
            descriptor.voices.clear();
            descriptor.default_voice_id = None;
            descriptor
        }

        fn synthesize(&self, _request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            Err(TtsError::NotAvailable)
        }

        fn stop(&self) {}

        fn is_speaking(&self) -> bool {
            false
        }

        fn available_voices(&self) -> Vec<VoiceInfo> {
            Vec::new()
        }

        fn voice_info(&self, _identifier: &str) -> Option<VoiceInfo> {
            None
        }
    }

    struct BlockingEngine {
        state: Mutex<bool>,
        changed: Condvar,
        started: AtomicBool,
        stops: AtomicUsize,
    }

    impl BlockingEngine {
        fn new() -> Self {
            Self {
                state: Mutex::new(false),
                changed: Condvar::new(),
                started: AtomicBool::new(false),
                stops: AtomicUsize::new(0),
            }
        }

        fn wait_for_start(&self) {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !self.started.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline, "blocking engine did not start");
                thread::yield_now();
            }
        }

        fn release(&self) {
            *self.state.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }

    impl TtsEngine for BlockingEngine {
        fn descriptor(&self) -> EngineDescriptor {
            descriptor()
        }

        fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            self.started.store(true, Ordering::Release);
            let mut released = self.state.lock().unwrap();
            while !*released {
                released = self.changed.wait(released).unwrap();
            }
            Ok(SynthesisResult::audio(
                "mock",
                request.requested_voice.clone(),
                AudioBuffer::empty(),
            ))
        }

        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::AcqRel);
        }

        fn is_speaking(&self) -> bool {
            self.started.load(Ordering::Acquire)
        }

        fn available_voices(&self) -> Vec<VoiceInfo> {
            Vec::new()
        }

        fn voice_info(&self, _identifier: &str) -> Option<VoiceInfo> {
            None
        }
    }

    fn hello(request_id: u64) -> HelperRequest {
        HelperRequest::new(
            request_id,
            HelperRequestBody::Hello {
                supported_protocol_versions: SUPPORTED_HELPER_PROTOCOL_VERSIONS.to_vec(),
            },
        )
    }

    fn synthesis(request_id: u64) -> HelperRequest {
        HelperRequest::new(
            request_id,
            HelperRequestBody::Synthesize {
                text: "Helper protocol".to_owned(),
                settings: HelperSynthesisSettings {
                    voice_id: Some("mock:test".to_owned()),
                    rate: 0.5,
                    pitch: 1.0,
                    volume: 1.0,
                    pitch_range: None,
                    stress: None,
                    richness: None,
                },
                anchors: Some(Vec::new()),
            },
        )
    }

    #[test]
    fn negotiates_and_streams_bounded_canonical_pcm() {
        let writer = SharedWriter::default();
        let engine: Arc<dyn TtsEngine> = Arc::new(ImmediateEngine);
        let runtime = Arc::new(
            HelperRuntime::new(
                engine,
                writer.clone(),
                "Test helper".to_owned(),
                "1".to_owned(),
            )
            .unwrap(),
        );

        runtime.handle(hello(1)).unwrap();
        runtime
            .handle(HelperRequest::new(2, HelperRequestBody::Describe))
            .unwrap();
        runtime.handle(synthesis(3)).unwrap();
        writer.wait_for(|body| matches!(body, HelperResponseBody::SynthesisCompleted { .. }));

        let responses = writer.responses();
        assert!(responses.iter().all(|response| response.validate().is_ok()));
        let described = responses.iter().find_map(|response| match &response.body {
            HelperResponseBody::Descriptor { descriptor } => Some(descriptor),
            _ => None,
        });
        assert_eq!(
            described.unwrap().capabilities.cancellation,
            CancellationSupport::SynthesisAndPlayback
        );
        let chunks = responses
            .iter()
            .filter_map(|response| match &response.body {
                HelperResponseBody::AudioChunk { chunk } => Some(chunk.decode_samples().unwrap()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(chunks, vec![16384, -16384, 32767, -32768]);
        assert!(responses.iter().any(|response| matches!(
            response.body,
            HelperResponseBody::SynthesisCompleted { frame_count: 2 }
        )));
    }

    #[test]
    fn cancel_is_acknowledged_while_native_synthesis_is_blocked() {
        let writer = SharedWriter::default();
        let native = Arc::new(BlockingEngine::new());
        let engine: Arc<dyn TtsEngine> = native.clone();
        let runtime = Arc::new(
            HelperRuntime::new(
                engine,
                writer.clone(),
                "Test helper".to_owned(),
                "1".to_owned(),
            )
            .unwrap(),
        );

        runtime.handle(hello(1)).unwrap();
        runtime.handle(synthesis(2)).unwrap();
        native.wait_for_start();
        runtime
            .handle(HelperRequest::new(
                3,
                HelperRequestBody::Cancel {
                    target_request_id: 2,
                },
            ))
            .unwrap();
        writer.wait_for(|body| matches!(body, HelperResponseBody::CancelAccepted { .. }));
        assert_eq!(native.stops.load(Ordering::Acquire), 1);

        native.release();
        writer.wait_for(|body| matches!(body, HelperResponseBody::SynthesisCancelled));
        let responses = writer.responses();
        assert!(!responses
            .iter()
            .any(|response| matches!(response.body, HelperResponseBody::AudioChunk { .. })));
        assert!(responses
            .iter()
            .any(|response| matches!(response.body, HelperResponseBody::SynthesisCancelled)));
        let accepted = responses
            .iter()
            .position(|response| matches!(response.body, HelperResponseBody::CancelAccepted { .. }))
            .unwrap();
        let cancelled = responses
            .iter()
            .position(|response| matches!(response.body, HelperResponseBody::SynthesisCancelled))
            .unwrap();
        assert!(accepted < cancelled);
    }

    #[test]
    fn shutdown_does_not_wait_for_a_blocked_native_call() {
        let writer = SharedWriter::default();
        let native = Arc::new(BlockingEngine::new());
        let engine: Arc<dyn TtsEngine> = native.clone();
        let runtime = Arc::new(
            HelperRuntime::new(
                engine,
                writer.clone(),
                "Test helper".to_owned(),
                "1".to_owned(),
            )
            .unwrap(),
        );

        runtime.handle(hello(1)).unwrap();
        runtime.handle(synthesis(2)).unwrap();
        native.wait_for_start();
        let started_at = Instant::now();
        assert_eq!(
            runtime
                .handle(HelperRequest::new(3, HelperRequestBody::Shutdown))
                .unwrap(),
            HandleOutcome::Shutdown
        );
        assert!(started_at.elapsed() < Duration::from_millis(100));
        writer.wait_for(|body| matches!(body, HelperResponseBody::ShuttingDown));
        assert_eq!(native.stops.load(Ordering::Acquire), 1);

        native.release();
        writer.wait_for(|body| matches!(body, HelperResponseBody::SynthesisCancelled));
    }

    #[test]
    fn malformed_input_receives_an_unowned_protocol_error() {
        let writer = SharedWriter::default();
        let engine: Arc<dyn TtsEngine> = Arc::new(ImmediateEngine);
        let result = run_helper(
            BufReader::new(Cursor::new(b"{not-json}\n")),
            writer.clone(),
            engine,
            "Test helper",
            "1",
        );

        assert!(matches!(result, Err(HelperServerError::Protocol(_))));
        let responses = writer.responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].request_id, None);
        assert!(matches!(
            responses[0].body,
            HelperResponseBody::Error {
                code: HelperErrorCode::InvalidRequest,
                ..
            }
        ));
    }

    #[test]
    fn unavailable_engine_negotiates_and_reports_runtime_diagnostics() {
        let writer = SharedWriter::default();
        let engine: Arc<dyn TtsEngine> = Arc::new(UnavailableEngine);
        let runtime = Arc::new(
            HelperRuntime::new(
                engine,
                writer.clone(),
                "Test helper".to_owned(),
                "1".to_owned(),
            )
            .unwrap(),
        );

        runtime.handle(hello(1)).unwrap();
        runtime
            .handle(HelperRequest::new(2, HelperRequestBody::Describe))
            .unwrap();
        runtime.handle(synthesis(3)).unwrap();
        runtime
            .handle(HelperRequest::new(4, HelperRequestBody::Ping))
            .unwrap();
        assert_eq!(
            runtime
                .handle(HelperRequest::new(5, HelperRequestBody::Shutdown))
                .unwrap(),
            HandleOutcome::Shutdown
        );

        let responses = writer.responses();
        let unavailable = responses
            .iter()
            .filter(|response| {
                matches!(
                    &response.body,
                    HelperResponseBody::Error {
                        code: HelperErrorCode::NotAvailable,
                        message,
                        retryable: true,
                    } if message == "test runtime is missing"
                )
            })
            .count();
        assert_eq!(unavailable, 2);
        assert!(responses
            .iter()
            .any(|response| matches!(response.body, HelperResponseBody::Pong)));
        assert!(responses
            .iter()
            .any(|response| matches!(response.body, HelperResponseBody::ShuttingDown)));
    }
}
