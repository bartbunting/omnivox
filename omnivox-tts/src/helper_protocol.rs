//! Versioned IPC contract for out-of-process speech engines.
//!
//! Version 1 uses one UTF-8 JSON object per line. Helpers reserve stdout for
//! protocol frames and write diagnostics to stderr. PCM is transferred in
//! bounded Base64 chunks so the same protocol is straightforward to implement
//! in Rust and the existing .NET Framework x86 bridges.

use std::io::{self, BufRead, Read, Write};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::EngineDescriptor;
use crate::{RequestedAnchor, MAX_SYNTHESIS_ANCHORS, MAX_SYNTHESIS_ANCHOR_ID_BYTES};

pub const HELPER_PROTOCOL_VERSION: u16 = 2;
pub const HELPER_PROTOCOL_V1: u16 = 1;
pub const SUPPORTED_HELPER_PROTOCOL_VERSIONS: &[u16] =
    &[HELPER_PROTOCOL_VERSION, HELPER_PROTOCOL_V1];
pub const MAX_HELPER_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_HELPER_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_HELPER_AUDIO_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_HELPER_SYNTHESIS_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_HELPER_MARKERS: usize = 4096;
pub const MAX_HELPER_VOICES: usize = 4096;
const MAX_HELPER_STRING_BYTES: usize = 16 * 1024;
const MAX_SUPPORTED_VERSIONS: usize = 16;
const MAX_SAMPLE_RATE: u32 = 384_000;
const MAX_CHANNELS: u16 = 8;

#[derive(Debug, Error)]
pub enum HelperProtocolError {
    #[error("helper protocol I/O failed: {0}")]
    Io(#[from] io::Error),

    #[error("helper protocol JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),

    #[error("helper frame exceeds the {MAX_HELPER_FRAME_BYTES}-byte limit")]
    FrameTooLarge,

    #[error("helper frame ended before its newline terminator")]
    TruncatedFrame,

    #[error("unsupported helper protocol version {0}")]
    UnsupportedVersion(u16),

    #[error("helper request ID must be positive")]
    InvalidRequestId,

    #[error("helper text exceeds the {MAX_HELPER_TEXT_BYTES}-byte limit")]
    TextTooLarge,

    #[error("helper PCM chunk exceeds the {MAX_HELPER_AUDIO_CHUNK_BYTES}-byte limit")]
    AudioChunkTooLarge,

    #[error("helper PCM chunk is not valid Base64: {0}")]
    InvalidAudioEncoding(base64::DecodeError),

    #[error("helper PCM chunk must contain complete signed 16-bit samples")]
    InvalidAudioLength,

    #[error("helper marker batch exceeds the {MAX_HELPER_MARKERS}-marker limit")]
    TooManyMarkers,

    #[error("helper synthesis request exceeds the {MAX_SYNTHESIS_ANCHORS}-anchor limit")]
    TooManyAnchors,

    #[error("invalid helper protocol field: {0}")]
    InvalidField(&'static str),
}

/// Encode one newline-terminated helper frame after enforcing its wire bound.
pub fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, HelperProtocolError> {
    let mut frame = serde_json::to_vec(message)?;
    if frame.len() > MAX_HELPER_FRAME_BYTES {
        return Err(HelperProtocolError::FrameTooLarge);
    }
    frame.push(b'\n');
    Ok(frame)
}

/// Write and flush one complete helper frame.
pub fn write_frame<W: Write, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), HelperProtocolError> {
    writer.write_all(&encode_frame(message)?)?;
    writer.flush()?;
    Ok(())
}

/// Read one bounded helper frame. Clean EOF before a frame returns `None`.
pub fn read_frame<R: BufRead, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<Option<T>, HelperProtocolError> {
    let mut frame = Vec::new();
    let bytes_read = (&mut *reader)
        .take((MAX_HELPER_FRAME_BYTES + 2) as u64)
        .read_until(b'\n', &mut frame)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    if !frame.ends_with(b"\n") {
        return Err(if frame.len() > MAX_HELPER_FRAME_BYTES {
            HelperProtocolError::FrameTooLarge
        } else {
            HelperProtocolError::TruncatedFrame
        });
    }
    frame.pop();
    if frame.ends_with(b"\r") {
        frame.pop();
    }
    if frame.len() > MAX_HELPER_FRAME_BYTES {
        return Err(HelperProtocolError::FrameTooLarge);
    }
    Ok(Some(serde_json::from_slice(&frame)?))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelperRequest {
    pub protocol_version: u16,
    pub request_id: u64,
    #[serde(flatten)]
    pub body: HelperRequestBody,
}

impl HelperRequest {
    pub fn new(request_id: u64, body: HelperRequestBody) -> Self {
        Self::with_version(HELPER_PROTOCOL_VERSION, request_id, body)
    }

    pub fn with_version(
        protocol_version: u16,
        request_id: u64,
        body: HelperRequestBody,
    ) -> Self {
        Self {
            protocol_version,
            request_id,
            body,
        }
    }

    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        validate_version(self.protocol_version)?;
        validate_request_id(self.request_id)?;
        match &self.body {
            HelperRequestBody::Hello {
                supported_protocol_versions,
            } => {
                if supported_protocol_versions.is_empty()
                    || supported_protocol_versions.len() > MAX_SUPPORTED_VERSIONS
                    || !supported_protocol_versions.contains(&self.protocol_version)
                    || supported_protocol_versions
                        .iter()
                        .enumerate()
                        .any(|(index, version)| {
                            *version == 0 || supported_protocol_versions[..index].contains(version)
                        })
                {
                    return Err(HelperProtocolError::InvalidField(
                        "supported_protocol_versions",
                    ));
                }
            }
            HelperRequestBody::Describe | HelperRequestBody::Ping | HelperRequestBody::Shutdown => {
            }
            HelperRequestBody::Synthesize {
                text,
                settings,
                anchors,
            } => {
                if text.len() > MAX_HELPER_TEXT_BYTES {
                    return Err(HelperProtocolError::TextTooLarge);
                }
                settings.validate()?;
                match (self.protocol_version, anchors) {
                    (HELPER_PROTOCOL_V1, None) => {}
                    (HELPER_PROTOCOL_VERSION, Some(anchors)) => {
                        validate_requested_anchors(anchors, text)?;
                    }
                    (HELPER_PROTOCOL_V1, Some(_)) => {
                        return Err(HelperProtocolError::InvalidField("anchors"));
                    }
                    (_, None) => {
                        return Err(HelperProtocolError::InvalidField("anchors"));
                    }
                    _ => unreachable!("validated helper protocol version"),
                }
            }
            HelperRequestBody::Cancel { target_request_id } => {
                validate_request_id(*target_request_id)?;
                if *target_request_id == self.request_id {
                    return Err(HelperProtocolError::InvalidField("target_request_id"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperRequestBody {
    Hello {
        supported_protocol_versions: Vec<u16>,
    },
    Describe,
    Synthesize {
        text: String,
        settings: HelperSynthesisSettings,
        /// Present in protocol v2 and absent in protocol v1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchors: Option<Vec<RequestedAnchor>>,
    },
    Cancel {
        target_request_id: u64,
    },
    Ping,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelperSynthesisSettings {
    pub voice_id: Option<String>,
    pub rate: f32,
    pub pitch: f32,
    pub volume: f32,
}

impl HelperSynthesisSettings {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        if self
            .voice_id
            .as_ref()
            .is_some_and(|voice| voice.is_empty() || voice.len() > MAX_HELPER_STRING_BYTES)
        {
            return Err(HelperProtocolError::InvalidField("voice_id"));
        }
        if !self.rate.is_finite() || !(0.0..=1.0).contains(&self.rate) {
            return Err(HelperProtocolError::InvalidField("rate"));
        }
        if !self.pitch.is_finite() || !(0.5..=2.0).contains(&self.pitch) {
            return Err(HelperProtocolError::InvalidField("pitch"));
        }
        if !self.volume.is_finite() || !(0.0..=1.0).contains(&self.volume) {
            return Err(HelperProtocolError::InvalidField("volume"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelperResponse {
    pub protocol_version: u16,
    pub request_id: Option<u64>,
    #[serde(flatten)]
    pub body: HelperResponseBody,
}

impl HelperResponse {
    pub fn for_request(request_id: u64, body: HelperResponseBody) -> Self {
        Self::for_request_version(HELPER_PROTOCOL_VERSION, request_id, body)
    }

    pub fn for_request_version(
        protocol_version: u16,
        request_id: u64,
        body: HelperResponseBody,
    ) -> Self {
        Self {
            protocol_version,
            request_id: Some(request_id),
            body,
        }
    }

    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        validate_version(self.protocol_version)?;
        if let Some(request_id) = self.request_id {
            validate_request_id(request_id)?;
        } else if !matches!(self.body, HelperResponseBody::Error { .. }) {
            return Err(HelperProtocolError::InvalidRequestId);
        }

        match &self.body {
            HelperResponseBody::Hello {
                selected_protocol_version,
                helper_name,
                helper_version,
            } => {
                validate_version(*selected_protocol_version)?;
                validate_nonempty_string(helper_name, "helper_name")?;
                validate_nonempty_string(helper_version, "helper_version")?;
            }
            HelperResponseBody::Descriptor { descriptor } => {
                validate_nonempty_string(&descriptor.id, "descriptor.id")?;
                if descriptor.voices.len() > MAX_HELPER_VOICES {
                    return Err(HelperProtocolError::InvalidField("descriptor.voices"));
                }
            }
            HelperResponseBody::SynthesisStarted {
                format,
                actual_voice_id,
            } => {
                format.validate()?;
                validate_nonempty_string(actual_voice_id, "actual_voice_id")?;
            }
            HelperResponseBody::AudioChunk { chunk } => {
                chunk.decode_bytes()?;
            }
            HelperResponseBody::Markers { markers } => {
                if markers.len() > MAX_HELPER_MARKERS {
                    return Err(HelperProtocolError::TooManyMarkers);
                }
                for marker in markers {
                    if self.protocol_version == HELPER_PROTOCOL_V1
                        && marker.kind == HelperMarkerKind::RequestedAnchor
                    {
                        return Err(HelperProtocolError::InvalidField("marker.kind"));
                    }
                    marker.validate()?;
                }
            }
            HelperResponseBody::CancelAccepted { target_request_id } => {
                validate_request_id(*target_request_id)?;
            }
            HelperResponseBody::Error { message, .. } => {
                validate_nonempty_string(message, "error.message")?;
            }
            HelperResponseBody::Pong
            | HelperResponseBody::SynthesisCompleted { .. }
            | HelperResponseBody::SynthesisCancelled
            | HelperResponseBody::ShuttingDown => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperResponseBody {
    Hello {
        selected_protocol_version: u16,
        helper_name: String,
        helper_version: String,
    },
    Descriptor {
        descriptor: EngineDescriptor,
    },
    Pong,
    SynthesisStarted {
        format: HelperAudioFormat,
        actual_voice_id: String,
    },
    AudioChunk {
        chunk: HelperPcmChunk,
    },
    Markers {
        markers: Vec<HelperMarker>,
    },
    SynthesisCompleted {
        frame_count: u64,
    },
    SynthesisCancelled,
    CancelAccepted {
        target_request_id: u64,
    },
    ShuttingDown,
    Error {
        code: HelperErrorCode,
        message: String,
        retryable: bool,
    },
}

impl HelperResponseBody {
    pub fn is_synthesis_terminal(&self) -> bool {
        matches!(
            self,
            Self::SynthesisCompleted { .. } | Self::SynthesisCancelled | Self::Error { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperSampleFormat {
    PcmS16Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperAudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: HelperSampleFormat,
}

impl HelperAudioFormat {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        if self.sample_rate == 0 || self.sample_rate > MAX_SAMPLE_RATE {
            return Err(HelperProtocolError::InvalidField("sample_rate"));
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(HelperProtocolError::InvalidField("channels"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperPcmChunk {
    pub sequence: u32,
    pub data_base64: String,
}

impl HelperPcmChunk {
    pub fn from_bytes(sequence: u32, bytes: &[u8]) -> Result<Self, HelperProtocolError> {
        validate_pcm_bytes(bytes)?;
        Ok(Self {
            sequence,
            data_base64: BASE64.encode(bytes),
        })
    }

    pub fn decode_bytes(&self) -> Result<Vec<u8>, HelperProtocolError> {
        let maximum_encoded_length = MAX_HELPER_AUDIO_CHUNK_BYTES.div_ceil(3) * 4;
        if self.data_base64.len() > maximum_encoded_length {
            return Err(HelperProtocolError::AudioChunkTooLarge);
        }
        let bytes = BASE64
            .decode(&self.data_base64)
            .map_err(HelperProtocolError::InvalidAudioEncoding)?;
        validate_pcm_bytes(&bytes)?;
        Ok(bytes)
    }

    pub fn decode_samples(&self) -> Result<Vec<i16>, HelperProtocolError> {
        Ok(self
            .decode_bytes()?
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect())
    }
}

fn validate_pcm_bytes(bytes: &[u8]) -> Result<(), HelperProtocolError> {
    if bytes.len() > MAX_HELPER_AUDIO_CHUNK_BYTES {
        return Err(HelperProtocolError::AudioChunkTooLarge);
    }
    if bytes.len() & 1 != 0 {
        return Err(HelperProtocolError::InvalidAudioLength);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperMarkerKind {
    Word,
    Sentence,
    Phoneme,
    NativeIndex,
    /// Protocol-v2 exact resolution of one requested opaque anchor.
    RequestedAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperMarker {
    pub kind: HelperMarkerKind,
    pub frame_offset: u64,
    pub text_start: Option<u32>,
    pub text_length: Option<u32>,
    pub value: Option<String>,
}

impl HelperMarker {
    fn validate(&self) -> Result<(), HelperProtocolError> {
        if self
            .value
            .as_ref()
            .is_some_and(|value| value.len() > MAX_HELPER_STRING_BYTES)
        {
            return Err(HelperProtocolError::InvalidField("marker.value"));
        }
        if self.kind == HelperMarkerKind::RequestedAnchor
            && self
                .value
                .as_ref()
                .is_none_or(|value| value.is_empty() || value.len() > MAX_SYNTHESIS_ANCHOR_ID_BYTES)
        {
            return Err(HelperProtocolError::InvalidField("marker.value"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperErrorCode {
    InvalidRequest,
    UnsupportedVersion,
    PayloadTooLarge,
    NotAvailable,
    VoiceNotFound,
    InvalidParameter,
    Busy,
    SynthesisFailed,
    Internal,
}

fn validate_version(version: u16) -> Result<(), HelperProtocolError> {
    if !SUPPORTED_HELPER_PROTOCOL_VERSIONS.contains(&version) {
        return Err(HelperProtocolError::UnsupportedVersion(version));
    }
    Ok(())
}

fn validate_requested_anchors(
    anchors: &[RequestedAnchor],
    text: &str,
) -> Result<(), HelperProtocolError> {
    if anchors.len() > MAX_SYNTHESIS_ANCHORS {
        return Err(HelperProtocolError::TooManyAnchors);
    }
    let mut identifiers = std::collections::HashSet::with_capacity(anchors.len());
    for anchor in anchors {
        if anchor.id.is_empty()
            || anchor.id.len() > MAX_SYNTHESIS_ANCHOR_ID_BYTES
            || !identifiers.insert(anchor.id.as_str())
        {
            return Err(HelperProtocolError::InvalidField("anchors.id"));
        }
        let offset = anchor.text_offset as usize;
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err(HelperProtocolError::InvalidField("anchors.text_offset"));
        }
    }
    Ok(())
}

fn validate_request_id(request_id: u64) -> Result<(), HelperProtocolError> {
    if request_id == 0 {
        return Err(HelperProtocolError::InvalidRequestId);
    }
    Ok(())
}

fn validate_nonempty_string(value: &str, field: &'static str) -> Result<(), HelperProtocolError> {
    if value.is_empty() || value.len() > MAX_HELPER_STRING_BYTES {
        return Err(HelperProtocolError::InvalidField(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    fn synthesis_request() -> HelperRequest {
        HelperRequest::new(
            17,
            HelperRequestBody::Synthesize {
                text: "Héllo from Eloquence".to_owned(),
                settings: HelperSynthesisSettings {
                    voice_id: Some("eloquence:reed".to_owned()),
                    rate: 0.6,
                    pitch: 1.1,
                    volume: 0.8,
                },
                anchors: Some(Vec::new()),
            },
        )
    }

    #[test]
    fn request_round_trips_as_one_utf8_json_line() {
        let request = synthesis_request();
        let encoded = encode_frame(&request).unwrap();
        assert_eq!(encoded.last(), Some(&b'\n'));

        let decoded: HelperRequest = read_frame(&mut BufReader::new(Cursor::new(encoded)))
            .unwrap()
            .unwrap();
        assert_eq!(decoded, request);
        decoded.validate().unwrap();
    }

    #[test]
    fn protocol_v1_synthesis_omits_the_v2_anchor_field() {
        let request = HelperRequest::with_version(
            HELPER_PROTOCOL_V1,
            19,
            HelperRequestBody::Synthesize {
                text: "legacy".to_owned(),
                settings: HelperSynthesisSettings {
                    voice_id: None,
                    rate: 0.5,
                    pitch: 1.0,
                    volume: 1.0,
                },
                anchors: None,
            },
        );

        request.validate().unwrap();
        let json = String::from_utf8(encode_frame(&request).unwrap()).unwrap();
        assert!(!json.contains("anchors"));
    }

    #[test]
    fn protocol_v2_validates_utf8_anchor_boundaries_and_ids() {
        let mut request = synthesis_request();
        if let HelperRequestBody::Synthesize { anchors, .. } = &mut request.body {
            *anchors = Some(vec![RequestedAnchor::new(
                "accent",
                3,
                crate::AnchorAffinity::Before,
            )]);
        }
        request.validate().unwrap();

        if let HelperRequestBody::Synthesize { anchors, .. } = &mut request.body {
            anchors.as_mut().unwrap()[0].text_offset = 2;
        }
        assert!(matches!(
            request.validate(),
            Err(HelperProtocolError::InvalidField("anchors.text_offset"))
        ));
    }

    #[test]
    fn reader_accepts_crlf_and_reports_clean_eof() {
        let mut encoded = encode_frame(&HelperRequest::new(2, HelperRequestBody::Ping)).unwrap();
        encoded.insert(encoded.len() - 1, b'\r');
        let mut reader = BufReader::new(Cursor::new(encoded));

        let request: HelperRequest = read_frame(&mut reader).unwrap().unwrap();
        assert_eq!(request.body, HelperRequestBody::Ping);
        assert!(read_frame::<_, HelperRequest>(&mut reader)
            .unwrap()
            .is_none());
    }

    #[test]
    fn reader_rejects_oversized_and_truncated_frames() {
        let oversized = vec![b'x'; MAX_HELPER_FRAME_BYTES + 2];
        let error = read_frame::<_, HelperRequest>(&mut BufReader::new(Cursor::new(oversized)))
            .unwrap_err();
        assert!(matches!(error, HelperProtocolError::FrameTooLarge));

        let error =
            read_frame::<_, HelperRequest>(&mut BufReader::new(Cursor::new(b"{}"))).unwrap_err();
        assert!(matches!(error, HelperProtocolError::TruncatedFrame));
    }

    #[test]
    fn writer_rejects_an_oversized_frame() {
        let oversized = serde_json::json!({
            "payload": "x".repeat(MAX_HELPER_FRAME_BYTES)
        });
        assert!(matches!(
            encode_frame(&oversized),
            Err(HelperProtocolError::FrameTooLarge)
        ));
    }

    #[test]
    fn validation_rejects_invalid_identifiers_text_and_settings() {
        let mut request = synthesis_request();
        request.request_id = 0;
        assert!(matches!(
            request.validate(),
            Err(HelperProtocolError::InvalidRequestId)
        ));

        request.request_id = 17;
        if let HelperRequestBody::Synthesize { text, .. } = &mut request.body {
            *text = "x".repeat(MAX_HELPER_TEXT_BYTES + 1);
        }
        assert!(matches!(
            request.validate(),
            Err(HelperProtocolError::TextTooLarge)
        ));

        if let HelperRequestBody::Synthesize { text, settings, .. } = &mut request.body {
            text.clear();
            settings.rate = f32::NAN;
        }
        assert!(matches!(
            request.validate(),
            Err(HelperProtocolError::InvalidField("rate"))
        ));
    }

    #[test]
    fn pcm_chunks_round_trip_signed_little_endian_samples() {
        let samples = [-32_768_i16, -1, 0, 32_767];
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let chunk = HelperPcmChunk::from_bytes(3, &bytes).unwrap();

        assert_eq!(chunk.sequence, 3);
        assert_eq!(chunk.decode_bytes().unwrap(), bytes);
        assert_eq!(chunk.decode_samples().unwrap(), samples);
    }

    #[test]
    fn pcm_chunks_enforce_alignment_encoding_and_size_bounds() {
        assert!(matches!(
            HelperPcmChunk::from_bytes(0, &[1]),
            Err(HelperProtocolError::InvalidAudioLength)
        ));
        assert!(matches!(
            HelperPcmChunk::from_bytes(0, &vec![0; MAX_HELPER_AUDIO_CHUNK_BYTES + 2]),
            Err(HelperProtocolError::AudioChunkTooLarge)
        ));
        assert!(matches!(
            HelperPcmChunk {
                sequence: 0,
                data_base64: "%%%".to_owned()
            }
            .decode_bytes(),
            Err(HelperProtocolError::InvalidAudioEncoding(_))
        ));
    }

    #[test]
    fn response_requires_an_owned_request_except_for_unowned_errors() {
        let response = HelperResponse {
            protocol_version: HELPER_PROTOCOL_VERSION,
            request_id: None,
            body: HelperResponseBody::Pong,
        };
        assert!(matches!(
            response.validate(),
            Err(HelperProtocolError::InvalidRequestId)
        ));

        let error = HelperResponse {
            protocol_version: HELPER_PROTOCOL_VERSION,
            request_id: None,
            body: HelperResponseBody::Error {
                code: HelperErrorCode::InvalidRequest,
                message: "could not decode request ID".to_owned(),
                retryable: false,
            },
        };
        error.validate().unwrap();
    }

    #[test]
    fn synthesis_terminal_classification_is_explicit() {
        assert!(HelperResponseBody::SynthesisCompleted { frame_count: 12 }.is_synthesis_terminal());
        assert!(HelperResponseBody::SynthesisCancelled.is_synthesis_terminal());
        assert!(HelperResponseBody::Error {
            code: HelperErrorCode::SynthesisFailed,
            message: "native callback failed".to_owned(),
            retryable: true,
        }
        .is_synthesis_terminal());
        assert!(!HelperResponseBody::AudioChunk {
            chunk: HelperPcmChunk::from_bytes(0, &[0, 0]).unwrap(),
        }
        .is_synthesis_terminal());
    }
}
