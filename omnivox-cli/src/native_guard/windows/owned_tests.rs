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
        helpers: Vec::new(),
        pending: None,
        channel: None,
        plan: Plan {
            helpers: Vec::new(),
        },
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
    native
        .plan
        .helpers
        .push(Helper::capture(HelperEngineConfig::new("flite", program)).unwrap());
    native.helpers.push(None);
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
        launch: native.plan.helpers.first().unwrap().identity.clone(),
    }
}

fn reply(native: &mut Native, client: &mut os::Blocking) -> serde_json::Value {
    let channel = native.channel.as_mut().unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while matches!(
        channel.wait_for(),
        crate::native_guard::wire::WaitFor::Write
    ) {
        assert!(Instant::now() < deadline);
        channel.poll_as::<helper_wire::Request>().unwrap();
        std::thread::sleep(TICK);
    }
    crate::native_guard::wire::read_reply(&mut std::io::BufReader::new(client)).unwrap()
}

fn second_slot(native: &mut Native, absent: bool) {
    // These are ownership tests: two independent instances of the fixture
    // helper, not qualification of a second engine's native protocol.
    let mut config = native.plan.helpers[0].config.clone();
    config.engine_id = "second-fixture".into();
    if absent {
        config.program = std::env::temp_dir().join(format!(
            "omnivox-absent-{}.exe",
            omnivox_tts::voice_library::local::new_uuid().unwrap()
        ));
    }
    native.plan.helpers.push(Helper::capture(config).unwrap());
    native.helpers.push(None);
}

fn request_slot(native: &Native, id: u64, slot: usize) -> helper_wire::Request {
    helper_wire::Request {
        request_id: id,
        command: "create".into(),
        engine: native.plan.helpers[slot].config.engine_id.clone(),
        launch: native.plan.helpers[slot].identity.clone(),
    }
}

fn start_slot(native: &mut Native, client: &mut os::Blocking, id: u64, slot: usize) {
    send(client, &request_slot(native, id, slot));
    until(native, |n| n.offer.is_some());
    assert_eq!(reply(native, client)["state"], "offered");
    send(
        client,
        &helper_wire::Request {
            request_id: id,
            command: "acknowledge".into(),
            engine: String::new(),
            launch: String::new(),
        },
    );
    until(native, |n| n.offer.is_none());
    assert_eq!(reply(native, client)["state"], "started");
}

#[test]
#[ignore = "requires explicit private native runtime and Flite helper"]
fn independent_helpers_recover_and_retire_by_actual_handle() {
    let (mut native, mut client) = fixture();
    second_slot(&mut native, false);
    start_slot(&mut native, &mut client, 1, 0);
    start_slot(&mut native, &mut client, 2, 1);
    let first = native.helpers[0]
        .as_ref()
        .unwrap()
        .process
        .try_clone()
        .unwrap();
    let second = native.helpers[1]
        .as_ref()
        .unwrap()
        .process
        .try_clone()
        .unwrap();
    native.helpers[0].as_mut().unwrap().terminate().unwrap();
    until(&mut native, |n| n.helpers[0].is_none());
    assert!(native::signalled(first.as_handle()).unwrap());
    assert!(!native::signalled(second.as_handle()).unwrap());
    assert!(!native.worker.as_ref().unwrap().ended().unwrap());
    start_slot(&mut native, &mut client, 3, 0);
    let replacement = native.helpers[0]
        .as_ref()
        .unwrap()
        .process
        .try_clone()
        .unwrap();
    assert_eq!(native.total, 4);
    native.closing = true;
    until(&mut native, |n| n.retired);
    for process in [second, replacement] {
        assert!(native::signalled(process.as_handle()).unwrap());
    }
}

#[test]
#[ignore = "requires explicit private native runtime and Flite helper"]
fn second_helper_rejection_preserves_existing_speech_processes() {
    let (mut native, mut client) = fixture();
    second_slot(&mut native, true);
    start_slot(&mut native, &mut client, 1, 0);
    send(&mut client, &request_slot(&native, 2, 1));
    until(&mut native, |n| n.pending.is_some());
    until(&mut native, |n| n.pending.is_none());
    assert_eq!(reply(&mut native, &mut client)["state"], "rejected");
    assert!(!native.closing && native.error.is_none());
    assert!(!native.helpers[0].as_ref().unwrap().ended().unwrap());
    assert!(!native.worker.as_ref().unwrap().ended().unwrap());
    assert_eq!(native.total, 2);
}

#[test]
#[ignore = "requires explicit private native runtime and Flite helper"]
fn occupied_helper_slot_rejects_new_identity_without_losing_handles() {
    let (mut native, mut client) = fixture();
    second_slot(&mut native, false);
    start_slot(&mut native, &mut client, 1, 0);
    start_slot(&mut native, &mut client, 2, 1);
    let handles: Vec<_> = native
        .helpers
        .iter()
        .flatten()
        .map(|h| h.process.try_clone().unwrap())
        .collect();
    send(&mut client, &request_slot(&native, 3, 0));
    until(&mut native, |n| n.retired);
    assert!(native.error.as_deref().unwrap().contains("still owned"));
    assert_eq!(native.total, 3);
    assert!(handles
        .iter()
        .all(|h| native::signalled(h.as_handle()).unwrap()));
}

#[test]
fn pending_creation_keeps_retirement_unconfirmed_and_control_responsive() {
    let mut native = empty();
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    native.pending = Some(
        Pending::start(None, move || {
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
        let helper = native.helpers[0]
            .as_ref()
            .unwrap()
            .process
            .try_clone()
            .unwrap();
        assert!(!native::signalled(helper.as_handle()).unwrap());
        if lose_link {
            drop(client);
        } else {
            native.offer.as_mut().unwrap().deadline = Instant::now();
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
    if let Some(helper) = &native.helpers[0] {
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

#[test]
#[ignore = "requires explicit private native runtime and Flite helper"]
fn failed_helper_launch_preserves_the_worker_and_creation_link() {
    let (mut native, mut client) = fixture();
    let mut config = native.plan.helpers.first().unwrap().config.clone();
    config.program = std::env::temp_dir().join(format!(
        "omnivox-absent-{}.exe",
        omnivox_tts::voice_library::local::new_uuid().unwrap()
    ));
    native.plan.helpers[0] = Helper::capture(config).unwrap();
    for id in [1, 2] {
        let mut request = create(&native);
        request.request_id = id;
        send(&mut client, &request);
        until(&mut native, |n| n.pending.is_some());
        until(&mut native, |n| n.pending.is_none());
        let channel = native.channel.as_mut().unwrap();
        let deadline = Instant::now() + TIMEOUT;
        while matches!(
            channel.wait_for(),
            crate::native_guard::wire::WaitFor::Write
        ) {
            assert!(Instant::now() < deadline);
            channel.poll_as::<helper_wire::Request>().unwrap();
            std::thread::sleep(TICK);
        }
        let mut reader = std::io::BufReader::new(&mut client);
        let reply = crate::native_guard::wire::read_reply(&mut reader).unwrap();
        assert_eq!(reply["state"], "rejected");
        assert_eq!(reply["request_id"], id);
        assert!(reply["handles"].is_null());
        assert!(!native.closing && !native.retired && native.error.is_none());
        assert!(!native.worker.as_ref().unwrap().ended().unwrap());
        assert_eq!(native.total, 1);
    }
}
