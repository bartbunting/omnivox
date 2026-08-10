//! Omnivox Core Library
//!
//! Core types and functionality for the Omnivox speech server.
//! This includes command parsing, queue management, and state handling.

pub mod command;
pub mod queue;
pub mod state;
pub mod timeline;

pub use command::{
    parse_command, parse_presentation_tone_arguments, parse_tone_arguments, Command, CommandId,
    ParseError, PresentationToneCommand, ToneCommand, DEPRECATED_PROTOCOL_COMMANDS,
};
pub use queue::{CommandQueue, QueueItem, TonePlacement};
pub use state::{AudioRouting, ChannelMode, PunctuationLevel, TtsState};
pub use timeline::{
    ActionAffinity, AudioActionMode, EffectBus, EffectStateChange, EffectStateId, FrameMap,
    PresentationPosition, ResolvedTimelineAction, ScheduledTimeline, TimelineAction,
    TimelineActionId, TimelineActionKind, TimelineError,
};

/// Initialize logging with default settings
pub fn init_logging() {
    use tracing_subscriber::fmt::format::FmtSpan;

    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .init();
}
