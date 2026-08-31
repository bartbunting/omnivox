//! Build the vendored, maintained libpiper C API for the native Cargo target.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const VENDORED_PIPER: &str = "../third-party/piper1-gpl";
const PIPER_VERSION: &str = "1.7.0";

fn run(command: &mut Command) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("could not start {command:?}: {error}"));
    assert!(status.success(), "command failed: {command:?}");
}

fn cmake_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap_or_else(|error| {
        panic!(
            "could not create Piper source copy {}: {error}",
            destination.display()
        )
    });
    for entry in fs::read_dir(source)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()))
    {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "could not read an entry under {}: {error}",
                source.display()
            )
        });
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("could not inspect {}: {error}", source_path.display()));
        if file_type.is_dir() {
            copy_tree(&source_path, &destination_path);
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).unwrap_or_else(|error| {
                panic!(
                    "could not copy {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            });
        } else {
            panic!(
                "unsupported non-file entry in vendored Piper source: {}",
                source_path.display()
            );
        }
    }
}

fn validate_native_target() -> (String, String) {
    let host = env::var("HOST").expect("Cargo did not provide HOST");
    let target = env::var("TARGET").expect("Cargo did not provide TARGET");
    if host != target {
        panic!(
            "omnivox-piper-sys currently requires a native build because libpiper's native \n\
             dependencies must be runtime-tested for the requested platform (host={host}, \n\
             target={target})"
        );
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let supported = match target_os.as_str() {
        "linux" => matches!(target_arch.as_str(), "x86_64" | "aarch64") && target_env == "gnu",
        "macos" => matches!(target_arch.as_str(), "x86_64" | "aarch64"),
        "windows" => target_arch == "x86_64" && target_env == "msvc",
        _ => false,
    };
    assert!(
        supported,
        "unsupported native Piper target {target}; supported targets are Linux GNU x64/ARM64, \n\
         macOS x64/ARM64, and Windows MSVC x64"
    );
    (target_os, target)
}

fn configure_and_build(
    source: &Path,
    build: &Path,
    install: &Path,
    inputs: &Path,
    target_os: &str,
) {
    fs::create_dir_all(build)
        .unwrap_or_else(|error| panic!("could not create {}: {error}", build.display()));

    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(source)
        .arg("-B")
        .arg(build)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", cmake_path(install)))
        .arg("-DPIPER_BUILD_TESTS=OFF")
        .arg(format!(
            "-DPIPER_ESPEAK_SOURCE_DIR={}",
            cmake_path(&inputs.join("sources/espeak-ng"))
        ))
        .arg(format!(
            "-DPIPER_SONIC_SOURCE_DIR={}",
            cmake_path(&inputs.join("sources/sonic"))
        ))
        .arg(format!(
            "-DONNXRUNTIME_DIR={}",
            cmake_path(&inputs.join("sources/onnxruntime"))
        ))
        .current_dir(build);
    if target_os == "linux" {
        configure.arg("-DCMAKE_INSTALL_RPATH=$ORIGIN");
    } else if target_os == "macos" {
        configure.arg("-DCMAKE_INSTALL_RPATH=@loader_path");
    }
    run(&mut configure);

    let mut compile = Command::new("cmake");
    compile
        .arg("--build")
        .arg(build)
        .args(["--config", "Release", "--parallel"]);
    run(&mut compile);

    let mut install_command = Command::new("cmake");
    install_command
        .arg("--install")
        .arg(build)
        .args(["--config", "Release"]);
    run(&mut install_command);
}

fn cargo_target_base(out_dir: &Path) -> PathBuf {
    let cargo_build_dir = out_dir
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "build"))
        .unwrap_or_else(|| {
            panic!(
                "could not locate Cargo target directory from {}",
                out_dir.display()
            )
        });
    let target_base = cargo_build_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| {
            panic!(
                "could not locate Cargo target directory from {}",
                out_dir.display()
            )
        });
    target_base.to_path_buf()
}

fn native_build_root(target_base: &Path, target: &str) -> PathBuf {
    target_base
        .join("piper-native")
        .join(PIPER_VERSION)
        .join(target)
}

fn verified_inputs_root(target_base: &Path, target: &str) -> PathBuf {
    env::var_os("OMNIVOX_PIPER_INPUTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            target_base
                .join("piper-inputs")
                .join(PIPER_VERSION)
                .join(target)
        })
}

fn replace_once(contents: &mut String, original: &str, replacement: &str, label: &str) {
    assert_eq!(
        contents.matches(original).count(),
        1,
        "vendored libpiper CMake no longer has the expected {label} block"
    );
    *contents = contents.replacen(original, replacement, 1);
}

fn require_verified_inputs(source: &Path, target_base: &Path, target: &str) -> PathBuf {
    let inputs = verified_inputs_root(target_base, target);
    let required = [
        "PREPARED.json",
        "sources/espeak-ng/CMakeLists.txt",
        "sources/sonic/sonic.c",
        "sources/onnxruntime/include/onnxruntime_c_api.h",
    ];
    for relative in required {
        assert!(
            inputs.join(relative).is_file(),
            "verified Piper input {relative} is missing under {}; run `python3 \
             tools/prepare_piper_inputs.py --target {target}` first",
            inputs.display()
        );
    }

    // Keep the vendored upstream source byte-for-byte intact. This explicit
    // overlay changes only the generated build copy so ExternalProject and
    // FetchContent consume the checksum-verified cache prepared by Omnivox.
    let cmake_path = source.join("CMakeLists.txt");
    let mut cmake = fs::read_to_string(&cmake_path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", cmake_path.display()));
    // Git may check text files out with CRLF on Windows. Normalize only this
    // generated build copy so the reviewed upstream-block replacements remain
    // exact and the pristine vendored source is untouched.
    cmake = cmake.replace("\r\n", "\n");
    replace_once(
        &mut cmake,
        "ExternalProject_Add(espeak_ng_external\n    GIT_REPOSITORY https://github.com/espeak-ng/espeak-ng.git\n    GIT_TAG 212928b394a96e8fd2096616bfd54e17845c48f6  # 2025-Mar-22\n    PREFIX ${ESPEAKNG_BUILD_DIR}",
        "ExternalProject_Add(espeak_ng_external\n    SOURCE_DIR ${PIPER_ESPEAK_SOURCE_DIR}\n    DOWNLOAD_COMMAND \"\"\n    UPDATE_COMMAND \"\"\n    PREFIX ${ESPEAKNG_BUILD_DIR}",
        "eSpeak ExternalProject",
    );
    replace_once(
        &mut cmake,
        "        -DUSE_SPEECHPLAYER:BOOL=OFF\n        -DEXTRA_cmn:BOOL=ON",
        "        -DUSE_SPEECHPLAYER:BOOL=OFF\n        \"-DFETCHCONTENT_SOURCE_DIR_SONIC-GIT=${PIPER_SONIC_SOURCE_DIR}\"\n        -DFETCHCONTENT_FULLY_DISCONNECTED:BOOL=ON\n        -DEXTRA_cmn:BOOL=ON",
        "Sonic FetchContent arguments",
    );
    fs::write(&cmake_path, cmake)
        .unwrap_or_else(|error| panic!("could not patch {}: {error}", cmake_path.display()));
    inputs
}

fn main() {
    let (target_os, target) = validate_native_target();
    let relocatable = env::var("OMNIVOX_PIPER_RELOCATABLE").as_deref() == Ok("1");
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo did not provide CARGO_MANIFEST_DIR"),
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo did not provide OUT_DIR"));
    let vendored_root = manifest_dir.join(VENDORED_PIPER);
    let vendored_source = vendored_root.join("libpiper");
    assert!(
        vendored_source.join("include/piper.h").is_file(),
        "vendored libpiper source is missing from {}",
        vendored_source.display()
    );

    // Work from a generated copy so dependency-source overlays and CMake
    // outputs never modify the byte-for-byte vendored libpiper source tree.
    // eSpeak's phoneme compiler uses fixed-size path buffers. Cargo's normal
    // package OUT_DIR is long enough to truncate asset names, so keep the
    // native scratch tree directly under the target tree.
    let target_base = cargo_target_base(&out_dir);
    let build_root = native_build_root(&target_base, &target);
    let source_copy = build_root.join("source");
    copy_tree(&vendored_root, &source_copy);
    let source = source_copy.join("libpiper");
    let inputs = require_verified_inputs(&source, &target_base, &target);
    // Keep the verified-input graph separate from pre-migration CMake caches,
    // whose ExternalProject source directory points at an in-tree Git clone.
    let build = build_root.join("b1");
    let install = build_root.join("i1");
    configure_and_build(&source, &build, &install, &inputs, &target_os);

    let library_dir = install.join("lib");
    let espeak_data_dir = install.join("share/espeak-ng-data");
    assert!(
        espeak_data_dir.join("phontab").is_file(),
        "libpiper did not install eSpeak data under {}",
        espeak_data_dir.display()
    );

    println!("cargo:rustc-link-search=native={}", library_dir.display());
    println!("cargo:rustc-link-lib=dylib=piper");
    if target_os == "linux" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    } else if target_os == "macos" {
        println!("cargo:rustc-link-lib=dylib=c++");
    }

    // The helper build script consumes these values through DEP_PIPER_*.
    println!("cargo:RPATH={}", library_dir.display());
    println!("cargo:RUNTIME_DIR={}", library_dir.display());
    println!("cargo:TARGET={target}");
    if relocatable {
        println!("cargo:rustc-env=PIPER_ESPEAK_DATA_DIR=");
    } else {
        println!(
            "cargo:rustc-env=PIPER_ESPEAK_DATA_DIR={}",
            espeak_data_dir.display()
        );
    }

    let header = source.join("include/piper.h");
    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_args(["-x", "c++", "-std=c++17"])
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_function("piper_.*")
        .allowlist_type("piper_.*")
        .allowlist_var("PIPER_.*")
        .generate()
        .expect("could not generate libpiper bindings");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("could not write libpiper bindings");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", vendored_source.display());
    println!(
        "cargo:rerun-if-changed={}",
        vendored_root.join("setup.py").display()
    );
    for variable in [
        "CMAKE",
        "CMAKE_GENERATOR",
        "CMAKE_TOOLCHAIN_FILE",
        "OMNIVOX_PIPER_RELOCATABLE",
        "OMNIVOX_PIPER_INPUTS_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("native-inputs.json").display()
    );
}
