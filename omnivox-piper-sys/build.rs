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

/// Clone piper source tree if not already present.
/// We only need the source files, not the piper cmake build system.
fn ensure_piper_source(piper_dir: &Path) {
    // Check for piper.hpp as the definitive marker that the source is present.
    if piper_dir.join("src").join("cpp").join("piper.hpp").exists() {
        return;
    }
    println!("cargo:warning=Cloning piper source (first build only)...");
    run(Command::new("git").args([
        "clone",
        "--depth=1",
        "https://github.com/rhasspy/piper",
        piper_dir.to_str().unwrap(),
    ]));
}

/// Run our CMakeLists.txt (in manifest_dir) which downloads deps and builds
/// libpiper_bridge.a.  The cmake source is manifest_dir itself (our custom
/// CMakeLists.txt lives there); deps and outputs go into build_dir.
fn cmake_build(manifest_dir: &Path, build_dir: &Path) {
    std::fs::create_dir_all(build_dir).unwrap();

    // Configure — source is manifest_dir (our CMakeLists.txt)
    let mut cfg = Command::new("cmake");
    cfg.arg(manifest_dir)
        .arg(format!("-B{}", build_dir.display()))
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .current_dir(build_dir);

    run(&mut cfg);

    // Build (--parallel lets cmake use all cores)
    let mut build = Command::new("cmake");
    build.args([
        "--build",
        build_dir.to_str().unwrap(),
        "--config",
        "Release",
        "--parallel",
    ]);
    run(&mut build);

    // Install libpiper_bridge.a into build_dir/install/lib
    let mut install = Command::new("cmake");
    install.args([
        "--install",
        build_dir.to_str().unwrap(),
        "--config",
        "Release",
    ]);
    run(&mut install);
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let piper_src = manifest_dir.join("piper");
    let build_dir = out_dir.join("piper-build");
    // Our CMakeLists.txt installs everything (libs + headers) here.
    let install_dir = build_dir.join("install");

    // --- Clone piper source if needed ------------------------------------
    ensure_piper_source(&piper_src);

    // --- CMake build ------------------------------------------------------
    // Builds libpiper_bridge.a and all deps (fmt, spdlog, piper-phonemize).
    cmake_build(&manifest_dir, &build_dir);

    // --- Link paths -------------------------------------------------------
    let lib_dir = install_dir.join("lib");

    // Add the install lib directory to the linker search path.
    println!("cargo:rustc-link-search={}", lib_dir.display());

    // piper_bridge.a is also left in build_dir by cmake before install.
    println!("cargo:rustc-link-search={}", build_dir.display());

    // Walk for any additional .a files (spdlog builds into a sub-dir).
    for dir in walkdir_libs(&build_dir) {
        println!("cargo:rustc-link-search={}", dir.display());
    }

    // Static libs (built from source by cmake)
    println!("cargo:rustc-link-lib=static=piper_bridge");
    println!("cargo:rustc-link-lib=static=fmt");
    println!("cargo:rustc-link-lib=static=spdlog");

    // piper-phonemize, espeak-ng, and onnxruntime are built as dynamic libs
    // on macOS (and Linux). They are loaded at runtime from install/lib/.
    println!("cargo:rustc-link-lib=dylib=piper_phonemize");
    println!("cargo:rustc-link-lib=dylib=onnxruntime");
    // espeak-ng is a transitive dep of piper_phonemize; link it explicitly
    // so the linker finds the right copy (not espeak-rs-sys's static copy).
    println!("cargo:rustc-link-lib=dylib=espeak-ng");

    // Expose the lib dir path via the DEP_ mechanism so the final binary's
    // build script (omnivox-cli/build.rs) can embed the rpath.
    // cargo:rustc-link-arg in a lib build script does NOT propagate to the
    // final binary — only cargo:rustc-link-lib and cargo:rustc-link-search do.
    println!("cargo:RPATH={}", lib_dir.display());

    // Platform C++ stdlib
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    } else if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=stdc++");
    }

    // --- Bindgen (no cc::Build needed — piper.cpp is compiled by cmake) ---
    // We still need include paths for bindgen to parse piper_bridge.h.
    let include_dir = install_dir.join("include");

    // Emit the espeak-ng data path so the Rust engine can find it at runtime.
    if let Some(data_path) = find_piper_espeak_data(&install_dir) {
        println!(
            "cargo:rustc-env=PIPER_ESPEAK_DATA_DIR={}",
            data_path.display()
        );
    } else {
        println!("cargo:rustc-env=PIPER_ESPEAK_DATA_DIR=");
    }

    // --- Bindgen -----------------------------------------------------------
    let mut builder = bindgen::Builder::default()
        .header(manifest_dir.join("wrapper.h").to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_function("piper_.*")
        .allowlist_type("PiperState");

    // Add include paths so bindgen can resolve stdint.h etc.
    if include_dir.exists() {
        builder = builder.clang_arg(format!("-I{}", include_dir.display()));
    }

    let bindings = builder.generate().expect("bindgen failed");

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
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lib_dirs(&path, out);
        } else if let Some(ext) = path.extension() {
            if matches!(ext.to_str(), Some("a" | "lib" | "dylib" | "so")) {
                if let Some(parent) = path.parent() {
                    out.push(parent.to_path_buf());
                }
            }
        }
    }
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
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
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
