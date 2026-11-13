//! Keep canonical PCM metadata stable across Rodio queue boundaries.

use crate::buffer::{CHANNELS, SAMPLE_RATE};
use rodio::mixer::Mixer;
use rodio::queue::SourcesQueueOutput;
use rodio::{ChannelCount, Player, SampleRate, Source};
use std::time::Duration;

/// A queue containing only Omnivox's 44.1 kHz stereo sources.
///
/// Rodio 0.19's empty queue reports mono until `next()` switches from its
/// silence filler to a queued source. Its mixer reads that stale format first,
/// interpreting the first 512 stereo samples as mono and doubling their length.
/// Keep the canonical format at the queue boundary, including during silence;
/// the device mixer can then convert this continuous stream to its own format.
pub(crate) struct CanonicalQueue(SourcesQueueOutput);

impl CanonicalQueue {
    pub(crate) fn new(queue: SourcesQueueOutput) -> Self {
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
    fn current_span_len(&self) -> Option<usize> {
        // Neither source transitions nor idle silence change the PCM format.
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(CHANNELS).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(SAMPLE_RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub(crate) fn new_idle_sink() -> (Player, CanonicalQueue) {
    let (sink, queue) = Player::new();
    (sink, CanonicalQueue::new(queue))
}

pub(crate) fn new_sink(mixer: &Mixer) -> Player {
    let (sink, queue) = new_idle_sink();
    mixer.add(queue);
    sink
}
