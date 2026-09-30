//! Keep canonical PCM metadata stable across Rodio queue boundaries.

use crate::buffer::{CHANNELS, SAMPLE_RATE};
use rodio::queue::SourcesQueueOutput;
use rodio::{OutputStreamHandle, PlayError, Sink, Source};
use std::time::Duration;

/// A queue containing only Omnivox's 44.1 kHz stereo sources.
///
/// Rodio 0.19's empty queue reports mono until `next()` switches from its
/// silence filler to a queued source. Its mixer reads that stale format first,
/// interpreting the first 512 stereo samples as mono and doubling their length.
/// Keep the canonical format at the queue boundary, including during silence;
/// the device mixer can then convert this continuous stream to its own format.
pub(crate) struct CanonicalQueue(SourcesQueueOutput<f32>);

impl CanonicalQueue {
    pub(crate) fn new(queue: SourcesQueueOutput<f32>) -> Self {
        Self(queue)
    }
}

impl Iterator for CanonicalQueue {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl Source for CanonicalQueue {
    fn current_frame_len(&self) -> Option<usize> {
        // Neither source transitions nor idle silence change the PCM format.
        None
    }

    fn channels(&self) -> u16 {
        CHANNELS
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub(crate) fn new_idle_sink() -> (Sink, CanonicalQueue) {
    let (sink, queue) = Sink::new_idle();
    (sink, CanonicalQueue::new(queue))
}

pub(crate) fn new_sink(handle: &OutputStreamHandle) -> Result<Sink, PlayError> {
    let (sink, queue) = new_idle_sink();
    handle.play_raw(queue)?;
    Ok(sink)
}
