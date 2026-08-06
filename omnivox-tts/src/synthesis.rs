//! Structured synthesis request and result contracts.

use crate::contracts::{AcssDimension, PhysicalVoiceId};
use crate::{AudioBuffer, TtsError, TtsSettings, STANDARD_SAMPLE_RATE};

/// Everything an engine needs to synthesize one utterance.
#[derive(Debug, Clone)]
pub struct SynthesisRequest {
    pub text: String,
    pub settings: TtsSettings,
    /// Exact physical target selected by logical routing, when applicable.
    pub requested_voice: Option<PhysicalVoiceId>,
    /// Logical Emacs voice that caused this request, when applicable.
    pub logical_voice_id: Option<String>,
    /// Requested language independent of a particular physical voice.
    pub language: Option<String>,
}

impl SynthesisRequest {
    pub fn new(text: impl Into<String>, settings: TtsSettings) -> Self {
        Self {
            text: text.into(),
            settings,
            requested_voice: None,
            logical_voice_id: None,
            language: None,
        }
    }

    pub fn with_route(
        mut self,
        logical_voice_id: impl Into<String>,
        requested_voice: PhysicalVoiceId,
    ) -> Self {
        self.logical_voice_id = Some(logical_voice_id.into());
        self.requested_voice = Some(requested_voice);
        self
    }

    /// Return the engine-local voice selector, validating an exact routed target.
    pub fn voice_id_for_engine(&self, engine_id: &str) -> Result<&str, TtsError> {
        if let Some(voice) = &self.requested_voice {
            if voice.engine_id != engine_id {
                return Err(TtsError::InvalidParameter(format!(
                    "request targets engine {} but was sent to {engine_id}",
                    voice.engine_id
                )));
            }
            return Ok(&voice.voice_id);
        }
        Ok(&self.settings.voice)
    }
}

/// Kind of synchronization marker returned by a speech engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthesisMarkerKind {
    Word,
    Sentence,
    Phoneme,
    NativeIndex,
}

/// A position in synthesized audio with optional source-text metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesisMarker {
    pub kind: SynthesisMarkerKind,
    /// Offset in audio frames at the result buffer's sample rate.
    pub frame_offset: u64,
    /// UTF-8 byte offset into the request text.
    pub text_start: Option<u32>,
    /// UTF-8 byte length in the request text.
    pub text_length: Option<u32>,
    /// Optional engine-provided phoneme, index, or other marker value.
    pub value: Option<String>,
}

/// Buffered synthesis output and metadata describing what was realized.
#[derive(Debug, Clone)]
pub struct SynthesisResult {
    pub audio: AudioBuffer,
    /// Engine that actually produced this result.
    pub engine_id: String,
    /// Exact physical voice, when the backend can identify it truthfully.
    pub actual_voice: Option<PhysicalVoiceId>,
    pub markers: Vec<SynthesisMarker>,
    /// Requested ACSS dimensions omitted by the selected engine.
    pub degraded_acss: Vec<AcssDimension>,
}

impl SynthesisResult {
    pub fn new(
        engine_id: impl Into<String>,
        actual_voice: Option<PhysicalVoiceId>,
        audio: AudioBuffer,
        markers: Vec<SynthesisMarker>,
    ) -> Self {
        Self {
            audio,
            engine_id: engine_id.into(),
            actual_voice,
            markers,
            degraded_acss: Vec::new(),
        }
    }

    pub fn audio(
        engine_id: impl Into<String>,
        actual_voice: Option<PhysicalVoiceId>,
        audio: AudioBuffer,
    ) -> Self {
        Self::new(engine_id, actual_voice, audio, Vec::new())
    }

    /// Validate engine identity and marker bounds against the originating request.
    pub fn validate(&self, request: &SynthesisRequest) -> Result<(), TtsError> {
        if self.engine_id.is_empty() {
            return Err(invalid_result("engine ID is empty"));
        }
        if let Some(voice) = &self.actual_voice {
            if voice.engine_id != self.engine_id {
                return Err(invalid_result(format!(
                    "actual voice engine {} does not match result engine {}",
                    voice.engine_id, self.engine_id
                )));
            }
        }
        if let Some(requested_voice) = &request.requested_voice {
            if requested_voice.engine_id != self.engine_id {
                return Err(invalid_result(format!(
                    "result engine {} does not match requested engine {}",
                    self.engine_id, requested_voice.engine_id
                )));
            }
            if self.actual_voice.as_ref() != Some(requested_voice) {
                return Err(invalid_result(format!(
                    "actual voice {:?} does not match exact requested voice {requested_voice:?}",
                    self.actual_voice
                )));
            }
        }

        let frame_count = self.audio.frame_count() as u64;
        for marker in &self.markers {
            if marker.frame_offset > frame_count {
                return Err(invalid_result(format!(
                    "marker offset {} exceeds audio frame count {}",
                    marker.frame_offset, frame_count
                )));
            }
            validate_text_range(marker, &request.text)?;
        }
        Ok(())
    }

    /// Convert PCM and marker frame offsets to Omnivox's standard audio format.
    pub fn into_standard_format(mut self) -> Self {
        let source_rate = self.audio.sample_rate;
        if source_rate == STANDARD_SAMPLE_RATE {
            self.audio = self.audio.to_stereo();
            return self;
        }

        self.audio = self.audio.to_standard_format();
        let target_frames = self.audio.frame_count() as u64;
        for marker in &mut self.markers {
            marker.frame_offset = scale_frame_offset(
                marker.frame_offset,
                source_rate,
                STANDARD_SAMPLE_RATE,
                target_frames,
            );
        }
        self
    }
}

fn validate_text_range(marker: &SynthesisMarker, text: &str) -> Result<(), TtsError> {
    let (Some(start), Some(length)) = (marker.text_start, marker.text_length) else {
        if marker.text_start.is_some() || marker.text_length.is_some() {
            return Err(invalid_result(
                "marker source range must contain both start and length",
            ));
        }
        return Ok(());
    };
    let start = start as usize;
    let end = start
        .checked_add(length as usize)
        .ok_or_else(|| invalid_result("marker source range overflowed"))?;
    if end > text.len() {
        return Err(invalid_result(format!(
            "marker source range {start}..{end} exceeds text length {}",
            text.len()
        )));
    }
    if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return Err(invalid_result(format!(
            "marker source range {start}..{end} is not on UTF-8 boundaries"
        )));
    }
    Ok(())
}

fn scale_frame_offset(offset: u64, source_rate: u32, target_rate: u32, target_frames: u64) -> u64 {
    if source_rate == 0 {
        return 0;
    }
    let numerator = u128::from(offset) * u128::from(target_rate) + u128::from(source_rate / 2);
    let scaled = numerator / u128::from(source_rate);
    u64::try_from(scaled).unwrap_or(u64::MAX).min(target_frames)
}

fn invalid_result(message: impl Into<String>) -> TtsError {
    TtsError::SynthesisFailed(format!("invalid synthesis result: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str) -> SynthesisRequest {
        SynthesisRequest::new(text, TtsSettings::default())
    }

    #[test]
    fn validates_realized_engine_and_marker_bounds() {
        let result = SynthesisResult::new(
            "helper",
            Some(PhysicalVoiceId::new("helper", "voice")),
            AudioBuffer::new(vec![0.0; 20], 10, 1),
            vec![SynthesisMarker {
                kind: SynthesisMarkerKind::Word,
                frame_offset: 20,
                text_start: Some(0),
                text_length: Some(5),
                value: None,
            }],
        );

        assert!(result.validate(&request("hello")).is_ok());
    }

    #[test]
    fn rejects_marker_outside_audio() {
        let result = SynthesisResult::new(
            "helper",
            None,
            AudioBuffer::new(vec![0.0; 10], 10, 1),
            vec![SynthesisMarker {
                kind: SynthesisMarkerKind::Word,
                frame_offset: 11,
                text_start: None,
                text_length: None,
                value: None,
            }],
        );

        assert!(result.validate(&request("hello")).is_err());
    }

    #[test]
    fn rejects_marker_inside_utf8_codepoint() {
        let result = SynthesisResult::new(
            "helper",
            None,
            AudioBuffer::new(vec![0.0; 10], 10, 1),
            vec![SynthesisMarker {
                kind: SynthesisMarkerKind::Word,
                frame_offset: 0,
                text_start: Some(1),
                text_length: Some(1),
                value: None,
            }],
        );

        assert!(result.validate(&request("é")).is_err());
    }

    #[test]
    fn rejects_a_different_voice_for_an_exact_route() {
        let request = SynthesisRequest::new("hello", TtsSettings::default()).with_route(
            "logical",
            PhysicalVoiceId::new("helper", "requested"),
        );
        let result = SynthesisResult::audio(
            "helper",
            Some(PhysicalVoiceId::new("helper", "different")),
            AudioBuffer::new(vec![0.0; 10], 10, 1),
        );

        assert!(result.validate(&request).is_err());
    }

    #[test]
    fn standard_format_rescales_markers() {
        let result = SynthesisResult::new(
            "helper",
            None,
            AudioBuffer::new(vec![0.0; 11025], 11025, 1),
            vec![SynthesisMarker {
                kind: SynthesisMarkerKind::Word,
                frame_offset: 5512,
                text_start: None,
                text_length: None,
                value: None,
            }],
        )
        .into_standard_format();

        assert_eq!(result.audio.sample_rate, STANDARD_SAMPLE_RATE);
        assert_eq!(result.audio.channels, 2);
        assert_eq!(result.markers[0].frame_offset, 22048);
    }
}
