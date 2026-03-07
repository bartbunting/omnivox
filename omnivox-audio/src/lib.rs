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
pub mod tone;

pub use buffer::AudioBuffer;
pub use effects::{ChannelRouter, SilenceTrimmer, VolumeAdjust};
pub use loader::AudioFileLoader;
pub use output::{AudioControl, AudioOutput, AudioStreams, StreamType};
pub use pipeline::{AudioEffect, AudioPipeline};
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
}
