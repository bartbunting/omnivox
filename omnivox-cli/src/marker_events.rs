//! Playback marker event preparation and asynchronous stdout delivery.

use omnivox_audio::{
    AudioBuffer, AudioControl, AudioError, PlaybackCue, PlaybackTicket, StreamType,
};
use omnivox_tts::contracts::{AcssDimension, PhysicalVoiceId, PostSynthesisDimension};
use omnivox_tts::marker_protocol::{
    format_marker_event, MarkerEvent, MarkerEventEnvelope, MARKER_PROTOCOL_VERSION,
    TIMELINE_EVENT_PROTOCOL_VERSION,
};
use omnivox_tts::{AnchorResolution, SynthesisMarker};
use omnivox_core::timeline::TimelineActionId;
use std::cell::Cell;
use std::io::{self, Write};
use std::sync::{mpsc, Arc};
use tracing::warn;

enum MarkerReporterMessage {
    Event(Arc<MarkerEventEnvelope>),
    Flush(mpsc::Sender<()>),
}

/// Nonblocking producer for the marker stdout reporter.
#[derive(Clone)]
pub struct MarkerEventOutput {
    sender: mpsc::Sender<MarkerReporterMessage>,
}

impl MarkerEventOutput {
    fn emit(&self, event: Arc<MarkerEventEnvelope>) {
        let _ = self.sender.send(MarkerReporterMessage::Event(event));
    }

    /// Wait until every marker submitted before this call has been written.
    pub fn flush(&self) {
        let (sender, receiver) = mpsc::channel();
        if self
            .sender
            .send(MarkerReporterMessage::Flush(sender))
            .is_ok()
        {
            let _ = receiver.recv();
        }
    }
}

/// Spawn the single writer that serializes marker events to stdout.
pub fn spawn_marker_event_reporter(
) -> (MarkerEventOutput, std::thread::JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("omnivox-marker-reporter".to_owned())
        .spawn(move || marker_event_reporter(receiver))
        .expect("Failed to spawn marker event reporter thread");
    (MarkerEventOutput { sender }, handle)
}

fn marker_event_reporter(receiver: mpsc::Receiver<MarkerReporterMessage>) {
    for message in receiver {
        match message {
            MarkerReporterMessage::Event(event) => match format_marker_event(&event) {
                Ok(record) => {
                    let mut stdout = io::stdout().lock();
                    if let Err(error) =
                        writeln!(stdout, "{}", record).and_then(|_| stdout.flush())
                    {
                        warn!("Could not write playback marker event: {}", error);
                    }
                }
                Err(error) => warn!("Could not encode playback marker event: {}", error),
            },
            MarkerReporterMessage::Flush(acknowledge) => {
                let _ = acknowledge.send(());
            }
        }
    }
}

/// Per-dispatch sequence and route context used while synthesis queues chunks.
pub struct MarkerDispatchContext {
    dispatch_id: u64,
    protocol_version: u32,
    next_sequence: Cell<u64>,
    next_utterance_id: Cell<u64>,
    output: MarkerEventOutput,
}

impl MarkerDispatchContext {
    pub fn new(dispatch_id: u64, output: MarkerEventOutput) -> Self {
        Self {
            dispatch_id,
            protocol_version: MARKER_PROTOCOL_VERSION,
            next_sequence: Cell::new(0),
            next_utterance_id: Cell::new(0),
            output,
        }
    }

    /// Create a dispatch capable of emitting playback-bound semantic actions.
    #[allow(dead_code)] // Activated by the structured timeline transport slice.
    pub fn with_timeline_events(dispatch_id: u64, output: MarkerEventOutput) -> Self {
        Self {
            dispatch_id,
            protocol_version: TIMELINE_EVENT_PROTOCOL_VERSION,
            next_sequence: Cell::new(0),
            next_utterance_id: Cell::new(0),
            output,
        }
    }

    pub fn supports_timeline_events(&self) -> bool {
        self.protocol_version == TIMELINE_EVENT_PROTOCOL_VERSION
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_utterance(
        &self,
        text: &str,
        engine_id: &str,
        actual_voice: Option<&PhysicalVoiceId>,
        logical_voice_id: Option<&str>,
        sample_rate: u32,
        frame_count: usize,
        markers: &[SynthesisMarker],
        semantic_events: &[PlaybackSemanticEvent],
    ) -> PreparedMarkerPlayback {
        self.prepare_timeline_utterance(
            text,
            engine_id,
            actual_voice,
            logical_voice_id,
            sample_rate,
            frame_count,
            markers,
            semantic_events,
            &[],
            &[],
            &[],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_timeline_utterance(
        &self,
        text: &str,
        engine_id: &str,
        actual_voice: Option<&PhysicalVoiceId>,
        logical_voice_id: Option<&str>,
        sample_rate: u32,
        frame_count: usize,
        markers: &[SynthesisMarker],
        semantic_events: &[PlaybackSemanticEvent],
        resolutions: &[PlaybackTimelineResolution],
        degraded_acss: &[AcssDimension],
        degraded_effects: &[PostSynthesisDimension],
    ) -> PreparedMarkerPlayback {
        assert!(
            (semantic_events.is_empty()
                && resolutions.is_empty()
                && degraded_acss.is_empty()
                && degraded_effects.is_empty())
                || self.protocol_version == TIMELINE_EVENT_PROTOCOL_VERSION,
            "timeline playback events require a version 2 dispatch"
        );
        let utterance_id = increment(&self.next_utterance_id);
        let diagnostic_count = resolutions.len()
            + usize::from(!degraded_acss.is_empty() || !degraded_effects.is_empty());
        let mut events =
            Vec::with_capacity(markers.len() + semantic_events.len() + diagnostic_count + 1);
        let mut cues =
            Vec::with_capacity(markers.len() + semantic_events.len() + diagnostic_count + 1);
        push_event(
            &mut events,
            &mut cues,
            0,
            MarkerEventEnvelope {
                protocol_version: self.protocol_version,
                dispatch_id: self.dispatch_id,
                sequence: increment(&self.next_sequence),
                event: MarkerEvent::UtteranceStarted {
                    utterance_id,
                    text: text.to_owned(),
                    engine_id: engine_id.to_owned(),
                    actual_voice: actual_voice.cloned(),
                    logical_voice_id: logical_voice_id.map(str::to_owned),
                    sample_rate,
                    frame_count: frame_count as u64,
                },
            },
        );
        for resolution in resolutions {
            push_event(
                &mut events,
                &mut cues,
                0,
                MarkerEventEnvelope {
                    protocol_version: self.protocol_version,
                    dispatch_id: self.dispatch_id,
                    sequence: increment(&self.next_sequence),
                    event: MarkerEvent::TimelineActionResolved {
                        utterance_id,
                        action_id: resolution.action_id.as_str().to_owned(),
                        resolution: resolution.resolution,
                    },
                },
            );
        }
        if !degraded_acss.is_empty() || !degraded_effects.is_empty() {
            push_event(
                &mut events,
                &mut cues,
                0,
                MarkerEventEnvelope {
                    protocol_version: self.protocol_version,
                    dispatch_id: self.dispatch_id,
                    sequence: increment(&self.next_sequence),
                    event: MarkerEvent::TimelineStyleDegraded {
                        utterance_id,
                        degraded_acss: degraded_acss.to_vec(),
                        degraded_effects: degraded_effects.to_vec(),
                    },
                },
            );
        }

        let mut pending = markers
            .iter()
            .cloned()
            .enumerate()
            .map(|(order, marker)| {
                (
                    marker.frame_offset,
                    order,
                    MarkerEvent::MarkerReached {
                        utterance_id,
                        marker,
                    },
                )
            })
            .chain(semantic_events.iter().enumerate().map(|(index, event)| {
                (
                    event.frame_offset,
                    markers.len() + index,
                    MarkerEvent::SemanticEventReached {
                        utterance_id,
                        action_id: event.action_id.as_str().to_owned(),
                    },
                )
            }))
            .collect::<Vec<_>>();
        pending.sort_by_key(|(frame_offset, order, _)| (*frame_offset, *order));
        for (frame_offset, _, event) in pending {
            push_event(
                &mut events,
                &mut cues,
                frame_offset,
                MarkerEventEnvelope {
                    protocol_version: self.protocol_version,
                    dispatch_id: self.dispatch_id,
                    sequence: increment(&self.next_sequence),
                    event,
                },
            );
        }

        PreparedMarkerPlayback {
            cues,
            events: Arc::new(events),
            output: self.output.clone(),
        }
    }
}

/// An opaque semantic action already mapped to the mixed output clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSemanticEvent {
    pub action_id: TimelineActionId,
    pub frame_offset: u64,
}

/// One requested action and the placement grade realized by synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackTimelineResolution {
    pub action_id: TimelineActionId,
    pub resolution: AnchorResolution,
}

pub struct PreparedMarkerPlayback {
    cues: Vec<PlaybackCue>,
    events: Arc<Vec<Arc<MarkerEventEnvelope>>>,
    output: MarkerEventOutput,
}

impl PreparedMarkerPlayback {
    pub fn queue(
        self,
        control: &AudioControl,
        buffer: &AudioBuffer,
    ) -> Result<Option<PlaybackTicket>, AudioError> {
        let events = self.events;
        let output = self.output;
        control.queue_tracked_with_cue_callback(
            StreamType::Speech,
            buffer,
            self.cues,
            move |cue| {
                if let Some(event) = events.get(cue.identifier as usize) {
                    output.emit(event.clone());
                }
            },
        )
    }
}

fn increment(counter: &Cell<u64>) -> u64 {
    let next = counter.get().checked_add(1).expect("marker sequence overflow");
    counter.set(next);
    next
}

fn push_event(
    events: &mut Vec<Arc<MarkerEventEnvelope>>,
    cues: &mut Vec<PlaybackCue>,
    frame_offset: u64,
    event: MarkerEventEnvelope,
) {
    let identifier = events.len() as u64;
    events.push(Arc::new(event));
    cues.push(PlaybackCue {
        frame_offset,
        identifier,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::SynthesisMarkerKind;

    fn marker(frame_offset: u64, value: &str) -> SynthesisMarker {
        SynthesisMarker {
            kind: SynthesisMarkerKind::Word,
            frame_offset,
            text_start: None,
            text_length: None,
            value: Some(value.to_owned()),
        }
    }

    #[test]
    fn prepares_started_event_then_stably_sorted_markers() {
        let (sender, _receiver) = mpsc::channel();
        let context = MarkerDispatchContext::new(73, MarkerEventOutput { sender });
        let prepared = context.prepare_utterance(
            "hello world",
            "helper",
            Some(&PhysicalVoiceId::new("helper", "paul")),
            Some("source-code"),
            44100,
            100,
            &[marker(50, "second"), marker(10, "first"), marker(10, "same")],
            &[],
        );

        assert_eq!(
            prepared
                .cues
                .iter()
                .map(|cue| cue.frame_offset)
                .collect::<Vec<_>>(),
            vec![0, 10, 10, 50]
        );
        assert_eq!(
            prepared
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert!(matches!(
            prepared.events[0].event,
            MarkerEvent::UtteranceStarted {
                ref text,
                ref engine_id,
                ref logical_voice_id,
                ..
            } if text == "hello world"
                && engine_id == "helper"
                && logical_voice_id.as_deref() == Some("source-code")
        ));
        assert!(matches!(
            prepared.events[1].event,
            MarkerEvent::MarkerReached {
                ref marker,
                ..
            } if marker.value.as_deref() == Some("first")
        ));
        assert!(matches!(
            prepared.events[2].event,
            MarkerEvent::MarkerReached {
                ref marker,
                ..
            } if marker.value.as_deref() == Some("same")
        ));
    }

    #[test]
    fn v2_semantic_events_are_stably_merged_at_playback_frames() {
        let (sender, _receiver) = mpsc::channel();
        let context = MarkerDispatchContext::with_timeline_events(
            91,
            MarkerEventOutput { sender },
        );
        let prepared = context.prepare_utterance(
            "hello",
            "helper",
            None,
            None,
            44100,
            100,
            &[marker(20, "word")],
            &[
                PlaybackSemanticEvent {
                    action_id: TimelineActionId::new("same-frame").unwrap(),
                    frame_offset: 20,
                },
                PlaybackSemanticEvent {
                    action_id: TimelineActionId::new("earlier").unwrap(),
                    frame_offset: 10,
                },
            ],
        );

        assert_eq!(
            prepared
                .cues
                .iter()
                .map(|cue| cue.frame_offset)
                .collect::<Vec<_>>(),
            vec![0, 10, 20, 20]
        );
        assert!(prepared.events.iter().all(|event| {
            event.protocol_version == TIMELINE_EVENT_PROTOCOL_VERSION
        }));
        assert!(matches!(
            prepared.events[1].event,
            MarkerEvent::SemanticEventReached { ref action_id, .. }
                if action_id == "earlier"
        ));
        assert!(matches!(
            prepared.events[2].event,
            MarkerEvent::MarkerReached { .. }
        ));
        assert!(matches!(
            prepared.events[3].event,
            MarkerEvent::SemanticEventReached { ref action_id, .. }
                if action_id == "same-frame"
        ));
    }

    #[test]
    fn v2_reports_anchor_and_style_degradation_at_utterance_start() {
        let (sender, _receiver) = mpsc::channel();
        let context = MarkerDispatchContext::with_timeline_events(
            92,
            MarkerEventOutput { sender },
        );
        let prepared = context.prepare_timeline_utterance(
            "hello",
            "helper",
            None,
            Some("comment"),
            44100,
            100,
            &[],
            &[],
            &[PlaybackTimelineResolution {
                action_id: TimelineActionId::new("cue").unwrap(),
                resolution: AnchorResolution::WordBoundary,
            }],
            &[AcssDimension::Richness],
            &[PostSynthesisDimension::Echo],
        );

        assert_eq!(
            prepared
                .cues
                .iter()
                .map(|cue| cue.frame_offset)
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert!(matches!(
            prepared.events[1].event,
            MarkerEvent::TimelineActionResolved {
                ref action_id,
                resolution: AnchorResolution::WordBoundary,
                ..
            } if action_id == "cue"
        ));
        assert!(matches!(
            prepared.events[2].event,
            MarkerEvent::TimelineStyleDegraded {
                ref degraded_acss,
                ref degraded_effects,
                ..
            } if degraded_acss == &[AcssDimension::Richness]
                && degraded_effects == &[PostSynthesisDimension::Echo]
        ));
    }
}
