//! Omnivox Core Library
//!
//! Core types and functionality for the Omnivox speech server.
//! This includes command parsing, queue management, and state handling.

pub mod command;
pub mod queue;
pub mod state;

pub use command::{parse_command, Command, CommandId, ParseError};
pub use queue::{CommandQueue, QueueItem};
pub use state::{AudioRouting, ChannelMode, PunctuationLevel, TtsState};

/// Initialize logging with default settings
pub fn init_logging() {
    use tracing_subscriber::fmt::format::FmtSpan;

    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .init();
}
