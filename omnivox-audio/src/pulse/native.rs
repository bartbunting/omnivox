//! Narrow runtime binding to the system libpulse asynchronous playback API.
//!
//! All pointers stay on their owning output worker. The native event thread
//! uses libpulse's lock; callbacks only touch stable, boxed atomic state. No
//! source iterator or user callback runs while that lock is held.

use super::{PlaybackDevice, BYTES_PER_FRAME, WRITE_FRAMES};
use crate::buffer::{CHANNELS, SAMPLE_RATE};
use crate::CancellationToken;
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

type Ptr = *mut c_void;
type Notify = Option<unsafe extern "C" fn(Ptr, Ptr)>;
type Success = Option<unsafe extern "C" fn(Ptr, c_int, Ptr)>;
const TIMEOUT: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(1);

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

macro_rules! api {
    ($($name:ident: $ty:ty),+ $(,)?) => {
        struct Api { $($name: $ty,)+ _library: Library }
        impl Api {
            fn load() -> Result<Self, String> {
                // The OS client library is optional; missing libpulse must not
                // prevent the existing device/null backends from starting.
                unsafe {
                    let library = Library::new("libpulse.so.0")
                        .map_err(|e| format!("system libpulse.so.0: {e}"))?;
                    Ok(Self {
                        $($name: *library.get(concat!(stringify!($name), "\0").as_bytes())
                            .map_err(|e| e.to_string())?,)+
                        _library: library,
                    })
                }
            }
        }
    };
}

api! {
    pa_threaded_mainloop_new: unsafe extern "C" fn() -> Ptr,
    pa_threaded_mainloop_free: unsafe extern "C" fn(Ptr),
    pa_threaded_mainloop_start: unsafe extern "C" fn(Ptr) -> c_int,
    pa_threaded_mainloop_stop: unsafe extern "C" fn(Ptr),
    pa_threaded_mainloop_lock: unsafe extern "C" fn(Ptr),
    pa_threaded_mainloop_unlock: unsafe extern "C" fn(Ptr),
    pa_threaded_mainloop_get_api: unsafe extern "C" fn(Ptr) -> Ptr,
    pa_context_new: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    pa_context_connect: unsafe extern "C" fn(Ptr, *const c_char, u32, Ptr) -> c_int,
    pa_context_disconnect: unsafe extern "C" fn(Ptr),
    pa_context_unref: unsafe extern "C" fn(Ptr),
    pa_context_get_state: unsafe extern "C" fn(Ptr) -> c_int,
    pa_context_set_state_callback: unsafe extern "C" fn(Ptr, Notify, Ptr),
    pa_context_errno: unsafe extern "C" fn(Ptr) -> c_int,
    pa_strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    pa_stream_new: unsafe extern "C" fn(Ptr, *const c_char, *const SampleSpec, Ptr) -> Ptr,
    pa_stream_connect_playback: unsafe extern "C" fn(Ptr, *const c_char, *const BufferAttr, u32, Ptr, Ptr) -> c_int,
    pa_stream_disconnect: unsafe extern "C" fn(Ptr) -> c_int,
    pa_stream_unref: unsafe extern "C" fn(Ptr),
    pa_stream_get_state: unsafe extern "C" fn(Ptr) -> c_int,
    pa_stream_set_state_callback: unsafe extern "C" fn(Ptr, Notify, Ptr),
    pa_stream_set_underflow_callback: unsafe extern "C" fn(Ptr, Notify, Ptr),
    pa_stream_writable_size: unsafe extern "C" fn(Ptr) -> usize,
    pa_stream_write: unsafe extern "C" fn(Ptr, *const c_void, usize, Option<unsafe extern "C" fn(Ptr)>, i64, c_int) -> c_int,
    pa_stream_cork: unsafe extern "C" fn(Ptr, c_int, Success, Ptr) -> Ptr,
    pa_stream_flush: unsafe extern "C" fn(Ptr, Success, Ptr) -> Ptr,
    pa_stream_drain: unsafe extern "C" fn(Ptr, Success, Ptr) -> Ptr,
    pa_stream_get_latency: unsafe extern "C" fn(Ptr, *mut u64, *mut c_int) -> c_int,
    pa_stream_get_buffer_attr: unsafe extern "C" fn(Ptr) -> *const BufferAttr,
    pa_operation_get_state: unsafe extern "C" fn(Ptr) -> c_int,
    pa_operation_cancel: unsafe extern "C" fn(Ptr),
    pa_operation_unref: unsafe extern "C" fn(Ptr),
}

struct Callbacks {
    closed: CancellationToken,
    context_state: unsafe extern "C" fn(Ptr) -> c_int,
    stream_state: unsafe extern "C" fn(Ptr) -> c_int,
    active: AtomicBool,
    underflows: AtomicU64,
    active_underflows: AtomicU64,
}

unsafe extern "C" fn context_changed(context: Ptr, data: Ptr) {
    let state = unsafe { &*data.cast::<Callbacks>() };
    if unsafe { (state.context_state)(context) } >= 5 {
        state.closed.cancel();
    }
}

unsafe extern "C" fn stream_changed(stream: Ptr, data: Ptr) {
    let state = unsafe { &*data.cast::<Callbacks>() };
    if unsafe { (state.stream_state)(stream) } >= 3 {
        state.closed.cancel();
    }
}

unsafe extern "C" fn underflow(_stream: Ptr, data: Ptr) {
    let state = unsafe { &*data.cast::<Callbacks>() };
    state.underflows.fetch_add(1, Ordering::Relaxed);
    if state.active.load(Ordering::Acquire) {
        state.active_underflows.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe extern "C" fn success(_stream: Ptr, ok: c_int, data: Ptr) {
    // The result lives until the operation is done or explicitly cancelled,
    // always under the same mainloop lock.
    unsafe { *data.cast::<Option<bool>>() = Some(ok != 0) };
}

pub(super) struct Client {
    api: Api,
    mainloop: Ptr,
    started: bool,
    context: Ptr,
    stream: Ptr,
    callbacks: Box<Callbacks>,
    drain: Ptr,
    drain_result: Box<Option<bool>>,
    name: &'static str,
    last_report: Option<Instant>,
}

struct Lock<'a>(&'a Client);
impl Drop for Lock<'_> {
    fn drop(&mut self) {
        unsafe { (self.0.api.pa_threaded_mainloop_unlock)(self.0.mainloop) };
    }
}

impl Client {
    fn lock(&self) -> Lock<'_> {
        unsafe { (self.api.pa_threaded_mainloop_lock)(self.mainloop) };
        Lock(self)
    }

    fn error(&self, action: &str) -> String {
        let _lock = self.lock();
        let detail = unsafe {
            CStr::from_ptr((self.api.pa_strerror)((self.api.pa_context_errno)(
                self.context,
            )))
        };
        format!(
            "PulseAudio {} {action}: {}",
            self.name,
            detail.to_string_lossy()
        )
    }

    fn check(&self) -> Result<(), String> {
        if self.callbacks.closed.is_cancelled() {
            Err(self.error("connection closed"))
        } else {
            Ok(())
        }
    }

    pub(super) fn open(
        name: &'static str,
        latency_ms: u32,
        closed: CancellationToken,
    ) -> Result<Self, String> {
        let api = Api::load()?;
        let callbacks = Box::new(Callbacks {
            closed,
            context_state: api.pa_context_get_state,
            stream_state: api.pa_stream_get_state,
            active: AtomicBool::new(false),
            underflows: AtomicU64::new(0),
            active_underflows: AtomicU64::new(0),
        });
        let mut client = Self {
            mainloop: unsafe { (api.pa_threaded_mainloop_new)() },
            started: false,
            context: ptr::null_mut(),
            stream: ptr::null_mut(),
            drain: ptr::null_mut(),
            drain_result: Box::new(None),
            api,
            callbacks,
            name,
            last_report: None,
        };
        if client.mainloop.is_null() {
            return Err("PulseAudio mainloop allocation failed".into());
        }
        if unsafe { (client.api.pa_threaded_mainloop_start)(client.mainloop) } < 0 {
            return Err("PulseAudio mainloop start failed".into());
        }
        client.started = true;
        let label = CString::new(format!("Omnivox {name}")).unwrap();
        let data = (&mut *client.callbacks as *mut Callbacks).cast();
        unsafe {
            let _lock = client.lock();
            let context = (client.api.pa_context_new)(
                (client.api.pa_threaded_mainloop_get_api)(client.mainloop),
                label.as_ptr(),
            );
            if context.is_null() {
                return Err("PulseAudio context allocation failed".into());
            }
            drop(_lock);
            client.context = context;
            let _lock = client.lock();
            (client.api.pa_context_set_state_callback)(context, Some(context_changed), data);
            // Respect PULSE_SERVER/default routing; never autospawn a daemon.
            if (client.api.pa_context_connect)(context, ptr::null(), 1, ptr::null_mut()) < 0 {
                return Err(client.error("connect"));
            }
        }
        client.wait_ready(false)?;
        let spec = SampleSpec {
            format: if cfg!(target_endian = "little") { 5 } else { 6 },
            rate: SAMPLE_RATE,
            channels: CHANNELS as u8,
        };
        let target = SAMPLE_RATE * latency_ms / 1000 * BYTES_PER_FRAME as u32;
        let attr = BufferAttr {
            maxlength: target * 4,
            tlength: target,
            // A single frame reenables automatic prebuffering on underrun.
            // With zero, the read cursor can overtake the write cursor and
            // discard the next speech prefix after a producer/idle gap.
            prebuf: BYTES_PER_FRAME as u32,
            minreq: (WRITE_FRAMES * BYTES_PER_FRAME) as u32,
            fragsize: u32::MAX,
        };
        unsafe {
            let _lock = client.lock();
            let stream =
                (client.api.pa_stream_new)(client.context, label.as_ptr(), &spec, ptr::null_mut());
            if stream.is_null() {
                return Err(client.error("stream allocation"));
            }
            drop(_lock);
            client.stream = stream;
            let _lock = client.lock();
            (client.api.pa_stream_set_state_callback)(stream, Some(stream_changed), data);
            (client.api.pa_stream_set_underflow_callback)(stream, Some(underflow), data);
            // START_CORKED | INTERPOLATE_TIMING | AUTO_TIMING_UPDATE | ADJUST_LATENCY.
            if (client.api.pa_stream_connect_playback)(
                stream,
                ptr::null(),
                &attr,
                0x200b,
                ptr::null_mut(),
                ptr::null_mut(),
            ) < 0
            {
                return Err(client.error("playback connect"));
            }
        }
        client.wait_ready(true)?;
        {
            let _lock = client.lock();
            let actual = unsafe { (client.api.pa_stream_get_buffer_attr)(client.stream).as_ref() };
            tracing::info!(
                stream = name,
                requested_ms = latency_ms,
                ?actual,
                write_frames = WRITE_FRAMES,
                "Native PulseAudio output ready (buffer request is a hint)"
            );
        }
        Ok(client)
    }

    fn wait_ready(&self, stream: bool) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            {
                let _lock = self.lock();
                self.check()?;
                let ready = unsafe {
                    if stream {
                        (self.api.pa_stream_get_state)(self.stream) == 2
                    } else {
                        (self.api.pa_context_get_state)(self.context) == 4
                    }
                };
                if ready {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err("PulseAudio connection timed out".into());
            }
            std::thread::sleep(POLL);
        }
    }

    fn operation(&self, cork: Option<bool>) -> Result<(), String> {
        let action = match cork {
            Some(true) => "cork",
            Some(false) => "uncork",
            None => "flush",
        };
        let mut result = None;
        let operation = {
            let _lock = self.lock();
            self.check()?;
            let data = (&mut result as *mut Option<bool>).cast();
            unsafe {
                match cork {
                    Some(value) => {
                        (self.api.pa_stream_cork)(self.stream, value.into(), Some(success), data)
                    }
                    None => (self.api.pa_stream_flush)(self.stream, Some(success), data),
                }
            }
        };
        if operation.is_null() {
            return Err(self.error(action));
        }
        let deadline = Instant::now() + TIMEOUT;
        loop {
            {
                let _lock = self.lock();
                let state = unsafe { (self.api.pa_operation_get_state)(operation) };
                if state != 0 || Instant::now() >= deadline || self.callbacks.closed.is_cancelled()
                {
                    unsafe {
                        (self.api.pa_operation_cancel)(operation);
                        (self.api.pa_operation_unref)(operation);
                    }
                    return if state == 1 && result == Some(true) {
                        Ok(())
                    } else {
                        Err(self.error(&format!(
                            "{action} failed (operation_state={state}, result={result:?}, timed_out={})",
                            Instant::now() >= deadline,
                        )))
                    };
                }
            }
            std::thread::sleep(POLL);
        }
    }
}

impl PlaybackDevice for Client {
    fn writable_frames(&mut self) -> Result<usize, String> {
        let _lock = self.lock();
        self.check()?;
        let size = unsafe { (self.api.pa_stream_writable_size)(self.stream) };
        if size == usize::MAX {
            Err(self.error("writable size"))
        } else {
            Ok(size / BYTES_PER_FRAME)
        }
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), String> {
        let _lock = self.lock();
        self.check()?;
        // A null free callback makes libpulse copy the bounded slice before return.
        if unsafe {
            (self.api.pa_stream_write)(
                self.stream,
                samples.as_ptr().cast(),
                std::mem::size_of_val(samples),
                None,
                0,
                0,
            )
        } < 0
        {
            return Err(self.error("write"));
        }
        Ok(())
    }

    fn cork(&mut self, corked: bool) -> Result<(), String> {
        if corked {
            return self.operation(Some(true));
        }
        let _lock = self.lock();
        self.check()?;
        // Continue feeding while the server acknowledges resume. Waiting for
        // that round trip can exhaust a small buffer on WSL even when every
        // speech sample is already available. Stops/flushes still wait for
        // acknowledgement before any replacement PCM is submitted.
        let operation = unsafe { (self.api.pa_stream_cork)(self.stream, 0, None, ptr::null_mut()) };
        if operation.is_null() {
            return Err(self.error("uncork"));
        }
        unsafe { (self.api.pa_operation_unref)(operation) };
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        self.operation(None)
    }

    fn active(&mut self, active: bool) {
        self.callbacks.active.store(active, Ordering::Release);
    }

    fn begin_drain(&mut self) -> Result<(), String> {
        self.cancel_drain();
        let data = (&mut *self.drain_result as *mut Option<bool>).cast();
        let operation = {
            let _lock = self.lock();
            self.check()?;
            unsafe { (self.api.pa_stream_drain)(self.stream, Some(success), data) }
        };
        if operation.is_null() {
            return Err(self.error("drain"));
        }
        self.drain = operation;
        Ok(())
    }

    fn drained(&mut self) -> Result<bool, String> {
        let _lock = self.lock();
        self.check()?;
        let state = unsafe { (self.api.pa_operation_get_state)(self.drain) };
        match (state, *self.drain_result) {
            (0, _) => Ok(false),
            (1, Some(true)) => Ok(true),
            _ => Err(self.error("drain failed")),
        }
    }

    fn cancel_drain(&mut self) {
        if !self.drain.is_null() {
            {
                let _lock = self.lock();
                unsafe {
                    (self.api.pa_operation_cancel)(self.drain);
                    (self.api.pa_operation_unref)(self.drain);
                }
            }
            self.drain = ptr::null_mut();
            *self.drain_result = None;
        }
    }

    fn report(&mut self) {
        if self
            .last_report
            .is_some_and(|last| last.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.last_report = Some(Instant::now());
        let _lock = self.lock();
        let (mut usec, mut negative) = (0, 0);
        let latency =
            unsafe { (self.api.pa_stream_get_latency)(self.stream, &mut usec, &mut negative) };
        tracing::info!(
            stream = self.name,
            latency_available = latency == 0,
            latency_usec = usec,
            negative = negative != 0,
            source_active = self.callbacks.active.load(Ordering::Acquire),
            underflows = self.callbacks.underflows.load(Ordering::Relaxed),
            active_underflows = self.callbacks.active_underflows.load(Ordering::Relaxed),
            "PulseAudio timing estimate (not acoustic latency)"
        );
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.mainloop.is_null() {
            return;
        }
        self.cancel_drain();
        {
            let _lock = self.lock();
            unsafe {
                if !self.stream.is_null() {
                    (self.api.pa_stream_set_state_callback)(self.stream, None, ptr::null_mut());
                    (self.api.pa_stream_set_underflow_callback)(self.stream, None, ptr::null_mut());
                    (self.api.pa_stream_disconnect)(self.stream);
                    (self.api.pa_stream_unref)(self.stream);
                }
                if !self.context.is_null() {
                    (self.api.pa_context_set_state_callback)(self.context, None, ptr::null_mut());
                    (self.api.pa_context_disconnect)(self.context);
                    (self.api.pa_context_unref)(self.context);
                }
            }
        }
        unsafe {
            if self.started {
                (self.api.pa_threaded_mainloop_stop)(self.mainloop);
            }
            (self.api.pa_threaded_mainloop_free)(self.mainloop);
        }
    }
}
