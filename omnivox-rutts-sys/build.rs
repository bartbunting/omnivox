use std::env;
use std::path::{Path, PathBuf};

const RUTTS_SOURCES: &[&str] = &[
    "src/synth.c",
    "src/sink.c",
    "src/transcription.c",
    "src/text2speech.c",
    "src/utterance.c",
    "src/time_planner.c",
    "src/speechrate_control.c",
    "src/intonator.c",
    "src/soundproducer.c",
    "src/numerics.c",
    "src/male.c",
    "src/female.c",
];

fn require_file(path: &Path) {
    if !path.is_file() {
        panic!(
            "missing locked RuTTS source {}; run `make prepare-rutts` first",
            path.display()
        );
    }
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository = manifest.parent().unwrap();
    let source = env::var_os("OMNIVOX_RUTTS_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join("target/rutts-inputs/6.3.3/source"));
    let wrapper = manifest.join("native/omnivox_rutts.c");

    require_file(&source.join("LICENSE"));
    require_file(&source.join("src/ru_tts.h"));
    require_file(&wrapper);

    let mut build = cc::Build::new();
    build
        .warnings(false)
        .include(source.join("src"))
        .flag_if_supported("-std=gnu99")
        .file(&wrapper);
    for relative in RUTTS_SOURCES {
        let path = source.join(relative);
        require_file(&path);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }

    println!("cargo:rerun-if-env-changed=OMNIVOX_RUTTS_SOURCE_DIR");
    println!("cargo:rerun-if-changed={}", wrapper.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("source-inputs.json").display()
    );
    build.compile("omnivox_rutts");

    if env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
        println!("cargo:rustc-link-lib=m");
    }
}
