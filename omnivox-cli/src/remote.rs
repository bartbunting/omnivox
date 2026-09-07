//! Loopback broker. Authentication and bounded framing precede stdio workers.
use anyhow::{bail, Context, Result};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_LINE: usize = 512 * 1024;
const LEASE: Duration = Duration::from_secs(20);
const TICK: Duration = Duration::from_millis(100);
static STOP: AtomicBool = AtomicBool::new(false);

struct Config {
    address: SocketAddr,
    token: String,
    sound_root: Option<PathBuf>,
    audio_output: Option<String>,
}

impl Config {
    fn parse(args: &[String]) -> Result<Self> {
        let mut address = "127.0.0.1:6417".parse::<SocketAddr>()?;
        let mut token_file = None;
        let mut sound_root = None;
        let mut audio_output = None;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--serve" {
                continue;
            }
            let value = args.next().context("missing service option value")?;
            match arg.as_str() {
                "--listen" => address = value.parse().context("listen requires an IP and port")?,
                "--token-file" => token_file = Some(PathBuf::from(value)),
                "--sound-root" => {
                    let root = std::fs::canonicalize(value).context("invalid sound root")?;
                    if !root.is_dir() {
                        bail!("sound root must be a directory");
                    }
                    sound_root = Some(root);
                }
                "--audio-output" if matches!(value.as_str(), "device" | "pulse" | "null") => {
                    audio_output = Some(value.clone());
                }
                _ => bail!("unsupported service option (see docs/protocols/REMOTE-PROTOCOL.md)"),
            }
        }
        if !address.ip().is_loopback() {
            bail!("remote service must listen on loopback; use SSH reverse forwarding");
        }
        let file = std::fs::File::open(token_file.context("--serve requires --token-file PATH")?)
            .context("could not open remote token file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file.metadata()?.permissions().mode() & 0o077 != 0 {
                bail!("remote token file must be private (chmod 600)");
            }
        }
        let mut token = String::new();
        if file.metadata()?.len() > 66 {
            bail!("remote token file is too large");
        }
        file.take(67)
            .read_to_string(&mut token)
            .context("invalid token file")?;
        let token = token.strip_suffix('\n').unwrap_or(&token);
        let token = token.strip_suffix('\r').unwrap_or(token).to_string();
        if !hex_id(&token, 64) {
            bail!("remote token must contain exactly 64 lowercase hexadecimal characters");
        }
        Ok(Self {
            address,
            token,
            sound_root,
            audio_output,
        })
    }
}

fn hex_id(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn authenticate<'a>(line: &'a [u8], token: &str) -> Option<(&'a str, usize)> {
    let words: Vec<_> = std::str::from_utf8(line)
        .ok()?
        .split_ascii_whitespace()
        .collect();
    if words.len() != 5
        || words[0] != "OMNIVOX-REMOTE"
        || words[1] != "1"
        || !hex_id(words[2], 64)
        || !hex_id(words[3], 32)
    {
        return None;
    }
    // Always compare every token byte; neither the input nor the secret is logged.
    let difference = words[2]
        .bytes()
        .zip(token.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b));
    if difference != 0 {
        return None;
    }
    let lane = match words[4] {
        "speaker" => 0,
        "notification" => 1,
        _ => return None,
    };
    Some((words[3], lane))
}

#[derive(Default)]
struct Session {
    id: String,
    lanes: [bool; 2],
}

struct Lane {
    session: Arc<Mutex<Session>>,
    index: usize,
}

impl Lane {
    fn reserve(session: &Arc<Mutex<Session>>, id: &str, index: usize) -> Option<Self> {
        let mut state = session.lock().ok()?;
        if state.lanes.iter().any(|active| *active) && state.id != id || state.lanes[index] {
            return None;
        }
        state.id = id.to_string();
        state.lanes[index] = true;
        Some(Self {
            session: session.clone(),
            index,
        })
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        if let Ok(mut state) = self.session.lock() {
            state.lanes[self.index] = false;
        }
    }
}

/// Retain incomplete input across socket timeouts, with a hard allocation bound.
struct Records<R> {
    reader: R,
    pending: Vec<u8>,
}

impl<R: BufRead> Records<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
        }
    }

    fn next(&mut self, limit: usize) -> io::Result<Option<Vec<u8>>> {
        {
            let bytes = match self.reader.fill_buf() {
                Ok(bytes) => bytes,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(None)
                }
                Err(error) => return Err(error),
            };
            if bytes.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let end = bytes.iter().position(|b| *b == b'\n');
            let count = end.map_or(bytes.len(), |n| n + 1);
            if self.pending.len() + count > limit {
                return Err(io::ErrorKind::InvalidData.into());
            }
            self.pending.extend_from_slice(&bytes[..count]);
            self.reader.consume(count);
            if end.is_some() {
                if self.pending.contains(&0) || std::str::from_utf8(&self.pending).is_err() {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                return Ok(Some(std::mem::take(&mut self.pending)));
            }
            Ok(None)
        }
    }
}

struct Worker {
    child: Child,
    #[cfg(windows)]
    job: crate::remote_windows::Job,
}

impl Worker {
    fn start(config: &Config, lane: usize) -> Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env("OMNIVOX_REMOTE_WORKER", "1")
            .env_remove("OMNIVOX_REMOTE_SOUND_ROOT");
        if let Some(root) = &config.sound_root {
            command.env("OMNIVOX_REMOTE_SOUND_ROOT", root);
        }
        if let Some(output) = &config.audio_output {
            command.args(["--audio-output", output]);
        }
        // Device selection belongs to the workstation, never to the remote peer.
        if lane == 1 {
            if let Some(target) = std::env::var_os("OMNIVOX_REMOTE_NOTIFICATION_TARGET") {
                command.env("OMNIVOX_AUDIO_TARGET", target);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x00000200); // CREATE_NEW_PROCESS_GROUP
        }
        #[cfg(windows)]
        let job = crate::remote_windows::Job::new()?;
        let mut worker = Self {
            child: command
                .spawn()
                .context("could not start remote speech worker")?,
            #[cfg(windows)]
            job,
        };
        #[cfg(windows)]
        worker.job.assign(&worker.child)?;
        // No engine/helper can start before Windows job ownership is established.
        worker
            .child
            .stdin
            .as_mut()
            .context("missing worker stdin")?
            .write_all(b"START\n")?;
        Ok(worker)
    }
}

pub fn await_worker_start() -> Result<()> {
    if std::env::var_os("OMNIVOX_REMOTE_WORKER").is_some() {
        let mut start = [0u8; 6];
        io::stdin()
            .read_exact(&mut start)
            .context("remote broker closed before worker startup")?;
        if &start != b"START\n" {
            bail!("invalid remote worker startup");
        }
    }
    Ok(())
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Kill the owned tree even if the main worker has already exited.
        #[cfg(unix)]
        unsafe {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            // SAFETY: the child is the leader of the private group created above.
            kill(-(self.child.id() as i32), 9);
        }
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn reject(stream: &mut TcpStream, reason: &str) {
    let _ = writeln!(stream, "OMNIVOX-REMOTE 1 error {reason}");
}

fn connection(mut stream: TcpStream, config: &Config, session: &Arc<Mutex<Session>>) -> Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(TICK))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut input = Records::new(BufReader::new(stream.try_clone()?));
    let deadline = Instant::now() + Duration::from_secs(5);
    let hello = loop {
        if STOP.load(Ordering::Acquire) || Instant::now() >= deadline {
            reject(&mut stream, "handshake");
            return Ok(());
        }
        match input.next(256) {
            Ok(Some(line)) => break line,
            Ok(None) => (),
            Err(_) => {
                reject(&mut stream, "handshake");
                return Ok(());
            }
        }
    };
    let Some((id, index)) = authenticate(&hello, &config.token) else {
        reject(&mut stream, "authentication");
        return Ok(());
    };
    let Some(_lane) = Lane::reserve(session, id, index) else {
        reject(&mut stream, "busy");
        return Ok(());
    };
    let mut worker = match Worker::start(config, index) {
        Ok(worker) => worker,
        Err(_) => {
            reject(&mut stream, "worker");
            return Ok(());
        }
    };
    stream.write_all(b"OMNIVOX-REMOTE 1 ready\n")?;
    let socket = Arc::new(Mutex::new(stream.try_clone()?));
    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(4);
    let mut stdin = worker.child.stdin.take().context("missing worker stdin")?;
    let stdout = worker
        .child
        .stdout
        .take()
        .context("missing worker stdout")?;
    let writer_done = done.clone();
    let writer = thread::spawn(move || {
        while let Ok(line) = rx.recv() {
            if writer_done.load(Ordering::Acquire) || stdin.write_all(&line).is_err() {
                break;
            }
        }
        writer_done.store(true, Ordering::Release);
    });
    let output_done = done.clone();
    let output_socket = socket.clone();
    let output = thread::spawn(move || {
        let mut records = Records::new(BufReader::new(stdout));
        while !output_done.load(Ordering::Acquire) {
            match records.next(MAX_LINE) {
                Ok(Some(line)) => {
                    if output_socket.lock().unwrap().write_all(&line).is_err() {
                        break;
                    }
                }
                Ok(None) => continue,
                Err(_) => break,
            }
        }
        output_done.store(true, Ordering::Release);
    });
    let mut last_record = Instant::now();
    while !STOP.load(Ordering::Acquire)
        && !done.load(Ordering::Acquire)
        && last_record.elapsed() < LEASE
    {
        // Do not reap before retiring the process tree on Windows.
        match input.next(MAX_LINE) {
            Ok(Some(line)) => {
                last_record = Instant::now();
                if line == b"OMNIVOX-REMOTE ping\n" || line == b"OMNIVOX-REMOTE ping\r\n" {
                    if socket
                        .lock()
                        .unwrap()
                        .write_all(b"OMNIVOX-REMOTE pong\n")
                        .is_err()
                    {
                        break;
                    }
                } else if line.starts_with(b"OMNIVOX-REMOTE") || !forward(&tx, line, &done) {
                    break;
                }
            }
            Ok(None) => (),
            Err(_) => break,
        }
    }
    done.store(true, Ordering::Release);
    let _ = stream.shutdown(Shutdown::Both);
    drop(tx);
    drop(worker); // Unblock pipe writers/readers before joining either thread.
    let _ = writer.join();
    let _ = output.join();
    Ok(())
}

fn forward(tx: &mpsc::SyncSender<Vec<u8>>, mut line: Vec<u8>, done: &AtomicBool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if done.load(Ordering::Acquire) || STOP.load(Ordering::Acquire) {
            return false;
        }
        match tx.try_send(line) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                line = returned;
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
}

#[cfg(unix)]
fn install_interrupt_handler() {
    extern "C" fn stop(_: i32) {
        STOP.store(true, Ordering::Release);
    }
    unsafe extern "C" {
        fn signal(number: i32, handler: usize) -> usize;
    }
    // SAFETY: handler only stores to a lock-free atomic; the binary owns signals.
    unsafe {
        signal(2, stop as *const () as usize);
        signal(15, stop as *const () as usize);
    }
}

#[cfg(windows)]
fn install_interrupt_handler() {
    unsafe extern "system" fn stop(event: u32) -> i32 {
        if event <= 2 || event == 5 || event == 6 {
            STOP.store(true, Ordering::Release);
            // Windows close/logoff handlers must stay alive during cleanup.
            if event >= 2 {
                thread::sleep(Duration::from_secs(4));
            }
            1
        } else {
            0
        }
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    // SAFETY: this static handler outlives the service.
    unsafe {
        SetConsoleCtrlHandler(Some(stop), 1);
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let config = Arc::new(Config::parse(args)?);
    let listener = TcpListener::bind(config.address).context("could not bind remote listener")?;
    listener.set_nonblocking(true)?;
    install_interrupt_handler();
    eprintln!(
        "Omnivox remote service listening on {}",
        listener.local_addr()?
    );
    let session = Arc::new(Mutex::new(Session::default()));
    let count = Arc::new(AtomicUsize::new(0));
    thread::spawn(|| {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) if line.trim() == "quit" => {
                    STOP.store(true, Ordering::Release);
                    break;
                }
                Err(_) => break,
                _ => (),
            }
        }
    });
    let mut connections: Vec<thread::JoinHandle<()>> = Vec::new();
    while !STOP.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) if count.load(Ordering::Acquire) < 4 => {
                count.fetch_add(1, Ordering::AcqRel);
                let count = count.clone();
                let config = config.clone();
                let session = session.clone();
                connections.push(thread::spawn(move || {
                    let _ = connection(stream, &config, &session);
                    count.fetch_sub(1, Ordering::AcqRel);
                }));
            }
            Ok(_) => (), // Closing surplus sockets is bounded and needs no worker.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(TICK),
            Err(_) => {
                STOP.store(true, Ordering::Release);
            }
        }
        let mut index = 0;
        while index < connections.len() {
            if connections[index].is_finished() {
                let _ = connections.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
    }
    drop(listener);
    for connection in connections {
        let _ = connection.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_checks_version_token_session_and_lane() {
        let token = "a".repeat(64);
        let hello = format!("OMNIVOX-REMOTE 1 {token} {} speaker\n", "b".repeat(32));
        assert_eq!(authenticate(hello.as_bytes(), &token).unwrap().1, 0);
        for bad in [
            hello.replace("speaker", "admin"),
            hello.replace(" 1 ", " 2 "),
            hello.replace(&token, &"c".repeat(64)),
            hello.replace(&"b".repeat(32), "oops"),
        ] {
            assert!(authenticate(bad.as_bytes(), &token).is_none());
        }
    }

    #[test]
    fn two_lanes_share_only_one_session_and_release_after_retirement() {
        let state = Arc::new(Mutex::new(Session::default()));
        let speaker = Lane::reserve(&state, "one", 0).unwrap();
        assert!(Lane::reserve(&state, "one", 0).is_none());
        assert!(Lane::reserve(&state, "two", 1).is_none());
        let notification = Lane::reserve(&state, "one", 1).unwrap();
        drop(speaker);
        assert!(Lane::reserve(&state, "two", 0).is_none());
        drop(notification);
        assert!(Lane::reserve(&state, "two", 0).is_some());
    }

    #[test]
    fn framing_preserves_records_and_rejects_partial_and_oversized_input() {
        let mut lines = Records::new(io::Cursor::new(b"q hello\nd\n"));
        assert_eq!(lines.next(8).unwrap().unwrap(), b"q hello\n");
        assert_eq!(lines.next(8).unwrap().unwrap(), b"d\n");
        for bytes in [b"partial".as_slice(), b"12345678\n", b"bad\0\n", b"\xff\n"] {
            let mut records = Records::new(io::Cursor::new(bytes));
            match records.next(8) {
                Err(_) => (),
                Ok(None) => assert!(records.next(8).is_err()),
                Ok(Some(_)) => panic!("invalid record was accepted"),
            }
        }
    }
}
