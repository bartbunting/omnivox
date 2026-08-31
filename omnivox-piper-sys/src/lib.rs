//! Raw FFI bindings to the maintained libpiper C API.
//!
//! Piper provides high-quality neural text-to-speech via ONNX Runtime.
//! The native implementation is vendored from `OHF-Voice/piper1-gpl` and
//! remains separately licensed under GPL-3.0-or-later.
//!
//! # Safety
//!
//! All raw `piper_synthesizer` pointers must be treated as opaque and only
//! passed to the corresponding libpiper functions. Callers are responsible
//! for proper create/free lifecycle management and for copying chunk data
//! before the next native call invalidates it.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]

/// Exact `espeak-ng-data` directory installed by the native libpiper build.
pub const PIPER_ESPEAK_DATA_DIR: &str = env!("PIPER_ESPEAK_DATA_DIR");

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
