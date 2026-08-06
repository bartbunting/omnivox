//! Atomic validation and generation tracking for framed presentations.

use omnivox_core::state::{ChannelMode, PunctuationLevel};
use omnivox_core::{parse_command, Command, CommandId};
use omnivox_tts::presentation::decode_presentation_frame;

use crate::text::parse_resource_path;

const MAX_PRESENTATION_COMMANDS: usize = 4096;

#[derive(Debug)]
pub struct PreparedPresentation {
    pub generation: u64,
    pub commands: Vec<Command>,
}

#[derive(Debug, Default)]
pub struct PresentationGenerations {
    latest: u64,
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

    pub fn commit(&mut self, generation: u64) {
        self.latest = self.latest.max(generation);
    }

    #[cfg(test)]
    fn latest(&self) -> u64 {
        self.latest
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
        CommandId::TtsSplitCaps | CommandId::TtsAllCapsBeep => {
            matches!(arguments, Some("0" | "1"))
        }
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
        | CommandId::EmacsvoxTrackedDispatch
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
    use omnivox_tts::presentation::encode_presentation_script;

    use super::*;

    fn arguments(generation: u64, script: &str) -> String {
        format!(
            "{generation} {{{}}}",
            encode_presentation_script(script).unwrap()
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
}
