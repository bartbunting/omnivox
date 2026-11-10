//! Sleep on lifecycle readiness, never on speech admission or forwarding.
use super::os::{Control, Listener};
use super::wire::{Channel, WaitFor, TIMEOUT};
use super::{Driver, TICK};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Instant;

pub(super) fn wait(
    driver: &Driver,
    parent: &Channel<Control>,
    client: Option<&(Channel<Control>, bool, Instant)>,
    listener: &Listener,
) -> io::Result<()> {
    let mut descriptors = Vec::with_capacity(5);
    let mut deadline = (driver.closing && !driver.tree_retired()).then(|| Instant::now() + TICK);
    if !driver.closing {
        add(&mut descriptors, driver.parent.as_raw_fd(), libc::POLLIN);
    }
    if !driver.io_retired {
        add(
            &mut descriptors,
            driver.completion.as_raw_fd(),
            libc::POLLIN,
        );
    }
    if channel(parent, &mut descriptors, &mut deadline) {
        return Ok(());
    }
    if let Some((stream, authenticated, since)) = client {
        if channel(stream, &mut descriptors, &mut deadline) {
            return Ok(());
        }
        if !authenticated {
            earlier(&mut deadline, *since + TIMEOUT);
        }
    } else {
        add(&mut descriptors, listener.as_raw_fd(), libc::POLLIN);
    }
    let timeout = deadline.map_or(-1, |deadline| {
        let duration = deadline.saturating_duration_since(Instant::now());
        duration
            .as_millis()
            .saturating_add(u128::from(
                !duration.subsec_nanos().is_multiple_of(1_000_000),
            ))
            .min(i32::MAX as u128) as i32
    });
    // SAFETY: all descriptors are borrowed from live owners on this single
    // thread; the vector cannot move or be changed during this blocking call.
    let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, timeout) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

fn add(descriptors: &mut Vec<libc::pollfd>, fd: RawFd, events: i16) {
    descriptors.push(libc::pollfd {
        fd,
        events,
        revents: 0,
    });
}

fn earlier(deadline: &mut Option<Instant>, candidate: Instant) {
    *deadline = Some(deadline.map_or(candidate, |old| old.min(candidate)));
}

fn channel(
    channel: &Channel<Control>,
    descriptors: &mut Vec<libc::pollfd>,
    deadline: &mut Option<Instant>,
) -> bool {
    match channel.wait_for() {
        WaitFor::Closed => (),
        WaitFor::Buffered => return true,
        WaitFor::Write => add(descriptors, channel.stream.as_raw_fd(), libc::POLLOUT),
        WaitFor::Read(expiry) => {
            add(descriptors, channel.stream.as_raw_fd(), libc::POLLIN);
            if let Some(expiry) = expiry {
                earlier(deadline, expiry);
            }
        }
    }
    false
}
