//! Poll the owned native queue without exposing Rust callback lifetimes to Cocoa.

use super::{stream, MacOsTtsEngine, SynthTimings, VOICE_CACHE};
use crate::contracts::PhysicalVoiceId;
use crate::{SynthesisRequest, SynthesisStreamCompletion, SynthesisStreamSink, TtsError};
use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());
static STOP_EPOCH: AtomicU64 = AtomicU64::new(0);
static QUARANTINED: AtomicBool = AtomicBool::new(false);
// The pointer is used only while this lock is held. Retirement removes it
// before releasing the native reference, including on error or panic.
static ACTIVE: Mutex<Option<usize>> = Mutex::new(None);

extern "C" {
    fn omnivox_stream_open(
        text: *const c_char,
        language: *const c_char,
        name: *const c_char,
        identifier: *const c_char,
        rate: f32,
        pitch: f32,
        volume: f32,
    ) -> *mut c_void;
    fn omnivox_stream_next(
        handle: *mut c_void,
        samples: *mut f32,
        count: *mut u32,
        rate: *mut u32,
        channels: *mut u16,
        reason: *mut u32,
    ) -> c_int;
    fn omnivox_stream_cancel(handle: *mut c_void);
    fn omnivox_stream_retired(handle: *mut c_void) -> c_int;
    fn omnivox_stream_timings(handle: *mut c_void) -> SynthTimings;
    fn omnivox_stream_release(handle: *mut c_void);
}

pub(super) fn stop() {
    STOP_EPOCH.fetch_add(1, Ordering::AcqRel);
    if let Some(handle) = *ACTIVE.lock().unwrap_or_else(|poison| poison.into_inner()) {
        unsafe {
            omnivox_stream_cancel(handle as *mut c_void);
        }
    }
}

pub(super) fn is_speaking() -> bool {
    ACTIVE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .is_some()
}

fn cancelled(request: &SynthesisRequest, epoch: u64) -> bool {
    epoch != STOP_EPOCH.load(Ordering::Acquire)
        || request
            .cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
}

struct NativeStream(*mut c_void);

impl NativeStream {
    fn retire(&self) -> Result<(), TtsError> {
        unsafe {
            omnivox_stream_cancel(self.0);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { omnivox_stream_retired(self.0) } == 0 {
            if Instant::now() >= deadline {
                QUARANTINED.store(true, Ordering::Release);
                return Err(stream::failed(
                    "native retirement unconfirmed; restart Omnivox before reusing macOS speech",
                ));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        if !QUARANTINED.load(Ordering::Acquire) {
            if let Err(error) = self.retire() {
                tracing::error!(%error, "macOS native retirement failed");
            }
        }
        let mut active = ACTIVE.lock().unwrap_or_else(|poison| poison.into_inner());
        *active = None;
        unsafe {
            omnivox_stream_release(self.0);
        }
    }
}

pub(super) fn synthesize(
    request: &SynthesisRequest,
    sink: &mut dyn SynthesisStreamSink,
) -> Result<SynthesisStreamCompletion, TtsError> {
    let epoch = STOP_EPOCH.load(Ordering::Acquire);
    let _serial = loop {
        if cancelled(request, epoch) {
            return Err(stream::failed("cancelled before native startup"));
        }
        if QUARANTINED.load(Ordering::Acquire) {
            return Err(stream::failed(
                "native capture quarantined; restart Omnivox",
            ));
        }
        match SERIAL.try_lock() {
            Ok(guard) => break guard,
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(2)),
            Err(TryLockError::Poisoned(_)) => return Err(stream::failed("native owner panicked")),
        }
    };
    // Recheck after acquiring ownership: a previous owner may have quarantined
    // the native engine just before releasing the lock.
    if QUARANTINED.load(Ordering::Acquire) || cancelled(request, epoch) {
        return Err(stream::failed("native startup no longer permitted"));
    }
    let voice_id = request.voice_id_for_engine("macos")?;
    let selected = VOICE_CACHE
        .voices()
        .iter()
        .find(|voice| voice.identifier == voice_id);
    if request.requested_voice.is_some() && selected.is_none() {
        return Err(TtsError::VoiceNotFound(voice_id.to_owned()));
    }
    let actual_voice =
        selected.map(|voice| PhysicalVoiceId::new("macos", voice.identifier.clone()));
    if request.text.is_empty() {
        return crate::synthesis::stream_buffered_result(
            request,
            crate::contracts::AnchorSupport::None,
            crate::SynthesisResult::audio("macos", actual_voice, crate::AudioBuffer::empty()),
            sink,
        );
    }
    let text = CString::new(request.text.as_str()).map_err(stream::failed)?;
    let (language, name) = selected.map_or_else(
        || MacOsTtsEngine::parse_voice_id(voice_id),
        |voice| (Some(voice.language.clone()), Some(voice.name.clone())),
    );
    let language = language
        .map(CString::new)
        .transpose()
        .map_err(stream::failed)?;
    let name = name.map(CString::new).transpose().map_err(stream::failed)?;
    let identifier = selected
        .map(|voice| CString::new(voice.identifier.as_str()))
        .transpose()
        .map_err(stream::failed)?;
    let pointer = |value: &Option<CString>| {
        value
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr())
    };
    let handle = unsafe {
        omnivox_stream_open(
            text.as_ptr(),
            pointer(&language),
            pointer(&name),
            pointer(&identifier),
            request.settings.rate,
            request.settings.pitch,
            request.settings.volume,
        )
    };
    if handle.is_null() {
        return Err(stream::failed("could not allocate native capture"));
    }
    let native = NativeStream(handle);
    *ACTIVE.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(handle as usize);
    let interrupted = || cancelled(request, epoch);
    let mut converter =
        stream::Stream::new(request, actual_voice, sink).with_stop_check(&interrupted);
    let outcome = (|| {
        loop {
            if cancelled(request, epoch) {
                return Err(stream::failed("cancelled"));
            }
            let mut samples = [0.0; stream::WINDOW_SAMPLES];
            let (mut count, mut rate, mut channels, mut reason) = (0, 0, 0, 0);
            let status = unsafe {
                omnivox_stream_next(
                    handle,
                    samples.as_mut_ptr(),
                    &mut count,
                    &mut rate,
                    &mut channels,
                    &mut reason,
                )
            };
            if cancelled(request, epoch) {
                return Err(stream::failed("cancelled"));
            }
            match status {
                0 => (),
                1 => {
                    let window = samples
                        .get(..count as usize)
                        .ok_or_else(|| stream::failed("invalid native window"))?;
                    converter.push(window, rate, channels)?;
                }
                2 => break,
                _ => {
                    return Err(stream::failed(format!(
                        "native capture failed ({})",
                        super::completion_reason_name(reason)
                    )))
                }
            }
        }
        converter.finish()
    })();
    let retired = native.retire();
    let timings = unsafe { omnivox_stream_timings(handle) };
    tracing::info!(
        lifecycle_stage = "macos_buffer_capture", engine_id = "macos", streaming = true,
        queue_wait_us = timings.queue_wait_us, write_started_us = timings.write_started_us,
        first_buffer_us = ?SynthTimings::observed_us(timings.first_buffer_us),
        last_buffer_us = ?SynthTimings::observed_us(timings.last_buffer_us),
        completion_signal_us = ?SynthTimings::observed_us(timings.completion_signal_us),
        capture_completed_us = timings.capture_completed_us, bridge_elapsed_us = timings.bridge_elapsed_us,
        first_buffer_to_return_us = ?timings.first_buffer_to_return_us(),
        last_buffer_to_completion_us = ?timings.last_buffer_to_completion_us(),
        buffers_received = timings.buffers_received, completion_reason = timings.completion_reason_name(),
        "macOS speech buffer capture timings"
    );
    retired?;
    if cancelled(request, epoch) {
        return Err(stream::failed("cancelled during retirement"));
    }
    outcome
}
