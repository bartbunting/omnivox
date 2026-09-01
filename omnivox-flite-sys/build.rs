use std::env;
use std::path::{Path, PathBuf};

const FLITE_SOURCES: &[&str] = &[
    "src/utils/cst_alloc.c",
    "src/utils/cst_error.c",
    "src/utils/cst_string.c",
    "src/utils/cst_wchar.c",
    "src/utils/cst_tokenstream.c",
    "src/utils/cst_val.c",
    "src/utils/cst_features.c",
    "src/utils/cst_endian.c",
    "src/utils/cst_socket.c",
    "src/utils/cst_val_const.c",
    "src/utils/cst_val_user.c",
    "src/utils/cst_args.c",
    "src/utils/cst_url.c",
    "src/utils/cst_file_stdio.c",
    "src/regex/cst_regex.c",
    "src/regex/regexp.c",
    "src/regex/regsub.c",
    "src/hrg/cst_utterance.c",
    "src/hrg/cst_relation.c",
    "src/hrg/cst_item.c",
    "src/hrg/cst_ffeature.c",
    "src/hrg/cst_rel_io.c",
    "src/stats/cst_cart.c",
    "src/stats/cst_viterbi.c",
    "src/stats/cst_ss.c",
    "src/audio/audio.c",
    "src/audio/au_streaming.c",
    "src/audio/au_none.c",
    "src/speech/cst_wave.c",
    "src/speech/cst_wave_io.c",
    "src/speech/cst_track.c",
    "src/speech/cst_track_io.c",
    "src/speech/cst_wave_utils.c",
    "src/speech/cst_lpcres.c",
    "src/speech/rateconv.c",
    "src/speech/g721.c",
    "src/speech/g72x.c",
    "src/speech/g723_24.c",
    "src/speech/g723_40.c",
    "src/lexicon/cst_lexicon.c",
    "src/lexicon/cst_lts.c",
    "src/lexicon/cst_lts_rewrites.c",
    "src/synth/cst_synth.c",
    "src/synth/cst_utt_utils.c",
    "src/synth/cst_voice.c",
    "src/synth/cst_phoneset.c",
    "src/synth/cst_ffeatures.c",
    "src/synth/cst_ssml.c",
    "src/synth/flite.c",
    "src/wavesynth/cst_units.c",
    "src/wavesynth/cst_clunits.c",
    "src/wavesynth/cst_diphone.c",
    "src/wavesynth/cst_sigpr.c",
    "src/wavesynth/cst_sts.c",
    "src/wavesynth/cst_reflpc.c",
    "src/cg/cst_cg.c",
    "src/cg/cst_mlsa.c",
    "src/cg/cst_mlpg.c",
    "src/cg/cst_vc.c",
    "src/cg/cst_cg_load_voice.c",
    "src/cg/cst_cg_dump_voice.c",
    "src/cg/cst_cg_map.c",
    "src/cg/cst_spamf0.c",
    "lang/usenglish/us_int_accent_cart.c",
    "lang/usenglish/us_int_tone_cart.c",
    "lang/usenglish/us_f0_model.c",
    "lang/usenglish/us_dur_stats.c",
    "lang/usenglish/us_durz_cart.c",
    "lang/usenglish/us_f0lr.c",
    "lang/usenglish/us_phoneset.c",
    "lang/usenglish/us_ffeatures.c",
    "lang/usenglish/us_phrasing_cart.c",
    "lang/usenglish/us_gpos.c",
    "lang/usenglish/us_text.c",
    "lang/usenglish/us_expand.c",
    "lang/usenglish/us_nums_cart.c",
    "lang/usenglish/us_aswd.c",
    "lang/usenglish/usenglish.c",
    "lang/usenglish/us_pos_cart.c",
    "lang/cmulex/cmu_lts_rules.c",
    "lang/cmulex/cmu_lts_model.c",
    "lang/cmulex/cmu_lex.c",
    "lang/cmulex/cmu_lex_entries.c",
    "lang/cmulex/cmu_lex_data.c",
    "lang/cmulex/cmu_postlex.c",
    "lang/cmu_us_slt/cmu_us_slt.c",
    "lang/cmu_us_slt/cmu_us_slt_cg_single_mcep_trees.c",
    "lang/cmu_us_slt/cmu_us_slt_cg.c",
    "lang/cmu_us_slt/cmu_us_slt_cg_single_params.c",
    "lang/cmu_us_slt/cmu_us_slt_cg_durmodel.c",
    "lang/cmu_us_slt/cmu_us_slt_cg_phonestate.c",
    "lang/cmu_us_slt/cmu_us_slt_cg_f0_trees.c",
    "lang/cmu_us_slt/cmu_us_slt_spamf0_accent.c",
    "lang/cmu_us_slt/cmu_us_slt_spamf0_phrase.c",
    "lang/cmu_us_slt/cmu_us_slt_spamf0_accent_params.c",
];

const INCLUDE_DIRECTORIES: &[&str] = &[
    "include",
    "src/audio",
    "src/cg",
    "src/regex",
    "lang/usenglish",
    "lang/cmulex",
    "lang/cmu_us_slt",
];

fn require_file(path: &Path) {
    if !path.is_file() {
        panic!(
            "missing locked Flite source {}; run `make prepare-flite` first",
            path.display()
        );
    }
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository = manifest.parent().unwrap();
    let source = env::var_os("OMNIVOX_FLITE_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join("target/flite-inputs/2.2/source"));
    let wrapper = manifest.join("native/omnivox_flite.c");
    let target_family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap();

    require_file(&source.join("COPYING"));
    require_file(&source.join("lang/cmu_us_slt/cmu_us_slt.c"));
    require_file(&wrapper);

    let mut build = cc::Build::new();
    build
        .warnings(false)
        .define("CST_NO_SOCKETS", None)
        .define("CST_AUDIO_NONE", None)
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .flag_if_supported("-std=c99")
        .file(&wrapper);

    let mmap_source = if target_family == "windows" {
        build.define("WIN32", None);
        "src/utils/cst_mmap_win32.c"
    } else {
        "src/utils/cst_mmap_none.c"
    };
    let mmap_path = source.join(mmap_source);
    require_file(&mmap_path);
    println!("cargo:rerun-if-changed={}", mmap_path.display());
    build.file(mmap_path);

    for include in INCLUDE_DIRECTORIES {
        build.include(source.join(include));
    }
    for relative in FLITE_SOURCES {
        let path = source.join(relative);
        require_file(&path);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }

    println!("cargo:rerun-if-env-changed=OMNIVOX_FLITE_SOURCE_DIR");
    println!("cargo:rerun-if-changed={}", wrapper.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("source-inputs.json").display()
    );
    build.compile("omnivox_flite");

    if target_family == "unix" {
        println!("cargo:rustc-link-lib=m");
    }
}
