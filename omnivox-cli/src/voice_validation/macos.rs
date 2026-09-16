//! macOS validation owns a private group; launchd reaps orphaned descendants.
use std::ffi::c_void;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn getpgrp() -> i32;
    fn getpid() -> i32;
    fn getpgid(pid: i32) -> i32;
}

#[link(name = "proc")]
unsafe extern "C" {
    // libproc returns a PID count here, unlike proc_listpids' byte count.
    fn proc_listpgrppids(group: i32, buffer: *mut c_void, bytes: i32) -> i32;
    fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut c_void) -> i32;
}

// Darwin sys/resource.h, RUSAGE_INFO_V0. Keep the fixed V0 ABI, including the
// trailing fields; the kernel writes this complete record into our buffer.
#[repr(C)]
#[derive(Default)]
struct Usage {
    uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    package_idle_wakeups: u64,
    interrupt_wakeups: u64,
    pageins: u64,
    wired_size: u64,
    resident_size: u64,
    physical_footprint: u64,
    process_start: u64,
    process_exit: u64,
}

pub fn initialize() -> io::Result<()> {
    // Fail before spawning native work if this host cannot report footprint.
    // SAFETY: getpid has no pointer arguments or side effects.
    usage(unsafe { getpid() }).map(|_| ())
}

pub fn configure(command: &mut Command, _: usize) {
    command.process_group(0);
}

pub fn configure_speech(command: &mut Command) {
    command.process_group(0);
}

fn usage(pid: i32) -> io::Result<Usage> {
    let mut record = Usage::default();
    // SAFETY: the V0 flavor writes exactly the initialized V0 record above.
    if unsafe { proc_pid_rusage(pid, 0, (&raw mut record).cast()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(record)
}

fn in_group(pid: i32, group: i32) -> io::Result<bool> {
    // SAFETY: read-only identity query, with no pointer arguments.
    let actual = unsafe { getpgid(pid) };
    if actual != -1 {
        return Ok(actual == group);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(3) {
        return Ok(false); // ESRCH: exited since the group snapshot.
    }
    Err(error)
}

pub struct Tree {
    group: i32,
    memory: u64,
    killed: bool,
    cleaned: bool,
}
impl Tree {
    pub fn for_speech() -> io::Result<Self> {
        Self::new(0)
    }
    pub fn new(memory: usize) -> io::Result<Self> {
        Ok(Self {
            group: 0,
            memory: memory as u64,
            killed: false,
            cleaned: false,
        })
    }
    pub fn assign(&mut self, child: &Child) -> io::Result<()> {
        self.group = i32::try_from(child.id()).map_err(io::Error::other)?;
        Ok(())
    }
    pub fn check_memory(&self) -> io::Result<()> {
        // Darwin address-space limits can reject the already-mapped worker.
        // Instead sample aggregate physical footprint during the bounded wait.
        // This is a cutoff, not a reservation limit: short spikes can escape a
        // sample. Fail closed if accounting is unavailable or would truncate.
        let mut pids = [0i32; 256];
        // SAFETY: the buffer and byte length describe the same writable array.
        let count = unsafe {
            proc_listpgrppids(
                self.group,
                pids.as_mut_ptr().cast(),
                std::mem::size_of_val(&pids) as i32,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count as usize >= pids.len() {
            return Err(io::Error::other(
                "validation process accounting exceeded its bound",
            ));
        }
        let mut total = 0u64;
        for &pid in &pids[..count as usize] {
            if pid <= 0 || !in_group(pid, self.group)? {
                continue;
            }
            let record = match usage(pid) {
                Ok(record) => record,
                Err(_) if !in_group(pid, self.group)? => continue,
                Err(error) => return Err(error),
            };
            // Ignore a PID that exited or was reused outside our private group
            // during the query. The unreaped leader prevents group-ID reuse.
            if !in_group(pid, self.group)? {
                continue;
            }
            total = total.saturating_add(record.physical_footprint);
            if total > self.memory {
                return Err(io::Error::other(format!(
                    "validation memory footprint budget exceeded ({total} > {} bytes)",
                    self.memory
                )));
            }
        }
        Ok(())
    }
    pub fn terminate(&mut self) -> io::Result<()> {
        if self.killed || self.cleaned || self.group == 0 {
            return Ok(());
        }
        // SAFETY: signal the private group while its unreaped leader still pins
        // the identity. Never signal it again after reaping begins, even on retry.
        if unsafe { kill(-self.group, 9) } == -1 {
            let error = io::Error::last_os_error();
            // Darwin skips zombies when signalling a group and can return
            // EPERM when no live member remains. Reap our child and observe
            // absence below; EPERM alone never confirms successful cleanup.
            if !matches!(error.raw_os_error(), Some(1 | 3)) {
                return Err(error);
            }
        }
        self.killed = true;
        Ok(())
    }
    pub fn empty(&mut self) -> io::Result<bool> {
        if self.cleaned || self.group == 0 {
            return Ok(true);
        }
        // The caller has reaped the worker; launchd reaps orphaned descendants.
        // Only observe absence. A reused group conservatively blocks cleanup.
        // SAFETY: signal zero is a read-only existence/permission query.
        if unsafe { kill(-self.group, 0) } == 0 {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(1) {
            // An all-zombie group can report EPERM until the system reaps it.
            // A genuinely inaccessible or reused group also remains pending;
            // the caller's existing deadline prevents unbounded waiting.
            return Ok(false);
        }
        if error.raw_os_error() != Some(3) {
            return Err(error);
        }
        self.cleaned = true;
        Ok(true)
    }
}

pub fn parent_closed() -> ! {
    // SAFETY: the internal worker verified that it leads its private group.
    unsafe {
        kill(0, 9);
    }
    std::process::exit(1)
}

pub fn check_worker_group() -> io::Result<()> {
    // SAFETY: both functions read the caller's identity without pointers.
    if unsafe { getpgrp() != getpid() } {
        return Err(io::Error::other(
            "validation worker must lead a private process group",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn zombie_group_remains_pending_until_reaped() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        configure(&mut command, 0);
        let mut child = command.spawn().unwrap();
        let mut tree = Tree::new(0).unwrap();
        tree.assign(&child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        // Observe exit without wait/try_wait: keep a real zombie in the group
        // so Darwin's EPERM response is deterministic rather than timing-based.
        let observed = loop {
            match usage(tree.group) {
                Ok(record) if record.process_exit != 0 => break Ok(()),
                Err(error) => break Err(error),
                _ if Instant::now() >= deadline => {
                    break Err(io::Error::other("fixture did not become a zombie"));
                }
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        let pending = tree.empty();
        let termination = tree.terminate();
        // Clean up even when an observation above failed, before asserting.
        let _ = child.kill();
        child.wait().unwrap();
        observed.unwrap();
        assert!(!pending.unwrap(), "a zombie must not confirm group absence");
        termination.unwrap();
        assert!(tree.empty().unwrap());
    }
}
