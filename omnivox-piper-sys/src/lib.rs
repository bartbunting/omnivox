//! Raw FFI bindings to the piper TTS C++ library via a C bridge.
//!
//! Piper provides high-quality neural text-to-speech via ONNX Runtime.
//! This crate wraps piper's C++ API in a C bridge (`piper_bridge.h/cpp`)
//! that bindgen can handle.
//!
//! # Safety
//!
//! All raw pointers (`PiperState *`) must be treated as opaque and only
//! passed to the corresponding bridge functions. Callers are responsible
//! for proper init/destroy lifecycle management.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
