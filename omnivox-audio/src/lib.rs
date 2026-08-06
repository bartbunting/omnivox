//! Omnivox Audio Library
//!
//! Audio processing pipeline for the Omnivox speech server.
//! Provides buffer management, tone generation, file loading,
//! post-processing effects, and audio output.

pub mod buffer;
pub mod effects;
pub mod loader;
pub mod output;
pub mod pipeline;
pub mod timeline;
pub mod tone;

pub use buffer::AudioBuffer;
pub use effects::{ChannelRouter, SilenceTrimReport, SilenceTrimmer, VolumeAdjust};
pub use loader::{
    AudioFileLoader, MAX_AUDIO_CACHE_ENTRIES, MAX_AUDIO_CACHE_SAMPLES,
    MAX_AUDIO_DURATION_SECS, MAX_AUDIO_FILE_BYTES,
};
pub use output::{
    AudioControl, AudioOutput, AudioStreams, PlaybackCue, PlaybackStatus, PlaybackTicket,
    StreamType,
};
pub use pipeline::{AudioEffect, AudioPipeline};
pub use timeline::{
    PreparedAudioResource, RenderedSemanticEvent, RenderedTimelineWindow, TimelineAudioRenderer,
    MAX_TIMELINE_ACTIONS_PER_WINDOW, MAX_TIMELINE_RENDER_FRAMES,
};
pub use tone::ToneGenerator;

use thiserror::Error;

/// Audio processing errors
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("Invalid buffer format: {0}")]
    InvalidFormat(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Decode error: {0}")]
    DecodeError(String),

    #[error("Playback error: {0}")]
    PlaybackError(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Effect error: {0}")]
    EffectError(String),

    #[error("Timeline render error: {0}")]
    TimelineError(String),
}
