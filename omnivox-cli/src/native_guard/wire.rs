use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Read, Write};
use std::time::{Duration, Instant};

pub const MAX_FRAME: usize = 8192;
pub const TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: u64,
    pub command: String,
    #[serde(default)]
    pub guard: String,
    #[serde(default)]
    pub worker: String,
    #[serde(default)]
    pub token: String,
}

pub fn read_reply(reader: &mut impl BufRead) -> Result<serde_json::Value> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= MAX_FRAME && bytes.last() == Some(&b'\n'),
        "guard response closed or exceeded bound"
    );
    serde_json::from_slice(&bytes).context("invalid guard response")
}

pub struct Channel<S> {
    pub stream: S,
    input: Vec<u8>,
    output: Vec<u8>,
    offset: usize,
    since: Option<Instant>,
    pub closed: bool,
}

impl<S: Read + Write> Channel<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            input: Vec::new(),
            output: Vec::new(),
            offset: 0,
            since: None,
            closed: false,
        }
    }
    pub fn reply(&mut self, value: &impl Serialize) -> Result<()> {
        anyhow::ensure!(self.output.is_empty(), "guard reply already pending");
        self.output = serde_json::to_vec(value)?;
        self.output.push(b'\n');
        anyhow::ensure!(self.output.len() <= MAX_FRAME, "guard reply exceeds bound");
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Option<Request>> {
        if self.closed {
            return Ok(None);
        }
        if !self.output.is_empty() {
            match self.stream.write(&self.output[self.offset..]) {
                Ok(0) => self.closed = true,
                Ok(n) => {
                    self.offset += n;
                    if self.offset == self.output.len() {
                        self.output.clear();
                        self.offset = 0;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => self.closed = true,
            }
            // Backpressure may delay this reply, never the native cleanup loop.
            return Ok(None);
        }
        if self.since.is_some_and(|since| since.elapsed() > TIMEOUT) {
            anyhow::bail!("incomplete guard frame timed out");
        }
        if !self.input.contains(&b'\n') {
            let mut bytes = [0; 2048];
            match self.stream.read(&mut bytes) {
                Ok(0) => self.closed = true,
                Ok(n) => {
                    self.since.get_or_insert_with(Instant::now);
                    self.input.extend_from_slice(&bytes[..n]);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => self.closed = true,
            }
        }
        anyhow::ensure!(self.input.len() <= MAX_FRAME, "guard request exceeds bound");
        if let Some(end) = self.input.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.input.drain(..=end).collect();
            self.since = if self.input.is_empty() {
                None
            } else {
                Some(Instant::now())
            };
            let request: Request = serde_json::from_slice(&line)?;
            anyhow::ensure!(request.request_id > 0, "missing guard request identity");
            return Ok(Some(request));
        }
        Ok(None)
    }
}
