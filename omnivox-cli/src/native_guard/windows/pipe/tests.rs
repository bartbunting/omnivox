use super::*;
use std::time::Duration;

const BUDGET: Duration = Duration::from_secs(5);

#[test]
fn helper_stdio_supports_sync_native_and_async_rust_ends() {
    let (mut helper_input, worker_input) = helper_stdio(false).unwrap();
    let (mut helper_output, worker_output) = helper_stdio(true).unwrap();
    let helper = std::thread::spawn(move || {
        let mut input = Vec::new();
        helper_input.read_to_end(&mut input).unwrap();
        assert_eq!(input, vec![42; 256 * 1024]);
        helper_output.write_all(&input).unwrap();
    });
    let mut input = std::process::ChildStdin::from(OwnedHandle::from(worker_input));
    let mut output = std::process::ChildStdout::from(OwnedHandle::from(worker_output));
    input.write_all(&vec![42; 256 * 1024]).unwrap();
    drop(input);
    let mut echoed = Vec::new();
    output.read_to_end(&mut echoed).unwrap();
    assert_eq!(echoed, vec![42; 256 * 1024]);
    helper.join().unwrap();
}
fn until(mut poll: impl FnMut() -> bool) {
    let deadline = Instant::now() + BUDGET;
    while !poll() {
        assert!(
            Instant::now() < deadline,
            "native pipe operation did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn pair(listener: &mut Listener) -> (Stream, Stream) {
    let mut connector = Connector::new(listener.address(), Instant::now() + BUDGET).unwrap();
    let mut client = None;
    let mut server = None;
    until(|| {
        if client.is_none() {
            client = connector.poll(Instant::now()).unwrap();
        }
        if server.is_none() {
            server = listener.accept().unwrap();
        }
        client.is_some() && server.is_some()
    });
    (server.unwrap(), client.unwrap())
}
fn retire(stream: &mut Stream) {
    stream.close().unwrap();
    until(|| stream.poll_closed().unwrap());
    assert!(stream.handle.is_none());
    assert!(!stream.read.pending && !stream.write.pending && !stream.connect.pending);
    // Repetition cannot restart I/O or manufacture another handle.
    stream.close().unwrap();
    assert!(stream.poll_closed().unwrap());
    assert_eq!(
        stream.write(b"late").unwrap_err().kind(),
        io::ErrorKind::NotConnected
    );
}
fn send(stream: &mut Stream, bytes: &[u8]) {
    let mut offset = 0;
    until(|| {
        match stream.write(&bytes[offset..]) {
            Ok(n) => offset += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("write failed: {e}"),
        }
        offset == bytes.len()
    });
}
fn receive(stream: &mut Stream, expected: &[u8]) {
    let mut bytes = vec![0; expected.len()];
    let mut offset = 0;
    until(|| {
        match stream.read(&mut bytes[offset..]) {
            Ok(0) => panic!("unexpected EOF"),
            Ok(n) => offset += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("read failed: {e}"),
        }
        offset == bytes.len()
    });
    assert_eq!(bytes, expected);
}

#[test]
fn listener_created_on_finished_setup_thread_accepts_and_retires() {
    let mut listener = std::thread::spawn(|| {
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap())
    })
    .join()
    .unwrap()
    .unwrap();
    assert!(!listener.pending.as_ref().unwrap().connect.pending);
    let (mut server, mut client) = pair(&mut listener);
    send(&mut client, b"after setup thread exits");
    receive(&mut server, b"after setup thread exits");
    retire(&mut client);
    retire(&mut server);
    listener.close().unwrap();
    until(|| listener.poll_closed().unwrap());
}

#[test]
fn local_names_only_and_connection_deadline_never_retries_after_expiry() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    let address = listener.address().to_owned();
    for invalid in [
        address.replace(r"\\.\", r"\\localhost\"),
        address.replace(r"\\.\", r"\\server\"),
        format!("{address}\\other"),
        format!("{address}\0"),
        r"C:\pipe".into(),
        r"\\.\pipe\omnivox-guard-not-a-uuid".into(),
    ] {
        assert!(Connector::new(&invalid, Instant::now() + BUDGET).is_err());
    }
    assert!(valid_address(&address));
    // Occupy the sole listening instance without accepting it.
    let mut occupied = Connector::new(&address, Instant::now() + BUDGET).unwrap();
    let mut occupied = occupied.poll(Instant::now()).unwrap().unwrap();
    let now = Instant::now();
    let mut waiting = Connector::new(&address, now + BUDGET).unwrap();
    assert!(waiting.poll(now).unwrap().is_none());
    assert_eq!(
        waiting.poll(now + BUDGET).err().unwrap().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(
        waiting.poll(now).err().unwrap().kind(),
        io::ErrorKind::NotConnected
    );
    retire(&mut occupied);
    listener.close().unwrap();
    until(|| listener.poll_closed().unwrap());
}

#[test]
fn duplex_reads_peek_and_partial_consumption_preserve_coalesced_tail() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    let (mut server, mut client) = pair(&mut listener);
    assert_eq!(
        server.read(&mut [0; 8]).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(server.read.pending);
    send(&mut client, b"attached\nspeech-after-ack\n");
    let mut peek = [0; 128];
    until(|| match server.peek(&mut peek) {
        Ok(n) => {
            assert_eq!(&peek[..n], b"attached\nspeech-after-ack\n");
            true
        }
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => false,
        Err(e) => panic!("peek failed: {e}"),
    });
    receive(&mut server, b"attached\n");
    // Moving ownership must preserve both native operation storage and the tail.
    let mut moved = std::thread::spawn(move || {
        receive(&mut server, b"speech-after-ack\n");
        send(&mut server, b"reply\n");
        server
    })
    .join()
    .unwrap();
    receive(&mut client, b"reply\n");
    retire(&mut moved);
    retire(&mut client);
}

#[test]
fn saturated_writer_cancellation_does_not_block_other_channels() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    let (mut server, mut client) = pair(&mut listener);
    let bytes = [71; BUFFER];
    // A connected client deliberately never reads this direction.
    for _ in 0..256 {
        match server.write(&bytes) {
            Ok(n) => assert!(n > 0),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => panic!("write failed: {e}"),
        }
    }
    assert!(
        server.write.pending,
        "fixture did not saturate the native pipe"
    );
    assert_eq!(
        server.write(b"changed bytes").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(server.write.pending);
    assert_eq!(
        server.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(server.read.pending);
    // The reverse direction still progresses while the write is saturated.
    send(&mut client, b"reverse");
    receive(&mut server, b"reverse");
    let mut independent =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    let (mut other_server, mut other_client) = pair(&mut independent);
    send(&mut other_client, b"independent");
    receive(&mut other_server, b"independent");
    // Re-establish a pending read so close retires both operations together.
    assert_eq!(
        server.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    retire(&mut server);
    retire(&mut client);
    retire(&mut other_server);
    retire(&mut other_client);
}

#[test]
fn pending_accept_close_and_first_instance_exclusion() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    assert!(listener.accept().unwrap().is_none());
    assert!(listener.pending.as_ref().unwrap().connect.pending);
    // Never reuse an occupied name, even for the same user.
    assert_eq!(
        listener
            .security
            .pipe(&listener.name, true)
            .err()
            .unwrap()
            .raw_os_error(),
        Some(5)
    );
    assert!(!listener.poll_closed().is_ok());
    listener.close().unwrap();
    until(|| listener.poll_closed().unwrap());
    assert!(listener.pending.is_none());
    assert!(listener.accept().is_err());
    let mut connector = Connector::new(listener.address(), Instant::now() + BUDGET).unwrap();
    assert!(connector.poll(Instant::now()).is_err());
}

#[test]
fn native_acl_is_private_handles_do_not_inherit_and_restricted_user_is_denied() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetHandleInformation(handle: Handle, flags: *mut u32) -> i32;
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor: *const c_void,
            revision: u32,
            information: u32,
            text: *mut *mut u16,
            length: *mut u32,
        ) -> i32;
        fn CreateRestrictedToken(
            token: Handle,
            flags: u32,
            disable_count: u32,
            disable: *const SidAndAttributes,
            delete_count: u32,
            delete: *const c_void,
            restrict_count: u32,
            restrict: *const c_void,
            output: *mut Handle,
        ) -> i32;
        fn ImpersonateLoggedOnUser(token: Handle) -> i32;
        fn RevertToSelf() -> i32;
    }
    #[repr(C)]
    struct SidAndAttributes {
        sid: *const c_void,
        attributes: u32,
    }
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    let pending = listener.pending.as_ref().unwrap();
    for handle in [
        pending.handle.as_ref().unwrap(),
        &pending.connect.event,
        &pending.read.event,
        &pending.write.event,
    ] {
        let mut flags = 0;
        check(unsafe { GetHandleInformation(handle.as_raw_handle(), &mut flags) }).unwrap();
        assert_eq!(flags & 1, 0, "pipe/event handle is inheritable");
    }
    let mut descriptor = null_mut();
    // Read actual native owner/DACL, not just our proposed SDDL.
    assert_eq!(
        unsafe {
            GetSecurityInfo(
                pending.handle.as_ref().unwrap().as_raw_handle(),
                1,
                5,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        },
        0
    );
    let _descriptor = security::Local(descriptor);
    fn descriptor_text(descriptor: *const c_void) -> String {
        let mut text = null_mut();
        let mut length = 0;
        check(unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                1,
                5,
                &mut text,
                &mut length,
            )
        })
        .unwrap();
        let _text = security::Local(text.cast());
        String::from_utf16(unsafe { std::slice::from_raw_parts(text, length as usize - 1) })
            .unwrap()
    }
    let sid = security::user_sid().unwrap();
    // Windows expands generic-all to file-all (FA), and may abbreviate a
    // well-known account SID (for example the CI administrator) as LA. Render
    // the independently expected owner and protected DACL through the same
    // native conversion before comparing, without accepting other trustees.
    let expected_sddl = wide(&format!("O:{sid}D:P(A;;FA;;;{sid})"));
    let mut expected = null_mut();
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            expected_sddl.as_ptr(),
            1,
            &mut expected,
            null_mut(),
        )
    })
    .unwrap();
    let _expected = security::Local(expected);
    assert_eq!(descriptor_text(descriptor), descriptor_text(expected));

    let mut token = null_mut();
    check(unsafe { OpenProcessToken(GetCurrentProcess(), 14, &mut token) }).unwrap();
    let token = unsafe { owned(token) }.unwrap();
    let mut user = [0usize; 128];
    let mut length = 0;
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            1,
            user.as_mut_ptr().cast(),
            std::mem::size_of_val(&user) as u32,
            &mut length,
        )
    })
    .unwrap();
    let disable = SidAndAttributes {
        sid: user[0] as *const c_void,
        attributes: 0,
    };
    let mut restricted = null_mut();
    check(unsafe {
        CreateRestrictedToken(
            token.as_raw_handle(),
            1,
            1,
            &disable,
            0,
            null(),
            0,
            null(),
            &mut restricted,
        )
    })
    .unwrap();
    let restricted = unsafe { owned(restricted) }.unwrap();
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            // Leaving a reused test thread impersonated would invalidate later tests.
            if unsafe { RevertToSelf() } == 0 {
                std::process::abort();
            }
        }
    }
    check(unsafe { ImpersonateLoggedOnUser(restricted.as_raw_handle()) }).unwrap();
    let revert = Revert;
    let attempt = security::client(&listener.name);
    drop(revert);
    assert_eq!(attempt.err().unwrap().raw_os_error(), Some(5));
    // An ordinary same-user attachment still works after denied access.
    let (mut server, mut client) = pair(&mut listener);
    send(&mut client, b"authorized");
    receive(&mut server, b"authorized");
    retire(&mut server);
    retire(&mut client);
}

#[test]
fn abrupt_peer_process_exit_finishes_pending_read_without_reconnect() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    // The child receives only a synthetic pipe name, never a real guard token.
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_guard::os::pipe::tests::pipe_child",
            "--nocapture",
        ])
        .env("OMNIVOX_TEST_PRIVATE_PIPE", listener.address())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(child);
    let mut server = None;
    until(|| {
        server = listener.accept().unwrap();
        server.is_some()
    });
    let mut server = server.unwrap();
    receive(&mut server, b"connected");
    assert_eq!(
        server.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    until(|| match server.read(&mut [0]) {
        Ok(0) => true,
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => false,
        result => panic!("unexpected peer-exit read: {result:?}"),
    });
    assert!(!server.read.pending);
    retire(&mut server);
}

#[test]
fn pipe_child() {
    let Ok(address) = std::env::var("OMNIVOX_TEST_PRIVATE_PIPE") else {
        return;
    };
    let mut connector = Connector::new(&address, Instant::now() + BUDGET).unwrap();
    let mut stream = connector.poll(Instant::now()).unwrap().unwrap();
    send(&mut stream, b"connected");
    // The owning test kills this process; a timeout cannot pass its assertions.
    std::thread::sleep(Duration::from_secs(15));
    panic!("parent did not terminate pipe fixture");
}

#[test]
fn repeated_pending_io_retirement_keeps_handles_bounded() {
    if std::env::var_os("OMNIVOX_TEST_PIPE_HANDLE_COUNTS").is_none() {
        // Isolate counts from other unit tests' unrelated process handles.
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_guard::os::pipe::tests::repeated_pending_io_retirement_keeps_handles_bounded", "--nocapture"])
            .env("OMNIVOX_TEST_PIPE_HANDLE_COUNTS", "1")
            .status().unwrap();
        assert!(status.success());
        return;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHandleCount(process: Handle, count: *mut u32) -> i32;
    }
    let count = || {
        let mut n = 0;
        check(unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut n) }).unwrap();
        n
    };
    let before = count();
    let mut baseline = None;
    // The first native security/RNG calls initialize process-wide Windows
    // services. Measure forty more lifetimes after two warm-up cycles.
    for iteration in 0..42 {
        let mut listener =
            Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
        let (mut a, mut b) = pair(&mut listener);
        assert_eq!(
            a.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            b.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        retire(&mut a);
        retire(&mut b);
        listener.close().unwrap();
        until(|| listener.poll_closed().unwrap());
        drop(listener);
        drop(a);
        drop(b);
        if iteration == 1 {
            baseline = Some(count());
        }
        if matches!(iteration, 0 | 1 | 41) {
            eprintln!(
                "native handles: before={before}, iteration={iteration}, count={}",
                count()
            );
        }
    }
    assert!(
        count() <= baseline.unwrap() + 2,
        "native pipe handles accumulated"
    );
}

#[test]
fn large_duplex_transfer_survives_pending_writes_and_small_reads() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    let (mut server, mut client) = pair(&mut listener);
    let bytes: Vec<u8> = (0..5 * BUFFER + 71).map(|i| (i % 251) as u8).collect();
    let mut written = 0;
    let mut received = Vec::new();
    until(|| {
        if written < bytes.len() {
            match server.write(&bytes[written..]) {
                Ok(n) => written += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("large write failed: {e}"),
            }
        }
        let mut scratch = [0; 317];
        match client.read(&mut scratch) {
            Ok(0) => panic!("early EOF"),
            Ok(n) => received.extend_from_slice(&scratch[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("large read failed: {e}"),
        }
        written == bytes.len() && received.len() == bytes.len()
    });
    assert_eq!(received, bytes);
    retire(&mut server);
    retire(&mut client);
}

#[test]
fn pending_read_owner_can_move_to_another_thread_before_completion() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    let (mut server, mut client) = pair(&mut listener);
    assert_eq!(
        server.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    let thread = std::thread::spawn(move || {
        receive(&mut server, b"after ownership transfer");
        retire(&mut server);
    });
    send(&mut client, b"after ownership transfer");
    thread.join().unwrap();
    retire(&mut client);
}

#[test]
fn repeated_client_retirement_reuses_bounded_listener_without_leaked_instances() {
    let mut listener =
        Listener::bind(&omnivox_tts::voice_library::local::new_uuid().unwrap()).unwrap();
    for index in 0..64u8 {
        let (mut server, mut client) = pair(&mut listener);
        send(&mut client, &[index]);
        receive(&mut server, &[index]);
        send(&mut server, &[index + 64]);
        receive(&mut client, &[index + 64]);
        retire(&mut server);
        retire(&mut client);
    }
    listener.close().unwrap();
    until(|| listener.poll_closed().unwrap());
}
