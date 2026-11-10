//! Faults use actual private process handles. No installed runtime or PID lookup.
use super::*;
use crate::native_guard::windows_plan::Helper;
use omnivox_tts::{engine_configuration::LaunchEnvironment, helper_engine::HelperEngineConfig};
use std::io::Write;

fn null() -> File {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("NUL")
        .unwrap()
}
fn empty() -> Native {
    Native {
        job: Arc::new(Job::new().unwrap()),
        worker: None,
        helper: None,
        pending: None,
        channel: None,
        plan: Plan { helper: None },
        directory: std::env::temp_dir(),
        error_output: null(),
        total: 0,
        job_stopping: false,
        sequence: 0,
        offer: None,
        started: false,
        closing: false,
        retired: false,
        error: None,
    }
}
fn until(native: &mut Native, condition: impl Fn(&Native) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition(native) {
        assert!(
            Instant::now() < deadline,
            "native test deadline: {:?}; expected {}; job {:?}; pending {}",
            native.error,
            native.total,
            native.job.counts().map(|c| (c.total, c.active)),
            native.pending.is_some()
        );
        native.poll(false).unwrap();
        std::thread::sleep(TICK);
    }
}
fn suspended(job: &Job) -> Created {
    let exe = std::env::var("OMNIVOX_TEST_GUARD_EXECUTABLE").expect("private runtime required");
    let mut command = Command::new(exe);
    LaunchEnvironment::capture().apply(&mut command);
    command.arg("--version");
    let file = null();
    creation::spawn(&command, job.handle(), [file.as_handle(); 3], None).unwrap()
}
fn fixture() -> (Native, os::Blocking) {
    let mut native = empty();
    native.worker = Some(suspended(&native.job));
    native.total = 1;
    let program =
        std::env::var("OMNIVOX_TEST_GUARD_HELPER").expect("private Flite helper required");
    native.plan.helper = Some(Helper::capture(HelperEngineConfig::new("flite", program)).unwrap());
    let (server, client) = os::channel_pair().unwrap();
    native.channel = Some(Channel::new(server));
    (native, os::Blocking::from_handle(client).unwrap())
}
fn send(client: &mut os::Blocking, request: &helper_wire::Request) {
    serde_json::to_writer(&mut *client, request).unwrap();
    client.write_all(b"\n").unwrap();
}
fn create(native: &Native) -> helper_wire::Request {
    helper_wire::Request {
        request_id: 1,
        command: "create".into(),
        engine: "flite".into(),
        launch: native.plan.helper.as_ref().unwrap().identity.clone(),
    }
}

#[test]
fn pending_creation_keeps_retirement_unconfirmed_and_control_responsive() {
    let mut native = empty();
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    native.pending = Some(
        Pending::start(move || {
            wait.recv_timeout(TIMEOUT)?;
            anyhow::bail!("fixture creation cancelled")
        })
        .unwrap(),
    );
    // Neither polling nor retirement joins a blocked launch thread.
    for _ in 0..20 {
        native.poll(true).unwrap();
        assert!(!native.retired);
    }
    release.send(()).unwrap();
    until(&mut native, |n| n.retired);
    assert!(native
        .error
        .as_deref()
        .unwrap()
        .contains("fixture creation cancelled"));
}

#[test]
#[ignore = "requires explicit private native runtime and Flite helper"]
fn lost_offer_or_ack_retires_the_receiver_and_actual_helper() {
    for lose_link in [false, true] {
        let (mut native, mut client) = fixture();
        send(&mut client, &create(&native));
        until(&mut native, |n| n.offer.is_some());
        let worker = native.worker.as_ref().unwrap().process.try_clone().unwrap();
        let helper = native.helper.as_ref().unwrap().process.try_clone().unwrap();
        assert!(!native::signalled(helper.as_handle()).unwrap());
        if lose_link {
            drop(client);
        } else {
            native.offer.as_mut().unwrap().1 = Instant::now();
        }
        until(&mut native, |n| n.retired);
        assert!(native::signalled(worker.as_handle()).unwrap());
        assert!(native::signalled(helper.as_handle()).unwrap());
    }
}

#[test]
#[ignore = "requires explicit private native runtime and Flite helper"]
fn worker_loss_during_creation_cannot_release_a_late_helper() {
    let (mut native, mut client) = fixture();
    send(&mut client, &create(&native));
    until(&mut native, |n| n.pending.is_some());
    native.worker.as_mut().unwrap().terminate().unwrap();
    // Termination acceptance is not completion. Observe the actual worker
    // ending before allowing the guard to collect the concurrent creation.
    let deadline = Instant::now() + TIMEOUT;
    while !native.worker.as_ref().unwrap().ended().unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(TICK);
    }
    until(&mut native, |n| n.retired);
    assert!(native.offer.is_none());
    if let Some(helper) = &native.helper {
        assert!(helper.ended().unwrap());
    }
}

#[test]
#[ignore = "requires explicit private native runtime and Flite helper"]
fn changed_identity_and_duplicate_creation_fail_without_unbounded_children() {
    for duplicate in [false, true] {
        let (mut native, mut client) = fixture();
        let mut request = create(&native);
        if !duplicate {
            request.launch.push('0');
        }
        send(&mut client, &request);
        if duplicate {
            send(&mut client, &request);
        }
        until(&mut native, |n| n.retired);
        assert!(native.error.is_some());
        assert!(native.total <= 2);
    }
}

#[test]
#[ignore = "requires explicit private native runtime"]
fn unaccounted_job_member_never_becomes_a_retirement_receipt() {
    let mut native = empty();
    native.worker = Some(suspended(&native.job));
    native.total = 1;
    let unexpected = suspended(&native.job);
    native.closing = true;
    until(&mut native, |n| {
        n.worker.as_ref().unwrap().ended().unwrap() && unexpected.ended().unwrap()
    });
    native.poll(true).unwrap();
    assert!(!native.retired);
    assert!(native
        .error
        .as_deref()
        .unwrap()
        .contains("accounting differs"));
    // Drop still emits no proof and remains bounded by its shutdown deadline.
}
