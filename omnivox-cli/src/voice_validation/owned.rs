//! One owned worker tree. A new probe is admitted only after cleanup succeeds.
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
pub(crate) mod platform;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
pub(crate) mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
pub(crate) mod platform;

pub use platform::initialize;
pub const RECEIPT: &[u8] = b"OMNIVOX-VOICE-VALIDATION 1 OK\n";
const CLEANUP: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);

pub trait Recorder {
    fn starting(&mut self, _command: &Command) -> Result<()> {
        Ok(())
    }
    fn owned(&mut self, _pid: u32) -> Result<()> {
        Ok(())
    }
    fn cleaned(&mut self) -> Result<()> {
        Ok(())
    }
}
pub struct Unrecorded;
impl Recorder for Unrecorded {}

pub fn worker_gate() -> Result<()> {
    platform::check_worker_group()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("validation-owner-watch".into())
        .spawn(move || {
            let mut input = std::io::stdin().lock();
            let mut start = [0u8; 6];
            if input.read_exact(&mut start).is_err() || &start != b"START\n" {
                platform::parent_closed();
            }
            if sender.send(()).is_err() {
                platform::parent_closed();
            }
            // Any further input, EOF or read error retires this owned worker. This
            // descriptor is never used for helper protocol traffic.
            let mut byte = [0u8; 1];
            loop {
                match input.read(&mut byte) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    _ => platform::parent_closed(),
                }
            }
        })?;
    receiver
        .recv()
        .context("validation owner closed before startup")
}

pub fn cancellation_input() -> Result<Arc<AtomicBool>> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    thread::Builder::new()
        .name("validation-cancellation".into())
        .spawn(move || {
            let mut byte = [0u8; 1];
            loop {
                match std::io::stdin().read(&mut byte) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    _ => {
                        signal.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        })?;
    Ok(cancelled)
}

struct Owned {
    child: Child,
    tree: platform::Tree,
    readers: Vec<JoinHandle<()>>,
    replies: mpsc::Receiver<(bool, std::io::Result<Vec<u8>>)>,
    diagnostics: Vec<u8>,
    cleaned: bool,
}
impl Owned {
    fn spawn(command: &mut Command, memory: usize, recorder: &mut dyn Recorder) -> Result<Self> {
        recorder.starting(command)?;
        let tree = platform::Tree::new(memory)?;
        platform::configure(command, memory);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command
            .spawn()
            .context("could not start validation worker")?;
        let (sender, replies) = mpsc::channel();
        let mut owned = Self {
            child,
            tree,
            readers: Vec::new(),
            replies,
            diagnostics: Vec::new(),
            cleaned: false,
        };
        owned.tree.assign(&owned.child)?;
        // Persistence failure must leave START closed. Drop still attempts
        // cleanup, but cannot turn an unrecorded result into recovery authority.
        recorder.owned(owned.child.id())?;
        let stdout = owned
            .child
            .stdout
            .take()
            .context("missing validation stdout")?;
        let stderr = owned
            .child
            .stderr
            .take()
            .context("missing validation stderr")?;
        owned.readers.push(reader(stdout, true, sender.clone())?);
        owned.readers.push(reader(stderr, false, sender)?);
        // Neither worker nor helper may initialize before job/group ownership.
        owned
            .child
            .stdin
            .as_mut()
            .context("missing validation stdin")?
            .write_all(b"START\n")?;
        Ok(owned)
    }
    fn wait(&mut self, deadline: Instant, cancelled: &AtomicBool) -> Result<Vec<u8>> {
        loop {
            anyhow::ensure!(
                !cancelled.load(Ordering::Acquire),
                "voice validation cancelled"
            );
            anyhow::ensure!(
                Instant::now() < deadline,
                "native voice validation deadline exceeded"
            );
            #[cfg(target_os = "macos")]
            self.tree.check_memory()?;
            match self.replies.recv_timeout(POLL) {
                Ok((true, result)) => return result.context("could not read validation receipt"),
                Ok((false, result)) => {
                    // Never write to a potentially blocked client's pipe while
                    // native work is still alive. Report only after cleanup.
                    self.diagnostics = result.context("could not read validation diagnostics")?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("validation worker closed without a receipt")
                }
            }
        }
    }
    fn cleanup(&mut self, deadline: Instant) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        self.child.stdin.take();
        self.tree
            .terminate()
            .context("could not terminate validation process tree")?;
        // Covers failed Windows job assignment, before the START gate opened.
        // Do not reap the leader before the group's first termination signal.
        let kill = self.child.kill();
        loop {
            if self
                .child
                .try_wait()
                .context("could not reap validation worker")?
                .is_some()
            {
                break;
            }
            if let Err(error) = &kill {
                anyhow::bail!("could not terminate validation worker: {error}");
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "validation worker cleanup unconfirmed"
            );
            thread::sleep(POLL);
        }
        while !self
            .tree
            .empty()
            .context("could not verify validation tree exit")?
        {
            anyhow::ensure!(
                Instant::now() < deadline,
                "validation descendant cleanup unconfirmed"
            );
            thread::sleep(POLL);
        }
        while self.readers.iter().any(|reader| !reader.is_finished()) {
            anyhow::ensure!(
                Instant::now() < deadline,
                "validation pipe cleanup unconfirmed"
            );
            thread::sleep(POLL);
        }
        for reader in self.readers.drain(..) {
            reader
                .join()
                .map_err(|_| anyhow::anyhow!("validation reader panicked"))?;
        }
        for (stdout, result) in self.replies.try_iter() {
            if !stdout {
                self.diagnostics = result.context("could not read validation diagnostics")?;
            }
        }
        self.cleaned = true;
        Ok(())
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup(Instant::now() + CLEANUP) {
            eprintln!("Validation cleanup remains unconfirmed: {error:#}");
        }
    }
}
fn reader(
    input: impl Read + Send + 'static,
    stdout: bool,
    sender: mpsc::Sender<(bool, std::io::Result<Vec<u8>>)>,
) -> Result<JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name("validation-output".into())
        .spawn(move || {
            let mut input = input;
            let mut kept = Vec::new();
            let mut buffer = [0u8; 4096];
            let result = loop {
                match input.read(&mut buffer) {
                    Ok(0) => break Ok(kept),
                    Ok(count) => {
                        let limit: usize = if stdout { RECEIPT.len() + 1 } else { 8192 };
                        let retain = count.min(limit.saturating_sub(kept.len()));
                        kept.extend_from_slice(&buffer[..retain]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => break Err(error),
                }
            };
            let _ = sender.send((stdout, result));
        })?)
}

pub fn probe(
    command: &mut Command,
    timeout: Duration,
    memory: usize,
    cancelled: &AtomicBool,
    recorder: &mut dyn Recorder,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut owned = Owned::spawn(command, memory, recorder)?;
    let result = owned.wait(deadline, cancelled);
    // A receipt alone is never success. Cleanup takes precedence on every path;
    // the caller must return on error, never move to the next model.
    owned.cleanup(Instant::now() + CLEANUP)?;
    recorder.cleaned()?;
    if !owned.diagnostics.is_empty() {
        eprintln!("{}", String::from_utf8_lossy(&owned.diagnostics));
    }
    anyhow::ensure!(
        result? == RECEIPT,
        "native validation failed or returned an invalid receipt"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_ownership_record_keeps_the_worker_gate_closed() {
        struct RejectOwnership;
        impl Recorder for RejectOwnership {
            fn owned(&mut self, _: u32) -> Result<()> {
                anyhow::bail!("injected ownership-record failure")
            }
        }
        let directory = std::env::temp_dir().join(format!(
            "omnivox-gate-record-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let marker = directory.join("native-started");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "voice_validation::owned::tests::record_gate_fixture",
                "--nocapture",
            ])
            .env("OMNIVOX_RECORD_GATE_MARKER", &marker);
        let result = Owned::spawn(&mut command, 1024 * 1024 * 1024, &mut RejectOwnership);
        assert!(result.is_err());
        assert!(
            !marker.exists(),
            "worker passed START after persistence failed"
        );
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn record_gate_fixture() {
        let Some(marker) = std::env::var_os("OMNIVOX_RECORD_GATE_MARKER") else {
            return;
        };
        worker_gate().unwrap();
        std::fs::write(marker, b"started").unwrap();
    }

    fn owned_family() -> (Owned, std::path::PathBuf) {
        platform::initialize().unwrap();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "omnivox-validation-family-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let ready = directory.join("ready");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let mut command = {
            let mut command = Command::new("/bin/sh");
            command.args([
                "-c",
                "read start; sleep 30 & printf '%s' \"$!\" > \"$VALIDATION_CHILD_READY\"; wait",
            ]);
            command
        };
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command",
                "$null = [Console]::ReadLine(); $p = Start-Process powershell.exe -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30' -PassThru -NoNewWindow; [IO.File]::WriteAllText($env:VALIDATION_CHILD_READY, [string]$p.Id); $p.WaitForExit()"]);
            command
        };
        command.env("VALIDATION_CHILD_READY", &ready);
        let owned = Owned::spawn(&mut command, 1024 * 1024 * 1024, &mut Unrecorded).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "descendant never started");
            thread::sleep(POLL);
        }
        (owned, directory)
    }

    #[test]
    fn cleanup_retires_children_and_inherited_pipes_before_another_probe() {
        let (mut owned, directory) = owned_family();
        owned.cleanup(Instant::now() + CLEANUP).unwrap();
        assert!(owned.tree.empty().unwrap());
        assert!(owned.child.try_wait().unwrap().is_some());
        assert!(owned.readers.is_empty());
        owned.cleanup(Instant::now() + CLEANUP).unwrap();
        std::fs::remove_file(directory.join("ready")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn closing_the_last_job_handle_retires_the_entire_family() {
        let (mut owned, directory) = owned_family();
        let job = std::mem::replace(
            &mut owned.tree,
            platform::Tree::new(1024 * 1024 * 1024).unwrap(),
        );
        drop(job); // The kernel also closes this sole handle if the owner crashes.
        let deadline = Instant::now() + CLEANUP;
        while owned.child.try_wait().unwrap().is_none()
            || owned.readers.iter().any(|reader| !reader.is_finished())
        {
            assert!(
                Instant::now() < deadline,
                "family survived last job handle closure"
            );
            thread::sleep(POLL);
        }
        owned.cleanup(deadline).unwrap();
        std::fs::remove_file(directory.join("ready")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn allocation_fixture() {
        let Some(path) = std::env::var_os("OMNIVOX_VALIDATION_ALLOCATION_PROBE") else {
            return;
        };
        worker_gate().unwrap();
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn VirtualAlloc(
                address: *mut std::ffi::c_void,
                size: usize,
                allocation: u32,
                protection: u32,
            ) -> *mut std::ffi::c_void;
            fn VirtualFree(address: *mut std::ffi::c_void, size: usize, kind: u32) -> i32;
        }
        // Reserve and commit without touching pages: no 512 MiB resident-memory
        // spike even if the policy is broken. The OS must refuse this commit
        // inside the 256 MiB validation job.
        let allocation =
            unsafe { VirtualAlloc(std::ptr::null_mut(), 512 * 1024 * 1024, 0x3000, 4) };
        let limited = allocation.is_null();
        if !limited {
            // SAFETY: release exactly the allocation owned above.
            unsafe {
                VirtualFree(allocation, 0, 0x8000);
            }
        }
        std::fs::write(path, if limited { "limited" } else { "unlimited" }).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn job_memory_limit_refuses_excess_native_commit() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "omnivox-validation-memory-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("result");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "voice_validation::owned::tests::allocation_fixture",
            ])
            .env("OMNIVOX_VALIDATION_ALLOCATION_PROBE", &path);
        let mut owned = Owned::spawn(&mut command, 256 * 1024 * 1024, &mut Unrecorded).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !path.exists() {
            assert!(Instant::now() < deadline, "allocation probe did not finish");
            thread::sleep(POLL);
        }
        owned.cleanup(Instant::now() + CLEANUP).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "limited");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn footprint_fixture() {
        let Some(path) = std::env::var_os("OMNIVOX_VALIDATION_FOOTPRINT_PROBE") else {
            return;
        };
        if std::env::var_os("OMNIVOX_VALIDATION_FOOTPRINT_CHILD").is_some() {
            let ready = std::path::PathBuf::from(path);
            std::fs::write(&ready, "ready").unwrap();
            while !ready.with_extension("allocate").exists() {
                thread::sleep(POLL);
            }
            // Touch a bounded allocation in a descendant, not in the worker.
            // Hold it until the supervisor retires the whole group.
            let allocation = vec![0xa5u8; 320 * 1024 * 1024];
            std::hint::black_box(&allocation);
            loop {
                thread::sleep(Duration::from_secs(1));
                std::hint::black_box(&allocation);
            }
        }
        worker_gate().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "voice_validation::owned::tests::footprint_fixture",
            ])
            .env("OMNIVOX_VALIDATION_FOOTPRINT_CHILD", "1")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn footprint_budget_includes_helper_descendants_and_confirms_cleanup() {
        platform::initialize().unwrap();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "omnivox-validation-footprint-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("ready");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "voice_validation::owned::tests::footprint_fixture",
            ])
            .env("OMNIVOX_VALIDATION_FOOTPRINT_PROBE", &path)
            .env_remove("OMNIVOX_VALIDATION_FOOTPRINT_CHILD");
        let mut owned = Owned::spawn(&mut command, 256 * 1024 * 1024, &mut Unrecorded).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !path.exists() {
            assert!(Instant::now() < deadline, "footprint fixture never started");
            thread::sleep(POLL);
        }
        // Confirm the worker and idle descendant fit before asking only the
        // descendant to allocate. A broken ABI or early failure cannot pass.
        owned.tree.check_memory().unwrap();
        let allocate = path.with_extension("allocate");
        std::fs::write(&allocate, "allocate").unwrap();
        let result = owned.wait(
            Instant::now() + Duration::from_secs(15),
            &AtomicBool::new(false),
        );
        owned.cleanup(Instant::now() + CLEANUP).unwrap();
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("memory footprint budget exceeded"));
        assert!(owned.tree.empty().unwrap());
        assert!(owned.readers.is_empty());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(allocate).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn bounded_reader_rejects_oversized_receipt_without_retaining_output() {
        let (sender, receiver) = mpsc::channel();
        let handle = reader(std::io::Cursor::new(vec![b'x'; 1024 * 1024]), true, sender).unwrap();
        let (_, reply) = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        let reply = reply.unwrap();
        assert_eq!(reply.len(), RECEIPT.len() + 1);
        assert_ne!(reply, RECEIPT);
        handle.join().unwrap();
    }
}
