//! Checked edits to a generated upstream source copy; never modify the vendor.

fn replace_once(source: &mut String, old: &str, new: &str) {
    assert_eq!(
        source.matches(old).count(),
        1,
        "Piper lifecycle overlay source mismatch: {old}"
    );
    *source = source.replacen(old, new, 1);
}

pub fn apply(source: &str) -> String {
    let mut source = source.replace("\r\n", "\n");
    replace_once(
        &mut source,
        "#include <espeak-ng/speak_lib.h>",
        "#include <espeak-ng/speak_lib.h>\n#include \"omnivox_lifecycle.hpp\"",
    );
    replace_once(
        &mut source,
        "    -> struct piper_synthesizer * {\n  // onnx",
        "    -> struct piper_synthesizer * {\n  OmnivoxPiperConstruction construction;\n  if (!construction.owns_slot) { return nullptr; }\n  try {\n  // onnx",
    );
    replace_once(
        &mut source,
        "  if (phoneme_type == PhonemeType::Espeak &&\n      espeak_Initialize",
        "  construction.phonemizer_attempted = (phoneme_type == PhonemeType::Espeak);\n  if (phoneme_type == PhonemeType::Espeak &&\n      espeak_Initialize",
    );
    replace_once(
        &mut source,
        "  auto *synth = new piper_synthesizer();",
        "  construction.synth = std::make_unique<piper_synthesizer>();\n  auto *synth = construction.synth.get();",
    );
    replace_once(
        &mut source,
        "  return synth;\n}",
        "  return construction.release();\n  } catch (const std::exception &error) {\n    omnivox_piper_exception(\"model loading\", error.what());\n  } catch (...) {\n    omnivox_piper_exception(\"model loading\", \"unknown native exception\");\n  }\n  return nullptr;\n}",
    );
    replace_once(
        &mut source,
        "  delete synth;\n}",
        "  delete synth;\n  omnivox_piper_model_owned.store(false);\n}",
    );
    // Inference can throw too (e.g. an incompatible model graph). Convert those
    // exceptions to the existing synthesis error status, not VoiceNotFound.
    replace_once(
        &mut source,
        "                            const piper_synthesize_options *options) -> int {",
        "                            const piper_synthesize_options *options) -> int try {",
    );
    replace_once(
        &mut source,
        "                           struct piper_audio_chunk *chunk) -> int {",
        "                           struct piper_audio_chunk *chunk) -> int try {",
    );
    // Keep exceptions from both entry points inside the native library.
    let catches = " catch (const std::exception &error) {\n  omnivox_piper_exception(\"synthesis\", error.what());\n  return PIPER_ERR_GENERIC;\n} catch (...) {\n  omnivox_piper_exception(\"synthesis\", \"unknown native exception\");\n  return PIPER_ERR_GENERIC;\n}";
    for ending in [
        "  return PIPER_OK;\n}",
        "  return chunk->is_last ? PIPER_DONE : PIPER_OK;\n}",
    ] {
        replace_once(&mut source, ending, &format!("{ending}{catches}"));
    }
    source
}
