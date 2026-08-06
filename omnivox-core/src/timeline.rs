//! Engine-neutral presentation timeline vocabulary and frame projection.
//!
//! Source positions are retained until synthesis resolves them to primary
//! speech-frame boundaries.  The pure scheduler then projects those resolved
//! actions onto an output timeline.  Inserted audio advances the primary
//! clock; overlays do not.  No playback or audio-buffer type is involved here,
//! which keeps placement semantics testable before a particular mixer uses it.

use std::fmt;
use thiserror::Error;

/// Maximum UTF-8 size of an opaque action or effect-state identifier.
pub const MAX_TIMELINE_ID_BYTES: usize = 128;

/// A bounded, stable identifier for one timeline action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimelineActionId(String);

impl TimelineActionId {
    pub fn new(value: impl Into<String>) -> Result<Self, TimelineError> {
        let value = value.into();
        validate_identifier(&value, "action")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TimelineActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A bounded identifier for a complete persistent post-synthesis state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectStateId(String);

impl EffectStateId {
    pub fn new(value: impl Into<String>) -> Result<Self, TimelineError> {
        let value = value.into();
        validate_identifier(&value, "effect state")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_identifier(value: &str, kind: &'static str) -> Result<(), TimelineError> {
    if value.is_empty() {
        return Err(TimelineError::EmptyIdentifier(kind));
    }
    if value.len() > MAX_TIMELINE_ID_BYTES {
        return Err(TimelineError::IdentifierTooLong {
            kind,
            actual: value.len(),
        });
    }
    Ok(())
}

/// Which side of a source-text boundary owns an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionAffinity {
    Before,
    After,
}

/// A protocol-level position retained before an engine resolves audio frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationPosition {
    /// The beginning or end of a named speech span.
    SpanBoundary {
        span_id: u64,
        affinity: ActionAffinity,
    },
    /// A UTF-8 byte boundary within a named source-text span.
    TextOffset {
        span_id: u64,
        utf8_offset: u32,
        affinity: ActionAffinity,
    },
}

/// Whether an audio action advances the primary speech clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioActionMode {
    Insert,
    Overlay,
}

/// Which post-synthesis effect state an audio action uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectBus {
    /// Do not apply the current speech effect state.
    Dry,
    /// Use the persistent effect state active for speech at this boundary.
    Speech,
}

/// A complete-state transition on the persistent post-synthesis effect bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectStateChange {
    Begin(EffectStateId),
    Replace(EffectStateId),
    End,
}

/// One engine-neutral timeline operation.
#[derive(Debug, Clone, PartialEq)]
pub enum TimelineActionKind {
    Audio {
        mode: AudioActionMode,
        /// Duration after decoding and canonical resampling.
        duration_frames: u64,
        /// Normalized linear gain from silence through unity.
        volume: f32,
        effect_bus: EffectBus,
    },
    SemanticEvent,
    EffectState(EffectStateChange),
}

/// One requested action before its source position is resolved by synthesis.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineAction {
    pub id: TimelineActionId,
    pub position: PresentationPosition,
    pub kind: TimelineActionKind,
}

/// One action whose source position has resolved to a primary-frame boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTimelineAction {
    pub action: TimelineAction,
    pub source_frame: u64,
}

/// One insertion represented in a piecewise source-to-output map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInsertion {
    pub source_frame: u64,
    pub output_frame: u64,
    pub duration_frames: u64,
}

/// Piecewise source-to-output mapping produced by serial insertions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameMap {
    insertions: Vec<FrameInsertion>,
}

impl FrameMap {
    pub fn insertions(&self) -> &[FrameInsertion] {
        &self.insertions
    }

    /// Map immediately before insertions anchored at `source_frame`.
    pub fn map_before(&self, source_frame: u64) -> Result<u64, TimelineError> {
        self.map_with(source_frame, |insertion| {
            insertion.source_frame < source_frame
        })
    }

    /// Map after every insertion anchored at `source_frame`.
    pub fn map_after(&self, source_frame: u64) -> Result<u64, TimelineError> {
        self.map_with(source_frame, |insertion| {
            insertion.source_frame <= source_frame
        })
    }

    fn map_with(
        &self,
        source_frame: u64,
        include: impl Fn(&FrameInsertion) -> bool,
    ) -> Result<u64, TimelineError> {
        self.insertions
            .iter()
            .filter(|insertion| include(insertion))
            .try_fold(source_frame, |frame, insertion| {
                frame
                    .checked_add(insertion.duration_frames)
                    .ok_or(TimelineError::FrameOverflow)
            })
    }
}

/// One action placed on the output clock with effect-state snapshots.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledAction {
    pub id: TimelineActionId,
    pub source_frame: u64,
    pub output_frame: u64,
    pub kind: TimelineActionKind,
    pub effect_state_before: Option<EffectStateId>,
    pub effect_state_after: Option<EffectStateId>,
}

impl ScheduledAction {
    pub fn end_frame(&self) -> Result<u64, TimelineError> {
        match &self.kind {
            TimelineActionKind::Audio {
                duration_frames, ..
            } => self
                .output_frame
                .checked_add(*duration_frames)
                .ok_or(TimelineError::FrameOverflow),
            _ => Ok(self.output_frame),
        }
    }
}

/// Pure projection of resolved actions onto primary and audible output clocks.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledTimeline {
    pub frame_map: FrameMap,
    pub actions: Vec<ScheduledAction>,
    /// End of speech plus serial insertions, excluding overlay tails.
    pub primary_output_frames: u64,
    /// End of all primary audio and overlay tails.
    pub completion_frame: u64,
}

impl ScheduledTimeline {
    /// Schedule resolved actions. Equal source frames preserve input order.
    pub fn build(
        primary_source_frames: u64,
        actions: Vec<ResolvedTimelineAction>,
    ) -> Result<Self, TimelineError> {
        let mut indexed = actions.into_iter().enumerate().collect::<Vec<_>>();
        indexed.sort_by_key(|(sequence, action)| (action.source_frame, *sequence));

        let mut frame_map = FrameMap::default();
        let mut scheduled = Vec::with_capacity(indexed.len());
        let mut shift = 0_u64;
        let mut effect_state = None;
        let mut completion_frame = primary_source_frames;

        for (_, resolved) in indexed {
            if resolved.source_frame > primary_source_frames {
                return Err(TimelineError::SourceFrameOutOfBounds {
                    action: resolved.action.id,
                    source_frame: resolved.source_frame,
                    primary_source_frames,
                });
            }
            validate_action_kind(&resolved.action.kind)?;
            let output_frame = resolved
                .source_frame
                .checked_add(shift)
                .ok_or(TimelineError::FrameOverflow)?;
            let before = effect_state.clone();
            apply_effect_change(&resolved.action.kind, &mut effect_state)?;
            let after = effect_state.clone();
            let scheduled_action = ScheduledAction {
                id: resolved.action.id,
                source_frame: resolved.source_frame,
                output_frame,
                kind: resolved.action.kind,
                effect_state_before: before,
                effect_state_after: after,
            };
            completion_frame = completion_frame.max(scheduled_action.end_frame()?);

            if let TimelineActionKind::Audio {
                mode: AudioActionMode::Insert,
                duration_frames,
                ..
            } = &scheduled_action.kind
            {
                frame_map.insertions.push(FrameInsertion {
                    source_frame: scheduled_action.source_frame,
                    output_frame: scheduled_action.output_frame,
                    duration_frames: *duration_frames,
                });
                shift = shift
                    .checked_add(*duration_frames)
                    .ok_or(TimelineError::FrameOverflow)?;
            }
            scheduled.push(scheduled_action);
        }

        let primary_output_frames = primary_source_frames
            .checked_add(shift)
            .ok_or(TimelineError::FrameOverflow)?;
        completion_frame = completion_frame.max(primary_output_frames);
        Ok(Self {
            frame_map,
            actions: scheduled,
            primary_output_frames,
            completion_frame,
        })
    }

    /// Action IDs whose playback boundaries have not been consumed.
    pub fn unreached_action_ids(&self, consumed_through_frame: u64) -> Vec<&TimelineActionId> {
        self.actions
            .iter()
            .filter(|action| action.output_frame > consumed_through_frame)
            .map(|action| &action.id)
            .collect()
    }

    /// Audio actions whose audible tails cross the supplied output frame.
    pub fn active_audio_ids(&self, output_frame: u64) -> Vec<&TimelineActionId> {
        self.actions
            .iter()
            .filter(|action| {
                matches!(&action.kind, TimelineActionKind::Audio { .. })
                    && action.output_frame <= output_frame
                    && action.end_frame().is_ok_and(|end| end > output_frame)
            })
            .map(|action| &action.id)
            .collect()
    }
}

fn validate_action_kind(kind: &TimelineActionKind) -> Result<(), TimelineError> {
    if let TimelineActionKind::Audio {
        duration_frames,
        volume,
        ..
    } = kind
    {
        if *duration_frames == 0 {
            return Err(TimelineError::EmptyAudioAction);
        }
        if !volume.is_finite() || !(0.0..=1.0).contains(volume) {
            return Err(TimelineError::InvalidAudioVolume(*volume));
        }
    }
    Ok(())
}

fn apply_effect_change(
    kind: &TimelineActionKind,
    current: &mut Option<EffectStateId>,
) -> Result<(), TimelineError> {
    let TimelineActionKind::EffectState(change) = kind else {
        return Ok(());
    };
    match change {
        EffectStateChange::Begin(state) if current.is_none() => *current = Some(state.clone()),
        EffectStateChange::Replace(state) if current.is_some() => *current = Some(state.clone()),
        EffectStateChange::End if current.is_some() => *current = None,
        EffectStateChange::Begin(_) => return Err(TimelineError::EffectAlreadyActive),
        EffectStateChange::Replace(_) => return Err(TimelineError::NoEffectToReplace),
        EffectStateChange::End => return Err(TimelineError::NoEffectToEnd),
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq)]
pub enum TimelineError {
    #[error("{0} identifier must not be empty")]
    EmptyIdentifier(&'static str),
    #[error("{kind} identifier is {actual} bytes; maximum is {MAX_TIMELINE_ID_BYTES}")]
    IdentifierTooLong { kind: &'static str, actual: usize },
    #[error("timeline frame arithmetic overflowed")]
    FrameOverflow,
    #[error(
        "action {action} source frame {source_frame} exceeds primary frame count {primary_source_frames}"
    )]
    SourceFrameOutOfBounds {
        action: TimelineActionId,
        source_frame: u64,
        primary_source_frames: u64,
    },
    #[error("audio action duration must be positive")]
    EmptyAudioAction,
    #[error("audio action volume must be finite and between zero and one: {0}")]
    InvalidAudioVolume(f32),
    #[error("cannot begin an effect state while one is already active")]
    EffectAlreadyActive,
    #[error("cannot replace an effect state when none is active")]
    NoEffectToReplace,
    #[error("cannot end an effect state when none is active")]
    NoEffectToEnd,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> TimelineActionId {
        TimelineActionId::new(value).unwrap()
    }

    fn position(offset: u32) -> PresentationPosition {
        PresentationPosition::TextOffset {
            span_id: 1,
            utf8_offset: offset,
            affinity: ActionAffinity::Before,
        }
    }

    fn action(name: &str, source_frame: u64, kind: TimelineActionKind) -> ResolvedTimelineAction {
        ResolvedTimelineAction {
            action: TimelineAction {
                id: id(name),
                position: position(source_frame as u32),
                kind,
            },
            source_frame,
        }
    }

    fn audio(mode: AudioActionMode, duration_frames: u64) -> TimelineActionKind {
        TimelineActionKind::Audio {
            mode,
            duration_frames,
            volume: 1.0,
            effect_bus: EffectBus::Dry,
        }
    }

    #[test]
    fn identifiers_are_bounded() {
        assert_eq!(
            TimelineActionId::new(""),
            Err(TimelineError::EmptyIdentifier("action"))
        );
        assert!(matches!(
            EffectStateId::new("x".repeat(MAX_TIMELINE_ID_BYTES + 1)),
            Err(TimelineError::IdentifierTooLong {
                kind: "effect state",
                ..
            })
        ));
    }

    #[test]
    fn equal_frames_keep_input_order_and_insertions_shift_later_actions() {
        let timeline = ScheduledTimeline::build(
            100,
            vec![
                action("first-overlay", 20, audio(AudioActionMode::Overlay, 8)),
                action("serial", 20, audio(AudioActionMode::Insert, 10)),
                action("event", 20, TimelineActionKind::SemanticEvent),
                action("later", 30, TimelineActionKind::SemanticEvent),
            ],
        )
        .unwrap();

        assert_eq!(
            timeline
                .actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first-overlay", "serial", "event", "later"]
        );
        assert_eq!(timeline.actions[0].output_frame, 20);
        assert_eq!(timeline.actions[1].output_frame, 20);
        assert_eq!(timeline.actions[2].output_frame, 30);
        assert_eq!(timeline.actions[3].output_frame, 40);
        assert_eq!(timeline.frame_map.map_before(20).unwrap(), 20);
        assert_eq!(timeline.frame_map.map_after(20).unwrap(), 30);
        assert_eq!(timeline.frame_map.map_before(30).unwrap(), 40);
        assert_eq!(timeline.primary_output_frames, 110);
    }

    #[test]
    fn overlay_tail_extends_completion_without_advancing_primary_clock() {
        let timeline = ScheduledTimeline::build(
            100,
            vec![action(
                "long-overlay",
                90,
                audio(AudioActionMode::Overlay, 40),
            )],
        )
        .unwrap();

        assert_eq!(timeline.primary_output_frames, 100);
        assert_eq!(timeline.completion_frame, 130);
        assert_eq!(timeline.frame_map.map_after(100).unwrap(), 100);
        assert_eq!(timeline.active_audio_ids(100), vec![&id("long-overlay")]);
    }

    #[test]
    fn effect_state_persists_and_changes_in_stable_order() {
        let room = EffectStateId::new("room").unwrap();
        let hall = EffectStateId::new("hall").unwrap();
        let timeline = ScheduledTimeline::build(
            100,
            vec![
                action(
                    "begin",
                    10,
                    TimelineActionKind::EffectState(EffectStateChange::Begin(room.clone())),
                ),
                action("speech-event", 20, TimelineActionKind::SemanticEvent),
                action(
                    "replace",
                    20,
                    TimelineActionKind::EffectState(EffectStateChange::Replace(hall.clone())),
                ),
                action("after-replace", 20, TimelineActionKind::SemanticEvent),
                action(
                    "end",
                    30,
                    TimelineActionKind::EffectState(EffectStateChange::End),
                ),
            ],
        )
        .unwrap();

        assert_eq!(timeline.actions[1].effect_state_before, Some(room.clone()));
        assert_eq!(timeline.actions[1].effect_state_after, Some(room.clone()));
        assert_eq!(timeline.actions[2].effect_state_before, Some(room));
        assert_eq!(timeline.actions[2].effect_state_after, Some(hall.clone()));
        assert_eq!(timeline.actions[3].effect_state_before, Some(hall));
        assert_eq!(timeline.actions[4].effect_state_after, None);
    }

    #[test]
    fn cancellation_projection_separates_tails_from_unreached_events() {
        let timeline = ScheduledTimeline::build(
            100,
            vec![
                action("overlay", 10, audio(AudioActionMode::Overlay, 80)),
                action("reached", 20, TimelineActionKind::SemanticEvent),
                action("unreached", 50, TimelineActionKind::SemanticEvent),
            ],
        )
        .unwrap();

        assert_eq!(timeline.active_audio_ids(30), vec![&id("overlay")]);
        assert_eq!(timeline.unreached_action_ids(30), vec![&id("unreached")]);
    }

    #[test]
    fn invalid_effect_transition_and_out_of_bounds_action_are_rejected() {
        assert_eq!(
            ScheduledTimeline::build(
                10,
                vec![action(
                    "replace",
                    0,
                    TimelineActionKind::EffectState(EffectStateChange::Replace(
                        EffectStateId::new("room").unwrap(),
                    )),
                )],
            ),
            Err(TimelineError::NoEffectToReplace)
        );
        assert!(matches!(
            ScheduledTimeline::build(
                10,
                vec![action("late", 11, TimelineActionKind::SemanticEvent)],
            ),
            Err(TimelineError::SourceFrameOutOfBounds { .. })
        ));
    }
}
