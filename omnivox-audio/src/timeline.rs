//! Bounded PCM rendering for resolved presentation timelines.
//!
//! The core timeline projects source positions onto an output clock. This
//! module realizes that projection in canonical stereo PCM. Inserted resources
//! extend the primary window, while overlays are mixed into it. Overlay tails
//! are retained between windows so playback can begin after one bounded
//! synthesis chunk rather than waiting for a complete dispatch.

use crate::buffer::CHANNELS;
use crate::{AudioBuffer, AudioError};
use omnivox_core::timeline::{
    AudioActionMode, FrameMap, ResolvedTimelineAction, ScheduledTimeline, TimelineActionId,
    TimelineActionKind,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Maximum number of timeline operations accepted in one render window.
pub const MAX_TIMELINE_ACTIONS_PER_WINDOW: usize = 4096;

/// Maximum primary output retained in one render window (two minutes).
pub const MAX_TIMELINE_RENDER_FRAMES: usize = 120 * crate::buffer::SAMPLE_RATE as usize;

/// Canonical, pre-decoded PCM attached to one audio action identifier.
#[derive(Debug, Clone)]
pub struct PreparedAudioResource {
    pub id: TimelineActionId,
    pub audio: AudioBuffer,
}

impl PreparedAudioResource {
    pub fn new(id: TimelineActionId, audio: AudioBuffer) -> Self {
        Self { id, audio }
    }
}

/// Canonical PCM shared immutably by a timeline action and its source cache.
///
/// This complements [`PreparedAudioResource`] for callers that do not need to
/// mutate action PCM. Cloning the resource only increments the reference count.
#[derive(Debug, Clone)]
pub struct SharedPreparedAudioResource {
    pub id: TimelineActionId,
    pub audio: Arc<AudioBuffer>,
}

impl SharedPreparedAudioResource {
    pub fn new(id: TimelineActionId, audio: Arc<AudioBuffer>) -> Self {
        Self { id, audio }
    }
}

trait AudioResourceView {
    fn id(&self) -> &TimelineActionId;
    fn audio(&self) -> &AudioBuffer;
}

impl AudioResourceView for PreparedAudioResource {
    fn id(&self) -> &TimelineActionId {
        &self.id
    }

    fn audio(&self) -> &AudioBuffer {
        &self.audio
    }
}

impl AudioResourceView for SharedPreparedAudioResource {
    fn id(&self) -> &TimelineActionId {
        &self.id
    }

    fn audio(&self) -> &AudioBuffer {
        &self.audio
    }
}

/// One rendered primary window and its insertion map.
#[derive(Debug, Clone)]
pub struct RenderedTimelineWindow {
    pub audio: AudioBuffer,
    /// Final overlay-only tail beginning at the primary window boundary.
    pub overlay_tail: Option<AudioBuffer>,
    /// Zero-duration semantic actions on the rendered output clock.
    pub semantic_events: Vec<RenderedSemanticEvent>,
    pub frame_map: FrameMap,
}

/// One opaque semantic action ready to become a playback cue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedSemanticEvent {
    pub id: TimelineActionId,
    pub frame_offset: u64,
}

impl RenderedTimelineWindow {
    /// Map a dry primary frame after insertions at the same boundary.
    pub fn map_primary_frame(&self, source_frame: u64) -> Result<u64, AudioError> {
        self.frame_map
            .map_after(source_frame)
            .map_err(|error| AudioError::TimelineError(error.to_string()))
    }
}

/// Stateful bounded renderer. State consists only of an overlay tail crossing
/// the previous primary-window boundary.
#[derive(Debug, Default)]
pub struct TimelineAudioRenderer {
    overlay_carry: Vec<f32>,
}

impl TimelineAudioRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_overlay_carry(&self) -> bool {
        !self.overlay_carry.is_empty()
    }

    /// Render one scheduled timeline over PRIMARY.
    ///
    /// Every returned primary buffer has exactly the scheduled primary length.
    /// Non-final windows retain overlay samples beyond that boundary. A final
    /// window returns that remainder separately so playback can overlap it
    /// with later boundary actions without advancing the primary clock.
    pub fn render_window(
        &mut self,
        primary: &AudioBuffer,
        timeline: &ScheduledTimeline,
        resources: &[PreparedAudioResource],
        final_window: bool,
    ) -> Result<RenderedTimelineWindow, AudioError> {
        self.render_window_with_resources(primary, timeline, resources, final_window)
    }

    /// Render a scheduled timeline whose canonical PCM is shared immutably.
    pub fn render_shared_window(
        &mut self,
        primary: &AudioBuffer,
        timeline: &ScheduledTimeline,
        resources: &[SharedPreparedAudioResource],
        final_window: bool,
    ) -> Result<RenderedTimelineWindow, AudioError> {
        self.render_window_with_resources(primary, timeline, resources, final_window)
    }

    /// Render one bounded piece of a progressively synthesized timeline.
    ///
    /// `actions` retain their absolute primary-source frame offsets. Every
    /// action must fall within this window, including either boundary. The
    /// returned frame map and semantic offsets are relative to this rendered
    /// window; callers add their already-published output frame count when
    /// producing dispatch-wide events.
    pub fn render_incremental_shared_window(
        &mut self,
        primary: &AudioBuffer,
        source_start: u64,
        actions: &[ResolvedTimelineAction],
        resources: &[SharedPreparedAudioResource],
        final_window: bool,
    ) -> Result<RenderedTimelineWindow, AudioError> {
        let source_end = source_start
            .checked_add(primary.frame_count() as u64)
            .ok_or_else(|| AudioError::TimelineError("primary frame range overflowed".into()))?;
        let relative_actions = actions
            .iter()
            .map(|resolved| {
                if !(source_start..=source_end).contains(&resolved.source_frame) {
                    return Err(AudioError::TimelineError(format!(
                        "action {} at source frame {} is outside progressive window {source_start}..={source_end}",
                        resolved.action.id, resolved.source_frame
                    )));
                }
                Ok(ResolvedTimelineAction {
                    action: resolved.action.clone(),
                    source_frame: resolved.source_frame - source_start,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let timeline = ScheduledTimeline::build(primary.frame_count() as u64, relative_actions)
            .map_err(|error| AudioError::TimelineError(error.to_string()))?;
        self.render_shared_window(primary, &timeline, resources, final_window)
    }

    fn render_window_with_resources<R: AudioResourceView>(
        &mut self,
        primary: &AudioBuffer,
        timeline: &ScheduledTimeline,
        resources: &[R],
        final_window: bool,
    ) -> Result<RenderedTimelineWindow, AudioError> {
        validate_window(primary, timeline, resources)?;
        let primary_frames = usize::try_from(timeline.primary_output_frames).map_err(|_| {
            AudioError::TimelineError("primary frame count does not fit memory".into())
        })?;
        if primary_frames > MAX_TIMELINE_RENDER_FRAMES {
            return Err(AudioError::TimelineError(format!(
                "primary render window has {primary_frames} frames; maximum is {MAX_TIMELINE_RENDER_FRAMES}"
            )));
        }

        let resource_map = resources
            .iter()
            .map(|resource| (resource.id().as_str(), resource.audio()))
            .collect::<HashMap<_, _>>();
        let mut mixed = vec![0.0_f32; primary_frames * CHANNELS as usize];
        copy_primary_with_insertions(primary, timeline, &mut mixed)?;

        mix_samples_extending(&mut mixed, 0, &self.overlay_carry, 1.0)?;
        for action in &timeline.actions {
            let TimelineActionKind::Audio { mode, volume, .. } = &action.kind else {
                continue;
            };
            let resource = resource_map
                .get(action.id.as_str())
                .expect("validated timeline resource exists");
            match mode {
                AudioActionMode::Insert => {
                    mix_samples(&mut mixed, action.output_frame, &resource.samples, *volume)?;
                }
                AudioActionMode::Overlay => {
                    mix_samples_extending(
                        &mut mixed,
                        action.output_frame,
                        &resource.samples,
                        *volume,
                    )?;
                }
            }
        }

        let primary_samples = primary_frames * CHANNELS as usize;
        clamp_samples(&mut mixed);
        let tail = if mixed.len() > primary_samples {
            Some(AudioBuffer::new(mixed.split_off(primary_samples)))
        } else {
            None
        };
        let (next_carry, overlay_tail) = if final_window {
            (Vec::new(), tail)
        } else {
            (tail.map(|buffer| buffer.samples).unwrap_or_default(), None)
        };
        self.overlay_carry = next_carry;
        Ok(RenderedTimelineWindow {
            audio: AudioBuffer::new(mixed),
            overlay_tail,
            semantic_events: timeline
                .actions
                .iter()
                .filter(|action| matches!(action.kind, TimelineActionKind::SemanticEvent))
                .map(|action| RenderedSemanticEvent {
                    id: action.id.clone(),
                    frame_offset: action.output_frame,
                })
                .collect(),
            frame_map: timeline.frame_map.clone(),
        })
    }
}

fn validate_window<R: AudioResourceView>(
    primary: &AudioBuffer,
    timeline: &ScheduledTimeline,
    resources: &[R],
) -> Result<(), AudioError> {
    if timeline.actions.len() > MAX_TIMELINE_ACTIONS_PER_WINDOW {
        return Err(AudioError::TimelineError(format!(
            "render window has {} actions; maximum is {MAX_TIMELINE_ACTIONS_PER_WINDOW}",
            timeline.actions.len()
        )));
    }
    if timeline
        .frame_map
        .map_after(primary.frame_count() as u64)
        .map_err(|error| AudioError::TimelineError(error.to_string()))?
        != timeline.primary_output_frames
    {
        return Err(AudioError::TimelineError(
            "scheduled primary length does not match the input buffer".into(),
        ));
    }

    let mut resource_ids = HashSet::new();
    for resource in resources {
        if !resource_ids.insert(resource.id().as_str()) {
            return Err(AudioError::TimelineError(format!(
                "duplicate prepared audio resource {}",
                resource.id()
            )));
        }
    }
    for action in &timeline.actions {
        let TimelineActionKind::Audio {
            duration_frames, ..
        } = &action.kind
        else {
            continue;
        };
        let Some(resource) = resources
            .iter()
            .find(|resource| resource.id() == &action.id)
        else {
            return Err(AudioError::TimelineError(format!(
                "missing prepared audio resource {}",
                action.id
            )));
        };
        if resource.audio().frame_count() as u64 != *duration_frames {
            return Err(AudioError::TimelineError(format!(
                "resource {} has {} frames; action declares {duration_frames}",
                action.id,
                resource.audio().frame_count()
            )));
        }
    }
    Ok(())
}

fn copy_primary_with_insertions(
    primary: &AudioBuffer,
    timeline: &ScheduledTimeline,
    output: &mut [f32],
) -> Result<(), AudioError> {
    let mut source_frame = 0_usize;
    let mut output_frame = 0_usize;
    for insertion in timeline.frame_map.insertions() {
        let insertion_source = usize::try_from(insertion.source_frame)
            .map_err(|_| AudioError::TimelineError("insertion source frame is too large".into()))?;
        copy_frame_range(
            primary,
            source_frame,
            insertion_source,
            output,
            output_frame,
        )?;
        source_frame = insertion_source;
        output_frame = usize::try_from(
            insertion
                .output_frame
                .checked_add(insertion.duration_frames)
                .ok_or_else(|| AudioError::TimelineError("insertion frame overflow".into()))?,
        )
        .map_err(|_| AudioError::TimelineError("insertion output frame is too large".into()))?;
    }
    copy_frame_range(
        primary,
        source_frame,
        primary.frame_count(),
        output,
        output_frame,
    )
}

fn copy_frame_range(
    primary: &AudioBuffer,
    source_start: usize,
    source_end: usize,
    output: &mut [f32],
    output_start: usize,
) -> Result<(), AudioError> {
    if source_start > source_end || source_end > primary.frame_count() {
        return Err(AudioError::TimelineError(
            "insertion map contains an invalid source range".into(),
        ));
    }
    let frame_count = source_end - source_start;
    let source_samples = source_start * CHANNELS as usize..source_end * CHANNELS as usize;
    let output_sample = output_start
        .checked_mul(CHANNELS as usize)
        .ok_or_else(|| AudioError::TimelineError("output sample offset overflow".into()))?;
    let output_end = output_sample
        .checked_add(frame_count * CHANNELS as usize)
        .ok_or_else(|| AudioError::TimelineError("output sample range overflow".into()))?;
    let destination = output
        .get_mut(output_sample..output_end)
        .ok_or_else(|| AudioError::TimelineError("insertion map exceeds output buffer".into()))?;
    destination.copy_from_slice(&primary.samples[source_samples]);
    Ok(())
}

fn mix_samples(
    output: &mut [f32],
    output_frame: u64,
    input: &[f32],
    volume: f32,
) -> Result<(), AudioError> {
    let sample_offset = frame_to_sample_offset(output_frame)?;
    let output_end = sample_offset
        .checked_add(input.len())
        .ok_or_else(|| AudioError::TimelineError("audio action sample range overflow".into()))?;
    let destination = output
        .get_mut(sample_offset..output_end)
        .ok_or_else(|| AudioError::TimelineError("inserted audio exceeds primary buffer".into()))?;
    for (output, input) in destination.iter_mut().zip(input) {
        *output += input * volume;
    }
    Ok(())
}

fn mix_samples_extending(
    output: &mut Vec<f32>,
    output_frame: u64,
    input: &[f32],
    volume: f32,
) -> Result<(), AudioError> {
    let sample_offset = frame_to_sample_offset(output_frame)?;
    let required = sample_offset
        .checked_add(input.len())
        .ok_or_else(|| AudioError::TimelineError("audio action sample range overflow".into()))?;
    output.resize(output.len().max(required), 0.0);
    mix_samples(output, output_frame, input, volume)
}

fn frame_to_sample_offset(frame: u64) -> Result<usize, AudioError> {
    usize::try_from(frame)
        .ok()
        .and_then(|frame| frame.checked_mul(CHANNELS as usize))
        .ok_or_else(|| AudioError::TimelineError("audio action frame offset is too large".into()))
}

fn clamp_samples(samples: &mut [f32]) {
    for sample in samples {
        *sample = sample.clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_core::timeline::{
        ActionAffinity, EffectBus, PresentationPosition, ResolvedTimelineAction, TimelineAction,
    };

    fn id(value: &str) -> TimelineActionId {
        TimelineActionId::new(value).unwrap()
    }

    fn action(
        name: &str,
        source_frame: u64,
        mode: AudioActionMode,
        resource: &AudioBuffer,
    ) -> ResolvedTimelineAction {
        ResolvedTimelineAction {
            action: TimelineAction {
                id: id(name),
                position: PresentationPosition::TextOffset {
                    span_id: 1,
                    utf8_offset: source_frame as u32,
                    affinity: ActionAffinity::Before,
                },
                kind: TimelineActionKind::Audio {
                    mode,
                    duration_frames: resource.frame_count() as u64,
                    volume: 1.0,
                    effect_bus: EffectBus::Dry,
                },
            },
            source_frame,
        }
    }

    fn semantic(name: &str, source_frame: u64) -> ResolvedTimelineAction {
        ResolvedTimelineAction {
            action: TimelineAction {
                id: id(name),
                position: PresentationPosition::TextOffset {
                    span_id: 1,
                    utf8_offset: source_frame as u32,
                    affinity: ActionAffinity::Before,
                },
                kind: TimelineActionKind::SemanticEvent,
            },
            source_frame,
        }
    }

    #[test]
    fn insertion_shifts_primary_and_frame_mapping() {
        let primary = AudioBuffer::new(vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
        let inserted = AudioBuffer::new(vec![0.8, 0.8, 0.9, 0.9]);
        let timeline = ScheduledTimeline::build(
            3,
            vec![action("insert", 1, AudioActionMode::Insert, &inserted)],
        )
        .unwrap();
        let mut renderer = TimelineAudioRenderer::new();

        let rendered = renderer
            .render_window(
                &primary,
                &timeline,
                &[PreparedAudioResource::new(id("insert"), inserted)],
                true,
            )
            .unwrap();

        assert_eq!(
            rendered.audio.samples,
            vec![0.1, 0.1, 0.8, 0.8, 0.9, 0.9, 0.2, 0.2, 0.3, 0.3]
        );
        assert_eq!(rendered.map_primary_frame(0).unwrap(), 0);
        assert_eq!(rendered.map_primary_frame(1).unwrap(), 3);
    }

    #[test]
    fn overlay_does_not_shift_primary_and_final_tail_is_retained() {
        let primary = AudioBuffer::new(vec![0.1; 6]);
        let overlay = AudioBuffer::new(vec![0.4; 8]);
        let timeline = ScheduledTimeline::build(
            3,
            vec![action("overlay", 1, AudioActionMode::Overlay, &overlay)],
        )
        .unwrap();
        let mut renderer = TimelineAudioRenderer::new();

        let rendered = renderer
            .render_window(
                &primary,
                &timeline,
                &[PreparedAudioResource::new(id("overlay"), overlay)],
                true,
            )
            .unwrap();

        assert_eq!(rendered.audio.frame_count(), 3);
        assert_eq!(rendered.audio.samples[..2], [0.1, 0.1]);
        assert_eq!(rendered.audio.samples[2..6], [0.5, 0.5, 0.5, 0.5]);
        assert_eq!(
            rendered.overlay_tail.as_ref().unwrap().samples,
            [0.4, 0.4, 0.4, 0.4]
        );
        assert_eq!(rendered.map_primary_frame(2).unwrap(), 2);
    }

    #[test]
    fn overlay_tail_crosses_into_the_next_bounded_window() {
        let first = AudioBuffer::new(vec![0.1; 6]);
        let second = AudioBuffer::new(vec![0.2; 6]);
        let overlay = AudioBuffer::new(vec![0.4; 8]);
        let first_timeline = ScheduledTimeline::build(
            3,
            vec![action("overlay", 1, AudioActionMode::Overlay, &overlay)],
        )
        .unwrap();
        let second_timeline = ScheduledTimeline::build(3, Vec::new()).unwrap();
        let mut renderer = TimelineAudioRenderer::new();

        let rendered_first = renderer
            .render_window(
                &first,
                &first_timeline,
                &[PreparedAudioResource::new(id("overlay"), overlay)],
                false,
            )
            .unwrap();
        assert_eq!(rendered_first.audio.frame_count(), 3);
        assert!(renderer.has_overlay_carry());

        let rendered_second = renderer
            .render_window(&second, &second_timeline, &[], true)
            .unwrap();
        assert_eq!(rendered_second.audio.frame_count(), 3);
        assert_eq!(rendered_second.audio.samples[..4], [0.6, 0.6, 0.6, 0.6]);
        assert_eq!(rendered_second.audio.samples[4..], [0.2, 0.2]);
        assert!(rendered_second.overlay_tail.is_none());
        assert!(!renderer.has_overlay_carry());
    }

    #[test]
    fn missing_or_mismatched_resources_are_rejected() {
        let primary = AudioBuffer::new(vec![0.1; 6]);
        let overlay = AudioBuffer::new(vec![0.4; 4]);
        let timeline = ScheduledTimeline::build(
            3,
            vec![action("overlay", 1, AudioActionMode::Overlay, &overlay)],
        )
        .unwrap();
        let mut renderer = TimelineAudioRenderer::new();

        assert!(renderer
            .render_window(&primary, &timeline, &[], true)
            .unwrap_err()
            .to_string()
            .contains("missing prepared audio resource"));
        assert!(renderer
            .render_window(
                &primary,
                &timeline,
                &[PreparedAudioResource::new(
                    id("overlay"),
                    AudioBuffer::new(vec![0.4; 2]),
                )],
                true,
            )
            .unwrap_err()
            .to_string()
            .contains("action declares"));
    }

    #[test]
    fn semantic_event_uses_the_insertion_shifted_output_frame() {
        let primary = AudioBuffer::new(vec![0.1; 6]);
        let inserted = AudioBuffer::new(vec![0.8; 4]);
        let timeline = ScheduledTimeline::build(
            3,
            vec![
                action("insert", 1, AudioActionMode::Insert, &inserted),
                semantic("meaning", 1),
            ],
        )
        .unwrap();
        let mut renderer = TimelineAudioRenderer::new();

        let rendered = renderer
            .render_window(
                &primary,
                &timeline,
                &[PreparedAudioResource::new(id("insert"), inserted)],
                true,
            )
            .unwrap();

        assert_eq!(
            rendered.semantic_events,
            vec![RenderedSemanticEvent {
                id: id("meaning"),
                frame_offset: 3,
            }]
        );
    }

    #[test]
    fn shared_resources_render_without_taking_pcm_ownership() {
        let primary = AudioBuffer::new(vec![0.1; 4]);
        let shared = Arc::new(AudioBuffer::new(vec![0.8; 2]));
        let timeline = ScheduledTimeline::build(
            2,
            vec![action("insert", 1, AudioActionMode::Insert, &shared)],
        )
        .unwrap();
        let resource = SharedPreparedAudioResource::new(id("insert"), Arc::clone(&shared));
        assert_eq!(Arc::strong_count(&shared), 2);

        let rendered = TimelineAudioRenderer::new()
            .render_shared_window(&primary, &timeline, &[resource], true)
            .unwrap();

        assert_eq!(rendered.audio.samples, vec![0.1, 0.1, 0.8, 0.8, 0.1, 0.1]);
        assert_eq!(Arc::strong_count(&shared), 1);
    }

    #[test]
    fn incremental_windows_match_one_complete_timeline_at_shared_boundaries() {
        let primary = AudioBuffer::new(vec![0.1; 8]);
        let overlay = Arc::new(AudioBuffer::new(vec![0.2; 6]));
        let inserted = Arc::new(AudioBuffer::new(vec![0.4; 4]));
        let actions = vec![
            action("overlay", 2, AudioActionMode::Overlay, &overlay),
            action("insert", 2, AudioActionMode::Insert, &inserted),
            semantic("meaning", 3),
        ];
        let resources = vec![
            SharedPreparedAudioResource::new(id("overlay"), Arc::clone(&overlay)),
            SharedPreparedAudioResource::new(id("insert"), Arc::clone(&inserted)),
        ];
        let complete_timeline = ScheduledTimeline::build(4, actions.clone()).unwrap();
        let complete = TimelineAudioRenderer::new()
            .render_shared_window(&primary, &complete_timeline, &resources, true)
            .unwrap();

        let mut incremental_renderer = TimelineAudioRenderer::new();
        let first = incremental_renderer
            .render_incremental_shared_window(
                &AudioBuffer::new(primary.samples[..4].to_vec()),
                0,
                &actions[..1],
                &resources,
                false,
            )
            .unwrap();
        let second = incremental_renderer
            .render_incremental_shared_window(
                &AudioBuffer::new(primary.samples[4..].to_vec()),
                2,
                &actions[1..],
                &resources,
                true,
            )
            .unwrap();
        let mut incremental_samples = first.audio.samples;
        incremental_samples.extend(second.audio.samples);

        assert_eq!(incremental_samples, complete.audio.samples);
        assert_eq!(
            second.overlay_tail.as_ref().map(|tail| &tail.samples),
            complete.overlay_tail.as_ref().map(|tail| &tail.samples)
        );
        assert_eq!(
            second.semantic_events,
            vec![RenderedSemanticEvent {
                id: id("meaning"),
                frame_offset: 3,
            }]
        );
    }

    #[test]
    fn incremental_window_rejects_actions_outside_its_absolute_range() {
        let primary = AudioBuffer::new(vec![0.1; 4]);
        let action = semantic("early", 4);

        let error = TimelineAudioRenderer::new()
            .render_incremental_shared_window(&primary, 5, &[action], &[], false)
            .unwrap_err();

        assert!(error.to_string().contains("outside progressive window"));
    }
}
