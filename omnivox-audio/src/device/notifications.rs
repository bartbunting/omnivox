//! Opaque callback cookies: native callbacks never dereference Rust storage.
//! Removing a registration invalidates delayed callbacks, including when native
//! unregister fails. Cookies are never reused during the process lifetime.

use super::{Event, Signals};
use crate::AudioError;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

struct Entry {
    signals: Weak<Signals>,
    default_changed: bool,
}

fn entries() -> &'static Mutex<HashMap<usize, Entry>> {
    static ENTRIES: OnceLock<Mutex<HashMap<usize, Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(Mutex::default)
}

pub(super) struct Registration(pub(super) usize);

impl Registration {
    pub(super) fn new(signals: &Arc<Signals>, default_changed: bool) -> Result<Self, AudioError> {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| AudioError::PlaybackError("native callback identity exhausted".into()))?;
        entries().lock().unwrap().insert(
            id,
            Entry {
                signals: Arc::downgrade(signals),
                default_changed,
            },
        );
        Ok(Self(id))
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        entries().lock().unwrap().remove(&self.0);
    }
}

pub(super) fn notify(id: usize) {
    let target = entries()
        .lock()
        .unwrap()
        .get(&id)
        .and_then(|entry| Some((entry.signals.upgrade()?, entry.default_changed)));
    if let Some((signals, default_changed)) = target {
        signals.notify(if default_changed {
            Event::DefaultChanged
        } else {
            Event::Rescan
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_core::audio_device::{DeviceSelection, DeviceState};
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::mpsc;

    fn signals() -> (Arc<Signals>, mpsc::Receiver<Event>) {
        let (sender, receiver) = mpsc::sync_channel(super::super::EVENT_CAPACITY);
        (
            Arc::new(Signals {
                sender,
                revision: AtomicU64::new(0),
                overflow: AtomicBool::new(false),
                retry_requested: AtomicBool::new(false),
                closed: AtomicBool::new(false),
                routing: Mutex::new(DeviceState::new(DeviceSelection::SystemDefault)),
            }),
            receiver,
        )
    }

    #[test]
    fn callbacks_route_events_and_expired_cookies_cannot_reach_a_new_owner() {
        let (signals, receiver) = signals();
        let default = Registration::new(&signals, true).unwrap();
        let devices = Registration::new(&signals, false).unwrap();
        notify(default.0);
        notify(devices.0);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Event::DefaultChanged
        ));
        assert!(matches!(receiver.try_recv().unwrap(), Event::Rescan));
        let expired = default.0;
        drop(default);
        let new = Registration::new(&signals, true).unwrap();
        assert_ne!(expired, new.0);
        notify(expired);
        assert!(receiver.try_recv().is_err());
        signals.closed.store(true, Ordering::Release);
        notify(new.0);
        notify(devices.0);
        assert!(receiver.try_recv().is_err());
        drop(signals);
        notify(new.0);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn callback_overflow_is_bounded_and_requests_a_rescan() {
        let (signals, receiver) = signals();
        let registration = Registration::new(&signals, false).unwrap();
        for _ in 0..100 {
            notify(registration.0);
        }
        assert!(signals.overflow.load(Ordering::Acquire));
        assert_eq!(receiver.try_iter().count(), super::super::EVENT_CAPACITY);
    }
}
