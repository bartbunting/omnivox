//! Explicit output changes use the existing native owner and queue generations.

use super::*;
use omnivox_core::audio_device::{DeviceChange, DeviceOperation, DeviceOutcome};

impl DeviceSink {
    pub(crate) fn device_state(&self) -> DeviceState {
        self.signals.routing.lock().unwrap().clone()
    }

    pub(crate) fn begin_device_change(&self, change: DeviceChange) -> DeviceOutcome {
        self.signals.begin_device_change(change)
    }

    pub(crate) fn cancel_device_change(&self, sequence: u64) -> DeviceOutcome {
        let mut state = self.signals.routing.lock().unwrap();
        match state
            .last_operation
            .as_mut()
            .filter(|op| op.change.sequence == sequence)
        {
            Some(op) if op.outcome == DeviceOutcome::Preparing => {
                op.outcome = DeviceOutcome::Cancelled;
                op.reason = Some("cancelled before publication".into());
                op.outcome
            }
            Some(op) => op.outcome,
            None => DeviceOutcome::Stale,
        }
    }
}

impl Signals {
    fn begin_device_change(&self, change: DeviceChange) -> DeviceOutcome {
        let mut state = self.routing.lock().unwrap();
        let previous = state.last_operation.as_ref();
        if change.sequence == 0
            || change.selection.validate().is_err()
            || self.closed.load(Ordering::Acquire)
        {
            return DeviceOutcome::Rejected;
        }
        if let Some(previous) = previous.filter(|op| op.change.sequence == change.sequence) {
            return if previous.change == change {
                previous.outcome
            } else {
                DeviceOutcome::Stale
            };
        }
        if previous.is_some_and(|op| op.outcome == DeviceOutcome::Preparing) {
            return DeviceOutcome::Busy;
        }
        if previous.map_or(0, |op| op.change.sequence).checked_add(1) != Some(change.sequence)
            || change.expected_revision != state.revision
        {
            return DeviceOutcome::Stale;
        }
        state.last_operation = Some(DeviceOperation {
            change,
            outcome: DeviceOutcome::Preparing,
            reason: None,
        });
        // The queued notification already wakes the owner if this channel is full.
        if matches!(
            self.sender.try_send(Event::Apply),
            Err(mpsc::TrySendError::Disconnected(_))
        ) {
            state.last_operation.as_mut().unwrap().outcome = DeviceOutcome::Rejected;
            state.last_operation.as_mut().unwrap().reason = Some("output owner unavailable".into());
            return DeviceOutcome::Rejected;
        }
        DeviceOutcome::Preparing
    }
}

impl<B: Backend> Owner<B> {
    fn finish_change(&self, change: &DeviceChange, outcome: DeviceOutcome, reason: Option<&str>) {
        let mut state = self.signals.routing.lock().unwrap();
        if let Some(op) = state
            .last_operation
            .as_mut()
            .filter(|op| op.change == *change && op.outcome == DeviceOutcome::Preparing)
        {
            op.outcome = outcome;
            op.reason = reason.map(str::to_owned);
        }
    }

    pub(super) fn apply_pending(&mut self) {
        let change = {
            let state = self.signals.routing.lock().unwrap();
            let Some(op) = state
                .last_operation
                .as_ref()
                .filter(|op| op.outcome == DeviceOutcome::Preparing)
            else {
                return;
            };
            op.change.clone()
        };
        if self.selection == change.selection && self.connection.is_some() {
            self.finish_change(&change, DeviceOutcome::Unchanged, None);
            return;
        }
        let revision = self.signals.revision.load(Ordering::Acquire);
        let prepared = (|| {
            let target = self
                .backend
                .resolve(&change.selection)?
                .ok_or_else(|| AudioError::DeviceNotFound("selected device unavailable".into()))?;
            let (connection, queues) = self.backend.open(&target)?;
            if self.backend.resolve(&change.selection)?.as_ref() != Some(&target) {
                return Err(AudioError::DeviceNotFound(
                    "selected device changed during preparation".into(),
                ));
            }
            Ok((target, connection, queues))
        })();
        let (target, connection, queues) = match prepared {
            Ok(candidate) => candidate,
            Err(error) => {
                tracing::warn!(%error, "Output preparation failed; retaining the previous selection");
                self.finish_change(
                    &change,
                    DeviceOutcome::Rejected,
                    Some("selected device could not be opened"),
                );
                return;
            }
        };
        let mut state = self.signals.routing.lock().unwrap();
        if state
            .last_operation
            .as_ref()
            .is_none_or(|op| op.change != change || op.outcome != DeviceOutcome::Preparing)
        {
            // Cancellation or a newer operation cannot publish this candidate.
            drop(state);
            drop(queues);
            drop(connection);
            return;
        }
        let Some(next_revision) = state.revision.checked_add(1) else {
            drop(state);
            self.finish_change(
                &change,
                DeviceOutcome::Rejected,
                Some("output revision exhausted"),
            );
            return;
        };
        let published = self.control.interrupt_output_if(
            || {
                !self.signals.closed.load(Ordering::Acquire)
                    && self.signals.revision.load(Ordering::Acquire) == revision
            },
            || {
                for (sink, queue) in self.sinks.iter().zip(queues) {
                    sink.replace(Some(queue));
                }
            },
        );
        if published {
            self.selection = change.selection;
            self.device = Some(target.clone());
            state.selection = self.selection.clone();
            state.connected = Some(target);
            state.revision = next_revision;
            state.last_operation.as_mut().unwrap().outcome = DeviceOutcome::Applied;
            drop(state);
            // Native teardown is outside routing and admission locks.
            self.connection = Some(connection);
        } else {
            drop(state);
            drop(connection);
            self.finish_change(
                &change,
                DeviceOutcome::Rejected,
                Some("output changed during preparation"),
            );
        }
    }
}
