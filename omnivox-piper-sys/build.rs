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

fn configure_and_build(source: &Path, build: &Path, install: &Path, target_os: &str) {
    fs::create_dir_all(build)
        .unwrap_or_else(|error| panic!("could not create {}: {error}", build.display()));

    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(source)
        .arg("-B")
        .arg(build)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", install.display()))
        .arg("-DPIPER_BUILD_TESTS=OFF")
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

fn native_build_root(out_dir: &Path, target: &str) -> PathBuf {
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
    target_base
        .join("piper-native")
        .join(PIPER_VERSION)
        .join(target)
}

fn main() {
    let (target_os, target) = validate_native_target();
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

    // Upstream extracts ONNX Runtime below its source directory. Work from an
    // OUT_DIR copy so a native build never writes generated files into the
    // byte-for-byte vendored source tree. Copying over an existing tree keeps
    // upstream's downloaded runtime cache for subsequent Cargo invocations.
    // eSpeak's phoneme compiler uses fixed-size path buffers. Cargo's normal
    // package OUT_DIR is long enough to truncate asset names, so keep the
    // native scratch tree directly under the target tree.
    let build_root = native_build_root(&out_dir, &target);
    let source_copy = build_root.join("source");
    copy_tree(&vendored_root, &source_copy);
    let source = source_copy.join("libpiper");
    let build = build_root.join("build");
    let install = build_root.join("install");
    configure_and_build(&source, &build, &install, &target_os);

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
    println!(
        "cargo:rustc-env=PIPER_ESPEAK_DATA_DIR={}",
        espeak_data_dir.display()
    );

    let header = source.join("include/piper.h");
    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
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
    for variable in ["CMAKE", "CMAKE_GENERATOR", "CMAKE_TOOLCHAIN_FILE"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
}
