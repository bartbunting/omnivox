use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "admission_tests.rs"]
mod admission_tests;

const OPERATION_ID: &str = "44444444-4444-4444-8444-444444444444";

fn plan() -> ValidationPlan {
    let generation = serde_json::json!({"schema_version":1,
        "target_id":"11111111-1111-4111-8111-111111111111",
        "profile_id":"22222222-2222-4222-8222-222222222222",
        "generation_id":"33333333-3333-4333-8333-333333333333",
        "piper":null,"flite":{"builtin_slt":true,"files":[]},"disabled_physical_ids":[]});
    let prefix = if cfg!(windows) {
        "C:/unused"
    } else {
        "/unused"
    };
    let document = ValidationPlanDocument {
        schema_version: 1,
        operation_kind: "native_validation".into(),
        operation_id: OPERATION_ID.into(),
        platform: std::env::consts::OS.into(),
        generation_json: generation.to_string(),
        validator_path: format!("{prefix}/omnivox"),
        helpers: BTreeMap::from([("flite".into(), format!("{prefix}/omnivox-flite-helper"))]),
        timeout_seconds: 60,
        memory_bytes: 4096 * 1024 * 1024,
        runtime_policy: "bundled-companions-v1".into(),
    };
    ValidationPlan::parse(&serde_json::to_vec(&document).unwrap()).unwrap()
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omnivox-operation-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn operation(&self) -> PathBuf {
        self.0.join(OPERATION_ID)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn validating() -> Transition {
    Transition {
        state: ValidationState::Validating,
        cleanup: Cleanup::Unconfirmed,
        evidence_sha256: None,
        detail: None,
    }
}
fn staged() -> Transition {
    Transition {
        state: ValidationState::Staged,
        cleanup: Cleanup::Confirmed,
        evidence_sha256: Some("a".repeat(64)),
        detail: None,
    }
}
fn failed(cleanup: Cleanup) -> Transition {
    Transition {
        state: ValidationState::Failed,
        cleanup,
        evidence_sha256: None,
        detail: Some("test failure".into()),
    }
}

#[test]
fn plan_parsing_is_bounded_and_does_not_open_saved_paths() {
    let plan = plan();
    assert!(!Path::new(&plan.document().validator_path).exists());
    let bytes = plan.source_bytes();
    let text = std::str::from_utf8(bytes).unwrap();
    let duplicate = text.replacen(
        "\"schema_version\":1",
        "\"schema_version\":1,\"schema_version\":1",
        1,
    );
    assert!(ValidationPlan::parse(duplicate.as_bytes()).is_err());
    let mut doc = plan.document().clone();
    doc.helpers.clear();
    assert!(ValidationPlan::parse(&serde_json::to_vec(&doc).unwrap()).is_err());
    let mut doc = plan.document().clone();
    doc.operation_id = "../escape".into();
    assert!(ValidationPlan::parse(&serde_json::to_vec(&doc).unwrap()).is_err());
    assert!(ValidationPlan::read(std::io::repeat(b' ').take(MAX_PLAN_BYTES as u64 + 1)).is_err());
}

#[test]
fn existing_operations_are_preserved_and_only_one_writer_can_own_them() {
    let fixture = Fixture::new();
    let operation = Operation::create(&fixture.0, plan()).unwrap();
    let original = fs::read(operation.path().join("plan.json")).unwrap();
    assert!(Operation::create(&fixture.0, plan()).is_err());
    assert_eq!(
        Operation::inspect(operation.path()).unwrap(),
        Inspection::Busy
    );
    assert!(Operation::try_open(operation.path()).unwrap().is_none());
    assert_eq!(
        fs::read(operation.path().join("plan.json")).unwrap(),
        original
    );
    drop(operation);
    let mut owner = Operation::try_open(&fixture.operation()).unwrap().unwrap();
    owner.append(failed(Cleanup::NotStarted)).unwrap();
    drop(owner);
    assert_eq!(
        Operation::inspect(&fixture.operation()).unwrap(),
        Inspection::Failed
    );
}

#[test]
fn clean_terminal_records_require_cleanup_and_remain_bound_to_the_plan() {
    let fixture = Fixture::new();
    let mut operation = Operation::create(&fixture.0, plan()).unwrap();
    assert!(operation.append(staged()).is_err());
    operation.append(validating()).unwrap();
    assert!(operation.append(failed(Cleanup::Unconfirmed)).is_err());
    assert!(operation.append(failed(Cleanup::NotStarted)).is_err());
    let mut invalid = staged();
    invalid.evidence_sha256 = None;
    assert!(operation.append(invalid).is_err());
    operation.append(staged()).unwrap();
    assert!(operation.append(validating()).is_err());
    drop(operation);
    assert_eq!(
        Operation::inspect(&fixture.operation()).unwrap(),
        Inspection::Staged
    );
    let mut reopened = Operation::try_open(&fixture.operation()).unwrap().unwrap();
    assert!(reopened.append(validating()).is_err());
    let mut changed = plan().source_bytes().to_vec();
    changed.push(b'\n');
    let other_plan = ValidationPlan::parse(&changed).unwrap();
    let journal = Journal::read(reopened.journal().source_bytes(), &other_plan).unwrap();
    assert!(journal.damage().is_some());
}

#[test]
fn partial_reordered_and_modified_frames_block_reuse_without_truncation() {
    let fixture = Fixture::new();
    let mut operation = Operation::create(&fixture.0, plan()).unwrap();
    operation.append(validating()).unwrap();
    let prefix = operation.journal().source_bytes().to_vec();
    let frame = operation
        .journal()
        .next_frame(operation.plan(), staged())
        .unwrap();
    drop(operation);
    let path = fixture.operation().join("journal.frames");
    for cut in [1, frame.len() / 2, frame.len() - 1] {
        let bytes = [prefix.as_slice(), &frame[..cut]].concat();
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            Operation::inspect(&fixture.operation()).unwrap(),
            Inspection::Damaged
        );
        let mut reopened = Operation::try_open(&fixture.operation()).unwrap().unwrap();
        assert_eq!(
            reopened.journal().state(),
            Some(ValidationState::Validating)
        );
        assert!(reopened.append(staged()).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let mut corrupted = [prefix.as_slice(), frame.as_slice()].concat();
    corrupted[prefix.len() + 2] ^= 1;
    assert!(Journal::read(corrupted.as_slice(), &plan())
        .unwrap()
        .damage()
        .is_some());
    assert!(Journal::read(
        [frame.as_slice(), prefix.as_slice()].concat().as_slice(),
        &plan()
    )
    .unwrap()
    .damage()
    .is_some());
    assert!(Journal::read(
        std::io::repeat(b' ').take(MAX_JOURNAL_BYTES as u64 + 1),
        &plan()
    )
    .is_err());
}

#[test]
fn changed_plan_or_journal_is_rejected_before_an_append() {
    let fixture = Fixture::new();
    let mut operation = Operation::create(&fixture.0, plan()).unwrap();
    let original = fs::read(operation.path().join("plan.json")).unwrap();
    fs::write(
        operation.path().join("plan.json"),
        [original.as_slice(), b"\n"].concat(),
    )
    .unwrap();
    assert!(operation.append(validating()).is_err());
    fs::write(operation.path().join("plan.json"), original).unwrap();
    OpenOptions::new()
        .append(true)
        .open(operation.path().join("journal.frames"))
        .unwrap()
        .write_all(b"partial")
        .unwrap();
    assert!(operation.append(validating()).is_err());
}

#[test]
fn replacing_the_journal_path_cannot_divert_an_append_to_an_old_file() {
    let fixture = Fixture::new();
    let mut operation = Operation::create(&fixture.0, plan()).unwrap();
    let current = operation.path().join("journal.frames");
    let previous = operation.path().join("old-journal.frames");
    let original = fs::read(&current).unwrap();
    fs::rename(&current, &previous).unwrap();
    let replacement = [original.as_slice(), b"unfinished replacement"].concat();
    fs::write(&current, &replacement).unwrap();
    assert!(operation.append(validating()).is_err());
    assert_eq!(fs::read(current).unwrap(), replacement);
    assert_eq!(fs::read(previous).unwrap(), original);
}

#[test]
fn missing_initialization_is_never_repaired_by_inspection() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.operation()).unwrap();
    assert!(Operation::inspect(&fixture.operation()).is_err());
    assert_eq!(fs::read_dir(fixture.operation()).unwrap().count(), 0);
    File::create(fixture.operation().join("owner.lock")).unwrap();
    assert!(Operation::inspect(&fixture.operation()).is_err());
    assert_eq!(fs::read_dir(fixture.operation()).unwrap().count(), 1);
}

struct OwnerChild(Option<Child>);
impl OwnerChild {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }
    fn kill_and_reap(&mut self) {
        let mut child = self.0.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
impl Drop for OwnerChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().is_ok_and(|status| status.is_none()) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

#[test]
fn process_crashes_release_the_lock_but_do_not_clear_unconfirmed_work() {
    for (mode, expected) in [
        ("prepared", Inspection::Prepared),
        ("validating", Inspection::Interrupted),
        ("partial", Inspection::Damaged),
        ("staged", Inspection::Staged),
    ] {
        let fixture = Fixture::new();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "voice_library::operations::tests::owner_fixture",
                "--nocapture",
            ])
            .env("OMNIVOX_OPERATION_FIXTURE_ROOT", &fixture.0)
            .env("OMNIVOX_OPERATION_FIXTURE_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = OwnerChild(Some(child));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !fixture.0.join("ready").exists() {
            assert!(
                child.child().try_wait().unwrap().is_none(),
                "owner fixture exited before readiness"
            );
            assert!(
                Instant::now() < deadline,
                "owner fixture did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            Operation::inspect(&fixture.operation()).unwrap(),
            Inspection::Busy
        );
        child.kill_and_reap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = Operation::inspect(&fixture.operation()).unwrap();
            if status != Inspection::Busy {
                assert_eq!(status, expected, "{mode}");
                break;
            }
            assert!(Instant::now() < deadline, "operation lease did not release");
            std::thread::sleep(Duration::from_millis(10));
        }
        if mode != "prepared" {
            let mut reopened = Operation::try_open(&fixture.operation()).unwrap().unwrap();
            assert!(
                reopened.append(staged()).is_err(),
                "{mode} was reusable after a crash"
            );
        }
    }
}

#[test]
fn owner_fixture() {
    let Some(root) = std::env::var_os("OMNIVOX_OPERATION_FIXTURE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("OMNIVOX_OPERATION_FIXTURE_MODE").unwrap();
    let mut operation = Operation::create(&root, plan()).unwrap();
    if mode != "prepared" {
        operation.append(validating()).unwrap();
    }
    if mode == "staged" {
        operation.append(staged()).unwrap();
    }
    if mode == "partial" {
        let frame = operation
            .journal()
            .next_frame(operation.plan(), staged())
            .unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(operation.path().join("journal.frames"))
            .unwrap();
        file.write_all(&frame[..frame.len() - 1]).unwrap();
        file.sync_all().unwrap();
    }
    fs::write(root.join("ready"), b"ready").unwrap();
    let _ = std::io::stdin().read_exact(&mut [0]);
    drop(operation);
}

#[cfg(unix)]
#[test]
fn operation_metadata_symlinks_are_rejected() {
    let fixture = Fixture::new();
    let operation = Operation::create(&fixture.0, plan()).unwrap();
    drop(operation);
    let path = fixture.operation().join("plan.json");
    let outside = fixture.0.join("outside-plan.json");
    fs::rename(&path, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(Operation::inspect(&fixture.operation()).is_err());
    assert_eq!(fs::read(outside).unwrap(), plan().source_bytes());
}

#[cfg(unix)]
#[test]
fn normal_retirement_releases_the_lease_during_an_unrelated_fork() {
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    unsafe extern "C" {
        fn fork() -> i32;
        fn close(fd: i32) -> i32;
        fn read(fd: i32, buffer: *mut u8, length: usize) -> isize;
        fn write(fd: i32, buffer: *const u8, length: usize) -> isize;
        fn alarm(seconds: u32) -> u32;
        fn _exit(status: i32) -> !;
        fn waitpid(pid: i32, status: *mut i32, flags: i32) -> i32;
    }
    let fixture = Fixture::new();
    let operation = Operation::create(&fixture.0, plan()).unwrap();
    let (mut parent, child) = UnixStream::pair().unwrap();
    parent
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let parent_fd = parent.as_raw_fd();
    let child_fd = child.as_raw_fd();
    // Model the interval after fork but before exec closes CLOEXEC descriptors.
    // The child performs only async-signal-safe calls and cannot run Rust drops.
    let pid = unsafe { fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            alarm(5);
            close(parent_fd);
            write(child_fd, b"R".as_ptr(), 1);
            read(child_fd, &mut 0u8, 1);
            _exit(0);
        }
    }
    drop(child);
    let ready = parent.read_exact(&mut [0]);
    drop(operation);
    let observed = Operation::try_open(&fixture.operation());
    // Retire the owned child before any assertion can unwind this test.
    let _ = parent.write_all(b"X");
    let mut status = 0;
    let waited = loop {
        let result = unsafe { waitpid(pid, &mut status, 0) };
        if result != -1 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            break result;
        }
    };
    ready.unwrap();
    assert_eq!(waited, pid);
    assert!(
        observed.unwrap().is_some(),
        "a pre-exec descriptor kept the retired owner's lease"
    );
}
