//! Build script for omnivox-piper-sys.
//!
//! Steps:
//!   1. Clone piper source (if not already present)
//!   2. Run cmake — piper's CMakeLists downloads onnxruntime + piper-phonemize
//!   3. Compile piper_bridge.cpp against piper + piper-phonemize headers
//!   4. Link the resulting static libraries
//!   5. Run bindgen to generate Rust bindings from piper_bridge.h

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(cmd: &mut Command) {
    let status = cmd.status().expect("command failed to start");
    assert!(status.success(), "command failed: {:?}", cmd);
}

/// Clone or update piper source tree.
fn ensure_piper_source(piper_dir: &Path) {
    if piper_dir.join("CMakeLists.txt").exists() {
        return;
    }
    println!("cargo:warning=Cloning piper source (first build only)...");
    run(Command::new("git").args([
        "clone",
        "--depth=1",
        "--recurse-submodules",
        "https://github.com/rhasspy/piper",
        piper_dir.to_str().unwrap(),
    ]));
}

/// Run cmake to configure and build piper (downloads onnxruntime + piper-phonemize).
fn cmake_build(piper_dir: &Path, build_dir: &Path) -> PathBuf {
    std::fs::create_dir_all(build_dir).unwrap();

    // Configure
    let mut cfg = Command::new("cmake");
    cfg.arg(piper_dir.join("src/cpp"))
        .arg(format!("-B{}", build_dir.display()))
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg("-DBUILD_SHARED_LIBS=OFF")  // static piper + piper-phonemize
        .arg("-DPIPER_PHONEMIZE_DIR=")   // let cmake fetch it
        .current_dir(build_dir);

    // On macOS silence the pcaudio warning
    if cfg!(target_os = "macos") {
        cfg.arg("-DUSE_LIBPCAUDIO=OFF");
    }

    run(&mut cfg);

    // Build
    let mut build = Command::new("cmake");
    build.args(["--build", build_dir.to_str().unwrap(),
                "--config", "Release",
                "--parallel"]);
    run(&mut build);

    build_dir.to_path_buf()
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let piper_src  = manifest_dir.join("piper");
    let build_dir  = out_dir.join("piper-build");

    // --- Source ------------------------------------------------------------
    ensure_piper_source(&piper_src);

    // --- CMake build -------------------------------------------------------
    cmake_build(&piper_src, &build_dir);

    // After cmake builds, onnxruntime pre-built lives under:
    //   build_dir/piper_phonemize-prefix/  (or similar FetchContent location)
    // and piper-phonemize static lib is somewhere under build_dir.
    // We discover the lib search paths by globbing.

    // Search paths for static libs
    for entry in walkdir_libs(&build_dir) {
        println!("cargo:rustc-link-search={}", entry.display());
    }

    // Link order matters: piper depends on piper_phonemize, which depends on
    // espeak-ng and onnxruntime.
    println!("cargo:rustc-link-lib=static=piper");
    println!("cargo:rustc-link-lib=static=piper_phonemize");
    println!("cargo:rustc-link-lib=static=espeak-ng");
    // onnxruntime from piper-phonemize's fetch — dynamic
    println!("cargo:rustc-link-lib=dylib=onnxruntime");

    // Platform C++ stdlib
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    } else if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=stdc++");
    }

    // --- Compile C++ bridge ------------------------------------------------
    // piper-phonemize installs headers into its own prefix dir.
    let phonemize_include = find_dir(&build_dir, "piper-phonemize");
    let ort_include       = find_dir(&build_dir, "onnxruntime");

    let mut bridge = cc::Build::new();
    bridge
        .cpp(true)
        .std("c++17")
        .file(manifest_dir.join("piper_bridge.cpp"))
        .include(manifest_dir.join("piper/src/cpp"));
    if let Some(ref p) = phonemize_include {
        bridge.include(p);
    }
    if let Some(ref p) = ort_include {
        bridge.include(p);
    }
    bridge.compile("piper_bridge");

    // Emit the espeak-ng data path bundled with piper-phonemize so that
    // the Rust TTS engine can find it at runtime without user configuration.
    if let Some(data_path) = find_piper_espeak_data(&build_dir) {
        println!(
            "cargo:rustc-env=PIPER_ESPEAK_DATA_DIR={}",
            data_path.display()
        );
    } else {
        println!("cargo:rustc-env=PIPER_ESPEAK_DATA_DIR=");
    }

    // --- Bindgen -----------------------------------------------------------
    let bindings = bindgen::Builder::default()
        .header(manifest_dir.join("wrapper.h").to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_function("piper_.*")
        .allowlist_type("PiperState")
        .generate()
        .expect("bindgen failed");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("failed to write bindings");

    println!("cargo:rerun-if-changed=piper_bridge.h");
    println!("cargo:rerun-if-changed=piper_bridge.cpp");
    println!("cargo:rerun-if-changed=wrapper.h");
}

/// Walk build_dir and collect all unique directories containing *.a files.
fn walkdir_libs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    collect_lib_dirs(root, &mut dirs);
    dirs.sort();
    dirs.dedup();
    dirs
}

fn collect_lib_dirs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lib_dirs(&path, out);
        } else if let Some(ext) = path.extension() {
            if ext == "a" || ext == "lib" {
                if let Some(parent) = path.parent() {
                    out.push(parent.to_path_buf());
                }
            }
        }
    }
}

/// Find the first directory under `root` whose name contains `needle`.
fn find_dir(root: &Path, needle: &str) -> Option<PathBuf> {
    find_dir_inner(root, needle)
}

fn find_dir_inner(dir: &Path, needle: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return None };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name()
                   .and_then(|n| n.to_str())
                   .map(|n| n.contains(needle))
                   .unwrap_or(false)
            {
                return Some(path);
            }
            if let Some(found) = find_dir_inner(&path, needle) {
                return Some(found);
            }
        }
    }
    None
}

/// Locate the espeak-ng-data directory installed by piper-phonemize.
///
/// piper-phonemize downloads and builds espeak-ng, placing its data under
/// its install prefix. We walk the build tree looking for the `phontab` file
/// that marks a valid espeak-ng-data directory.
fn find_piper_espeak_data(build_dir: &Path) -> Option<PathBuf> {
    // phontab lives directly inside espeak-ng-data/
    let phontab = find_file(build_dir, "phontab")?;
    // Return the parent of espeak-ng-data (the dir containing that dir)
    phontab.parent()?.parent().map(PathBuf::from)
}

/// Find the first file with the given name anywhere under `root`.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return None };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}
