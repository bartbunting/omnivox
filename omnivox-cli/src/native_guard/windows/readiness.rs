//! Wait for actual object/operation completion. No periodic healthy-idle wakeup.
use super::os::{self, Control, Listener};
use super::wire::{Channel, WaitFor, TIMEOUT};
use super::{Driver, TICK};
use std::io;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::time::Instant;

pub(super) fn wait(
    driver: &Driver,
    parent: &Channel<Control>,
    client: Option<&(Channel<Control>, bool, Instant)>,
    listener: &Listener,
) -> io::Result<()> {
    let mut events = Vec::with_capacity(10);
    let mut deadline = (driver.closing && !driver.tree_retired()).then(|| Instant::now() + TICK);
    if let Some(native) = &driver.native {
        if native.readiness(&mut events, &mut deadline)? {
            return Ok(());
        }
    }
    if !driver.closing || !driver.io_retired {
        events.push(driver.parent.as_raw_handle());
    }
    if !driver.io_retired {
        events.push(driver.completion.as_raw_handle());
    }
    if channel(parent, &mut events, &mut deadline) {
        return Ok(());
    }
    if let Some((channel_, authenticated, since)) = client {
        if channel(channel_, &mut events, &mut deadline) {
            return Ok(());
        }
        if !authenticated && !channel_.closed {
            earlier(&mut deadline, *since + TIMEOUT);
        }
    } else if let Some(event) = listener.event() {
        events.push(event);
    } else {
        return Ok(());
    } // A new listener must issue accept on this thread.
    os::native::wait(&events, os::millis(deadline))
}
pub(super) fn earlier(deadline: &mut Option<Instant>, candidate: Instant) {
    *deadline = Some(deadline.map_or(candidate, |old| old.min(candidate)));
}
pub(super) fn channel(
    channel: &Channel<Control>,
    events: &mut Vec<RawHandle>,
    deadline: &mut Option<Instant>,
) -> bool {
    match channel.wait_for() {
        WaitFor::Closed => channel.stream.closing_events(events),
        WaitFor::Buffered => return true,
        WaitFor::Write => match channel.stream.write_event() {
            Some(event) => events.push(event),
            None => return true,
        },
        WaitFor::Read(expiry) => {
            match channel.stream.read_event() {
                Some(event) => events.push(event),
                None => return true,
            }
            if let Some(expiry) = expiry {
                earlier(deadline, expiry);
            }
        }
    }
    false
}
