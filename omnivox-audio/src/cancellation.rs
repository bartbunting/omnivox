//! Cooperative cancellation shared by synthesis and playback.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Cloneable cooperative-cancellation signal for one logical request.
///
/// A request owner retains one clone and calls [`Self::cancel`] when the work
/// is superseded. Producers and consumers may check [`Self::is_cancelled`] at
/// safe interruption points so one cancellation lifetime can cover synthesis,
/// queued playback, and active playback.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create an active cancellation signal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Permanently mark this request as cancelled.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Return whether the request has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Return whether both handles refer to the same cancellation lifetime.
    pub fn same_token(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared_and_permanent() {
        let token = CancellationToken::new();
        let clone = token.clone();
        let other = CancellationToken::new();

        assert!(token.same_token(&clone));
        assert!(!token.same_token(&other));
        assert!(!clone.is_cancelled());

        token.cancel();

        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
        assert!(!other.is_cancelled());
    }
}
