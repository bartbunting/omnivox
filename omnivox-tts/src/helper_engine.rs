//! Host-side support for TTS engines backed by helper processes.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use thiserror::Error;
use tracing::{info, warn};

use crate::contracts::{
    AudioOutputMode, CancellationSupport, ConcurrencyModel, EngineDescriptor, PhysicalVoiceId,
};
use crate::engine_registry::validate_descriptor;
use crate::helper_protocol::{
    read_frame, write_frame, HelperAudioFormat, HelperErrorCode, HelperMarker, HelperMarkerKind,
    HelperRequest, HelperRequestBody, HelperResponse, HelperResponseBody, HelperSynthesisSettings,
    HELPER_PROTOCOL_V2, HELPER_PROTOCOL_VERSION, MAX_HELPER_MARKERS, MAX_HELPER_SYNTHESIS_BYTES,
    SUPPORTED_HELPER_PROTOCOL_VERSIONS,
};
use crate::{
    AnchorResolution, ResolvedAnchor, SynthesisMarker, SynthesisMarkerKind, SynthesisRequest,
    SynthesisResult, TtsEngine, TtsError, VoiceInfo,
};

const HELPER_CANCEL_GRACE: Duration = Duration::from_millis(250);

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
    engine_id: String,
    program: PathBuf,
    arguments: Vec<OsString>,
}

impl ProcessHelperConnector {
    fn new(config: &HelperEngineConfig) -> Self {
        Self {
            engine_id: config.engine_id.clone(),
            program: config.program.clone(),
            arguments: config.arguments.clone(),
        }
    }
}

impl HelperConnector for ProcessHelperConnector {
    fn connect(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        Ok(Arc::new(ProcessHelperConnection::spawn(
            &self.engine_id,
            &self.program,
            &self.arguments,
        )?))
    }
}

type HelperReadResult = Result<HelperResponse, HelperEngineError>;

struct ProcessHelperConnection {
    engine_id: String,
    child_id: u32,
    writer: Mutex<Option<BufWriter<ChildStdin>>>,
    responses: Mutex<mpsc::Receiver<HelperReadResult>>,
    child: Mutex<Child>,
    reader_handle: Mutex<Option<JoinHandle<()>>>,
}

impl ProcessHelperConnection {
    fn spawn(
        engine_id: &str,
        program: &Path,
        arguments: &[OsString],
    ) -> Result<Self, HelperEngineError> {
        info!(
            engine_id,
            program = %program.display(),
            "Starting TTS helper process"
        );
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
        let child_id = child.id();
        info!(engine_id, child_id, "TTS helper process started");
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HelperEngineError::Transport("helper stdin was not piped".to_owned()))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            HelperEngineError::Transport("helper stdout was not piped".to_owned())
        })?;
        let (response_sender, response_receiver) = mpsc::channel();
        let reader_engine_id = engine_id.to_owned();
        let reader_handle = std::thread::Builder::new()
            .name("omnivox-helper-reader".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let result = match read_frame(&mut reader) {
                        Ok(Some(response)) => Ok(response),
                        Ok(None) => {
                            info!(
                                engine_id = reader_engine_id,
                                child_id,
                                "TTS helper stdout closed"
                            );
                            Err(HelperEngineError::Exited)
                        }
                        Err(error) => {
                            warn!(
                                engine_id = reader_engine_id,
                                child_id,
                                %error,
                                "Could not read TTS helper response"
                            );
                            Err(error.into())
                        }
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
            engine_id: engine_id.to_owned(),
            child_id,
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
        let exit_status = {
            let mut child = self.child.lock().unwrap();
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            child.wait().ok()
        };
        info!(
            engine_id = self.engine_id,
            child_id = self.child_id,
            status = ?exit_status,
            "TTS helper process reaped"
        );
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
    Completed(HelperCompletedSynthesis),
    Cancelled,
}

#[derive(Debug)]
pub(crate) struct HelperCompletedSynthesis {
    samples: Vec<i16>,
    sample_rate: u32,
    channels: u16,
    actual_voice_id: String,
    markers: Vec<HelperMarker>,
}

fn split_helper_markers(
    markers: Vec<HelperMarker>,
) -> (Vec<SynthesisMarker>, Vec<ResolvedAnchor>) {
    let mut synthesis_markers = Vec::new();
    let mut anchors = Vec::new();
    for marker in markers {
        let kind = match marker.kind {
            HelperMarkerKind::Word => SynthesisMarkerKind::Word,
            HelperMarkerKind::Sentence => SynthesisMarkerKind::Sentence,
            HelperMarkerKind::Phoneme => SynthesisMarkerKind::Phoneme,
            HelperMarkerKind::NativeIndex => SynthesisMarkerKind::NativeIndex,
            HelperMarkerKind::RequestedAnchor => {
                anchors.push(ResolvedAnchor {
                    id: marker
                        .value
                        .expect("validated requested-anchor marker has an ID"),
                    frame_offset: Some(marker.frame_offset),
                    resolution: AnchorResolution::Exact,
                });
                continue;
            }
        };
        synthesis_markers.push(SynthesisMarker {
            kind,
            frame_offset: marker.frame_offset,
            text_start: marker.text_start,
            text_length: marker.text_length,
            value: marker.value,
        });
    }
    (synthesis_markers, anchors)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SynthesisPhase {
    AwaitingStart,
    Streaming,
    Terminal,
}

pub(crate) struct HelperSynthesisCollector {
    request_id: u64,
    protocol_version: u16,
    expected_voice_id: Option<String>,
    phase: SynthesisPhase,
    format: Option<HelperAudioFormat>,
    actual_voice_id: Option<String>,
    next_sequence: u32,
    samples: Vec<i16>,
    markers: Vec<HelperMarker>,
}

impl HelperSynthesisCollector {
    pub(crate) fn new(
        protocol_version: u16,
        request_id: u64,
        expected_voice_id: Option<String>,
    ) -> Self {
        Self {
            request_id,
            protocol_version,
            expected_voice_id,
            phase: SynthesisPhase::AwaitingStart,
            format: None,
            actual_voice_id: None,
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
        if response.protocol_version != self.protocol_version {
            return Err(HelperEngineError::UnexpectedResponse(
                "response uses a different negotiated protocol version",
            ));
        }
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
                self.actual_voice_id = Some(actual_voice_id);
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
                    HelperCompletedSynthesis {
                        samples: std::mem::take(&mut self.samples),
                        sample_rate: format.sample_rate,
                        channels: format.channels,
                        actual_voice_id: self
                            .actual_voice_id
                            .take()
                            .expect("streaming synthesis has an actual voice"),
                        markers: std::mem::take(&mut self.markers),
                    },
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
    protocol_version: AtomicU64,
    next_request_id: AtomicU64,
    active_request_id: Arc<AtomicU64>,
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
            protocol_version: AtomicU64::new(0),
            next_request_id: AtomicU64::new(1),
            active_request_id: Arc::new(AtomicU64::new(0)),
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
        info!(
            engine_id = self.config.engine_id,
            "Installing fresh TTS helper connection"
        );
        if let Some(connection) = self.connection.write().unwrap().take() {
            connection.terminate();
        }
        self.pending_cancellations.lock().unwrap().clear();

        let connection = self.connector.connect()?;
        let (descriptor, protocol_version) = match self.negotiate(&connection) {
            Ok(negotiated) => negotiated,
            Err(error) => {
                connection.terminate();
                return Err(error);
            }
        };
        *self.descriptor.write().unwrap() = Some(descriptor);
        self.protocol_version
            .store(u64::from(protocol_version), Ordering::Release);
        *self.connection.write().unwrap() = Some(connection);
        info!(
            engine_id = self.config.engine_id,
            protocol_version,
            "TTS helper connection ready"
        );
        Ok(())
    }

    fn negotiate(
        &self,
        connection: &Arc<dyn HelperConnection>,
    ) -> Result<(EngineDescriptor, u16), HelperEngineError> {
        let mut negotiated_response = None;
        for (index, protocol_version) in SUPPORTED_HELPER_PROTOCOL_VERSIONS.iter().enumerate() {
            let hello_id = self.allocate_request_id();
            connection.send(&HelperRequest::with_version(
                *protocol_version,
                hello_id,
                HelperRequestBody::Hello {
                    supported_protocol_versions: SUPPORTED_HELPER_PROTOCOL_VERSIONS[index..]
                        .to_vec(),
                },
            ))?;
            let response =
                receive_owned_response(connection, hello_id, self.config.startup_timeout)?;
            let try_older = matches!(
                response.body,
                HelperResponseBody::Error {
                    code: HelperErrorCode::UnsupportedVersion,
                    ..
                }
            ) && index + 1 < SUPPORTED_HELPER_PROTOCOL_VERSIONS.len();
            if try_older {
                continue;
            }
            negotiated_response = Some(response);
            break;
        }
        let response = negotiated_response.expect("helper protocol version list is nonempty");
        let response_version = response.protocol_version;
        let selected_protocol_version = match response.body {
            HelperResponseBody::Hello {
                selected_protocol_version,
                ..
            } if SUPPORTED_HELPER_PROTOCOL_VERSIONS.contains(&selected_protocol_version)
                && selected_protocol_version == response_version => selected_protocol_version,
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
        };

        let describe_id = self.allocate_request_id();
        connection.send(&HelperRequest::with_version(
            selected_protocol_version,
            describe_id,
            HelperRequestBody::Describe,
        ))?;
        let response =
            receive_owned_response(connection, describe_id, self.config.request_timeout)?;
        if response.protocol_version != selected_protocol_version {
            return Err(HelperEngineError::UnexpectedResponse(
                "descriptor uses a different negotiated protocol version",
            ));
        }
        let mut descriptor = match response.body {
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
        descriptor.capabilities.post_synthesis_dimensions =
            crate::contracts::buffered_post_synthesis_dimensions();
        self.validate_descriptor(&descriptor)?;
        Ok((descriptor, selected_protocol_version))
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

    fn connection_for_synthesis(&self) -> Result<Arc<dyn HelperConnection>, HelperEngineError> {
        match self.current_connection() {
            Ok(connection) => Ok(connection),
            Err(HelperEngineError::Exited) => {
                info!(
                    engine_id = self.config.engine_id,
                    "Restarting invalidated TTS helper before synthesis"
                );
                self.install_fresh_connection()?;
                self.current_connection()
            }
            Err(error) => Err(error),
        }
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
        request_id: u64,
        error: HelperEngineError,
    ) -> TtsError {
        warn!(
            engine_id = self.config.engine_id,
            request_id,
            %error,
            "TTS helper synthesis failed"
        );
        let invalidates_connection = match &error {
            HelperEngineError::Remote {
                code: HelperErrorCode::SynthesisFailed,
                retryable: true,
                ..
            } => true,
            HelperEngineError::Remote { .. } => false,
            _ => true,
        };
        if invalidates_connection {
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
        info!(
            engine_id = self.config.engine_id,
            "Preparing TTS helper recovery probe"
        );
        let _lifecycle = self.lifecycle.lock().unwrap();
        self.install_fresh_connection()
            .map_err(|error| {
                warn!(
                    engine_id = self.config.engine_id,
                    %error,
                    "TTS helper recovery preparation failed"
                );
                Self::map_error(error)
            })
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let _lifecycle = self.lifecycle.lock().unwrap();
        let connection = self.connection_for_synthesis().map_err(Self::map_error)?;
        let descriptor = self.descriptor();
        let protocol_version = self.protocol_version.load(Ordering::Acquire) as u16;
        let request_id = self.allocate_request_id();
        let started_at = Instant::now();
        let voice_id = request.voice_id_for_engine(&descriptor.id)?;
        let requested_voice_id = if voice_id.is_empty() {
            None
        } else {
            Some(voice_id.to_owned())
        };
        let acss = &request.normalized_acss;
        let supported_acss = &descriptor.capabilities.acss;
        let extended_acss = protocol_version >= HELPER_PROTOCOL_VERSION;
        let helper_request = HelperRequest::with_version(
            protocol_version,
            request_id,
            HelperRequestBody::Synthesize {
                text: request.text.clone(),
                settings: HelperSynthesisSettings {
                    voice_id: requested_voice_id.clone(),
                    rate: request.settings.rate,
                    pitch: request.settings.pitch,
                    volume: request.settings.volume,
                    pitch_range: (extended_acss && supported_acss.pitch_range)
                        .then_some(acss.pitch_range)
                        .flatten(),
                    stress: (extended_acss && supported_acss.stress)
                        .then_some(acss.stress)
                        .flatten(),
                    richness: (extended_acss && supported_acss.richness)
                        .then_some(acss.richness)
                        .flatten(),
                },
                anchors: (protocol_version >= HELPER_PROTOCOL_V2)
                    .then(|| request.anchors.clone()),
            },
        );
        helper_request.validate().map_err(|error| {
            TtsError::InvalidParameter(format!("invalid helper synthesis request: {error}"))
        })?;
        info!(
            engine_id = self.config.engine_id,
            request_id,
            voice_id,
            text_bytes = request.text.len(),
            anchors = request.anchors.len(),
            "Sending TTS helper synthesis request"
        );

        {
            let _dispatch = self.dispatch.lock().unwrap();
            if let Err(error) = connection.send(&helper_request) {
                return Err(self.synthesis_error(&connection, request_id, error));
            }
            self.active_request_id.store(request_id, Ordering::Release);
        }
        let _active = ActiveRequestGuard {
            request_id,
            active_request_id: &self.active_request_id,
            dispatch: &self.dispatch,
        };
        let mut collector =
            HelperSynthesisCollector::new(protocol_version, request_id, requested_voice_id);
        loop {
            let response = match connection.receive(self.config.synthesis_idle_timeout) {
                Ok(response) => response,
                Err(error) => {
                    return Err(self.synthesis_error(&connection, request_id, error));
                }
            };
            if response.protocol_version != protocol_version {
                return Err(self.synthesis_error(
                    &connection,
                    request_id,
                    HelperEngineError::UnexpectedResponse(
                        "response uses a different negotiated protocol version",
                    ),
                ));
            }
            if response.request_id != Some(request_id) {
                if let Err(error) = response.validate() {
                    return Err(self.synthesis_error(&connection, request_id, error.into()));
                }
                match self.consume_cancel_response(&response) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => {
                        return Err(self.synthesis_error(&connection, request_id, error));
                    }
                }
                return Err(self.synthesis_error(
                    &connection,
                    request_id,
                    HelperEngineError::RequestMismatch {
                        expected: request_id,
                        received: response.request_id,
                    },
                ));
            }
            let progress = match collector.accept(response) {
                Ok(progress) => progress,
                Err(error) => {
                    return Err(self.synthesis_error(&connection, request_id, error));
                }
            };
            match progress {
                Some(HelperSynthesisResult::Completed(completed)) => {
                    let actual_voice =
                        PhysicalVoiceId::new(descriptor.id.clone(), completed.actual_voice_id);
                    let (markers, anchors) = split_helper_markers(completed.markers);
                    let mut result = SynthesisResult::from_native_i16(
                        descriptor.id.clone(),
                        Some(actual_voice),
                        &completed.samples,
                        completed.sample_rate,
                        completed.channels,
                        markers,
                        anchors,
                    )?;
                    result.resolve_anchors(
                        request,
                        descriptor.capabilities.markers.requested_anchors,
                    );
                    result.validate(request)?;
                    info!(
                        engine_id = self.config.engine_id,
                        request_id,
                        frames = result.audio.frame_count(),
                        markers = result.markers.len(),
                        elapsed_ms = started_at.elapsed().as_millis(),
                        "TTS helper synthesis completed"
                    );
                    return Ok(result);
                }
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
        if self
            .pending_cancellations
            .lock()
            .unwrap()
            .values()
            .any(|pending| *pending == target_request_id)
        {
            return;
        }
        let request_id = self.allocate_request_id();
        let request = HelperRequest::with_version(
            self.protocol_version.load(Ordering::Acquire) as u16,
            request_id,
            HelperRequestBody::Cancel { target_request_id },
        );
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
            return;
        }

        let active_request_id = Arc::clone(&self.active_request_id);
        let watchdog_connection = Arc::clone(&connection);
        let engine_id = self.config.engine_id.clone();
        let watchdog = thread::Builder::new()
            .name(format!("omnivox-{engine_id}-cancel-watchdog"))
            .spawn(move || {
                thread::sleep(HELPER_CANCEL_GRACE);
                if active_request_id.load(Ordering::Acquire) == target_request_id {
                    warn!(
                        engine_id,
                        target_request_id,
                        grace_ms = HELPER_CANCEL_GRACE.as_millis(),
                        "TTS helper did not finish cancellation; terminating it"
                    );
                    watchdog_connection.terminate();
                }
            });
        if let Err(error) = watchdog {
            warn!(
                engine_id = self.config.engine_id,
                target_request_id,
                %error,
                "Could not start TTS helper cancellation watchdog; terminating it"
            );
            connection.terminate();
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
        AcssCapabilities, AnchorSupport, Availability, EngineCapabilities, EngineHealth,
        MarkerCapabilities, NormalizedAcss, PhysicalVoiceId, VoiceDescriptor,
    };
    use crate::helper_protocol::{
        HelperMarkerKind, HelperPcmChunk, HelperResponseBody, HelperSampleFormat,
        HELPER_PROTOCOL_V1, HELPER_PROTOCOL_VERSION,
    };
    use crate::{AnchorAffinity, RequestedAnchor, TtsSettings, VoiceQuality};

    #[derive(Debug, Clone, Copy)]
    enum MockSynthesisMode {
        Complete,
        WaitForCancel,
        IgnoreCancel,
        VoiceMissing,
        RetryableSynthesisFailure,
    }

    struct MockConnection {
        descriptor: EngineDescriptor,
        protocol_version: u16,
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
                protocol_version: HELPER_PROTOCOL_VERSION,
                mode,
                sent: Mutex::new(Vec::new()),
                responses: Mutex::new(VecDeque::new()),
                response_ready: Condvar::new(),
                terminated: AtomicBool::new(false),
            }
        }

        fn legacy(descriptor: EngineDescriptor, mode: MockSynthesisMode) -> Self {
            let mut connection = Self::new(descriptor, mode);
            connection.protocol_version = HELPER_PROTOCOL_V1;
            connection
        }

        fn push(&self, response: HelperResponse) {
            self.responses.lock().unwrap().push_back(response);
            self.response_ready.notify_all();
        }


        fn response(&self, request_id: u64, body: HelperResponseBody) -> HelperResponse {
            HelperResponse::for_request_version(self.protocol_version, request_id, body)
        }
    }

    impl HelperConnection for MockConnection {
        fn send(&self, request: &HelperRequest) -> Result<(), HelperEngineError> {
            request.validate()?;
            self.sent.lock().unwrap().push(request.clone());
            if request.protocol_version != self.protocol_version {
                if matches!(request.body, HelperRequestBody::Hello { .. }) {
                    self.push(self.response(
                        request.request_id,
                        HelperResponseBody::Error {
                            code: HelperErrorCode::UnsupportedVersion,
                            message: format!(
                                "mock helper supports protocol v{}",
                                self.protocol_version
                            ),
                            retryable: false,
                        },
                    ));
                    return Ok(());
                }
                return Err(HelperEngineError::UnexpectedResponse(
                    "mock request uses a different protocol version",
                ));
            }
            match &request.body {
                HelperRequestBody::Hello { .. } => self.push(self.response(
                    request.request_id,
                    HelperResponseBody::Hello {
                        selected_protocol_version: self.protocol_version,
                        helper_name: "mock x86 helper".to_owned(),
                        helper_version: "0.1.0".to_owned(),
                    },
                )),
                HelperRequestBody::Describe => {
                    let mut descriptor = self.descriptor.clone();
                    if self.protocol_version == HELPER_PROTOCOL_V1 {
                        descriptor.capabilities.markers.requested_anchors = AnchorSupport::None;
                    }
                    self.push(self.response(
                        request.request_id,
                        HelperResponseBody::Descriptor { descriptor },
                    ));
                }
                HelperRequestBody::Synthesize {
                    settings,
                    anchors,
                    ..
                } => match self.mode {
                    MockSynthesisMode::Complete => {
                        self.push(self.response(
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
                        let bytes = [-32_768_i16, 0, 16_384, 32_767]
                            .into_iter()
                            .flat_map(i16::to_le_bytes)
                            .collect::<Vec<_>>();
                        self.push(self.response(
                            request.request_id,
                            HelperResponseBody::AudioChunk {
                                chunk: HelperPcmChunk::from_bytes(0, &bytes).unwrap(),
                            },
                        ));
                        let mut markers = vec![HelperMarker {
                            kind: HelperMarkerKind::Word,
                            frame_offset: 1,
                            text_start: Some(0),
                            text_length: Some(5),
                            value: None,
                        }];
                        if let Some(anchors) = anchors {
                            markers.extend(anchors.iter().map(|anchor| HelperMarker {
                                kind: HelperMarkerKind::RequestedAnchor,
                                frame_offset: u64::from(anchor.text_offset).min(4),
                                text_start: Some(anchor.text_offset),
                                text_length: Some(0),
                                value: Some(anchor.id.clone()),
                            }));
                        }
                        self.push(self.response(
                            request.request_id,
                            HelperResponseBody::Markers {
                                markers,
                            },
                        ));
                        self.push(self.response(
                            request.request_id,
                            HelperResponseBody::SynthesisCompleted { frame_count: 4 },
                        ));
                    }
                    MockSynthesisMode::WaitForCancel | MockSynthesisMode::IgnoreCancel => {
                        self.push(HelperResponse::for_request_version(
                            self.protocol_version,
                            request.request_id,
                            HelperResponseBody::SynthesisStarted {
                                format: HelperAudioFormat {
                                    sample_rate: 22_050,
                                    channels: 1,
                                    sample_format: HelperSampleFormat::PcmS16Le,
                                },
                                actual_voice_id: "reed".to_owned(),
                            },
                        ))
                    }
                    MockSynthesisMode::VoiceMissing => self.push(self.response(
                        request.request_id,
                        HelperResponseBody::Error {
                            code: HelperErrorCode::VoiceNotFound,
                            message: "requested voice is missing".to_owned(),
                            retryable: false,
                        },
                    )),
                    MockSynthesisMode::RetryableSynthesisFailure => self.push(self.response(
                        request.request_id,
                        HelperResponseBody::Error {
                            code: HelperErrorCode::SynthesisFailed,
                            message: "native synchronization failed".to_owned(),
                            retryable: true,
                        },
                    )),
                },
                HelperRequestBody::Cancel { target_request_id } => {
                    if matches!(self.mode, MockSynthesisMode::IgnoreCancel) {
                        return Ok(());
                    }
                    self.push(self.response(
                        request.request_id,
                        HelperResponseBody::CancelAccepted {
                            target_request_id: *target_request_id,
                        },
                    ));
                    self.push(self.response(
                        *target_request_id,
                        HelperResponseBody::SynthesisCancelled,
                    ));
                }
                HelperRequestBody::Ping => {
                    self.push(self.response(request.request_id, HelperResponseBody::Pong));
                }
                HelperRequestBody::Shutdown => {
                    self.push(self.response(
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
                if self.terminated.load(Ordering::Acquire) {
                    return Err(HelperEngineError::Exited);
                }
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
                    pitch_range: true,
                    stress: true,
                    richness: true,
                    volume: true,
                    ..AcssCapabilities::default()
                },
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::SynthesisAndPlayback,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities {
                    word: true,
                    native_index: true,
                    requested_anchors: AnchorSupport::Exact,
                    ..MarkerCapabilities::default()
                },
                language_switching: false,
                post_synthesis_dimensions: Vec::new(),
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

    fn synthesis_request(text: &str) -> SynthesisRequest {
        SynthesisRequest::new(text, helper_settings())
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
        let mut collector = HelperSynthesisCollector::new(HELPER_PROTOCOL_VERSION, 11, None);
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
        let HelperSynthesisResult::Completed(completed) = result else {
            panic!("expected completed synthesis");
        };
        assert_eq!(completed.sample_rate, 22_050);
        assert_eq!(completed.channels, 1);
        assert_eq!(completed.samples.len(), 4);
        assert_eq!(completed.actual_voice_id, "reed");
        assert_eq!(completed.markers.len(), 1);
        assert_eq!(completed.samples[0], -32_768);
        assert_eq!(completed.samples[2], 16_384);
    }

    #[test]
    fn rejects_wrong_request_and_out_of_order_chunks() {
        let mut collector = HelperSynthesisCollector::new(HELPER_PROTOCOL_VERSION, 11, None);
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
        let mut collector = HelperSynthesisCollector::new(
            HELPER_PROTOCOL_VERSION,
            13,
            Some("paul".to_owned()),
        );
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
        let mut collector = HelperSynthesisCollector::new(HELPER_PROTOCOL_VERSION, 7, None);
        collector.accept(started(7, 2)).unwrap();
        let error = collector.accept(audio(7, 0, &[1])).unwrap_err();
        assert!(matches!(error, HelperEngineError::AudioFrameAlignment));

        let mut collector = HelperSynthesisCollector::new(HELPER_PROTOCOL_VERSION, 8, None);
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
        let mut cancelled =
            HelperSynthesisCollector::new(HELPER_PROTOCOL_VERSION, 4, None);
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

        let mut failed = HelperSynthesisCollector::new(HELPER_PROTOCOL_VERSION, 5, None);
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
        let mut collector = HelperSynthesisCollector::new(HELPER_PROTOCOL_VERSION, 9, None);
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
        let result = engine
            .synthesize(
                &SynthesisRequest::new(
                    "hello",
                    TtsSettings {
                        voice: "reed".to_owned(),
                        rate: 0.6,
                        pitch: 1.1,
                        volume: 0.8,
                    },
                )
                .with_normalized_acss(NormalizedAcss {
                    pitch_range: Some(0.2),
                    stress: Some(0.6),
                    richness: Some(0.8),
                    ..NormalizedAcss::default()
                })
                .with_route("logical-reed", PhysicalVoiceId::new("eloquence", "reed")),
            )
            .unwrap();
        assert_eq!(result.audio.sample_rate(), crate::STANDARD_SAMPLE_RATE);
        assert_eq!(result.audio.channels(), crate::STANDARD_CHANNELS);
        assert_eq!(result.audio.samples.len() % 2, 0);
        assert!(!result.audio.samples.is_empty());
        assert_eq!(
            result.actual_voice,
            Some(PhysicalVoiceId::new("eloquence", "reed"))
        );
        assert_eq!(result.markers.len(), 1);
        assert_eq!(result.markers[0].kind, SynthesisMarkerKind::Word);
        assert!(!engine.is_speaking());

        let sent = connection.sent.lock().unwrap();
        assert!(matches!(sent[0].body, HelperRequestBody::Hello { .. }));
        assert_eq!(sent[1].body, HelperRequestBody::Describe);
        let HelperRequestBody::Synthesize { settings, .. } = &sent[2].body else {
            panic!("expected synthesis request");
        };
        assert_eq!(settings.voice_id.as_deref(), Some("reed"));
        assert_eq!(settings.pitch_range, Some(0.2));
        assert_eq!(settings.stress, Some(0.6));
        assert_eq!(settings.richness, Some(0.8));
    }

    #[test]
    fn helper_v3_returns_exact_requested_anchors() {
        let connection = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::Complete,
        ));
        let engine = mock_engine(vec![Arc::clone(&connection)]).unwrap();
        let request = synthesis_request("hello")
            .with_anchors(vec![RequestedAnchor::new(
                "capital-1",
                3,
                AnchorAffinity::Before,
            )])
            .unwrap();

        let result = engine.synthesize(&request).unwrap();

        assert_eq!(
            result.anchors,
            vec![ResolvedAnchor {
                id: "capital-1".to_owned(),
                frame_offset: Some(6),
                resolution: AnchorResolution::Exact,
            }]
        );
        let sent = connection.sent.lock().unwrap();
        let HelperRequestBody::Synthesize { anchors, .. } = &sent[2].body else {
            panic!("expected synthesis request");
        };
        assert_eq!(anchors.as_deref(), Some(request.anchors.as_slice()));
    }

    #[test]
    fn helper_host_retries_negotiation_with_protocol_v1() {
        let connection = Arc::new(MockConnection::legacy(
            helper_descriptor("eloquence", "legacy"),
            MockSynthesisMode::Complete,
        ));
        let engine = mock_engine(vec![Arc::clone(&connection)]).unwrap();
        let request = synthesis_request("hello")
            .with_anchors(vec![RequestedAnchor::new(
                "legacy-anchor",
                2,
                AnchorAffinity::After,
            )])
            .unwrap();

        let result = engine.synthesize(&request).unwrap();

        assert_eq!(
            result.anchors,
            vec![ResolvedAnchor {
                id: "legacy-anchor".to_owned(),
                frame_offset: None,
                resolution: AnchorResolution::Omitted,
            }]
        );
        assert_eq!(
            engine.descriptor().capabilities.markers.requested_anchors,
            AnchorSupport::None
        );
        let sent = connection.sent.lock().unwrap();
        assert_eq!(sent[0].protocol_version, HELPER_PROTOCOL_VERSION);
        assert_eq!(sent[1].protocol_version, HELPER_PROTOCOL_V2);
        assert_eq!(sent[2].protocol_version, HELPER_PROTOCOL_V1);
        assert_eq!(sent[3].body, HelperRequestBody::Describe);
        let HelperRequestBody::Synthesize { anchors, settings, .. } = &sent[4].body else {
            panic!("expected synthesis request");
        };
        assert!(anchors.is_none());
        assert!(settings.pitch_range.is_none());
        assert!(settings.stress.is_none());
        assert!(settings.richness.is_none());
    }

    #[test]
    fn helper_engine_maps_physical_voice_failures() {
        let connection = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::VoiceMissing,
        ));
        let engine = mock_engine(vec![Arc::clone(&connection)]).unwrap();

        let error = engine
            .synthesize(&synthesis_request("hello"))
            .unwrap_err();
        assert!(matches!(
            error,
            TtsError::VoiceNotFound(message) if message == "requested voice is missing"
        ));
        assert!(!connection.terminated.load(Ordering::Acquire));
    }

    #[test]
    fn retryable_native_failure_retires_the_helper_before_next_synthesis() {
        let failed = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::RetryableSynthesisFailure,
        ));
        let recovered = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.1"),
            MockSynthesisMode::Complete,
        ));
        let engine = mock_engine(vec![Arc::clone(&failed), Arc::clone(&recovered)]).unwrap();

        let error = engine
            .synthesize(&synthesis_request("failing utterance"))
            .unwrap_err();
        assert!(matches!(
            error,
            TtsError::SynthesisFailed(message) if message.contains("native synchronization failed")
        ));
        assert!(failed.terminated.load(Ordering::Acquire));

        assert!(engine
            .synthesize(&synthesis_request("next utterance"))
            .is_ok());
        assert!(!recovered.terminated.load(Ordering::Acquire));
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
            worker_engine.synthesize(&synthesis_request("long utterance"))
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
    fn stop_restarts_a_helper_that_ignores_cancellation_before_next_synthesis() {
        let connection = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.0"),
            MockSynthesisMode::IgnoreCancel,
        ));
        let recovered = Arc::new(MockConnection::new(
            helper_descriptor("eloquence", "1.1"),
            MockSynthesisMode::Complete,
        ));
        let engine = Arc::new(
            mock_engine(vec![Arc::clone(&connection), Arc::clone(&recovered)]).unwrap(),
        );
        let worker_engine = Arc::clone(&engine);
        let synthesis = std::thread::spawn(move || {
            worker_engine.synthesize(&synthesis_request("hung utterance"))
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while !engine.is_speaking() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(engine.is_speaking());
        let started_at = Instant::now();
        engine.stop();

        let error = synthesis.join().unwrap().unwrap_err();
        assert!(matches!(
            error,
            TtsError::SynthesisFailed(message) if message.contains("exited")
        ));
        assert!(started_at.elapsed() < Duration::from_secs(1));
        assert!(connection.terminated.load(Ordering::Acquire));
        assert!(!engine.is_speaking());
        assert!(engine.synthesize(&synthesis_request("next utterance")).is_ok());
        assert!(!recovered.terminated.load(Ordering::Acquire));
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
        assert!(engine
            .synthesize(&synthesis_request("recovered"))
            .is_ok());
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

        let error = engine.synthesize(&synthesis_request("hung")).unwrap_err();
        assert!(matches!(
            error,
            TtsError::SynthesisFailed(message) if message.contains("timed out")
        ));
        assert!(hung.terminated.load(Ordering::Acquire));

        engine.prepare_recovery_probe().unwrap();
        assert!(engine
            .synthesize(&synthesis_request("recovered"))
            .is_ok());
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
