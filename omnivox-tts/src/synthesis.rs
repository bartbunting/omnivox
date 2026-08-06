//! Structured synthesis request and result contracts.

use crate::contracts::{AcssDimension, AnchorSupport, PhysicalVoiceId};
use crate::{AudioBuffer, TtsError, TtsSettings, STANDARD_SAMPLE_RATE};

/// Maximum number of requested anchors in one engine synthesis call.
pub const MAX_SYNTHESIS_ANCHORS: usize = 4096;
/// Maximum UTF-8 size of one opaque requested-anchor identifier.
pub const MAX_SYNTHESIS_ANCHOR_ID_BYTES: usize = 128;

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
    /// Bounded source-text positions to resolve against returned PCM.
    pub anchors: Vec<RequestedAnchor>,
}

impl SynthesisRequest {
    pub fn new(text: impl Into<String>, settings: TtsSettings) -> Self {
        Self {
            text: text.into(),
            settings,
            requested_voice: None,
            logical_voice_id: None,
            language: None,
            anchors: Vec::new(),
        }
    }


    /// Attach and validate source-text anchors for this request.
    pub fn with_anchors(mut self, anchors: Vec<RequestedAnchor>) -> Result<Self, TtsError> {
        validate_requested_anchors(&anchors, &self.text)?;
        self.anchors = anchors;
        Ok(self)
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

/// Which side of a requested source-text boundary owns an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorAffinity {
    Before,
    After,
}

/// One opaque source-text position requested by the presentation compiler.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RequestedAnchor {
    pub id: String,
    /// UTF-8 byte boundary in [`SynthesisRequest::text`].
    pub text_offset: u32,
    pub affinity: AnchorAffinity,
}

impl RequestedAnchor {
    pub fn new(id: impl Into<String>, text_offset: u32, affinity: AnchorAffinity) -> Self {
        Self {
            id: id.into(),
            text_offset,
            affinity,
        }
    }
}

/// Accuracy with which an engine or common fallback resolved an anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorResolution {
    Exact,
    WordBoundary,
    SpanBoundary,
    Omitted,
}

/// Result for exactly one requested opaque anchor ID.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedAnchor {
    pub id: String,
    /// Frame offset in the result buffer, absent only when omitted.
    pub frame_offset: Option<u64>,
    pub resolution: AnchorResolution,
}

/// Kind of synchronization marker returned by a speech engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisMarkerKind {
    Word,
    Sentence,
    Phoneme,
    NativeIndex,
}

/// A position in synthesized audio with optional source-text metadata.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Requested positions resolved to PCM frames or explicitly omitted.
    pub anchors: Vec<ResolvedAnchor>,
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
            anchors: Vec::new(),
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

    /// Complete anchor results according to an engine's advertised support.
    ///
    /// Exact engine results already present in `anchors` win. Missing results
    /// use word-start markers when available, otherwise they are explicitly
    /// omitted. This method is idempotent so routed callers may apply it at the
    /// common engine boundary even when a helper already finalized its result.
    pub fn resolve_anchors(
        &mut self,
        request: &SynthesisRequest,
        support: AnchorSupport,
    ) {
        let frame_count = self.audio.frame_count() as u64;
        for requested in &request.anchors {
            if self.anchors.iter().any(|anchor| anchor.id == requested.id) {
                continue;
            }
            let approximation = match support {
                AnchorSupport::None => None,
                AnchorSupport::WordBoundary | AnchorSupport::Exact => {
                    approximate_at_word_boundary(requested, &self.markers, frame_count)
                }
            };
            self.anchors.push(approximation.unwrap_or_else(|| ResolvedAnchor {
                id: requested.id.clone(),
                frame_offset: None,
                resolution: AnchorResolution::Omitted,
            }));
        }
        self.anchors.sort_by_key(|resolved| {
            request
                .anchors
                .iter()
                .position(|requested| requested.id == resolved.id)
                .unwrap_or(usize::MAX)
        });
    }

    /// Validate engine identity and marker bounds against the originating request.
    pub fn validate(&self, request: &SynthesisRequest) -> Result<(), TtsError> {
        validate_requested_anchors(&request.anchors, &request.text)?;
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
        validate_resolved_anchors(&self.anchors, &request.anchors, frame_count)?;
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
        for anchor in &mut self.anchors {
            if let Some(frame_offset) = &mut anchor.frame_offset {
                *frame_offset = scale_frame_offset(
                    *frame_offset,
                    source_rate,
                    STANDARD_SAMPLE_RATE,
                    target_frames,
                );
            }
        }
        self
    }
}

fn approximate_at_word_boundary(
    requested: &RequestedAnchor,
    markers: &[SynthesisMarker],
    frame_count: u64,
) -> Option<ResolvedAnchor> {
    let offset = requested.text_offset;
    let candidate = match requested.affinity {
        AnchorAffinity::Before => markers
            .iter()
            .filter(|marker| marker.kind == SynthesisMarkerKind::Word)
            .filter_map(|marker| marker.text_start.map(|start| (start, marker.frame_offset)))
            .filter(|(start, _)| *start >= offset)
            .min_by_key(|(start, _)| *start),
        AnchorAffinity::After => markers
            .iter()
            .filter(|marker| marker.kind == SynthesisMarkerKind::Word)
            .filter_map(|marker| marker.text_start.map(|start| (start, marker.frame_offset)))
            .filter(|(start, _)| *start <= offset)
            .max_by_key(|(start, _)| *start),
    };
    candidate.map(|(_, frame_offset)| ResolvedAnchor {
        id: requested.id.clone(),
        frame_offset: Some(frame_offset.min(frame_count)),
        resolution: AnchorResolution::WordBoundary,
    })
}

fn validate_requested_anchors(anchors: &[RequestedAnchor], text: &str) -> Result<(), TtsError> {
    if anchors.len() > MAX_SYNTHESIS_ANCHORS {
        return Err(TtsError::InvalidParameter(format!(
            "synthesis request exceeds the {MAX_SYNTHESIS_ANCHORS}-anchor limit"
        )));
    }
    let mut identifiers = std::collections::HashSet::with_capacity(anchors.len());
    for anchor in anchors {
        if anchor.id.is_empty() || anchor.id.len() > MAX_SYNTHESIS_ANCHOR_ID_BYTES {
            return Err(TtsError::InvalidParameter(
                "requested anchor ID is empty or too long".to_owned(),
            ));
        }
        if !identifiers.insert(anchor.id.as_str()) {
            return Err(TtsError::InvalidParameter(format!(
                "duplicate requested anchor ID: {}",
                anchor.id
            )));
        }
        let offset = anchor.text_offset as usize;
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err(TtsError::InvalidParameter(format!(
                "requested anchor {} is not a UTF-8 boundary in the synthesis text",
                anchor.id
            )));
        }
    }
    Ok(())
}

fn validate_resolved_anchors(
    resolved: &[ResolvedAnchor],
    requested: &[RequestedAnchor],
    frame_count: u64,
) -> Result<(), TtsError> {
    if resolved.len() != requested.len() {
        return Err(invalid_result(format!(
            "resolved {} anchors for {} requests",
            resolved.len(),
            requested.len()
        )));
    }
    let mut identifiers = std::collections::HashSet::with_capacity(resolved.len());
    for anchor in resolved {
        if !identifiers.insert(anchor.id.as_str())
            || !requested.iter().any(|request| request.id == anchor.id)
        {
            return Err(invalid_result(format!(
                "unknown or duplicate resolved anchor ID: {}",
                anchor.id
            )));
        }
        match (anchor.resolution, anchor.frame_offset) {
            (AnchorResolution::Omitted, None) => {}
            (AnchorResolution::Omitted, Some(_)) | (_, None) => {
                return Err(invalid_result(format!(
                    "resolved anchor {} has inconsistent omission state",
                    anchor.id
                )));
            }
            (_, Some(offset)) if offset > frame_count => {
                return Err(invalid_result(format!(
                    "anchor offset {offset} exceeds audio frame count {frame_count}"
                )));
            }
            _ => {}
        }
    }
    Ok(())
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
    fn requested_anchors_are_unique_bounded_utf8_positions() {
        let valid = request("Héllo")
            .with_anchors(vec![RequestedAnchor::new(
                "accent",
                3,
                AnchorAffinity::Before,
            )])
            .unwrap();
        assert_eq!(valid.anchors[0].text_offset, 3);

        assert!(request("Héllo")
            .with_anchors(vec![RequestedAnchor::new(
                "inside",
                2,
                AnchorAffinity::Before,
            )])
            .is_err());
        assert!(request("hello")
            .with_anchors(vec![
                RequestedAnchor::new("same", 0, AnchorAffinity::Before),
                RequestedAnchor::new("same", 5, AnchorAffinity::After),
            ])
            .is_err());
    }

    #[test]
    fn word_only_engines_resolve_by_affinity_and_explicitly_omit_without_support() {
        let request = request("one two")
            .with_anchors(vec![
                RequestedAnchor::new("before", 2, AnchorAffinity::Before),
                RequestedAnchor::new("after", 2, AnchorAffinity::After),
            ])
            .unwrap();
        let markers = vec![
            SynthesisMarker {
                kind: SynthesisMarkerKind::Word,
                frame_offset: 1,
                text_start: Some(0),
                text_length: Some(3),
                value: None,
            },
            SynthesisMarker {
                kind: SynthesisMarkerKind::Word,
                frame_offset: 20,
                text_start: Some(4),
                text_length: Some(3),
                value: None,
            },
        ];
        let mut word_result = SynthesisResult::new(
            "word-engine",
            None,
            AudioBuffer::new(vec![0.0; 40], 10, 1),
            markers,
        );
        word_result.resolve_anchors(&request, AnchorSupport::WordBoundary);
        assert_eq!(word_result.anchors[0].frame_offset, Some(20));
        assert_eq!(word_result.anchors[1].frame_offset, Some(1));
        assert!(word_result
            .anchors
            .iter()
            .all(|anchor| anchor.resolution == AnchorResolution::WordBoundary));

        let mut unsupported = SynthesisResult::audio(
            "markerless",
            None,
            AudioBuffer::new(vec![0.0; 40], 10, 1),
        );
        unsupported.resolve_anchors(&request, AnchorSupport::None);
        assert!(unsupported.anchors.iter().all(|anchor| {
            anchor.frame_offset.is_none() && anchor.resolution == AnchorResolution::Omitted
        }));
        unsupported.validate(&request).unwrap();
    }

    #[test]
    fn standard_format_rescales_markers() {
        let mut result = SynthesisResult::new(
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
        );
        result.anchors.push(ResolvedAnchor {
            id: "cue".to_owned(),
            frame_offset: Some(5512),
            resolution: AnchorResolution::Exact,
        });
        let result = result.into_standard_format();

        assert_eq!(result.audio.sample_rate, STANDARD_SAMPLE_RATE);
        assert_eq!(result.audio.channels, 2);
        assert_eq!(result.markers[0].frame_offset, 22048);
        assert_eq!(result.anchors[0].frame_offset, Some(22048));
    }
}
