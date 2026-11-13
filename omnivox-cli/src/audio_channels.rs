//! Temporary process-wide channel choices at the ordinary admission boundary.

use omnivox_core::state::{ChannelMode, TtsState};
use omnivox_tts::control::ControlResponse;

pub(crate) fn available() -> bool {
    std::env::var_os("OMNIVOX_REMOTE_WORKER").is_none() && !crate::emergency::worker()
}

pub(crate) fn apply(state: &mut TtsState, channel: ChannelMode) -> ControlResponse {
    state.set_process_channel_mode(channel);
    status(state)
}

pub(crate) fn status(state: &TtsState) -> ControlResponse {
    ControlResponse::AudioChannelsV1 {
        speech: state.speech_routing.channel_mode,
        tone: state.tone_routing.channel_mode,
        sound: state.sound_routing.channel_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_channels_cover_all_outputs_without_changing_admitted_work_or_reset_defaults() {
        let mut state = TtsState {
            startup_channel_mode: ChannelMode::Left,
            speech_rate: 310.0,
            pitch_multiplier: 1.4,
            ..TtsState::default()
        };
        state.set_process_channel_mode(ChannelMode::Left);
        let admitted = state.clone();
        let expected = ControlResponse::AudioChannelsV1 {
            speech: ChannelMode::Right,
            tone: ChannelMode::Right,
            sound: ChannelMode::Right,
        };
        assert_eq!(apply(&mut state, ChannelMode::Right), expected);
        assert_eq!(apply(&mut state, ChannelMode::Right), expected);
        assert_eq!(status(&state), expected);
        assert_eq!(admitted.sound_routing.channel_mode, ChannelMode::Left);
        assert_eq!(admitted.tone_routing.channel_mode, ChannelMode::Left);
        assert_eq!(admitted.speech_routing.channel_mode, ChannelMode::Left);
        assert_eq!(state.speech_rate, 310.0);
        assert_eq!(state.pitch_multiplier, 1.4);
        state.reset();
        assert_eq!(
            status(&state),
            ControlResponse::AudioChannelsV1 {
                speech: ChannelMode::Left,
                tone: ChannelMode::Left,
                sound: ChannelMode::Left,
            }
        );
    }
}
