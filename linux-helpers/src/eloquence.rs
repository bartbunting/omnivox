// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// Linux ECI capture adapter. Native calls remain on the helper's owner thread.

use super::markers::Plan;
use super::native::*;
use libloading::Library;
use omnivox_tts::contracts::{
    AnchorSupport, EngineDescriptor, MarkerCapabilities, NormalizedAcss, VoiceGender,
};
use omnivox_tts::SynthesisRequest;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_long, c_void};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

type Handle = *mut c_void;
type Callback = unsafe extern "C" fn(Handle, c_int, c_long, *mut c_void) -> c_int;
type HandleFn = unsafe extern "C" fn(Handle) -> c_int;
#[derive(Clone, Copy)]
struct Api {
    version: unsafe extern "C" fn(*mut c_char),
    new: unsafe extern "C" fn(c_int) -> Handle,
    delete: unsafe extern "C" fn(Handle),
    stop: HandleFn,
    clear: HandleFn,
    synthesize: HandleFn,
    sync: HandleFn,
    add: unsafe extern "C" fn(Handle, *const c_char) -> c_int,
    index: unsafe extern "C" fn(Handle, c_int) -> c_int,
    param: unsafe extern "C" fn(Handle, c_int, c_int) -> c_int,
    callback: unsafe extern "C" fn(Handle, Callback, *mut c_void),
    buffer: unsafe extern "C" fn(Handle, c_int, *mut i16) -> c_int,
}
impl Api {
    unsafe fn load(lib: &Library) -> Result<Self, String> {
        Ok(Self {
            version: symbol(lib, b"eciVersion\0")?,
            new: symbol(lib, b"eciNewEx\0")?,
            delete: symbol(lib, b"eciDelete\0")?,
            stop: symbol(lib, b"eciStop\0")?,
            clear: symbol(lib, b"eciClearInput\0")?,
            synthesize: symbol(lib, b"eciSynthesize\0")?,
            sync: symbol(lib, b"eciSynchronize\0")?,
            add: symbol(lib, b"eciAddText\0")?,
            index: symbol(lib, b"eciInsertIndex\0")?,
            param: symbol(lib, b"eciSetParam\0")?,
            callback: symbol(lib, b"eciRegisterCallback\0")?,
            buffer: symbol(lib, b"eciSetOutputBuffer\0")?,
        })
    }
}
const SAMPLES: usize = 512;
const PITCH: &[f32] = &[65.0, 81.0, 93.0, 56.0, 69.0, 89.0, 68.0, 61.0];
const VOICES: &[(&str, &str, Option<VoiceGender>)] = &[
    ("v1", "Adult male 1", Some(VoiceGender::Male)),
    ("v2", "Adult female 1", Some(VoiceGender::Female)),
    ("v3", "Child 1", None),
    ("v4", "Adult male 2", Some(VoiceGender::Male)),
    ("v5", "Adult male 3", Some(VoiceGender::Male)),
    ("v6", "Elderly female 2", Some(VoiceGender::Female)),
    ("v7", "Elderly female 1", Some(VoiceGender::Female)),
    ("v8", "Adult male 1 variant", Some(VoiceGender::Male)),
];
struct State {
    capture: Mutex<Option<EciCapture>>,
    failure: Mutex<Option<String>>,
    buffer: *const i16,
}
struct EciCapture {
    output: Capture,
    plan: Plan,
}
struct Eloquence {
    api: Api,
    handle: Handle,
    state: Box<State>,
    buffer: Box<[i16; SAMPLES]>,
    descriptor: EngineDescriptor,
    clear_input_status_unreliable: bool,
    _library: Library,
}

pub(super) fn load() -> Result<Box<dyn Runtime>, String> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if home.is_absolute() {
            candidates.push(home.join(".local/share/voxin/rfs/opt/oralux/voxin/lib/libvoxin.so"));
        }
    }
    candidates.extend(
        [
            "/usr/lib/x86_64-linux-gnu/libibmeci.so",
            "/usr/local/lib/libibmeci.so",
            "/usr/lib64/libibmeci.so",
            "/usr/lib/libibmeci.so",
            "/opt/oralux/voxin/lib/libvoxin.so",
            "/opt/IBM/ibmtts/lib/libibmeci.so",
            "/usr/share/ibmtts/lib/libibmeci.so",
        ]
        .map(PathBuf::from),
    );
    let path = absolute_file("OMNIVOX_ECI_LIBRARY", &candidates)?;
    let library = open_library(&path)?;
    let api = unsafe { Api::load(&library)? };
    // Voxin 1.6.3's voxind executes MSG_CLEAR_INPUT but omits the return value,
    // so its ECI wrapper always returns false. Keep strict status checks for
    // native ECI and any other unverified Voxin version.
    let clear_input_status_unreliable = unsafe {
        type VoxVersion = unsafe extern "C" fn(*mut c_int, *mut c_int, *mut c_int) -> c_int;
        match library.get::<VoxVersion>(b"voxGetVersion\0") {
            Ok(get_version) => {
                let (mut major, mut minor, mut patch) = (0, 0, 0);
                get_version(&mut major, &mut minor, &mut patch) == 0
                    && (major, minor, patch) == (1, 6, 3)
            }
            Err(_) => false,
        }
    };
    let mut version = [0 as c_char; 128];
    unsafe {
        (api.version)(version.as_mut_ptr());
    }
    version[127] = 0;
    let version = unsafe { CStr::from_ptr(version.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    if version.is_empty() {
        return Err("ECI did not report a runtime version".to_owned());
    }
    let handle = unsafe { (api.new)(0x00010000) };
    if handle.is_null() {
        return Err("ECI could not initialize its American English voice data".to_owned());
    }
    let buffer = Box::new([0; SAMPLES]);
    let state = Box::new(State {
        capture: Mutex::new(None),
        failure: Mutex::new(None),
        buffer: buffer.as_ptr(),
    });
    let mut runtime = Eloquence {
        api,
        handle,
        state,
        buffer,
        descriptor: descriptor("eloquence", version, VOICES),
        clear_input_status_unreliable,
        _library: library,
    };
    unsafe {
        (api.callback)(
            handle,
            callback,
            std::ptr::from_mut(runtime.state.as_mut()).cast(),
        );
    }
    runtime.configure()?;
    runtime.descriptor.capabilities.markers = MarkerCapabilities {
        word: true,
        sentence: true,
        requested_anchors: AnchorSupport::Exact,
        ..MarkerCapabilities::default()
    };
    Ok(Box::new(runtime))
}
impl Eloquence {
    fn configure(&mut self) -> Result<(), String> {
        unsafe {
            for (param, value) in [(1, 1), (0, 1), (5, 1)] {
                if (self.api.param)(self.handle, param, value) == -1 {
                    return Err(format!("ECI parameter {param} failed"));
                }
            }
            check(
                (self.api.buffer)(self.handle, SAMPLES as c_int, self.buffer.as_mut_ptr()),
                "set PCM buffer",
            )
        }
    }
}
impl Runtime for Eloquence {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }
    fn synthesize(&mut self, request: &SynthesisRequest, capture: Capture) -> Result<(), String> {
        let voice = request
            .voice_id_for_engine("eloquence")
            .map_err(|e| e.to_string())?;
        let index = VOICES
            .iter()
            .position(|v| v.0 == voice)
            .ok_or("unknown ECI voice")?;
        if capture.cancellation.cancelled() {
            return Err("ECI synthesis cancelled".to_owned());
        }
        // Preserve the established Eloquence host mapping; Linux calibration
        // remains provisional until a real installed runtime is audited.
        let rate = (20.0 + request.settings.rate * 110.0)
            .round()
            .clamp(0.0, 250.0);
        let pitch = (PITCH[index] * request.settings.pitch)
            .round()
            .clamp(0.0, 100.0);
        let plan = Plan::new(request, true)?;
        let insertions = plan.insertions.clone();
        let (parameters, volume) = voice_parameters(&request.normalized_acss);
        let prefix = encode_text(&format!(
            " `{voice} `vs{rate} `vb{pitch}{parameters} `vv{volume} "
        ))?;
        let text = request.text.replace('`', " ");
        unsafe {
            check((self.api.stop)(self.handle), "stop previous synthesis")?;
            let cleared = (self.api.clear)(self.handle);
            if !self.clear_input_status_unreliable {
                check(cleared, "clear input")?;
            }
        }
        self.configure()?;
        *self.state.failure.lock().unwrap() = None;
        let cancellation = capture.cancellation.clone();
        *self.state.capture.lock().unwrap() = Some(EciCapture {
            output: capture,
            plan,
        });
        let result = (|| unsafe {
            check(
                (self.api.add)(self.handle, prefix.as_ptr()),
                "add voice parameters",
            )?;
            let mut cursor = 0;
            for (position, index) in insertions {
                if position > cursor {
                    let segment = encode_text(&text[cursor..position])?;
                    check((self.api.add)(self.handle, segment.as_ptr()), "add text")?;
                }
                check(
                    (self.api.index)(self.handle, index as c_int),
                    "insert index",
                )?;
                cursor = position;
            }
            if cursor < text.len() {
                let segment = encode_text(&text[cursor..])?;
                check((self.api.add)(self.handle, segment.as_ptr()), "add text")?;
            }
            check((self.api.synthesize)(self.handle), "synthesize")?;
            check((self.api.sync)(self.handle), "synchronize")?;
            self.state
                .capture
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .plan
                .finish()
        })();
        // Stop from the owner after an abort; never call ECI cross-thread.
        if cancellation.cancelled() {
            unsafe {
                (self.api.stop)(self.handle);
            }
        }
        self.state.capture.lock().unwrap().take();
        if let Some(error) = self.state.failure.lock().unwrap().take() {
            return Err(error);
        }
        if cancellation.cancelled() {
            return Err("ECI synthesis cancelled".to_owned());
        }
        result
    }
}
unsafe extern "C" fn callback(
    _: Handle,
    message: c_int,
    samples: c_long,
    data: *mut c_void,
) -> c_int {
    if data.is_null() {
        return 2;
    }
    let state = &*(data as *const State);
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
            let mut slot = state.capture.lock().map_err(|e| e.to_string())?;
            let Some(capture) = slot.as_mut() else {
                return Ok(());
            };
            if capture.output.cancellation.cancelled() {
                return Err("ECI synthesis cancelled".to_owned());
            }
            if message == 0 {
                if samples < 0 || samples as usize > SAMPLES {
                    return Err("ECI returned an invalid PCM length".to_owned());
                }
                capture
                    .output
                    .audio(std::slice::from_raw_parts(state.buffer, samples as usize))?;
            } else if message == 2 {
                let index = u32::try_from(samples).map_err(|_| "ECI returned an invalid index")?;
                let marks = capture.plan.reached(index, capture.output.frames as u64)?;
                capture.output.markers(marks)?;
            }
            Ok(())
        }))
        .unwrap_or_else(|_| Err("ECI callback panicked".to_owned()));
    match result {
        Ok(()) => 1,
        Err(error) => {
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
            2
        }
    }
}

fn voice_parameters(style: &NormalizedAcss) -> (String, i32) {
    let mut parameters = String::new();
    for (value, command, levels) in [
        (
            style.pitch_range,
            "vf",
            [0, 5, 15, 20, 25, 30, 47, 64, 81, 100],
        ),
        (style.stress, "vr", [0, 10, 20, 30, 40, 50, 60, 70, 80, 90]),
        (style.richness, "vy", [0, 4, 8, 12, 16, 20, 24, 28, 32, 36]),
    ] {
        if let Some(value) = value {
            parameters.push_str(&format!(" `{command}{}", map_level(value, &levels)));
        }
    }
    let volume = style.richness.map_or(100, |value| {
        map_level(value, &[60, 78, 80, 84, 88, 92, 93, 95, 97, 100])
    });
    (parameters, volume)
}
impl Drop for Eloquence {
    fn drop(&mut self) {
        unsafe {
            (self.api.stop)(self.handle);
            (self.api.delete)(self.handle);
        }
    }
}
fn check(status: c_int, operation: &str) -> Result<(), String> {
    if status != 0 {
        Ok(())
    } else {
        Err(format!("ECI {operation} failed"))
    }
}
