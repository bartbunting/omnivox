use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn static_cpp_stdlib_directory(build: &cc::Build) -> PathBuf {
    let compiler = build.get_compiler();
    let output = compiler
        .to_command()
        .arg("-print-file-name=libstdc++.a")
        .output()
        .unwrap_or_else(|error| panic!("could not locate the GNU C++ runtime: {error}"));
    if !output.status.success() {
        panic!(
            "could not locate the GNU C++ runtime with {}",
            compiler.path().display()
        );
    }

    let path = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("GNU C++ runtime path was not UTF-8")
            .trim(),
    );
    if !path.is_absolute() || !path.is_file() {
        panic!(
            "{} did not report an absolute libstdc++.a path (reported {})",
            compiler.path().display(),
            path.display()
        );
    }
    path.parent()
        .expect("GNU C++ runtime path had no parent")
        .to_path_buf()
}

fn require_file(path: &Path) {
    if !path.is_file() {
        panic!(
            "missing locked TGSpeechBox source {}; run `make prepare-tgspeechbox` first",
            path.display()
        );
    }
}

fn sorted_cpp(directory: &Path) -> Vec<PathBuf> {
    let mut sources = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()))
        .map(|entry| {
            entry
                .expect("could not read TGSpeechBox source entry")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|extension| extension == "cpp"))
        .collect::<Vec<_>>();
    sources.sort();
    sources
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository = manifest.parent().unwrap();
    let source = env::var_os("OMNIVOX_TGSPEECHBOX_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join("target/tgspeechbox-inputs/v-310b802/source"));
    let wrapper = manifest.join("native/omnivox_tgspeechbox.cpp");

    require_file(&source.join("LICENSE"));
    require_file(&source.join("src/speechPlayer.h"));
    require_file(&source.join("src/frontend/nvspFrontend.h"));
    require_file(&source.join("packs/phonemes.yaml"));
    require_file(&wrapper);

    let mut sources = vec![
        source.join("src/frame.cpp"),
        source.join("src/speechPlayer.cpp"),
        source.join("src/speechWaveGenerator.cpp"),
    ];
    sources.extend(sorted_cpp(&source.join("src/frontend")));
    sources.extend(sorted_cpp(&source.join("src/frontend/passes")));

    let mut build = cc::Build::new();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env == "gnu" {
        // A companion must not depend on a MinGW libstdc++ DLL that is absent
        // from a normal Omnivox installation.
        build.cpp_link_stdlib(None);
    }
    build
        .cpp(true)
        .warnings(false)
        .define("NVSP_FRONTEND_EXPORTS", "1")
        .include(source.join("src"))
        .include(source.join("src/frontend"))
        .flag_if_supported("-std=c++17")
        .flag_if_supported("/std:c++17")
        .file(&wrapper);
    for path in &sources {
        require_file(path);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }

    let static_cpp_stdlib = (target_env == "gnu").then(|| static_cpp_stdlib_directory(&build));

    println!("cargo:rerun-if-env-changed=OMNIVOX_TGSPEECHBOX_SOURCE_DIR");
    println!("cargo:rerun-if-changed={}", wrapper.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("source-inputs.json").display()
    );
    build.compile("omnivox_tgspeechbox");

    if let Some(directory) = static_cpp_stdlib {
        println!("cargo:rustc-link-search=native={}", directory.display());
        println!("cargo:rustc-link-lib=static=stdc++");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=winmm");
    }
}
