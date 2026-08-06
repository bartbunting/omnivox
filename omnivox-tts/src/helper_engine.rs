//! Host-side support for TTS engines backed by helper processes.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use thiserror::Error;
use tracing::warn;

use crate::contracts::{AudioOutputMode, CancellationSupport, ConcurrencyModel, EngineDescriptor};
use crate::engine_registry::validate_descriptor;
use crate::helper_protocol::{
    read_frame, write_frame, HelperAudioFormat, HelperErrorCode, HelperMarker, HelperRequest,
    HelperRequestBody, HelperResponse, HelperResponseBody, HelperSynthesisSettings,
    HELPER_PROTOCOL_VERSION, MAX_HELPER_MARKERS, MAX_HELPER_SYNTHESIS_BYTES,
};
use crate::{AudioBuffer, TtsEngine, TtsError, TtsSettings, VoiceInfo};

#[derive(Debug, Error)]
pub enum HelperEngineError {
    #[error(transparent)]
    Protocol(#[from] crate::helper_protocol::HelperProtocolError),

    #[error("helper response belongs to request {received:?}, expected {expected}")]
    RequestMismatch {
        expected: u64,
        received: Option<u64>,
    },

    #[error("unexpected helper synthesis response: {0}")]
    UnexpectedResponse(&'static str),

    #[error("helper PCM chunk sequence {received} arrived; expected {expected}")]
    AudioSequenceMismatch { expected: u32, received: u32 },

    #[error("helper PCM chunk does not contain complete interleaved audio frames")]
    AudioFrameAlignment,

    #[error("helper synthesis exceeds the {MAX_HELPER_SYNTHESIS_BYTES}-byte limit")]
    SynthesisTooLarge,

    #[error("helper reported {reported} audio frames after returning {received}")]
    FrameCountMismatch { reported: u64, received: u64 },

    #[error("helper marker at frame {offset} exceeds the {frame_count}-frame result")]
    MarkerOutOfRange { offset: u64, frame_count: u64 },

    #[error("helper synthesized with voice {received}, expected {expected}")]
    ActualVoiceMismatch { expected: String, received: String },

    #[error("helper returned {code:?}: {message}")]
    Remote {
        code: HelperErrorCode,
        message: String,
        retryable: bool,
    },

    #[error("helper transport failed: {0}")]
    Transport(String),

    #[error("helper timed out while waiting for {0}")]
    Timeout(&'static str),

    #[error("helper exited before returning a complete response")]
    Exited,

    #[error("helper described engine {received}, expected {expected}")]
    EngineIdMismatch { expected: String, received: String },

    #[error("helper returned an invalid engine descriptor: {0}")]
    InvalidDescriptor(String),

    #[error("helper acknowledged cancellation of request {received}, expected {expected}")]
    CancelTargetMismatch { expected: u64, received: u64 },
}

#[derive(Debug, Clone)]
pub struct HelperEngineConfig {
    pub engine_id: String,
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
    pub synthesis_idle_timeout: Duration,
}

impl HelperEngineConfig {
    pub fn new(engine_id: impl Into<String>, program: impl Into<PathBuf>) -> Self {
        Self {
            engine_id: engine_id.into(),
            program: program.into(),
            arguments: Vec::new(),
            startup_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(10),
            synthesis_idle_timeout: Duration::from_secs(10),
        }
    }
}

trait HelperConnection: Send + Sync {
    fn send(&self, request: &HelperRequest) -> Result<(), HelperEngineError>;
    fn receive(&self, timeout: Duration) -> Result<HelperResponse, HelperEngineError>;
    fn terminate(&self);
}

trait HelperConnector: Send + Sync {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError>;
}

struct ProcessHelperConnector {
    program: PathBuf,
    arguments: Vec<OsString>,
}

impl ProcessHelperConnector {
    fn new(config: &HelperEngineConfig) -> Self {
        Self {
            program: config.program.clone(),
            arguments: config.arguments.clone(),
        }
    }
}

impl HelperConnector for ProcessHelperConnector {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        Ok(Arc::new(ProcessHelperConnection::spawn(
            &self.program,
            &self.arguments,
        )?))
    }
}

type HelperReadResult = Result<HelperResponse, HelperEngineError>;

struct ProcessHelperConnection {
    writer: Mutex<Option<BufWriter<ChildStdin>>>,
    responses: Mutex<mpsc::Receiver<HelperReadResult>>,
    child: Mutex<Child>,
    reader_handle: Mutex<Option<JoinHandle<()>>>,
}

impl ProcessHelperConnection {
    fn spawn(program: &Path, arguments: &[OsString]) -> Result<Self, HelperEngineError> {
        let mut child = Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                HelperEngineError::Transport(format!(
                    "could not start {}: {error}",
                    program.display()
                ))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HelperEngineError::Transport("helper stdin was not piped".to_owned()))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            HelperEngineError::Transport("helper stdout was not piped".to_owned())
        })?;
        let (response_sender, response_receiver) = mpsc::channel();
        let reader_handle = std::thread::Builder::new()
            .name("omnivox-helper-reader".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let result = match read_frame(&mut reader) {
                        Ok(Some(response)) => Ok(response),
                        Ok(None) => Err(HelperEngineError::Exited),
                        Err(error) => Err(error.into()),
                    };
                    let terminal = result.is_err();
                    if response_sender.send(result).is_err() || terminal {
                        break;
                    }
                }
            })
            .map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                HelperEngineError::Transport(format!(
                    "could not start helper reader thread: {error}"
                ))
            })?;

        Ok(Self {
            writer: Mutex::new(Some(BufWriter::new(stdin))),
            responses: Mutex::new(response_receiver),
            child: Mutex::new(child),
            reader_handle: Mutex::new(Some(reader_handle)),
        })
    }
}

impl HelperConnection for ProcessHelperConnection {
    fn send(&self, request: &HelperRequest) -> Result<(), HelperEngineError> {
        request.validate()?;
        let mut writer = self.writer.lock().unwrap();
        let writer = writer.as_mut().ok_or(HelperEngineError::Exited)?;
        write_frame(writer, request)?;
        Ok(())
    }

    fn receive(&self, timeout: Duration) -> Result<HelperResponse, HelperEngineError> {
        match self.responses.lock().unwrap().recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err(HelperEngineError::Timeout("helper response"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(HelperEngineError::Exited),
        }
    }

    fn terminate(&self) {
        self.writer.lock().unwrap().take();
        {
            let mut child = self.child.lock().unwrap();
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        if let Some(handle) = self.reader_handle.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProcessHelperConnection {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Debug)]
pub(crate) enum HelperSynthesisResult {
    Completed(AudioBuffer),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SynthesisPhase {
    AwaitingStart,
    Streaming,
    Terminal,
}

pub(crate) struct HelperSynthesisCollector {
    request_id: u64,
    expected_voice_id: Option<String>,
    phase: SynthesisPhase,
    format: Option<HelperAudioFormat>,
    next_sequence: u32,
    samples: Vec<i16>,
    markers: Vec<HelperMarker>,
}

impl HelperSynthesisCollector {
    pub(crate) fn new(request_id: u64, expected_voice_id: Option<String>) -> Self {
        Self {
            request_id,
            expected_voice_id,
            phase: SynthesisPhase::AwaitingStart,
            format: None,
            next_sequence: 0,
            samples: Vec::new(),
            markers: Vec::new(),
        }
    }

    pub(crate) fn accept(
        &mut self,
        response: HelperResponse,
    ) -> Result<Option<HelperSynthesisResult>, HelperEngineError> {
        response.validate()?;
        if response.request_id != Some(self.request_id) {
            return Err(HelperEngineError::RequestMismatch {
                expected: self.request_id,
                received: response.request_id,
            });
        }
        if self.phase == SynthesisPhase::Terminal {
            return Err(HelperEngineError::UnexpectedResponse(
                "response after terminal result",
            ));
        }

        match response.body {
            HelperResponseBody::SynthesisStarted {
                format,
                actual_voice_id,
            } if self.phase == SynthesisPhase::AwaitingStart => {
                if let Some(expected_voice_id) = &self.expected_voice_id {
                    if actual_voice_id != *expected_voice_id {
                        return Err(HelperEngineError::ActualVoiceMismatch {
                            expected: expected_voice_id.clone(),
                            received: actual_voice_id,
                        });
                    }
                }
                self.format = Some(format);
                self.phase = SynthesisPhase::Streaming;
                Ok(None)
            }
            HelperResponseBody::AudioChunk { chunk } if self.phase == SynthesisPhase::Streaming => {
                if chunk.sequence != self.next_sequence {
                    return Err(HelperEngineError::AudioSequenceMismatch {
                        expected: self.next_sequence,
                        received: chunk.sequence,
                    });
                }
                let bytes = chunk.decode_bytes()?;
                let format = self
                    .format
                    .expect("streaming synthesis has an audio format");
                let audio_frame_bytes = usize::from(format.channels) * size_of::<i16>();
                if !bytes.chunks_exact(audio_frame_bytes).remainder().is_empty() {
                    return Err(HelperEngineError::AudioFrameAlignment);
                }
                let decoded_length = self
                    .samples
                    .len()
                    .checked_mul(size_of::<i16>())
                    .and_then(|length| length.checked_add(bytes.len()));
                if match decoded_length {
                    Some(length) => length > MAX_HELPER_SYNTHESIS_BYTES,
                    None => true,
                } {
                    return Err(HelperEngineError::SynthesisTooLarge);
                }
                self.samples.extend(
                    bytes
                        .chunks_exact(2)
                        .map(|sample| i16::from_le_bytes([sample[0], sample[1]])),
                );
                self.next_sequence = self.next_sequence.wrapping_add(1);
                Ok(None)
            }
            HelperResponseBody::Markers { markers } if self.phase == SynthesisPhase::Streaming => {
                if self.markers.len().saturating_add(markers.len()) > MAX_HELPER_MARKERS {
                    return Err(crate::helper_protocol::HelperProtocolError::TooManyMarkers.into());
                }
                self.markers.extend(markers);
                Ok(None)
            }
            HelperResponseBody::SynthesisCompleted { frame_count }
                if self.phase == SynthesisPhase::Streaming =>
            {
                let format = self
                    .format
                    .expect("streaming synthesis has an audio format");
                let received_frame_count = self.samples.len() as u64 / u64::from(format.channels);
                if frame_count != received_frame_count {
                    return Err(HelperEngineError::FrameCountMismatch {
                        reported: frame_count,
                        received: received_frame_count,
                    });
                }
                if let Some(marker) = self
                    .markers
                    .iter()
                    .find(|marker| marker.frame_offset > frame_count)
                {
                    return Err(HelperEngineError::MarkerOutOfRange {
                        offset: marker.frame_offset,
                        frame_count,
                    });
                }
                self.phase = SynthesisPhase::Terminal;
                Ok(Some(HelperSynthesisResult::Completed(
                    AudioBuffer::from_i16(&self.samples, format.sample_rate, format.channels),
                )))
            }
            HelperResponseBody::SynthesisCancelled => {
                self.phase = SynthesisPhase::Terminal;
                Ok(Some(HelperSynthesisResult::Cancelled))
            }
            HelperResponseBody::Error {
                code,
                message,
                retryable,
            } => {
                self.phase = SynthesisPhase::Terminal;
                Err(HelperEngineError::Remote {
                    code,
                    message,
                    retryable,
                })
            }
            HelperResponseBody::SynthesisStarted { .. } => Err(
                HelperEngineError::UnexpectedResponse("duplicate synthesis_started"),
            ),
            HelperResponseBody::AudioChunk { .. }
            | HelperResponseBody::Markers { .. }
            | HelperResponseBody::SynthesisCompleted { .. } => Err(
                HelperEngineError::UnexpectedResponse("synthesis data before synthesis_started"),
            ),
            HelperResponseBody::Hello { .. }
            | HelperResponseBody::Descriptor { .. }
            | HelperResponseBody::Pong
            | HelperResponseBody::CancelAccepted { .. }
            | HelperResponseBody::ShuttingDown => Err(HelperEngineError::UnexpectedResponse(
                "non-synthesis response in synthesis stream",
            )),
        }
    }
}

pub struct HelperTtsEngine {
    config: HelperEngineConfig,
    connector: Arc<dyn HelperConnector>,
    connection: RwLock<Option<Arc<dyn HelperConnection>>>,
    descriptor: RwLock<Option<EngineDescriptor>>,
    next_request_id: AtomicU64,
    active_request_id: AtomicU64,
    pending_cancellations: Mutex<HashMap<u64, u64>>,
    lifecycle: Mutex<()>,
    dispatch: Mutex<()>,
}

impl HelperTtsEngine {
    pub fn new(config: HelperEngineConfig) -> Result<Self, HelperEngineError> {
        let connector = Arc::new(ProcessHelperConnector::new(&config));
        Self::with_connector(config, connector)
    }

    fn with_connector(
        config: HelperEngineConfig,
        connector: Arc<dyn HelperConnector>,
    ) -> Result<Self, HelperEngineError> {
        if config.engine_id.is_empty() {
            return Err(HelperEngineError::InvalidDescriptor(
                "configured engine ID is empty".to_owned(),
            ));
        }
        if config.startup_timeout.is_zero()
            || config.request_timeout.is_zero()
            || config.synthesis_idle_timeout.is_zero()
        {
            return Err(HelperEngineError::InvalidDescriptor(
                "helper timeouts must be positive".to_owned(),
            ));
        }

        let engine = Self {
            config,
            connector,
            connection: RwLock::new(None),
            descriptor: RwLock::new(None),
            next_request_id: AtomicU64::new(1),
            active_request_id: AtomicU64::new(0),
            pending_cancellations: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            dispatch: Mutex::new(()),
        };
        engine.install_fresh_connection()?;
        Ok(engine)
    }

    fn allocate_request_id(&self) -> u64 {
        loop {
            let request_id = self.next_request_id.fetch_add(1, Ordering::AcqRel);
            if request_id != 0 {
                return request_id;
            }
        }
    }

    fn install_fresh_connection(&self) -> Result<(), HelperEngineError> {
        if let Some(connection) = self.connection.write().unwrap().take() {
            connection.terminate();
        }
        self.pending_cancellations.lock().unwrap().clear();

        let connection = self.connector.connect()?;
        let descriptor = match self.negotiate(&connection) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                connection.terminate();
                return Err(error);
            }
        };
        *self.descriptor.write().unwrap() = Some(descriptor);
        *self.connection.write().unwrap() = Some(connection);
        Ok(())
    }

    fn negotiate(
        &self,
        connection: &Arc<dyn HelperConnection>,
    ) -> Result<EngineDescriptor, HelperEngineError> {
        let hello_id = self.allocate_request_id();
        let hello = HelperRequest::new(
            hello_id,
            HelperRequestBody::Hello {
                supported_protocol_versions: vec![HELPER_PROTOCOL_VERSION],
            },
        );
        connection.send(&hello)?;
        let response = receive_owned_response(connection, hello_id, self.config.startup_timeout)?;
        match response.body {
            HelperResponseBody::Hello {
                selected_protocol_version,
                ..
            } if selected_protocol_version == HELPER_PROTOCOL_VERSION => {}
            HelperResponseBody::Error {
                code,
                message,
                retryable,
            } => {
                return Err(HelperEngineError::Remote {
                    code,
                    message,
                    retryable,
                });
            }
            _ => {
                return Err(HelperEngineError::UnexpectedResponse(
                    "expected hello response",
                ));
            }
        }

        let describe_id = self.allocate_request_id();
        connection.send(&HelperRequest::new(
            describe_id,
            HelperRequestBody::Describe,
        ))?;
        let response =
            receive_owned_response(connection, describe_id, self.config.request_timeout)?;
        let descriptor = match response.body {
            HelperResponseBody::Descriptor { descriptor } => descriptor,
            HelperResponseBody::Error {
                code,
                message,
                retryable,
            } => {
                return Err(HelperEngineError::Remote {
                    code,
                    message,
                    retryable,
                });
            }
            _ => {
                return Err(HelperEngineError::UnexpectedResponse(
                    "expected descriptor response",
                ));
            }
        };
        self.validate_descriptor(&descriptor)?;
        Ok(descriptor)
    }

    fn validate_descriptor(&self, descriptor: &EngineDescriptor) -> Result<(), HelperEngineError> {
        if descriptor.id != self.config.engine_id {
            return Err(HelperEngineError::EngineIdMismatch {
                expected: self.config.engine_id.clone(),
                received: descriptor.id.clone(),
            });
        }
        validate_descriptor(descriptor)
            .map_err(|error| HelperEngineError::InvalidDescriptor(error.to_string()))?;
        if descriptor.capabilities.audio_output != AudioOutputMode::BufferedPcm {
            return Err(HelperEngineError::InvalidDescriptor(
                "helper engine must return buffered PCM".to_owned(),
            ));
        }
        if descriptor.capabilities.concurrency != ConcurrencyModel::Serialized {
            return Err(HelperEngineError::InvalidDescriptor(
                "helper protocol version 1 requires serialized synthesis".to_owned(),
            ));
        }
        if descriptor.capabilities.cancellation != CancellationSupport::SynthesisAndPlayback {
            return Err(HelperEngineError::InvalidDescriptor(
                "helper protocol version 1 requires synthesis cancellation".to_owned(),
            ));
        }
        if !descriptor.can_synthesize() {
            return Err(HelperEngineError::InvalidDescriptor(
                "helper engine is not currently available".to_owned(),
            ));
        }
        Ok(())
    }

    fn current_connection(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        self.connection
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .ok_or(HelperEngineError::Exited)
    }

    fn consume_cancel_response(
        &self,
        response: &HelperResponse,
    ) -> Result<bool, HelperEngineError> {
        let Some(request_id) = response.request_id else {
            return Ok(false);
        };
        let mut pending = self.pending_cancellations.lock().unwrap();
        let Some(expected_target) = pending.get(&request_id).copied() else {
            return Ok(false);
        };
        match &response.body {
            HelperResponseBody::CancelAccepted { target_request_id }
                if *target_request_id == expected_target =>
            {
                pending.remove(&request_id);
                Ok(true)
            }
            HelperResponseBody::CancelAccepted { target_request_id } => {
                Err(HelperEngineError::CancelTargetMismatch {
                    expected: expected_target,
                    received: *target_request_id,
                })
            }
            HelperResponseBody::Error { .. } => {
                pending.remove(&request_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn invalidate_connection(&self, connection: &Arc<dyn HelperConnection>) {
        let removed = {
            let mut current = self.connection.write().unwrap();
            if current
                .as_ref()
                .is_some_and(|candidate| Arc::ptr_eq(candidate, connection))
            {
                current.take()
            } else {
                None
            }
        };
        if let Some(connection) = removed {
            connection.terminate();
        }
    }

    fn synthesis_error(
        &self,
        connection: &Arc<dyn HelperConnection>,
        error: HelperEngineError,
    ) -> TtsError {
        if !matches!(error, HelperEngineError::Remote { .. }) {
            self.invalidate_connection(connection);
        }
        Self::map_error(error)
    }

    fn map_error(error: HelperEngineError) -> TtsError {
        match error {
            HelperEngineError::Remote {
                code: HelperErrorCode::VoiceNotFound,
                message,
                ..
            } => TtsError::VoiceNotFound(message),
            HelperEngineError::Remote {
                code: HelperErrorCode::NotAvailable | HelperErrorCode::UnsupportedVersion,
                ..
            } => TtsError::NotAvailable,
            HelperEngineError::Remote {
                code:
                    HelperErrorCode::InvalidRequest
                    | HelperErrorCode::PayloadTooLarge
                    | HelperErrorCode::InvalidParameter,
                message,
                ..
            } => TtsError::InvalidParameter(message),
            other => TtsError::SynthesisFailed(other.to_string()),
        }
    }
}

fn receive_owned_response(
    connection: &Arc<dyn HelperConnection>,
    request_id: u64,
    timeout: Duration,
) -> Result<HelperResponse, HelperEngineError> {
    let response = connection.receive(timeout)?;
    response.validate()?;
    if response.request_id != Some(request_id) {
        return Err(HelperEngineError::RequestMismatch {
            expected: request_id,
            received: response.request_id,
        });
    }
    Ok(response)
}

struct ActiveRequestGuard<'a> {
    request_id: u64,
    active_request_id: &'a AtomicU64,
    dispatch: &'a Mutex<()>,
}

impl Drop for ActiveRequestGuard<'_> {
    fn drop(&mut self) {
        let _dispatch = self.dispatch.lock().unwrap();
        let _ = self.active_request_id.compare_exchange(
            self.request_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

impl TtsEngine for HelperTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor
            .read()
            .unwrap()
            .as_ref()
            .expect("a constructed helper engine has a descriptor")
            .clone()
    }

    fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
        let _lifecycle = self.lifecycle.lock().unwrap();
        self.install_fresh_connection().map_err(Self::map_error)
    }

    fn synthesize(&self, text: &str, settings: &TtsSettings) -> Result<AudioBuffer, TtsError> {
        let _lifecycle = self.lifecycle.lock().unwrap();
        let connection = self.current_connection().map_err(Self::map_error)?;
        let request_id = self.allocate_request_id();
        let requested_voice_id = if settings.voice.is_empty() {
            None
        } else {
            Some(settings.voice.clone())
        };
        let request = HelperRequest::new(
            request_id,
            HelperRequestBody::Synthesize {
                text: text.to_owned(),
                settings: HelperSynthesisSettings {
                    voice_id: requested_voice_id.clone(),
                    rate: settings.rate,
                    pitch: settings.pitch,
                    volume: settings.volume,
                },
            },
        );
        request.validate().map_err(|error| {
            TtsError::InvalidParameter(format!("invalid helper synthesis request: {error}"))
        })?;

        {
            let _dispatch = self.dispatch.lock().unwrap();
            if let Err(error) = connection.send(&request) {
                return Err(self.synthesis_error(&connection, error));
            }
            self.active_request_id.store(request_id, Ordering::Release);
        }
        let _active = ActiveRequestGuard {
            request_id,
            active_request_id: &self.active_request_id,
            dispatch: &self.dispatch,
        };
        let mut collector = HelperSynthesisCollector::new(request_id, requested_voice_id);
        loop {
            let response = match connection.receive(self.config.synthesis_idle_timeout) {
                Ok(response) => response,
                Err(error) => return Err(self.synthesis_error(&connection, error)),
            };
            if response.request_id != Some(request_id) {
                if let Err(error) = response.validate() {
                    return Err(self.synthesis_error(&connection, error.into()));
                }
                match self.consume_cancel_response(&response) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => return Err(self.synthesis_error(&connection, error)),
                }
                return Err(self.synthesis_error(
                    &connection,
                    HelperEngineError::RequestMismatch {
                        expected: request_id,
                        received: response.request_id,
                    },
                ));
            }
            let progress = match collector.accept(response) {
                Ok(progress) => progress,
                Err(error) => return Err(self.synthesis_error(&connection, error)),
            };
            match progress {
                Some(HelperSynthesisResult::Completed(buffer)) => return Ok(buffer),
                Some(HelperSynthesisResult::Cancelled) => {
                    return Err(TtsError::SynthesisFailed(
                        "helper synthesis cancelled".to_owned(),
                    ));
                }
                None => {}
            }
        }
    }

    fn stop(&self) {
        let _dispatch = self.dispatch.lock().unwrap();
        let target_request_id = self.active_request_id.load(Ordering::Acquire);
        if target_request_id == 0 {
            return;
        }
        let Ok(connection) = self.current_connection() else {
            return;
        };
        let request_id = self.allocate_request_id();
        let request =
            HelperRequest::new(request_id, HelperRequestBody::Cancel { target_request_id });
        self.pending_cancellations
            .lock()
            .unwrap()
            .insert(request_id, target_request_id);
        if let Err(error) = connection.send(&request) {
            self.pending_cancellations
                .lock()
                .unwrap()
                .remove(&request_id);
            warn!("Could not cancel helper synthesis {target_request_id}: {error}");
            self.invalidate_connection(&connection);
        }
    }

    fn is_speaking(&self) -> bool {
        self.active_request_id.load(Ordering::Acquire) != 0
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.descriptor()
            .voices
            .into_iter()
            .map(|voice| VoiceInfo {
                identifier: voice.id.voice_id,
                name: voice.display_name,
                language: voice.language.unwrap_or_default(),
                quality: voice.quality,
            })
            .collect()
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|voice| voice.identifier == identifier)
    }
}

impl Drop for HelperTtsEngine {
    fn drop(&mut self) {
        if let Ok(connection) = self.connection.get_mut() {
            if let Some(connection) = connection.take() {
                connection.terminate();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicBool;
    use std::sync::Condvar;
    use std::time::Instant;

    use super::*;
    use crate::contracts::{
        AcssCapabilities, Availability, EngineCapabilities, EngineHealth, MarkerCapabilities,
        PhysicalVoiceId, VoiceDescriptor,
    };
    use crate::helper_protocol::{
        HelperMarkerKind, HelperPcmChunk, HelperResponseBody, HelperSampleFormat,
        HELPER_PROTOCOL_VERSION,
    };
    use crate::VoiceQuality;

    #[derive(Debug, Clone, Copy)]
    enum MockSynthesisMode {
        Complete,
        WaitForCancel,
        VoiceMissing,
    }

    struct MockConnection {
        descriptor: EngineDescriptor,
        mode: MockSynthesisMode,
        sent: Mutex<Vec<HelperRequest>>,
        responses: Mutex<VecDeque<HelperResponse>>,
        response_ready: Condvar,
        terminated: AtomicBool,
    }

    impl MockConnection {
        fn new(descriptor: EngineDescriptor, mode: MockSynthesisMode) -> Self {
            Self {
                descriptor,
                mode,
                sent: Mutex::new(Vec::new()),
                responses: Mutex::new(VecDeque::new()),
                response_ready: Condvar::new(),
                terminated: AtomicBool::new(false),
            }
        }

        fn push(&self, response: HelperResponse) {
            self.responses.lock().unwrap().push_back(response);
            self.response_ready.notify_all();
        }
    }

    impl HelperConnection for MockConnection {
        fn send(&self, request: &HelperRequest) -> Result<(), HelperEngineError> {
            request.validate()?;
            self.sent.lock().unwrap().push(request.clone());
            match &request.body {
                HelperRequestBody::Hello { .. } => self.push(response(
                    request.request_id,
                    HelperResponseBody::Hello {
                        selected_protocol_version: HELPER_PROTOCOL_VERSION,
                        helper_name: "mock x86 helper".to_owned(),
                        helper_version: "0.1.0".to_owned(),
                    },
                )),
                HelperRequestBody::Describe => self.push(response(
                    request.request_id,
                    HelperResponseBody::Descriptor {
                        descriptor: self.descriptor.clone(),
                    },
                )),
                HelperRequestBody::Synthesize { settings, .. } => match self.mode {
                    MockSynthesisMode::Complete => {
                        self.push(response(
                            request.request_id,
                            HelperResponseBody::SynthesisStarted {
                                format: HelperAudioFormat {
                                    sample_rate: 22_050,
                                    channels: 1,
                                    sample_format: HelperSampleFormat::PcmS16Le,
                                },
                                actual_voice_id: settings
                                    .voice_id
                                    .clone()
                                    .unwrap_or_else(|| "reed".to_owned()),
                            },
                        ));
                        self.push(audio(request.request_id, 0, &[-32_768, 0, 16_384, 32_767]));
                        self.push(response(
                            request.request_id,
                            HelperResponseBody::SynthesisCompleted { frame_count: 4 },
                        ));
                    }
                    MockSynthesisMode::WaitForCancel => self.push(started(request.request_id, 1)),
                    MockSynthesisMode::VoiceMissing => self.push(response(
                        request.request_id,
                        HelperResponseBody::Error {
                            code: HelperErrorCode::VoiceNotFound,
                            message: "requested voice is missing".to_owned(),
                            retryable: false,
                        },
                    )),
                },
                HelperRequestBody::Cancel { target_request_id } => {
                    self.push(response(
                        request.request_id,
                        HelperResponseBody::CancelAccepted {
                            target_request_id: *target_request_id,
                        },
                    ));
                    self.push(response(
                        *target_request_id,
                        HelperResponseBody::SynthesisCancelled,
                    ));
                }
                HelperRequestBody::Ping => {
                    self.push(response(request.request_id, HelperResponseBody::Pong));
                }
                HelperRequestBody::Shutdown => {
                    self.push(response(
                        request.request_id,
                        HelperResponseBody::ShuttingDown,
                    ));
                }
            }
            Ok(())
        }

        fn receive(&self, timeout: Duration) -> Result<HelperResponse, HelperEngineError> {
            let deadline = Instant::now() + timeout;
            let mut responses = self.responses.lock().unwrap();
            loop {
                if let Some(response) = responses.pop_front() {
                    return Ok(response);
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err(HelperEngineError::Timeout("mock response"));
                }
                let (waiting, result) = self
                    .response_ready
                    .wait_timeout(responses, deadline.saturating_duration_since(now))
                    .unwrap();
                responses = waiting;
                if result.timed_out() && responses.is_empty() {
                    return Err(HelperEngineError::Timeout("mock response"));
                }
            }
        }

        fn terminate(&self) {
            self.terminated.store(true, Ordering::Release);
            self.response_ready.notify_all();
        }
    }

    struct MockConnector {
        connections: Mutex<VecDeque<Arc<MockConnection>>>,
    }

    impl MockConnector {
        fn new(connections: Vec<Arc<MockConnection>>) -> Self {
            Self {
                connections: Mutex::new(connections.into()),
            }
        }
    }

    impl HelperConnector for MockConnector {
        fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
            self.connections
                .lock()
                .unwrap()
                .pop_front()
                .map(|connection| connection as Arc<dyn HelperConnection>)
                .ok_or_else(|| HelperEngineError::Transport("no mock helper available".to_owned()))
        }
    }

    fn helper_descriptor(engine_id: &str, version: &str) -> EngineDescriptor {
        EngineDescriptor {
            id: engine_id.to_owned(),
            display_name: "Mock Eloquence".to_owned(),
            version: Some(version.to_owned()),
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: EngineCapabilities {
                acss: AcssCapabilities {
                    rate: true,
                    average_pitch: true,
                    volume: true,
                    ..AcssCapabilities::default()
                },
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::SynthesisAndPlayback,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities {
                    word: true,
                    native_index: true,
                    ..MarkerCapabilities::default()
                },
                language_switching: false,
                native_extensions: Vec::new(),
            },
            voices: vec![VoiceDescriptor {
                id: PhysicalVoiceId::new(engine_id, "reed"),
                display_name: "Reed".to_owned(),
                language: Some("en-US".to_owned()),
                gender: None,
                quality: VoiceQuality::Compact,
                availability: Availability::Available,
            }],
            default_voice_id: Some("reed".to_owned()),
        }
    }

    fn mock_config(engine_id: &str) -> HelperEngineConfig {
        let mut config = HelperEngineConfig::new(engine_id, "unused-mock-helper");
        config.startup_timeout = Duration::from_secs(1);
        config.request_timeout = Duration::from_secs(1);
        config.synthesis_idle_timeout = Duration::from_secs(1);
        config
    }

    fn helper_settings() -> TtsSettings {
        TtsSettings {
            voice: "reed".to_owned(),
            ..TtsSettings::default()
        }
    }

    fn mock_engine(
        connections: Vec<Arc<MockConnection>>,
    ) -> Result<HelperTtsEngine, HelperEngineError> {
        HelperTtsEngine::with_connector(
            mock_config("eloquence"),
            Arc::new(MockConnector::new(connections)),
        )
    }

    fn response(request_id: u64, body: HelperResponseBody) -> HelperResponse {
        HelperResponse::for_request(request_id, body)
    }

    fn started(request_id: u64, channels: u16) -> HelperResponse {
        response(
            request_id,
            HelperResponseBody::SynthesisStarted {
                format: HelperAudioFormat {
                    sample_rate: 22_050,
                    channels,
                    sample_format: HelperSampleFormat::PcmS16Le,
                },
                actual_voice_id: "reed".to_owned(),
            },
        )
    }

    fn audio(request_id: u64, sequence: u32, samples: &[i16]) -> HelperResponse {
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        response(
            request_id,
            HelperResponseBody::AudioChunk {
                chunk: HelperPcmChunk::from_bytes(sequence, &bytes).unwrap(),
            },
        )
    }

    #[test]
    fn assembles_valid_pcm_and_markers_after_terminal_frame_count() {
        let mut collector = HelperSynthesisCollector::new(11, None);
        assert!(collector.accept(started(11, 1)).unwrap().is_none());
        assert!(collector
            .accept(audio(11, 0, &[-32_768, 0]))
            .unwrap()
            .is_none());
        assert!(collector
            .accept(response(
                11,
                HelperResponseBody::Markers {
                    markers: vec![HelperMarker {
                        kind: HelperMarkerKind::Word,
                        frame_offset: 1,
                        text_start: Some(0),
                        text_length: Some(5),
                        value: None,
                    }],
                },
            ))
            .unwrap()
            .is_none());
        assert!(collector
            .accept(audio(11, 1, &[16_384, 32_767]))
            .unwrap()
            .is_none());

        let result = collector
            .accept(response(
                11,
                HelperResponseBody::SynthesisCompleted { frame_count: 4 },
            ))
            .unwrap()
            .unwrap();
        let HelperSynthesisResult::Completed(buffer) = result else {
            panic!("expected completed synthesis");
        };
        assert_eq!(buffer.sample_rate, 22_050);
        assert_eq!(buffer.channels, 1);
        assert_eq!(buffer.samples.len(), 4);
        assert_eq!(buffer.samples[0], -1.0);
        assert!((buffer.samples[2] - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_wrong_request_and_out_of_order_chunks() {
        let mut collector = HelperSynthesisCollector::new(11, None);
        let error = collector.accept(started(12, 1)).unwrap_err();
        assert!(matches!(
            error,
            HelperEngineError::RequestMismatch {
                expected: 11,
                received: Some(12)
            }
        ));

        collector.accept(started(11, 1)).unwrap();
        let error = collector.accept(audio(11, 1, &[0])).unwrap_err();
        assert!(matches!(
            error,
            HelperEngineError::AudioSequenceMismatch {
                expected: 0,
                received: 1
            }
        ));
    }

    #[test]
    fn rejects_a_helper_that_synthesizes_with_the_wrong_voice() {
        let mut collector = HelperSynthesisCollector::new(13, Some("paul".to_owned()));
        let error = collector.accept(started(13, 1)).unwrap_err();
        assert!(matches!(
            error,
            HelperEngineError::ActualVoiceMismatch {
                expected,
                received
            } if expected == "paul" && received == "reed"
        ));
    }

    #[test]
    fn rejects_channel_misalignment_and_false_frame_counts() {
        let mut collector = HelperSynthesisCollector::new(7, None);
        collector.accept(started(7, 2)).unwrap();
        let error = collector.accept(audio(7, 0, &[1])).unwrap_err();
        assert!(matches!(error, HelperEngineError::AudioFrameAlignment));

        let mut collector = HelperSynthesisCollector::new(8, None);
        collector.accept(started(8, 1)).unwrap();
        collector.accept(audio(8, 0, &[1, 2])).unwrap();
        let error = collector
            .accept(response(
                8,
                HelperResponseBody::SynthesisCompleted { frame_count: 3 },
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            HelperEngineError::FrameCountMismatch {
                reported: 3,
                received: 2
            }
        ));
    }

    #[test]
    fn cancellation_and_remote_errors_are_terminal() {
        let mut cancelled = HelperSynthesisCollector::new(4, None);
        assert!(matches!(
            cancelled
                .accept(response(4, HelperResponseBody::SynthesisCancelled))
                .unwrap(),
            Some(HelperSynthesisResult::Cancelled)
        ));
        assert!(matches!(
            cancelled.accept(started(4, 1)),
            Err(HelperEngineError::UnexpectedResponse(
                "response after terminal result"
            ))
        ));

        let mut failed = HelperSynthesisCollector::new(5, None);
        let error = failed
            .accept(response(
                5,
                HelperResponseBody::Error {
                    code: HelperErrorCode::VoiceNotFound,
                    message: "Paul is unavailable".to_owned(),
                    retryable: false,
                },
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            HelperEngineError::Remote {
                code: HelperErrorCode::VoiceNotFound,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unowned_and_wrong_version_responses_before_assembly() {
        let mut collector = HelperSynthesisCollector::new(9, None);
        let error = collector
            .accept(HelperResponse {
                protocol_version: HELPER_PROTOCOL_VERSION + 1,
                request_id: Some(9),
                body: HelperResponseBody::SynthesisCancelled,
            })
            .unwrap_err();
        assert!(matches!(error, HelperEngineError::Protocol(_)));
    }

    #[test]
    fn helper_engine_negotiates_inventory_and_returns_validated_pcm() {
        let connection = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::Complete,
        ));
        let engine = mock_engine(vec![Arc::clone(&connection)]).unwrap();

        assert_eq!(engine.descriptor().id, "eloquence");
        assert_eq!(engine.available_voices()[0].identifier, "reed");
        let buffer = engine
            .synthesize(
                "hello",
                &TtsSettings {
                    voice: "reed".to_owned(),
                    rate: 0.6,
                    pitch: 1.1,
                    volume: 0.8,
                },
            )
            .unwrap();
        assert_eq!(buffer.sample_rate, 22_050);
        assert_eq!(buffer.channels, 1);
        assert_eq!(buffer.samples.len(), 4);
        assert!(!engine.is_speaking());

        let sent = connection.sent.lock().unwrap();
        assert!(matches!(sent[0].body, HelperRequestBody::Hello { .. }));
        assert_eq!(sent[1].body, HelperRequestBody::Describe);
        let HelperRequestBody::Synthesize { settings, .. } = &sent[2].body else {
            panic!("expected synthesis request");
        };
        assert_eq!(settings.voice_id.as_deref(), Some("reed"));
    }

    #[test]
    fn helper_engine_maps_physical_voice_failures() {
        let connection = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::VoiceMissing,
        ));
        let engine = mock_engine(vec![connection]).unwrap();

        let error = engine.synthesize("hello", &helper_settings()).unwrap_err();
        assert!(matches!(
            error,
            TtsError::VoiceNotFound(message) if message == "requested voice is missing"
        ));
    }

    #[test]
    fn stop_cancels_an_active_helper_request_without_waiting_for_synthesis() {
        let connection = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::WaitForCancel,
        ));
        let engine = Arc::new(mock_engine(vec![Arc::clone(&connection)]).unwrap());
        let worker_engine = Arc::clone(&engine);
        let synthesis = std::thread::spawn(move || {
            worker_engine.synthesize("long utterance", &helper_settings())
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while !engine.is_speaking() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(engine.is_speaking());
        engine.stop();

        let error = synthesis.join().unwrap().unwrap_err();
        assert!(matches!(
            error,
            TtsError::SynthesisFailed(message) if message.contains("cancelled")
        ));
        assert!(!engine.is_speaking());
        assert!(connection
            .sent
            .lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request.body, HelperRequestBody::Cancel { .. })));
    }

    #[test]
    fn recovery_probe_replaces_and_renegotiates_the_helper() {
        let failed = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::VoiceMissing,
        ));
        let recovered = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.1"),
            MockSynthesisMode::Complete,
        ));
        let engine = mock_engine(vec![Arc::clone(&failed), recovered]).unwrap();

        engine.prepare_recovery_probe().unwrap();

        assert!(failed.terminated.load(Ordering::Acquire));
        assert_eq!(engine.descriptor().version.as_deref(), Some("1.1"));
        assert!(engine.synthesize("recovered", &helper_settings()).is_ok());
    }

    #[test]
    fn synthesis_timeout_invalidates_the_child_until_recovery_restarts_it() {
        let hung = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::WaitForCancel,
        ));
        let recovered = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.1"),
            MockSynthesisMode::Complete,
        ));
        let mut config = mock_config("eloquence");
        config.synthesis_idle_timeout = Duration::from_millis(10);
        let engine = HelperTtsEngine::with_connector(
            config,
            Arc::new(MockConnector::new(vec![Arc::clone(&hung), recovered])),
        )
        .unwrap();

        let error = engine.synthesize("hung", &helper_settings()).unwrap_err();
        assert!(matches!(
            error,
            TtsError::SynthesisFailed(message) if message.contains("timed out")
        ));
        assert!(hung.terminated.load(Ordering::Acquire));

        engine.prepare_recovery_probe().unwrap();
        assert!(engine.synthesize("recovered", &helper_settings()).is_ok());
    }

    #[test]
    fn negotiation_rejects_a_helper_for_the_wrong_engine() {
        let connection = Arc::new(MockConnection::new(
            helper_descriptor("dectalk", "1.0"),
            MockSynthesisMode::Complete,
        ));
        let result = mock_engine(vec![Arc::clone(&connection)]);
        let Err(error) = result else {
            panic!("wrong helper engine ID was accepted");
        };
        assert!(matches!(
            error,
            HelperEngineError::EngineIdMismatch {
                expected,
                received
            } if expected == "eloquence" && received == "dectalk"
        ));
        assert!(connection.terminated.load(Ordering::Acquire));
    }
}
