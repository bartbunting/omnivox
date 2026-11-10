//! Native executable checks use an explicitly selected private payload. These
//! tests are ignored in generic suites; the Windows qualification invokes them.
use super::*;
use crate::native_guard::{wire, Peer};
use sha2::{Digest, Sha256};

fn executable() -> String {
    std::env::var("OMNIVOX_TEST_GUARD_EXECUTABLE")
        .expect("explicit private native executable required")
}
fn startup() -> local::Startup {
    let executable = executable();
    let mut startup = crate::emergency::startup("null", None).unwrap();
    let bytes = std::fs::read(&executable).unwrap();
    startup.executable = omnivox_tts::voice_library::AssetFile {
        path: executable.clone(),
        bytes: bytes.len() as u64,
        sha256: Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    };
    startup.working_directory = std::path::Path::new(&executable).parent().unwrap().into();
    startup
}
fn request(
    control: &mut BufReader<Blocking>,
    endpoint: &GuardEndpoint,
    id: u64,
    command: &str,
) -> serde_json::Value {
    let value = serde_json::json!({"command": command, "request_id": id, "guard": endpoint.guard, "worker": endpoint.worker, "token": endpoint.token});
    serde_json::to_writer(control.get_mut(), &value).unwrap();
    control.get_mut().write_all(b"\n").unwrap();
    let reply = wire::read_reply(control).unwrap();
    assert_eq!(reply["guard"], endpoint.guard);
    assert_eq!(reply["worker"], endpoint.worker);
    assert_eq!(reply["request_id"], id);
    assert!(reply["error"].is_null(), "{reply}");
    reply
}
#[test]
#[ignore = "requires explicit private native runtime"]
fn guarded_owners_reject_before_accessing_management_state() {
    let root =
        std::env::temp_dir().join(format!("omnivox-guard-gate-{}", local::new_uuid().unwrap()));
    std::fs::create_dir(&root).unwrap();
    let config = root.join("config.json");
    std::fs::write(&config, b"{broken preferences").unwrap();
    let voices = root.join("voices");
    for mode in ["--voice-library-owner", "--emergency-owner"] {
        let output = std::process::Command::new(executable())
            .args([mode, "--guarded"])
            .env("OMNIVOX_CONFIG_DIR", &root)
            .env("OMNIVOX_VOICE_ROOT", &voices)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("descendant retirement is not qualified"));
        assert!(!voices.exists(), "rejected startup opened management state");
        assert_eq!(std::fs::read(&config).unwrap(), b"{broken preferences");
    }
    std::fs::remove_file(config).unwrap();
    std::fs::remove_dir(root).unwrap();
}
#[test]
#[ignore = "requires explicit private native runtime"]
fn partial_bootstrap_expires_without_native_speech() {
    for partial in [&[0u8, 0][..], &[0, 0, 0, 64, b'{', b'"'][..]] {
        let mut spawned = launch(&executable()).unwrap();
        spawned.control.write_all(partial).unwrap();
        let deadline = Instant::now() + Duration::from_secs(9);
        while !native::signalled(spawned.child.handle()).unwrap() {
            assert!(
                Instant::now() < deadline,
                "partial bootstrap guard survived its deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // No worker can hold the far end after this incomplete bootstrap.
        assert_eq!(spawned.output.read(&mut [0]).unwrap(), 0);
    }
}
#[test]
#[ignore = "requires explicit private native runtime"]
fn partial_start_requires_real_io_retirement_after_job_cleanup() {
    let startup = startup();
    let worker = local::new_uuid().unwrap();
    let (mut peer, mut input, output) = Peer::prepare(&startup, &worker).unwrap();
    let endpoint = peer.endpoint();
    let mut control = BufReader::new(Blocking {
        stream: connect(&endpoint.address).unwrap(),
        timeout: Some(TIMEOUT),
    });
    request(&mut control, &endpoint, 1, "attach");
    peer.start().unwrap();
    input.write_all(b"STA").unwrap();
    let mut id = 2;
    let mut reply = request(&mut control, &endpoint, id, "retire");
    let deadline = Instant::now() + TIMEOUT;
    while reply["tree_retired"] != true {
        assert!(Instant::now() < deadline);
        id += 1;
        reply = request(&mut control, &endpoint, id, "status");
    }
    assert_eq!(reply["io_retired"], false, "live native I/O lease was lost");
    drop(input);
    drop(output);
    peer.finish_io().unwrap();
    id += 1;
    reply = request(&mut control, &endpoint, id, "status");
    assert_eq!(reply["tree_retired"], true);
    assert_eq!(reply["io_retired"], true);
    let child = peer._child;
    drop(peer.control);
    drop(control);
    let deadline = Instant::now() + TIMEOUT;
    while !native::signalled(child.handle()).unwrap() {
        assert!(
            Instant::now() < deadline,
            "completed guard retained a control owner"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
