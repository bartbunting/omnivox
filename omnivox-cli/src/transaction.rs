//! Atomic validation and generation tracking for framed presentations.

use omnivox_core::state::{CapitalizationPresentation, ChannelMode, PunctuationLevel};
use omnivox_core::{parse_command, Command, CommandId};
use omnivox_tts::presentation::decode_presentation_frame;
use omnivox_tts::timeline_protocol::{
    decode_multipart_presentation_timeline, decode_presentation_timeline,
    decode_presentation_timeline_part, PresentationTimelineEnvelope, PresentationTimelinePart,
    MAX_TIMELINE_AGGREGATE_ENCODED_BYTES,
};

use crate::text::parse_resource_path;

const MAX_PRESENTATION_COMMANDS: usize = 4096;

#[derive(Debug)]
pub struct PreparedPresentation {
    pub generation: u64,
    pub commands: Vec<Command>,
}

#[derive(Debug)]
pub struct PreparedStructuredPresentation {
    pub generation: u64,
    pub timeline: PresentationTimelineEnvelope,
}

#[derive(Debug)]
pub enum AdjacentTimelineSelection {
    Coalesced {
        selected: PreparedStructuredPresentation,
        cancelled_dispatch_id: u64,
    },
    PreserveOrder {
        current: PreparedStructuredPresentation,
        candidate: PreparedStructuredPresentation,
    },
}

#[derive(Debug, Default)]
pub struct PresentationGenerations {
    latest: u64,
}

/// Bounded in-order reconstruction of one V3 timeline transport aggregate.
#[derive(Debug)]
pub struct MultipartTimelineAssembler {
    generation: u64,
    dispatch_id: u64,
    part_count: usize,
    decoded_bytes: usize,
    next_part_index: usize,
    encoded_payload: String,
}

impl MultipartTimelineAssembler {
    pub fn start(arguments: &str) -> Result<Self, String> {
        let part =
            decode_presentation_timeline_part(arguments).map_err(|error| error.to_string())?;
        if part.part_index != 0 {
            return Err("multipart timeline must begin with part index 0".to_owned());
        }
        let expected_encoded_bytes = part.decoded_bytes.div_ceil(3) * 4;
        if expected_encoded_bytes > MAX_TIMELINE_AGGREGATE_ENCODED_BYTES {
            return Err("multipart timeline exceeds the encoded aggregate bound".to_owned());
        }
        let mut assembler = Self {
            generation: part.generation,
            dispatch_id: part.dispatch_id,
            part_count: part.part_count,
            decoded_bytes: part.decoded_bytes,
            next_part_index: 0,
            encoded_payload: String::with_capacity(expected_encoded_bytes),
        };
        assembler.push_part(part)?;
        Ok(assembler)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn dispatch_id(&self) -> u64 {
        self.dispatch_id
    }

    pub fn is_complete(&self) -> bool {
        self.next_part_index == self.part_count
    }

    pub fn push(&mut self, arguments: &str) -> Result<(), String> {
        let part =
            decode_presentation_timeline_part(arguments).map_err(|error| error.to_string())?;
        self.push_part(part)
    }

    fn push_part(&mut self, part: PresentationTimelinePart) -> Result<(), String> {
        if part.generation != self.generation
            || part.dispatch_id != self.dispatch_id
            || part.part_count != self.part_count
            || part.decoded_bytes != self.decoded_bytes
        {
            return Err("multipart timeline metadata changed between parts".to_owned());
        }
        if part.part_index != self.next_part_index {
            return Err(format!(
                "multipart timeline expected part {} but received {}",
                self.next_part_index, part.part_index
            ));
        }
        let expected_encoded_bytes = self.decoded_bytes.div_ceil(3) * 4;
        if self.encoded_payload.len() + part.encoded_fragment.len() > expected_encoded_bytes {
            return Err("multipart timeline contains more encoded data than declared".to_owned());
        }
        self.encoded_payload.push_str(&part.encoded_fragment);
        self.next_part_index += 1;
        Ok(())
    }

    pub fn finish(
        self,
        generations: &PresentationGenerations,
    ) -> Result<Option<PreparedStructuredPresentation>, String> {
        if !self.is_complete() {
            return Err(format!(
                "multipart timeline ended after {} of {} parts",
                self.next_part_index, self.part_count
            ));
        }
        let timeline =
            decode_multipart_presentation_timeline(&self.encoded_payload, self.decoded_bytes)
                .map_err(|error| error.to_string())?;
        if timeline.generation != self.generation || timeline.dispatch_id != self.dispatch_id {
            return Err("multipart timeline header does not match its decoded envelope".to_owned());
        }
        generations.prepare_decoded_timeline(timeline)
    }
}

impl PresentationGenerations {
    pub fn prepare(&self, arguments: &str) -> Result<Option<PreparedPresentation>, String> {
        let frame = decode_presentation_frame(arguments).map_err(|error| error.to_string())?;
        if frame.generation <= self.latest {
            return Ok(None);
        }

        let commands = frame
            .script
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(parse_command)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("invalid framed command: {error}"))?;
        validate_commands(&commands)?;

        Ok(Some(PreparedPresentation {
            generation: frame.generation,
            commands,
        }))
    }

    pub fn prepare_timeline(
        &self,
        payload: &str,
    ) -> Result<Option<PreparedStructuredPresentation>, String> {
        let timeline = decode_presentation_timeline(payload).map_err(|error| error.to_string())?;
        self.prepare_decoded_timeline(timeline)
    }

    fn prepare_decoded_timeline(
        &self,
        timeline: PresentationTimelineEnvelope,
    ) -> Result<Option<PreparedStructuredPresentation>, String> {
        if timeline.generation <= self.latest {
            return Ok(None);
        }
        Ok(Some(PreparedStructuredPresentation {
            generation: timeline.generation,
            timeline,
        }))
    }

    pub fn commit(&mut self, generation: u64) {
        self.latest = self.latest.max(generation);
    }

    #[cfg(test)]
    fn latest(&self) -> u64 {
        self.latest
    }
}

pub fn prefer_newer_timeline(
    current: PreparedStructuredPresentation,
    candidate: PreparedStructuredPresentation,
) -> PreparedStructuredPresentation {
    if candidate.generation > current.generation {
        candidate
    } else {
        current
    }
}

pub fn select_adjacent_timeline(
    current: PreparedStructuredPresentation,
    candidate: PreparedStructuredPresentation,
) -> AdjacentTimelineSelection {
    if !current
        .timeline
        .shares_replacement_domain(&candidate.timeline)
    {
        return AdjacentTimelineSelection::PreserveOrder { current, candidate };
    }

    let cancelled_dispatch_id = if candidate.generation > current.generation {
        current.timeline.dispatch_id
    } else {
        candidate.timeline.dispatch_id
    };
    AdjacentTimelineSelection::Coalesced {
        selected: prefer_newer_timeline(current, candidate),
        cancelled_dispatch_id,
    }
}

pub fn prefer_newer(
    current: PreparedPresentation,
    candidate: PreparedPresentation,
) -> PreparedPresentation {
    if candidate.generation > current.generation {
        candidate
    } else {
        current
    }
}

fn validate_commands(commands: &[Command]) -> Result<(), String> {
    if commands.is_empty() {
        return Err("framed presentation is empty".to_owned());
    }
    if commands.len() > MAX_PRESENTATION_COMMANDS {
        return Err(format!(
            "framed presentation has {} commands; limit is {MAX_PRESENTATION_COMMANDS}",
            commands.len()
        ));
    }
    let dispatches = commands
        .iter()
        .filter(|command| command.id == CommandId::Dispatch)
        .count();
    if dispatches != 1 || commands.last().map(|command| &command.id) != Some(&CommandId::Dispatch) {
        return Err("framed presentation must end with exactly one dispatch".to_owned());
    }

    for command in commands {
        validate_command(command)?;
    }
    Ok(())
}

fn validate_command(command: &Command) -> Result<(), String> {
    let arguments = command.args.as_deref();
    let valid = match command.id {
        CommandId::Queue | CommandId::Code => arguments.is_some(),
        CommandId::Dispatch => arguments.is_none(),
        CommandId::Tone => arguments.is_some_and(|arguments| {
            let fields = arguments.split_whitespace().collect::<Vec<_>>();
            fields.len() == 2
                && fields[0].parse::<u32>().is_ok()
                && fields[1].parse::<u32>().is_ok()
        }),
        CommandId::Silence => arguments.is_some_and(|value| value.parse::<u32>().is_ok()),
        CommandId::AudioIcon => arguments.is_some_and(|value| parse_resource_path(value).is_ok()),
        CommandId::TtsSetPunctuations => {
            arguments.is_some_and(|value| PunctuationLevel::parse(value).is_some())
        }
        CommandId::TtsSetSpeechRate
        | CommandId::TtsSetCharacterScale
        | CommandId::TtsSetPitchMultiplier
        | CommandId::TtsSetSoundVolume
        | CommandId::TtsSetToneVolume
        | CommandId::TtsSetVoiceVolume => arguments.is_some_and(valid_float),
        CommandId::TtsSplitCaps => matches!(arguments, Some("0" | "1")),
        CommandId::TtsSetCapitalizationPresentation => arguments
            .is_some_and(|value| CapitalizationPresentation::parse(value).is_some()),
        CommandId::TtsSyncState => arguments.is_some_and(|arguments| {
            let fields = arguments.split_whitespace().collect::<Vec<_>>();
            fields.len() == 4
                && PunctuationLevel::parse(fields[0]).is_some()
                && matches!(fields[1], "0" | "1")
                && matches!(fields[2], "0" | "1")
                && valid_float(fields[3])
        }),
        CommandId::TtsSetVoice => arguments.is_some_and(|value| !value.is_empty()),
        CommandId::TtsSetSpeechChannel | CommandId::TtsSetNotificationChannel => {
            arguments.is_some_and(|value| ChannelMode::parse(value).is_some())
        }
        CommandId::SetLang
        | CommandId::SetNextLang
        | CommandId::SetPreviousLang
        | CommandId::SetPreferredLang => true,
        CommandId::Stop
        | CommandId::Letter
        | CommandId::PlaySound
        | CommandId::TtsSay
        | CommandId::TtsReset
        | CommandId::Version
        | CommandId::OmnivoxControl
        | CommandId::EmacsvoxTx
        | CommandId::EmacsvoxTimeline
        | CommandId::EmacsvoxTimelinePart
        | CommandId::EmacsvoxTrackedDispatch
        | CommandId::EmacsvoxMarkerDispatch
        | CommandId::TtsExit => false,
    };

    if valid {
        Ok(())
    } else {
        Err(format!(
            "command {:?} is invalid inside a framed presentation",
            command.id
        ))
    }
}

fn valid_float(value: &str) -> bool {
    value.parse::<f32>().is_ok_and(f32::is_finite)
}

#[cfg(test)]
mod tests {
    use omnivox_tts::contracts::NormalizedAcss;
    use omnivox_tts::presentation::encode_presentation_script;
    use omnivox_tts::timeline_protocol::{
        encode_multipart_presentation_timeline, encode_presentation_timeline,
        PresentationDeliveryPolicy, PresentationEffectDirective, PresentationSpeechSpan,
        PresentationTimelineEnvelope, MAX_TIMELINE_ENCODED_BYTES,
        PRESENTATION_TIMELINE_PROTOCOL_VERSION,
    };

    use super::*;

    fn arguments(generation: u64, script: &str) -> String {
        format!(
            "{generation} {{{}}}",
            encode_presentation_script(script).unwrap()
        )
    }

    fn timeline_payload(generation: u64, dispatch_id: u64, text: &str) -> String {
        timeline_payload_with_delivery(
            generation,
            dispatch_id,
            text,
            PresentationDeliveryPolicy::Replaceable,
            Some("speaker"),
        )
    }

    fn timeline_payload_with_delivery(
        generation: u64,
        dispatch_id: u64,
        text: &str,
        delivery_policy: PresentationDeliveryPolicy,
        replacement_key: Option<&str>,
    ) -> String {
        encode_presentation_timeline(&timeline_envelope(
            generation,
            dispatch_id,
            text,
            delivery_policy,
            replacement_key,
        ))
        .unwrap()
    }

    fn timeline_envelope(
        generation: u64,
        dispatch_id: u64,
        text: &str,
        delivery_policy: PresentationDeliveryPolicy,
        replacement_key: Option<&str>,
    ) -> PresentationTimelineEnvelope {
        PresentationTimelineEnvelope {
            protocol_version: PRESENTATION_TIMELINE_PROTOCOL_VERSION,
            generation,
            dispatch_id,
            delivery_policy: Some(delivery_policy),
            replacement_key: replacement_key.map(str::to_owned),
            spans: vec![PresentationSpeechSpan {
                id: 1,
                text: text.to_owned(),
                logical_voice_id: None,
                acss: NormalizedAcss::default(),
                rate_offset: None,
                effects: PresentationEffectDirective::Retain,
            }],
            actions: Vec::new(),
        }
    }

    fn timeline_part_arguments(
        generation: u64,
        dispatch_id: u64,
        part_index: usize,
        part_count: usize,
        decoded_bytes: usize,
        fragment: &str,
    ) -> String {
        format!(
            "{PRESENTATION_TIMELINE_PROTOCOL_VERSION} {generation} {dispatch_id} {part_index} {part_count} {decoded_bytes} {fragment}"
        )
    }

    #[test]
    fn prepares_a_complete_transaction_before_commit() {
        let mut generations = PresentationGenerations::default();
        let prepared = generations
            .prepare(&arguments(7, "tts_sync_state all 1 0 60\nq {hello}\nd\n"))
            .unwrap()
            .unwrap();

        assert_eq!(prepared.generation, 7);
        assert_eq!(prepared.commands.len(), 3);
        assert_eq!(generations.latest(), 0);

        generations.commit(prepared.generation);
        assert_eq!(generations.latest(), 7);
        assert!(generations.prepare(&arguments(7, "q {retry}\nd\n")).unwrap().is_none());
    }

    #[test]
    fn failed_validation_does_not_consume_the_generation() {
        let mut generations = PresentationGenerations::default();

        assert!(generations.prepare(&arguments(3, "q {partial}\n")).is_err());
        assert_eq!(generations.latest(), 0);

        let retry = generations
            .prepare(&arguments(3, "q {complete}\nd\n"))
            .unwrap()
            .unwrap();
        generations.commit(retry.generation);
        assert_eq!(generations.latest(), 3);
    }

    #[test]
    fn rejects_control_and_stop_commands_atomically() {
        let generations = PresentationGenerations::default();

        assert!(generations.prepare(&arguments(1, "s\nd\n")).is_err());
        assert!(generations
            .prepare(&arguments(1, "omnivox_control {payload}\nd\n"))
            .is_err());
        assert!(generations
            .prepare(&arguments(
                1,
                "q {hello}\nemacsvox_tracked_dispatch 1\nd\n"
            ))
            .is_err());
        assert!(generations
            .prepare(&arguments(
                1,
                "q {hello}\nemacsvox_marker_dispatch 1\nd\n"
            ))
            .is_err());
        assert_eq!(generations.latest(), 0);
    }

    #[test]
    fn consecutive_frames_select_the_highest_generation() {
        let generations = PresentationGenerations::default();
        let first = generations
            .prepare(&arguments(10, "q {first}\nd\n"))
            .unwrap()
            .unwrap();
        let older = generations
            .prepare(&arguments(9, "q {older}\nd\n"))
            .unwrap()
            .unwrap();
        let newest = generations
            .prepare(&arguments(11, "q {newest}\nd\n"))
            .unwrap()
            .unwrap();

        let selected = prefer_newer(prefer_newer(first, older), newest);

        assert_eq!(selected.generation, 11);
        assert_eq!(selected.commands[0].args.as_deref(), Some("newest"));
    }

    #[test]
    fn stop_barrier_can_consume_a_selected_generation() {
        let mut generations = PresentationGenerations::default();
        let selected = generations
            .prepare(&arguments(5, "q {stale}\nd\n"))
            .unwrap()
            .unwrap();

        generations.commit(selected.generation);

        assert!(generations.prepare(&arguments(5, "q {return}\nd\n")).unwrap().is_none());
    }

    #[test]
    fn structured_and_legacy_presentations_share_one_generation_clock() {
        let mut generations = PresentationGenerations::default();
        let structured = generations
            .prepare_timeline(&timeline_payload(8, 42, "structured"))
            .unwrap()
            .unwrap();
        generations.commit(structured.generation);

        assert!(generations
            .prepare(&arguments(7, "q {legacy}\nd\n"))
            .unwrap()
            .is_none());
        let newer = generations
            .prepare_timeline(&timeline_payload(9, 43, "newer"))
            .unwrap()
            .unwrap();
        assert_eq!(newer.timeline.dispatch_id, 43);
    }

    #[test]
    fn adjacent_timelines_coalesce_only_with_the_same_replacement_key() {
        let generations = PresentationGenerations::default();
        let current = generations
            .prepare_timeline(&timeline_payload(10, 40, "first"))
            .unwrap()
            .unwrap();
        let same_key = generations
            .prepare_timeline(&timeline_payload(11, 41, "latest"))
            .unwrap()
            .unwrap();

        assert!(matches!(
            select_adjacent_timeline(current, same_key),
            AdjacentTimelineSelection::Coalesced {
                selected,
                cancelled_dispatch_id: 40,
            } if selected.generation == 11 && selected.timeline.dispatch_id == 41
        ));

        let current = generations
            .prepare_timeline(&timeline_payload(12, 42, "navigation"))
            .unwrap()
            .unwrap();
        let other_key = generations
            .prepare_timeline(&timeline_payload_with_delivery(
                13,
                43,
                "completion",
                PresentationDeliveryPolicy::Replaceable,
                Some("completion"),
            ))
            .unwrap()
            .unwrap();
        assert!(matches!(
            select_adjacent_timeline(current, other_key),
            AdjacentTimelineSelection::PreserveOrder { current, .. }
                if current.timeline.dispatch_id == 42
        ));
    }

    #[test]
    fn adjacent_ordered_and_urgent_timelines_preserve_order() {
        let generations = PresentationGenerations::default();
        for (policy, dispatch_id) in [
            (PresentationDeliveryPolicy::Ordered, 50),
            (PresentationDeliveryPolicy::Urgent, 51),
        ] {
            let current = generations
                .prepare_timeline(&timeline_payload_with_delivery(
                    dispatch_id,
                    dispatch_id,
                    "important",
                    policy,
                    None,
                ))
                .unwrap()
                .unwrap();
            let candidate = generations
                .prepare_timeline(&timeline_payload(
                    dispatch_id + 1,
                    dispatch_id + 1,
                    "navigation",
                ))
                .unwrap()
                .unwrap();

            assert!(matches!(
                select_adjacent_timeline(current, candidate),
                AdjacentTimelineSelection::PreserveOrder { current, .. }
                    if current.timeline.dispatch_id == dispatch_id
            ));
        }
    }

    #[test]
    fn older_same_key_candidate_is_cancelled_without_reordering() {
        let generations = PresentationGenerations::default();
        let current = generations
            .prepare_timeline(&timeline_payload(20, 60, "current"))
            .unwrap()
            .unwrap();
        let older = generations
            .prepare_timeline(&timeline_payload(19, 59, "late arrival"))
            .unwrap()
            .unwrap();

        assert!(matches!(
            select_adjacent_timeline(current, older),
            AdjacentTimelineSelection::Coalesced {
                selected,
                cancelled_dispatch_id: 59,
            } if selected.generation == 20 && selected.timeline.dispatch_id == 60
        ));
    }

    #[test]
    fn stop_barrier_consumes_a_structured_generation() {
        let mut generations = PresentationGenerations::default();
        let selected = generations
            .prepare_timeline(&timeline_payload(30, 70, "cancelled"))
            .unwrap()
            .unwrap();

        generations.commit(selected.generation);

        assert!(generations
            .prepare_timeline(&timeline_payload(30, 71, "cannot return"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn multipart_timeline_is_prepared_only_after_exact_reassembly() {
        let generations = PresentationGenerations::default();
        let text = "café 日本 ".repeat(40_000);
        let timeline = timeline_envelope(41, 81, &text, PresentationDeliveryPolicy::Ordered, None);
        let encoded = encode_multipart_presentation_timeline(&timeline).unwrap();
        assert!(encoded.len() > MAX_TIMELINE_ENCODED_BYTES);
        let padding = encoded
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'=')
            .count();
        let decoded_bytes = (encoded.len() / 4) * 3 - padding;
        let fragments = encoded
            .as_bytes()
            .chunks(MAX_TIMELINE_ENCODED_BYTES - (MAX_TIMELINE_ENCODED_BYTES % 4))
            .map(|fragment| std::str::from_utf8(fragment).unwrap())
            .collect::<Vec<_>>();

        let mut assembler = MultipartTimelineAssembler::start(&timeline_part_arguments(
            41,
            81,
            0,
            fragments.len(),
            decoded_bytes,
            fragments[0],
        ))
        .unwrap();
        assert!(!assembler.is_complete());
        for (index, fragment) in fragments.iter().enumerate().skip(1) {
            assembler
                .push(&timeline_part_arguments(
                    41,
                    81,
                    index,
                    fragments.len(),
                    decoded_bytes,
                    fragment,
                ))
                .unwrap();
        }

        let prepared = assembler.finish(&generations).unwrap().unwrap();
        assert_eq!(prepared.generation, 41);
        assert_eq!(prepared.timeline.dispatch_id, 81);
        assert_eq!(prepared.timeline.spans[0].text, text);
        assert_eq!(generations.latest(), 0);
    }

    #[test]
    fn multipart_timeline_rejects_missing_reordered_and_mismatched_parts() {
        let first = timeline_part_arguments(7, 19, 0, 2, 12, "eyJw");
        let mut missing = MultipartTimelineAssembler::start(&first).unwrap();
        assert!(missing
            .push(&timeline_part_arguments(7, 19, 0, 2, 12, "YXJ0"))
            .is_err());

        let mismatched = MultipartTimelineAssembler::start(&first).unwrap();
        assert!(mismatched
            .finish(&PresentationGenerations::default())
            .is_err());

        let mut changed = MultipartTimelineAssembler::start(&first).unwrap();
        assert!(changed
            .push(&timeline_part_arguments(8, 19, 1, 2, 12, "YXJ0"))
            .is_err());
    }
}
