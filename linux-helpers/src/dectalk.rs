// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// Linux DECtalk ABI and PCM capture. See the Windows adapter for the original
// voice/rate mappings; Unix LONG is pointer-sized while DWORD remains 32-bit.

use super::markers::{Mark, Plan, MAX_MARKERS};
use super::native::*;
use libloading::Library;
use omnivox_tts::contracts::{
    AnchorSupport, EngineDescriptor, MarkerCapabilities, NormalizedAcss, VoiceGender,
};
use omnivox_tts::rate_calibration::interpolate;
use omnivox_tts::{SynthesisMarker, SynthesisMarkerKind, SynthesisRequest};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_long, c_void};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const BUFFER_SAMPLES: usize = 512;
const RECORDS: usize = 128;
const VOICES: &[(&str, &str, Option<VoiceGender>)] = &[
    ("paul", "Perfect Paul", Some(VoiceGender::Male)),
    ("betty", "Beautiful Betty", Some(VoiceGender::Female)),
    ("harry", "Huge Harry", Some(VoiceGender::Male)),
    ("frank", "Frail Frank", Some(VoiceGender::Male)),
    ("kit", "Kit the Kid", None),
    ("rita", "Rough Rita", Some(VoiceGender::Female)),
    ("ursula", "Uppity Ursula", Some(VoiceGender::Female)),
    ("dennis", "Doctor Dennis", Some(VoiceGender::Male)),
    ("wendy", "Whispering Wendy", Some(VoiceGender::Female)),
];
const CODES: &[&str] = &["np", "nb", "nh", "nf", "nk", "nr", "nu", "nd", "nw"];
const PITCH: &[f32] = &[122.0, 208.0, 89.0, 155.0, 306.0, 106.0, 240.0, 110.0, 200.0];

type Handle = *mut c_void;
type Callback = unsafe extern "C" fn(c_long, c_long, u32, u32);
type HandleFn = unsafe extern "C" fn(Handle) -> u32;
type Reset = unsafe extern "C" fn(Handle, u8) -> u32;
type AddBuffer = unsafe extern "C" fn(Handle, *mut Buffer) -> u32;
type ValueFn = unsafe extern "C" fn(Handle, u32) -> u32;

#[derive(Clone, Copy)]
struct Api {
    startup: unsafe extern "C" fn(*mut Handle, u32, u32, Callback, c_long, *const c_char) -> u32,
    shutdown: HandleFn,
    sync: HandleFn,
    close: HandleFn,
    reset: Reset,
    speak: unsafe extern "C" fn(Handle, *const c_char, u32) -> u32,
    open: ValueFn,
    rate: ValueFn,
    add: AddBuffer,
    version: unsafe extern "C" fn(*mut *const c_char) -> u32,
}
impl Api {
    unsafe fn load(lib: &Library) -> Result<Self, String> {
        Ok(Self {
            startup: symbol(lib, b"TextToSpeechStartupExFonix\0")?,
            shutdown: symbol(lib, b"TextToSpeechShutdown\0")?,
            sync: symbol(lib, b"TextToSpeechSync\0")?,
            close: symbol(lib, b"TextToSpeechCloseInMemory\0")?,
            reset: symbol(lib, b"TextToSpeechReset\0")?,
            speak: symbol(lib, b"TextToSpeechSpeak\0")?,
            open: symbol(lib, b"TextToSpeechOpenInMemory\0")?,
            rate: symbol(lib, b"TextToSpeechSetRate\0")?,
            add: symbol(lib, b"TextToSpeechAddBuffer\0")?,
            version: symbol(lib, b"TextToSpeechVersion\0")?,
        })
    }
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Phoneme {
    phoneme: u32,
    sample: u32,
    duration: u32,
    reserved: u32,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Index {
    value: u32,
    sample: u32,
    reserved: u32,
}
#[repr(C)]
struct Buffer {
    data: *mut i16,
    phonemes: *mut Phoneme,
    indexes: *mut Index,
    max_bytes: u32,
    max_phonemes: u32,
    max_indexes: u32,
    bytes: u32,
    phoneme_count: u32,
    index_count: u32,
    reserved: u32,
}
struct Slot {
    buffer: Box<Buffer>,
    _data: Box<[i16; BUFFER_SAMPLES]>,
    _phonemes: Box<[Phoneme; RECORDS]>,
    _indexes: Box<[Index; RECORDS]>,
}
impl Slot {
    fn new() -> Self {
        let mut data = Box::new([0; BUFFER_SAMPLES]);
        let mut phonemes = Box::new([Phoneme::default(); RECORDS]);
        let mut indexes = Box::new([Index::default(); RECORDS]);
        let buffer = Box::new(Buffer {
            data: data.as_mut_ptr(),
            phonemes: phonemes.as_mut_ptr(),
            indexes: indexes.as_mut_ptr(),
            max_bytes: (BUFFER_SAMPLES * 2) as u32,
            max_phonemes: RECORDS as u32,
            max_indexes: RECORDS as u32,
            bytes: 0,
            phoneme_count: 0,
            index_count: 0,
            reserved: 0,
        });
        Self {
            buffer,
            _data: data,
            _phonemes: phonemes,
            _indexes: indexes,
        }
    }
}

struct State {
    capture: Mutex<Option<DectalkCapture>>,
    failure: Mutex<Option<String>>,
    recycling: AtomicBool,
    handle: AtomicUsize,
    add: AddBuffer,
    buffers: Vec<usize>,
}
struct DectalkCapture {
    output: Capture,
    plan: Plan,
    pending_audio: Option<Vec<i16>>,
    pending_marks: Vec<Mark>,
    phonemes_left: usize,
}
impl DectalkCapture {
    fn flush_marks(&mut self, through: u64) -> Result<(), String> {
        self.pending_marks.sort_by_key(Mark::frame);
        let count = self
            .pending_marks
            .partition_point(|mark| mark.frame() <= through);
        self.output
            .markers(self.pending_marks.drain(..count).collect())
    }
    fn audio(&mut self, samples: &[i16]) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        // Native index records can arrive in the callback after their PCM.
        // Keep one 512-sample block, as on Windows, and publish its marks first.
        if let Some(ready) = self.pending_audio.replace(samples.to_vec()) {
            self.flush_marks((self.output.frames + ready.len()) as u64)?;
            self.output.audio(&ready)?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        self.plan.finish()?;
        let frames = self.output.frames + self.pending_audio.as_ref().map_or(0, Vec::len);
        self.pending_marks.extend(
            self.plan
                .trailing
                .drain(..)
                .map(|mark| mark.at(frames as u64)),
        );
        self.flush_marks(frames as u64)?;
        if !self.pending_marks.is_empty() {
            return Err("DECtalk marker exceeds completed audio".to_owned());
        }
        if let Some(audio) = self.pending_audio.take() {
            self.output.audio(&audio)?;
        }
        Ok(())
    }
}
// DECtalk truncates its opaque instance parameter to DWORD even on LP64.
// A single runtime owns this helper process, so never put a pointer there.
static CALLBACK_STATE: AtomicPtr<State> = AtomicPtr::new(std::ptr::null_mut());

struct Dectalk {
    api: Api,
    handle: Handle,
    descriptor: EngineDescriptor,
    state: Box<State>,
    _slots: Vec<Slot>,
    _library: Library,
}

pub(super) fn load() -> Result<Box<dyn Runtime>, String> {
    // Load the language library directly: libtts.so is a dispatcher that can
    // search a configuration/current directory for another library.
    let path = absolute_file(
        "OMNIVOX_DECTALK_LIBRARY",
        &[
            "/usr/local/lib/libtts_us.so",
            "/opt/dectalk/lib/libtts_us.so",
            "/usr/lib/x86_64-linux-gnu/libtts_us.so",
            "/usr/lib/libtts_us.so",
        ],
    )?;
    let dictionary = absolute_file(
        "OMNIVOX_DECTALK_DICTIONARY",
        &[
            "/opt/dectalk/dic/dtalk_us.dic",
            "/usr/local/share/dectalk/dtalk_us.dic",
            "/usr/share/dectalk/dtalk_us.dic",
        ],
    )?;
    let dictionary =
        CString::new(dictionary.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    if dictionary.as_bytes().len() >= 256 {
        return Err("DECtalk dictionary path exceeds its native 255-byte limit".to_owned());
    }
    let library = open_library(&path)?;
    let api = unsafe { Api::load(&library)? };
    let mut version_text = std::ptr::null();
    let version = unsafe { (api.version)(&mut version_text) };
    if version == 0 || version_text.is_null() {
        return Err("DECtalk did not report its version".to_owned());
    }
    let version = unsafe { CStr::from_ptr(version_text) }
        .to_string_lossy()
        .into_owned();
    let mut slots: Vec<_> = (0..4).map(|_| Slot::new()).collect();
    let mut state = Box::new(State {
        capture: Mutex::new(None),
        failure: Mutex::new(None),
        recycling: AtomicBool::new(true),
        handle: AtomicUsize::new(0),
        add: api.add,
        buffers: slots
            .iter_mut()
            .map(|s| std::ptr::from_mut(s.buffer.as_mut()) as usize)
            .collect(),
    });
    CALLBACK_STATE
        .compare_exchange(
            std::ptr::null_mut(),
            state.as_mut(),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| "only one DECtalk runtime may own a helper process".to_owned())?;
    let mut runtime = Dectalk {
        api,
        handle: std::ptr::null_mut(),
        descriptor: descriptor("dectalk", version, VOICES),
        state,
        _slots: slots,
        _library: library,
    };
    unsafe {
        check(
            (api.startup)(
                &mut runtime.handle,
                u32::MAX,
                0x8000_0000,
                callback,
                0,
                dictionary.as_ptr(),
            ),
            "startup",
        )?;
        runtime
            .state
            .handle
            .store(runtime.handle as usize, Ordering::Release);
        check((api.open)(runtime.handle, 4), "open PCM capture")?;
        for slot in &mut runtime._slots {
            check(
                (api.add)(runtime.handle, slot.buffer.as_mut()),
                "add PCM buffer",
            )?;
        }
    }
    runtime.descriptor.capabilities.markers = MarkerCapabilities {
        word: true,
        sentence: true,
        phoneme: true,
        requested_anchors: AnchorSupport::WordBoundary,
        ..MarkerCapabilities::default()
    };
    Ok(Box::new(runtime))
}

impl Runtime for Dectalk {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }
    fn synthesize(&mut self, request: &SynthesisRequest, capture: Capture) -> Result<(), String> {
        let voice = request
            .voice_id_for_engine("dectalk")
            .map_err(|e| e.to_string())?;
        let index = VOICES
            .iter()
            .position(|v| v.0 == voice)
            .ok_or("unknown DECtalk voice")?;
        let pitch = (PITCH[index] * request.settings.pitch)
            .round()
            .clamp(50.0, 500.0);
        // Literal speech cannot inject DECtalk inline commands.
        let mut plan = Plan::new(request, false)?;
        let literal = request.text.replace(['[', ']'], " ");
        let mut text = format!(
            "[:{} :dv ap {pitch}{}] ",
            CODES[index],
            voice_parameters(&request.normalized_acss)
        );
        let mut cursor = 0;
        for &(position, value) in &plan.insertions {
            text.push_str(&literal[cursor..position]);
            text.push_str(&format!("[:index mark {value}]"));
            cursor = position;
        }
        text.push_str(&literal[cursor..]);
        let text = encode_text(&text)?;
        if capture.cancellation.cancelled() {
            return Err("DECtalk synthesis cancelled".to_owned());
        }
        unsafe {
            check((self.api.reset)(self.handle, 0), "reset")?;
            check(
                (self.api.rate)(self.handle, map_rate(request.settings.rate)),
                "set rate",
            )?;
        }
        let cancellation = capture.cancellation.clone();
        *self.state.failure.lock().unwrap() = None;
        let phonemes_left = MAX_MARKERS - plan.count;
        let pending_marks = std::mem::take(&mut plan.leading);
        *self.state.capture.lock().unwrap() = Some(DectalkCapture {
            output: capture,
            plan,
            pending_audio: None,
            pending_marks,
            phonemes_left,
        });
        let done = AtomicBool::new(false);
        let handle = self.handle as usize;
        let reset = self.api.reset;
        let result = std::thread::scope(|scope| {
            let watcher = scope.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    if cancellation.cancelled() {
                        // PCM callbacks first discard/unblock on cancellation;
                        // reset runs outside their callback mutex. The existing
                        // helper watchdog contains a native reset that hangs.
                        unsafe {
                            reset(handle as Handle, 0);
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            });
            let result = unsafe {
                check((self.api.speak)(self.handle, text.as_ptr(), 1), "speak")
                    .and_then(|_| check((self.api.sync)(self.handle), "synchronize"))
            };
            done.store(true, Ordering::Release);
            watcher
                .join()
                .map_err(|_| "DECtalk cancellation watcher panicked".to_owned())?;
            result?;
            self.state
                .capture
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .finish()
        });
        self.state.capture.lock().unwrap().take();
        if let Some(error) = self.state.failure.lock().unwrap().take() {
            return Err(error);
        }
        if cancellation.cancelled() {
            return Err("DECtalk synthesis cancelled".to_owned());
        }
        result
    }
}

unsafe extern "C" fn callback(_: c_long, buffer: c_long, _: u32, message: u32) {
    if message != 9 || buffer == 0 {
        return;
    }
    let state = CALLBACK_STATE.load(Ordering::Acquire);
    if state.is_null() {
        return;
    }
    let state = &*state;
    if !state.buffers.contains(&(buffer as usize)) {
        return;
    }
    let buffer = &mut *(buffer as *mut Buffer);
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
            if buffer.bytes as usize > BUFFER_SAMPLES * 2
                || !buffer.bytes.is_multiple_of(2)
                || buffer.phoneme_count as usize > RECORDS
                || buffer.index_count as usize > RECORDS
            {
                return Err("DECtalk returned invalid capture lengths".to_owned());
            }
            let mut slot = state.capture.lock().map_err(|e| e.to_string())?;
            if let Some(capture) = slot.as_mut() {
                if !capture.output.cancellation.cancelled() {
                    for mark in
                        std::slice::from_raw_parts(buffer.indexes, buffer.index_count as usize)
                    {
                        capture
                            .pending_marks
                            .extend(capture.plan.reached(mark.value, u64::from(mark.sample))?);
                    }
                    for phoneme in
                        std::slice::from_raw_parts(buffer.phonemes, buffer.phoneme_count as usize)
                    {
                        if capture.phonemes_left == 0 {
                            break;
                        }
                        capture.phonemes_left -= 1;
                        capture.pending_marks.push(Mark::Text(SynthesisMarker {
                            kind: SynthesisMarkerKind::Phoneme,
                            frame_offset: u64::from(phoneme.sample),
                            text_start: None,
                            text_length: None,
                            value: Some(phoneme.phoneme.to_string()),
                        }));
                    }
                    capture.audio(std::slice::from_raw_parts(
                        buffer.data,
                        buffer.bytes as usize / 2,
                    ))?;
                }
            }
            Ok(())
        }))
        .unwrap_or_else(|_| Err("DECtalk capture callback panicked".to_owned()));
    if let Err(error) = result {
        if let Ok(mut failure) = state.failure.lock() {
            *failure = Some(error);
        }
        if let Ok(mut slot) = state.capture.lock() {
            if let Some(capture) = slot.as_mut() {
                capture
                    .output
                    .cancellation
                    .aborted
                    .store(true, Ordering::Release);
            }
        }
    }
    buffer.bytes = 0;
    buffer.phoneme_count = 0;
    buffer.index_count = 0;
    if state.recycling.load(Ordering::Acquire) {
        let status = (state.add)(state.handle.load(Ordering::Acquire) as Handle, buffer);
        if status != 0 {
            if let Ok(mut failure) = state.failure.lock() {
                *failure = Some(format!("DECtalk buffer recycling failed: {status}"));
            }
        }
    }
}

impl Drop for Dectalk {
    fn drop(&mut self) {
        self.state.recycling.store(false, Ordering::Release);
        if !self.handle.is_null() {
            unsafe {
                (self.api.close)(self.handle);
                (self.api.shutdown)(self.handle);
            }
        }
        CALLBACK_STATE.store(std::ptr::null_mut(), Ordering::Release);
    }
}
fn check(code: u32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("DECtalk {operation} failed: {code}"))
    }
}
fn map_rate(rate: f32) -> u32 {
    // Existing Windows DECtalk curve, provisional until a Linux rate audit.
    interpolate(
        rate,
        &[
            (0.0, 75.0),
            (0.1, 75.0),
            (0.2, 114.0177),
            (0.3, 161.4411),
            (0.4, 222.5271),
            (0.5, 288.5925),
            (0.6, 368.4105),
            (0.7, 426.14775),
            (0.8, 477.4305),
            (0.9, 509.9715),
            (1.0, 544.38),
            (1.2, 600.0),
        ],
    )
    .round() as u32
}

fn voice_parameters(style: &NormalizedAcss) -> String {
    let mut parameters = String::new();
    for (value, command, levels) in [
        (
            style.pitch_range,
            "pr",
            [0, 20, 40, 60, 80, 100, 137, 174, 211, 250],
        ),
        (
            style.pitch_range,
            "as",
            [0, 10, 20, 30, 40, 50, 60, 70, 80, 100],
        ),
        (style.stress, "hr", [0, 3, 6, 9, 12, 18, 34, 48, 63, 80]),
        (style.stress, "sr", [0, 6, 12, 18, 24, 32, 50, 65, 82, 90]),
        (
            style.stress,
            "qu",
            [0, 20, 40, 60, 80, 100, 100, 100, 100, 100],
        ),
        (style.stress, "bf", [0, 3, 6, 9, 14, 18, 20, 35, 60, 40]),
        (
            style.richness,
            "ri",
            [0, 14, 28, 42, 56, 70, 60, 70, 80, 100],
        ),
        (style.richness, "sm", [100, 80, 60, 40, 20, 3, 24, 16, 8, 0]),
    ] {
        if let Some(value) = value {
            parameters.push_str(&format!(" {command} {}", map_level(value, &levels)));
        }
    }
    parameters
}
