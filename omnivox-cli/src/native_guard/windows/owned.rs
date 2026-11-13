//! One bounded creation task; the control loop always retains retirement authority.
use super::{
    helper_wire, os, readiness,
    windows_plan::Plan,
    wire::{Channel, TIMEOUT},
    TICK,
};
use crate::remote_windows::Job;
use anyhow::{Context, Result};
use omnivox_tts::voice_library::local::Startup;
use os::native::{
    self,
    creation::{self, Created},
};
use std::fs::File;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, RawHandle};
use std::process::Command;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

enum Product {
    Worker {
        child: Created,
        channel: os::Control,
    },
    Helper {
        child: Created,
        input: File,
        output: File,
        id: u64,
        slot: usize,
    },
}
struct Pending {
    request: Option<u64>,
    thread: JoinHandle<Result<Product>>,
    done: Arc<native::Event>,
    deadline: Instant,
}
impl Pending {
    fn start(
        request: Option<u64>,
        work: impl FnOnce() -> Result<Product> + Send + 'static,
    ) -> Result<Self> {
        let done = Arc::new(native::Event::new()?);
        let notify = Arc::clone(&done);
        let thread = std::thread::Builder::new()
            .name("native-creation".into())
            .spawn(move || {
                struct Notify(Arc<native::Event>);
                impl Drop for Notify {
                    fn drop(&mut self) {
                        let _ = self.0.signal();
                    }
                }
                let _notify = Notify(notify);
                work()
            })?;
        Ok(Self {
            request,
            thread,
            done,
            deadline: Instant::now() + TIMEOUT,
        })
    }
}
#[derive(Clone, Copy)]
struct Offer {
    id: u64,
    slot: usize,
    deadline: Instant,
}
pub(super) struct Native {
    // The job outlives every pending create, including a blocked kernel call.
    job: Arc<Job>,
    worker: Option<Created>,
    helpers: Vec<Option<Created>>,
    pending: Option<Pending>,
    channel: Option<Channel<os::Control>>,
    plan: Plan,
    directory: std::path::PathBuf,
    error_output: File,
    total: u32,
    job_stopping: bool,
    sequence: u64,
    offer: Option<Offer>,
    pub started: bool,
    pub closing: bool,
    pub retired: bool,
    pub error: Option<String>,
}
impl Native {
    pub fn start(startup: Startup, pipes: [File; 3], plan: Plan) -> Result<Self> {
        let job = Arc::new(Job::new()?);
        let pending_job = Arc::clone(&job);
        let directory = startup.working_directory.clone();
        let [input, output, error] = pipes;
        let error_output = error.try_clone()?;
        let pending = Pending::start(None, move || {
            crate::emergency::timed("guard-worker-verify", || startup.verify())?;
            let (channel, worker_channel) = os::channel_pair()?;
            let mut command = Command::new(&startup.executable.path);
            startup.environment.apply(&mut command);
            command
                .args(&startup.arguments)
                .current_dir(&startup.working_directory)
                .env_remove("OMNIVOX_REMOTE_WORKER")
                .env("OMNIVOX_OWNED_WORKER", "1");
            let child = crate::emergency::timed("guard-worker-create", || {
                creation::spawn(
                    &command,
                    pending_job.handle(),
                    [input.as_handle(), output.as_handle(), error.as_handle()],
                    Some(worker_channel.as_handle()),
                )
            })?;
            Ok(Product::Worker { child, channel })
        })?;
        Ok(Self {
            job,
            worker: None,
            helpers: (0..plan.helpers.len()).map(|_| None).collect(),
            pending: Some(pending),
            channel: None,
            plan,
            directory,
            error_output,
            total: 0,
            job_stopping: false,
            sequence: 0,
            offer: None,
            started: false,
            closing: false,
            retired: false,
            error: None,
        })
    }
    fn fail(&mut self, error: anyhow::Error) {
        self.error.get_or_insert_with(|| error.to_string());
        self.closing = true;
    }
    pub fn poll(&mut self, closing: bool) -> Result<()> {
        self.closing |= closing;
        if self.retired {
            return Ok(());
        }
        if self
            .worker
            .as_ref()
            .map(Created::ended)
            .transpose()?
            .unwrap_or(false)
        {
            self.closing = true;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| Instant::now() >= p.deadline)
            || self
                .offer
                .is_some_and(|offer| Instant::now() >= offer.deadline)
        {
            self.fail(anyhow::anyhow!("native creation or handoff timed out"));
        }
        if let Err(error) = self.complete() {
            self.fail(error);
        }
        if !self.closing {
            if let Err(error) = self.requests() {
                self.fail(error);
            }
        }
        if self.closing {
            if self.pending.is_some() {
                // Stop known audio owners immediately, while suspended creation
                // finishes independently. TerminateJobObject must not race the
                // atomic job assignment inside an unfinished CreateProcess.
                for child in self
                    .worker
                    .iter_mut()
                    .chain(self.helpers.iter_mut().flatten())
                {
                    child.terminate()?;
                }
            } else if !self.job_stopping {
                self.job.terminate_checked()?;
                self.job_stopping = true;
            }
            // Termination happens before any cancellation that could retain I/O.
            if let Some(channel) = &mut self.channel {
                channel.closed = true;
                if os::close(&mut channel.stream)? {
                    self.channel.take();
                }
            }
            let worker_done = self
                .worker
                .as_ref()
                .map(Created::ended)
                .transpose()?
                .unwrap_or(true);
            let mut helpers_done = true;
            for child in self.helpers.iter().flatten() {
                helpers_done &= child.ended()?;
            }
            if self.pending.is_none() && worker_done && helpers_done && self.channel.is_none() {
                let counts = self.job.counts()?;
                // Counts detect unaccounted creation; they never replace handle completion.
                if counts.total == self.total && counts.active == 0 {
                    self.retired = true;
                } else {
                    self.error.get_or_insert_with(|| {
                        "native job accounting differs from retained creation".into()
                    });
                }
            }
        }
        Ok(())
    }
    fn complete(&mut self) -> Result<()> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.thread.is_finished())
        {
            return Ok(());
        }
        let pending = self.pending.take().unwrap();
        let result = pending
            .thread
            .join()
            .map_err(|_| anyhow::anyhow!("native creation task panicked"))?;
        let product = match result {
            Ok(product) => product,
            Err(error) if pending.request.is_some() && !self.closing => {
                return self.reject(pending.request.unwrap(), error);
            }
            Err(error) => return Err(error),
        };
        self.total = self
            .total
            .checked_add(1)
            .context("native creation count exhausted")?;
        match product {
            Product::Worker { child, channel } => {
                self.worker = Some(child);
                self.channel = Some(Channel::new(channel));
                if !self.closing {
                    self.worker.as_mut().unwrap().release()?;
                    self.started = true;
                }
            }
            Product::Helper {
                child,
                input,
                output,
                id,
                slot,
            } => {
                self.helpers[slot] = Some(child);
                if !self.closing {
                    let target = self
                        .worker
                        .as_ref()
                        .context("missing native receiving worker")?
                        .process
                        .as_handle();
                    let helper = self.helpers[slot].as_ref().unwrap();
                    // Only wait/query/cancel authority. No helper child-creation authority.
                    let process =
                        creation::transfer(helper.process.as_handle(), target, Some(0x10_1001))?;
                    let input = creation::transfer(input.as_handle(), target, None)?;
                    let output = creation::transfer(output.as_handle(), target, None)?;
                    self.offer = Some(Offer {
                        id,
                        slot,
                        deadline: Instant::now() + TIMEOUT,
                    });
                    self.channel
                        .as_mut()
                        .context("missing native creation channel")?
                        .reply(&helper_wire::Reply {
                            request_id: id,
                            state: "offered".into(),
                            handles: Some([process, input, output]),
                            error: None,
                        })?;
                }
            }
        }
        Ok(())
    }
    fn requests(&mut self) -> Result<()> {
        for (slot, helper) in self.helpers.iter_mut().enumerate() {
            if helper
                .as_ref()
                .map(Created::ended)
                .transpose()?
                .unwrap_or(false)
            {
                anyhow::ensure!(
                    self.offer.is_none_or(|offer| offer.slot != slot),
                    "unacknowledged helper ended"
                );
                helper.take();
            }
        }
        let Some(channel) = &mut self.channel else {
            return Ok(());
        };
        let request = channel.poll_as::<helper_wire::Request>()?;
        anyhow::ensure!(!channel.closed, "native helper creation link closed");
        let Some(request) = request else {
            return Ok(());
        };
        anyhow::ensure!(request.request_id > 0, "missing native creation identity");
        match request.command.as_str() {
            "create" => {
                anyhow::ensure!(
                    request.request_id > self.sequence
                        && self.pending.is_none()
                        && self.offer.is_none()
                        && self.total < u32::MAX,
                    "native creation is busy or obsolete"
                );
                let slot = self
                    .plan
                    .helpers
                    .iter()
                    .position(|h| {
                        h.config.engine_id == request.engine && h.identity == request.launch
                    })
                    .context("native helper launch differs from retained snapshot")?;
                anyhow::ensure!(self.helpers[slot].is_none(), "native helper is still owned");
                let helper = self.plan.helpers[slot].clone();
                self.sequence = request.request_id;
                match self.create_helper(helper, request.request_id, slot) {
                    Ok(pending) => self.pending = Some(pending),
                    Err(error) => self.reject(request.request_id, error)?,
                }
            }
            "acknowledge" => {
                anyhow::ensure!(
                    self.offer
                        .is_some_and(|offer| offer.id == request.request_id)
                        && request.engine.is_empty()
                        && request.launch.is_empty(),
                    "obsolete native helper acknowledgement"
                );
                self.helpers[self.offer.unwrap().slot]
                    .as_mut()
                    .context("missing offered helper")?
                    .release()?;
                self.offer.take();
                channel.reply(&helper_wire::Reply {
                    request_id: request.request_id,
                    state: "started".into(),
                    handles: None,
                    error: None,
                })?;
            }
            _ => anyhow::bail!("unsupported native creation request"),
        }
        Ok(())
    }
    fn reject(&mut self, id: u64, error: anyhow::Error) -> Result<()> {
        // The creation API failed before returning a process, and no offer or
        // handle transfer occurred. Refuse this helper, preserving existing
        // speech. Unexpected job activity is still an ownership failure.
        let counts = self.job.counts()?;
        anyhow::ensure!(
            self.offer.is_none()
                && counts.total == self.total
                && counts.active as usize <= 1 + self.helpers.iter().flatten().count(),
            "failed helper creation changed native ownership"
        );
        self.channel
            .as_mut()
            .context("missing native creation channel")?
            .reply(&helper_wire::Reply {
                request_id: id,
                state: "rejected".into(),
                handles: None,
                error: Some(error.to_string()),
            })
    }
    fn create_helper(
        &self,
        helper: super::windows_plan::Helper,
        id: u64,
        slot: usize,
    ) -> Result<Pending> {
        let job = Arc::clone(&self.job);
        let directory = self.directory.clone();
        let error = self.error_output.try_clone()?;
        Pending::start(Some(id), move || {
            let (input, writer) = os::helper_stdio(false)?;
            let (output, reader) = os::helper_stdio(true)?;
            let mut command = Command::new(&helper.config.program);
            helper.config.environment.apply(&mut command);
            command
                .args(&helper.config.arguments)
                .current_dir(directory);
            let child = creation::spawn(
                &command,
                job.handle(),
                [input.as_handle(), output.as_handle(), error.as_handle()],
                None,
            )?;
            Ok(Product::Helper {
                child,
                input: writer,
                output: reader,
                id,
                slot,
            })
        })
    }
    pub fn readiness(
        &self,
        events: &mut Vec<RawHandle>,
        deadline: &mut Option<Instant>,
    ) -> io::Result<bool> {
        if let Some(pending) = &self.pending {
            if pending.thread.is_finished() {
                return Ok(true);
            }
            if native::signalled(pending.done.handle())? {
                // Signalling precedes thread return by a tiny interval; only join
                // completion can release creation ownership. Avoid spinning.
                readiness::earlier(deadline, Instant::now() + TICK);
            } else {
                events.push(pending.done.handle().as_raw_handle());
            }
            if !self.closing {
                readiness::earlier(deadline, pending.deadline);
            }
        }
        for child in self.worker.iter().chain(self.helpers.iter().flatten()) {
            if !child.ended()? {
                events.push(child.process.as_raw_handle());
            }
        }
        if let Some(channel) = &self.channel {
            if readiness::channel(channel, events, deadline) {
                return Ok(true);
            }
        }
        if !self.closing {
            if let Some(offer) = self.offer {
                readiness::earlier(deadline, offer.deadline);
            }
        }
        Ok(false)
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.closing = true;
        let deadline = Instant::now() + TIMEOUT;
        while !self.retired && Instant::now() < deadline {
            let _ = self.poll(true);
            std::thread::sleep(TICK);
        }
        // No receipt is produced by Drop. A pending task retains its job until
        // return; guard process exit closes all remaining handles and threads.
    }
}

#[cfg(test)]
#[path = "owned_tests.rs"]
mod tests;
