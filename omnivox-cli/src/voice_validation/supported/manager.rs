//! The command's client owns a cancellation pipe, never the native worker tree.
//! A separate supervisor retains both admission leases through client death.
use super::owned;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(super) fn run(args: &[String]) -> Result<()> {
    anyhow::ensure!(
        args.len() == 4,
        "expected --run-voice-validation-operation ROOT PROFILE_UUID OPERATION_UUID"
    );
    let cancelled = owned::cancellation_input()?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--internal-voice-validation-supervisor")
        .args(&args[1..]);
    supervise(&mut command, &cancelled)
}

fn supervise(command: &mut Command, cancelled: &AtomicBool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Own no console: closing the client's console must not retire cleanup.
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("could not start validation supervisor")?;
    if !cancelled.load(Ordering::Acquire) {
        child
            .stdin
            .as_mut()
            .context("missing supervisor input")?
            .write_all(b"START\n")?;
    } else {
        child.stdin.take();
    }
    loop {
        if cancelled.load(Ordering::Acquire) {
            child.stdin.take();
        }
        if let Some(status) = child
            .try_wait()
            .context("could not wait for validation supervisor")?
        {
            anyhow::ensure!(
                status.success(),
                "validation supervisor failed ({status}); inspect retained operation history"
            );
            return Ok(());
        }
        // The supervisor owns native deadlines and cleanup. Killing it here
        // would discard the very ownership needed after a client disconnect.
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn await_start() -> Result<()> {
    let mut start = [0; 6];
    std::io::stdin()
        .read_exact(&mut start)
        .context("validation manager closed before supervisor startup")?;
    anyhow::ensure!(
        &start == b"START\n",
        "invalid validation supervisor startup gate"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Child;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    const ROOT: &str = "OMNIVOX_MANAGER_LIFETIME_FIXTURE";
    fn fixture_command(fixture: &str, root: &Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", fixture])
            .env(ROOT, root)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    struct Manager(Option<Child>);
    impl Drop for Manager {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                if child.try_wait().is_ok_and(|status| status.is_none()) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }

    struct Supervisor {
        #[cfg(windows)]
        handle: std::os::windows::io::OwnedHandle,
        #[cfg(unix)]
        pid: i32,
    }
    impl Supervisor {
        fn observe(pid: u32) -> Self {
            #[cfg(windows)]
            {
                use std::os::windows::io::FromRawHandle;
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
                }
                // SAFETY: obtain only a wait handle while this owned fixture
                // is still waiting on its live manager; never request signals.
                let handle = unsafe { OpenProcess(0x0010_0000, 0, pid) };
                assert!(!handle.is_null());
                Self {
                    handle: unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) },
                }
            }
            #[cfg(unix)]
            Self {
                pid: i32::try_from(pid).unwrap(),
            }
        }
        fn exited(&self) -> bool {
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn WaitForSingleObject(handle: *mut std::ffi::c_void, millis: u32) -> u32;
                }
                // SAFETY: read-only polling of the owned process wait handle.
                match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
                    0 => true,
                    258 => false,
                    result => panic!("could not observe fixture exit: {result}"),
                }
            }
            #[cfg(unix)]
            {
                unsafe extern "C" {
                    fn kill(pid: i32, signal: i32) -> i32;
                }
                #[cfg(target_os = "linux")]
                {
                    unsafe extern "C" {
                        fn waitpid(pid: i32, status: *mut i32, flags: i32) -> i32;
                    }
                    // SAFETY: reap only this fixture if adopted by our subreaper.
                    if unsafe { waitpid(self.pid, std::ptr::null_mut(), 1) } == self.pid {
                        return true;
                    }
                }
                // SAFETY: signal zero observes existence, never terminates.
                // PID reuse can only conservatively keep this check pending.
                (unsafe { kill(self.pid, 0) }) == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(3)
            }
        }
    }

    #[test]
    fn supervisor_finishes_cleanup_after_manager_death_or_cancellation() {
        owned::initialize().unwrap();
        for kill in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "omnivox-manager-{}-{}-{kill}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            let mut manager = Manager(Some(
                fixture_command(
                    "voice_validation::supported::manager::tests::manager_fixture",
                    &root,
                )
                .spawn()
                .unwrap(),
            ));
            let deadline = Instant::now() + Duration::from_secs(20);
            while !root.join("ready").exists() {
                assert!(manager.0.as_mut().unwrap().try_wait().unwrap().is_none());
                assert!(
                    Instant::now() < deadline,
                    "supervisor fixture never started"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let supervisor = Supervisor::observe(
                fs::read_to_string(root.join("ready"))
                    .unwrap()
                    .parse()
                    .unwrap(),
            );
            if kill {
                // Signal only the directly owned, still-unreaped child.
                manager.0.as_mut().unwrap().kill().unwrap();
            } else {
                manager.0.as_mut().unwrap().stdin.take();
            }
            while !root.join("cleaned").exists() {
                assert!(
                    Instant::now() < deadline,
                    "supervisor lost cleanup ownership with its manager"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            while !supervisor.exited() {
                assert!(
                    Instant::now() < deadline,
                    "supervisor fixture did not exit after cleanup"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            while manager.0.as_mut().unwrap().try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline, "manager did not exit");
                std::thread::sleep(Duration::from_millis(10));
            }
            manager.0.take();
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn manager_fixture() {
        let Some(root) = std::env::var_os(ROOT) else {
            return;
        };
        let cancelled = owned::cancellation_input().unwrap();
        supervise(
            &mut fixture_command(
                "voice_validation::supported::manager::tests::supervisor_fixture",
                Path::new(&root),
            ),
            &cancelled,
        )
        .unwrap();
    }

    #[test]
    fn supervisor_fixture() {
        let Some(root) = std::env::var_os(ROOT) else {
            return;
        };
        await_start().unwrap();
        let root = PathBuf::from(root);
        let cancelled = owned::cancellation_input().unwrap();
        let ready = root.join("ready-pending");
        fs::write(&ready, std::process::id().to_string()).unwrap();
        fs::rename(ready, root.join("ready")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(25);
        while !cancelled.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "fixture cancellation pipe remained open"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::write(root.join("cleaned"), b"cleaned").unwrap();
    }
}
