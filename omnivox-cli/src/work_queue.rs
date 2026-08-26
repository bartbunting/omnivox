//! Bounded, nonblocking handoff from the protocol loop to synthesis.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// Resource limits for work waiting behind the active synthesis request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkQueueLimits {
    pub(crate) max_items: usize,
    pub(crate) max_payload_bytes: usize,
}

/// Metadata needed to bound and selectively coalesce queued work.
pub(crate) trait BoundedWork {
    fn queued_payload_bytes(&self) -> usize;
    fn generation(&self) -> u64;
    fn is_replaceable(&self) -> bool;
    fn shares_replacement_domain(&self, other: &Self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetirementReason {
    Replaced,
    EvictedForCapacity,
    StaleGeneration,
    Saturated,
    ReceiverClosed,
}

pub(crate) struct RetiredWork<T> {
    pub(crate) work: T,
    pub(crate) reason: RetirementReason,
}

pub(crate) struct EnqueueOutcome<T> {
    pub(crate) accepted: bool,
    pub(crate) retired: Vec<RetiredWork<T>>,
}

struct QueuedWork<T> {
    payload_bytes: usize,
    work: T,
}

struct QueueState<T> {
    entries: VecDeque<QueuedWork<T>>,
    payload_bytes: usize,
    sender_closed: bool,
    receiver_alive: bool,
}

struct SharedQueue<T> {
    state: Mutex<QueueState<T>>,
    available: Condvar,
    limits: WorkQueueLimits,
}

/// Single-producer queue handle. Admission never waits for the consumer.
pub(crate) struct WorkQueueSender<T> {
    shared: Arc<SharedQueue<T>>,
}

/// Single-consumer queue handle used by the synthesis worker.
pub(crate) struct WorkQueueReceiver<T> {
    shared: Arc<SharedQueue<T>>,
}

pub(crate) fn bounded_work_queue<T>(
    limits: WorkQueueLimits,
) -> (WorkQueueSender<T>, WorkQueueReceiver<T>) {
    assert!(
        limits.max_items > 0,
        "work queue item limit must be positive"
    );
    assert!(
        limits.max_payload_bytes > 0,
        "work queue payload limit must be positive"
    );
    let shared = Arc::new(SharedQueue {
        state: Mutex::new(QueueState {
            entries: VecDeque::new(),
            payload_bytes: 0,
            sender_closed: false,
            receiver_alive: true,
        }),
        available: Condvar::new(),
        limits,
    });
    (
        WorkQueueSender {
            shared: Arc::clone(&shared),
        },
        WorkQueueReceiver { shared },
    )
}

impl<T: BoundedWork> WorkQueueSender<T> {
    /// Admit work without blocking, coalescing or evicting queued replaceable
    /// work only when the incoming request can be committed atomically.
    pub(crate) fn try_send(&self, work: T) -> EnqueueOutcome<T> {
        self.try_send_with_commit(work, |_| {})
    }

    /// Admit work and run `commit` after it is queued but before the consumer
    /// can observe it.
    ///
    /// The callback is not run when admission fails. It may therefore commit
    /// side effects that must occur atomically with successful queue admission.
    pub(crate) fn try_send_with_commit(
        &self,
        work: T,
        commit: impl FnOnce(&mut T),
    ) -> EnqueueOutcome<T> {
        let payload_bytes = work.queued_payload_bytes();
        let mut state = self.shared.state.lock().unwrap();
        if !state.receiver_alive {
            return rejected(work, RetirementReason::ReceiverClosed);
        }
        if payload_bytes > self.shared.limits.max_payload_bytes {
            return rejected(work, RetirementReason::Saturated);
        }

        let mut retire = vec![false; state.entries.len()];
        let mut projected_items = state.entries.len().saturating_add(1);
        let mut projected_bytes = state.payload_bytes.saturating_add(payload_bytes);

        // A newer request in the same replacement domain supersedes every
        // older queued member, but only if the newer request can be admitted.
        for (index, queued) in state.entries.iter().enumerate() {
            if work.shares_replacement_domain(&queued.work) {
                retire[index] = true;
                projected_items = projected_items.saturating_sub(1);
                projected_bytes = projected_bytes.saturating_sub(queued.payload_bytes);
            }
        }

        // Capacity pressure may discard other replaceable navigation in FIFO
        // order. Ordered and urgent work is never selected for eviction.
        if exceeds_limits(projected_items, projected_bytes, self.shared.limits) {
            for (index, queued) in state.entries.iter().enumerate() {
                if !retire[index] && queued.work.is_replaceable() {
                    retire[index] = true;
                    projected_items = projected_items.saturating_sub(1);
                    projected_bytes = projected_bytes.saturating_sub(queued.payload_bytes);
                    if !exceeds_limits(projected_items, projected_bytes, self.shared.limits) {
                        break;
                    }
                }
            }
        }

        if exceeds_limits(projected_items, projected_bytes, self.shared.limits) {
            return rejected(work, RetirementReason::Saturated);
        }

        let mut retired = Vec::new();
        let mut kept = VecDeque::with_capacity(projected_items.saturating_sub(1));
        for (index, queued) in state.entries.drain(..).enumerate() {
            if retire[index] {
                let reason = if work.shares_replacement_domain(&queued.work) {
                    RetirementReason::Replaced
                } else {
                    RetirementReason::EvictedForCapacity
                };
                retired.push(RetiredWork {
                    work: queued.work,
                    reason,
                });
            } else {
                kept.push_back(queued);
            }
        }
        state.entries = kept;
        state.payload_bytes = projected_bytes.saturating_sub(payload_bytes);
        state.entries.push_back(QueuedWork {
            payload_bytes,
            work,
        });
        commit(
            &mut state
                .entries
                .back_mut()
                .expect("admitted work must be present in the queue")
                .work,
        );
        state.payload_bytes = projected_bytes;
        drop(state);
        self.shared.available.notify_one();

        EnqueueOutcome {
            accepted: true,
            retired,
        }
    }

    /// Remove queued requests made obsolete by an interrupt generation.
    pub(crate) fn retire_before_generation(&self, minimum: u64) -> Vec<RetiredWork<T>> {
        let mut state = self.shared.state.lock().unwrap();
        let mut retired = Vec::new();
        let mut kept = VecDeque::with_capacity(state.entries.len());
        let mut kept_bytes = 0usize;
        while let Some(queued) = state.entries.pop_front() {
            if queued.work.generation() < minimum {
                retired.push(RetiredWork {
                    work: queued.work,
                    reason: RetirementReason::StaleGeneration,
                });
            } else {
                kept_bytes = kept_bytes.saturating_add(queued.payload_bytes);
                kept.push_back(queued);
            }
        }
        state.entries = kept;
        state.payload_bytes = kept_bytes;
        retired
    }
}

impl<T> Drop for WorkQueueSender<T> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.sender_closed = true;
        drop(state);
        self.shared.available.notify_all();
    }
}

impl<T> WorkQueueReceiver<T> {
    pub(crate) fn recv(&self) -> Option<T> {
        let mut state = self.shared.state.lock().unwrap();
        loop {
            if let Some(queued) = state.entries.pop_front() {
                state.payload_bytes = state.payload_bytes.saturating_sub(queued.payload_bytes);
                return Some(queued.work);
            }
            if state.sender_closed {
                return None;
            }
            state = self.shared.available.wait(state).unwrap();
        }
    }
}

impl<T> Drop for WorkQueueReceiver<T> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.receiver_alive = false;
        drop(state);
        self.shared.available.notify_all();
    }
}

fn exceeds_limits(items: usize, bytes: usize, limits: WorkQueueLimits) -> bool {
    items > limits.max_items || bytes > limits.max_payload_bytes
}

fn rejected<T>(work: T, reason: RetirementReason) -> EnqueueOutcome<T> {
    EnqueueOutcome {
        accepted: false,
        retired: vec![RetiredWork { work, reason }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct FakeWork {
        id: usize,
        bytes: usize,
        generation: u64,
        domain: Option<&'static str>,
    }

    impl FakeWork {
        fn ordered(id: usize, bytes: usize) -> Self {
            Self {
                id,
                bytes,
                generation: 1,
                domain: None,
            }
        }

        fn replaceable(id: usize, bytes: usize, domain: &'static str) -> Self {
            Self {
                id,
                bytes,
                generation: 1,
                domain: Some(domain),
            }
        }

        fn at_generation(mut self, generation: u64) -> Self {
            self.generation = generation;
            self
        }
    }

    impl BoundedWork for FakeWork {
        fn queued_payload_bytes(&self) -> usize {
            self.bytes
        }

        fn generation(&self) -> u64 {
            self.generation
        }

        fn is_replaceable(&self) -> bool {
            self.domain.is_some()
        }

        fn shares_replacement_domain(&self, other: &Self) -> bool {
            self.domain.is_some() && self.domain == other.domain
        }
    }

    fn queue(
        max_items: usize,
        max_payload_bytes: usize,
    ) -> (WorkQueueSender<FakeWork>, WorkQueueReceiver<FakeWork>) {
        bounded_work_queue(WorkQueueLimits {
            max_items,
            max_payload_bytes,
        })
    }

    #[test]
    fn exact_item_and_payload_limits_are_accepted() {
        let (sender, receiver) = queue(2, 10);

        assert!(sender.try_send(FakeWork::ordered(1, 4)).accepted);
        assert!(sender.try_send(FakeWork::ordered(2, 6)).accepted);
        let rejected = sender.try_send(FakeWork::ordered(3, 1));

        assert!(!rejected.accepted);
        assert_eq!(rejected.retired.len(), 1);
        assert_eq!(rejected.retired[0].reason, RetirementReason::Saturated);
        assert_eq!(receiver.recv().unwrap().id, 1);
        assert_eq!(receiver.recv().unwrap().id, 2);
    }

    #[test]
    fn one_request_larger_than_the_byte_limit_is_rejected() {
        let (sender, _receiver) = queue(2, 10);

        let rejected = sender.try_send(FakeWork::ordered(1, 11));

        assert!(!rejected.accepted);
        assert_eq!(rejected.retired[0].work.id, 1);
        assert_eq!(rejected.retired[0].reason, RetirementReason::Saturated);
    }

    #[test]
    fn admission_commit_runs_only_for_accepted_work() {
        let (sender, _receiver) = queue(1, 10);
        let commits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepted_commits = Arc::clone(&commits);

        let accepted = sender.try_send_with_commit(FakeWork::ordered(1, 10), move |_| {
            accepted_commits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let rejected_commits = Arc::clone(&commits);
        let rejected = sender.try_send_with_commit(FakeWork::ordered(2, 1), move |_| {
            rejected_commits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        assert!(accepted.accepted);
        assert!(!rejected.accepted);
        assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn same_domain_replacement_is_atomic_and_keeps_the_newest_request() {
        let (sender, receiver) = queue(2, 10);
        assert!(
            sender
                .try_send(FakeWork::replaceable(1, 4, "navigation"))
                .accepted
        );

        let replacement = sender.try_send(FakeWork::replaceable(2, 6, "navigation"));

        assert!(replacement.accepted);
        assert_eq!(replacement.retired.len(), 1);
        assert_eq!(replacement.retired[0].work.id, 1);
        assert_eq!(replacement.retired[0].reason, RetirementReason::Replaced);
        assert_eq!(receiver.recv().unwrap().id, 2);
    }

    #[test]
    fn capacity_pressure_evicts_replaceable_work_before_ordered_work() {
        let (sender, receiver) = queue(2, 10);
        assert!(
            sender
                .try_send(FakeWork::replaceable(1, 4, "navigation"))
                .accepted
        );
        assert!(sender.try_send(FakeWork::ordered(2, 4)).accepted);

        let incoming = sender.try_send(FakeWork::ordered(3, 6));

        assert!(incoming.accepted);
        assert_eq!(incoming.retired.len(), 1);
        assert_eq!(incoming.retired[0].work.id, 1);
        assert_eq!(
            incoming.retired[0].reason,
            RetirementReason::EvictedForCapacity
        );
        assert_eq!(receiver.recv().unwrap().id, 2);
        assert_eq!(receiver.recv().unwrap().id, 3);
    }

    #[test]
    fn failed_replacement_does_not_discard_admitted_work() {
        let (sender, receiver) = queue(2, 10);
        assert!(sender.try_send(FakeWork::ordered(1, 8)).accepted);
        assert!(
            sender
                .try_send(FakeWork::replaceable(2, 2, "navigation"))
                .accepted
        );

        let rejected = sender.try_send(FakeWork::replaceable(3, 3, "navigation"));

        assert!(!rejected.accepted);
        assert_eq!(rejected.retired.len(), 1);
        assert_eq!(rejected.retired[0].work.id, 3);
        assert_eq!(receiver.recv().unwrap().id, 1);
        assert_eq!(receiver.recv().unwrap().id, 2);
    }

    #[test]
    fn interrupt_retires_only_older_generations() {
        let (sender, receiver) = queue(4, 20);
        assert!(
            sender
                .try_send(FakeWork::ordered(1, 4).at_generation(1))
                .accepted
        );
        assert!(
            sender
                .try_send(FakeWork::ordered(2, 4).at_generation(2))
                .accepted
        );

        let retired = sender.retire_before_generation(2);

        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].work.id, 1);
        assert_eq!(retired[0].reason, RetirementReason::StaleGeneration);
        assert_eq!(receiver.recv().unwrap().id, 2);
    }

    #[test]
    fn interrupt_frees_a_full_ordered_queue_for_current_work() {
        let (sender, receiver) = queue(2, 8);
        assert!(
            sender
                .try_send(FakeWork::ordered(1, 4).at_generation(1))
                .accepted
        );
        assert!(
            sender
                .try_send(FakeWork::ordered(2, 4).at_generation(1))
                .accepted
        );

        let retired = sender.retire_before_generation(2);
        let admitted = sender.try_send(FakeWork::ordered(3, 8).at_generation(2));

        assert_eq!(retired.len(), 2);
        assert!(admitted.accepted);
        assert_eq!(receiver.recv().unwrap().id, 3);
    }

    #[test]
    fn receiver_drains_admitted_work_after_sender_closes() {
        let (sender, receiver) = queue(2, 10);
        assert!(sender.try_send(FakeWork::ordered(1, 4)).accepted);
        drop(sender);

        assert_eq!(receiver.recv().unwrap().id, 1);
        assert!(receiver.recv().is_none());
    }

    #[test]
    fn sender_rejects_work_after_the_receiver_closes() {
        let (sender, receiver) = queue(2, 10);
        drop(receiver);

        let rejected = sender.try_send(FakeWork::ordered(1, 4));

        assert!(!rejected.accepted);
        assert_eq!(rejected.retired[0].work.id, 1);
        assert_eq!(rejected.retired[0].reason, RetirementReason::ReceiverClosed);
    }

    #[test]
    fn producer_and_consumer_make_progress_under_sustained_pressure() {
        let (sender, receiver) = queue(4, 16);
        let consumer = std::thread::spawn(move || {
            let mut received = Vec::new();
            while let Some(work) = receiver.recv() {
                received.push(work.id);
            }
            received
        });

        for id in 0..1_000 {
            let mut work = FakeWork::ordered(id, 4);
            loop {
                let mut outcome = sender.try_send(work);
                if outcome.accepted {
                    break;
                }
                work = outcome.retired.pop().unwrap().work;
                std::thread::yield_now();
            }
        }
        drop(sender);

        assert_eq!(consumer.join().unwrap(), (0..1_000).collect::<Vec<_>>());
    }
}
