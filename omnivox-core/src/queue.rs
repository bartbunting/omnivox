//! Command Queue Management
//!
//! Manages queuing and dispatching of Emacspeak commands.

use std::collections::VecDeque;
use std::path::PathBuf;

/// How a queued tone participates in presentation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TonePlacement {
    /// Play immediately on the independent tone stream.
    Independent,
    /// Serialize with speech and advance the primary presentation clock.
    Insert,
    /// Start at the current presentation boundary without advancing speech.
    Overlay,
}

/// Items that can be queued for processing
#[derive(Debug, Clone, PartialEq)]
pub enum QueueItem {
    /// Text to be spoken
    Speech(String),

    /// Inline codes (voice changes, pitch adjustments, etc.)
    Code(String),

    /// Tone with frequency (Hz), duration (ms), and explicit clock placement.
    Tone {
        frequency: f32,
        duration: u32,
        placement: TonePlacement,
    },

    /// Silence/pause for specified duration (ms)
    Silence { duration: u32 },

    /// Audio icon/sound file to play
    AudioIcon { path: PathBuf },
}

/// Command queue system
///
/// Manages separate queues for different command types and processes them
/// sequentially on dispatch.
#[derive(Debug, Default)]
pub struct CommandQueue {
    items: VecDeque<QueueItem>,
}

impl CommandQueue {
    /// Create a new empty command queue
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
        }
    }

    /// Enqueue an item
    pub fn enqueue(&mut self, item: QueueItem) {
        self.items.push_back(item);
    }

    /// Get the number of queued items
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Dispatch (retrieve and remove) all queued items
    ///
    /// Returns all items in FIFO order and clears the queue.
    pub fn dispatch(&mut self) -> Vec<QueueItem> {
        self.items.drain(..).collect()
    }

    /// Clear the queue without returning items
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Peek at the next item without removing it
    pub fn peek(&self) -> Option<&QueueItem> {
        self.items.front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_queue_is_empty() {
        let queue = CommandQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_enqueue_speech() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Speech("Hello".to_string()));
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());
    }

    #[test]
    fn test_enqueue_multiple_items() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Speech("Hello".to_string()));
        queue.enqueue(QueueItem::Code("[{voice en-US:Alex}]".to_string()));
        queue.enqueue(QueueItem::Speech("World".to_string()));
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn test_dispatch_returns_fifo() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Speech("First".to_string()));
        queue.enqueue(QueueItem::Speech("Second".to_string()));
        queue.enqueue(QueueItem::Speech("Third".to_string()));

        let items = queue.dispatch();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], QueueItem::Speech("First".to_string()));
        assert_eq!(items[1], QueueItem::Speech("Second".to_string()));
        assert_eq!(items[2], QueueItem::Speech("Third".to_string()));
    }

    #[test]
    fn test_dispatch_clears_queue() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Speech("Test".to_string()));
        queue.dispatch();
        assert!(queue.is_empty());
    }

    #[test]
    fn test_clear() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Speech("Test".to_string()));
        queue.clear();
        assert!(queue.is_empty());
    }

    #[test]
    fn test_peek() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Speech("First".to_string()));
        queue.enqueue(QueueItem::Speech("Second".to_string()));

        let peeked = queue.peek();
        assert_eq!(peeked, Some(&QueueItem::Speech("First".to_string())));
        assert_eq!(queue.len(), 2); // Peek doesn't remove
    }

    #[test]
    fn test_peek_empty_queue() {
        let queue = CommandQueue::new();
        assert_eq!(queue.peek(), None);
    }

    #[test]
    fn test_queue_tone() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Tone {
            frequency: 440.0,
            duration: 50,
            placement: TonePlacement::Independent,
        });
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn test_queue_silence() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::Silence { duration: 100 });
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn test_queue_audio_icon() {
        let mut queue = CommandQueue::new();
        queue.enqueue(QueueItem::AudioIcon {
            path: PathBuf::from("/sounds/beep.wav"),
        });
        assert_eq!(queue.len(), 1);
    }
}
