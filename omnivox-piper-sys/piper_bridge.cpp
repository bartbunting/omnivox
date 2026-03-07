/**
 * C bridge implementation over piper's C++ API.
 */

#include "piper_bridge.h"
// piper.hpp is on the include path via cmake target_include_directories
#include "piper.hpp"

#include <cstdlib>
#include <cstring>
#include <memory>
#include <stdexcept>
#include <vector>

#ifdef __APPLE__
#include <dlfcn.h>
// AUDIO_OUTPUT_SYNCHRONOUS = 2 (from espeak-ng/speak_lib.h)
static const int ESPEAK_AUDIO_OUTPUT_SYNCHRONOUS = 2;

// On macOS the binary statically links libespeak-ng.a (from espeak-rs-sys)
// AND loads libespeak-ng.dylib at runtime (as a transitive dep of
// piper_phonemize.dylib).  These are TWO separate espeak-ng instances with
// separate global state.  piper::initialize() calls espeak_Initialize() on
// the STATIC instance; piper_phonemize.dylib's espeak_SetVoiceByName() goes
// to the DYNAMIC instance (two-level namespace binding).  If the dynamic
// instance is never initialized, SetVoiceByName fails with "Failed to set
// eSpeak-ng voice".
//
// Fix: open the piper espeak dylib by its install path (baked in at build
// time) and call its espeak_Initialize directly.
static void init_dynamic_espeak(const char *data_path) {
    using EspeakInitFn = int (*)(int, int, const char *, int);
    // PIPER_LIB_DIR is defined by cmake to the install/lib directory.
    void *lib = dlopen(PIPER_LIB_DIR "/libespeak-ng.1.dylib",
                       RTLD_NOW | RTLD_LOCAL);
    if (!lib) return;
    auto fn = reinterpret_cast<EspeakInitFn>(dlsym(lib, "espeak_Initialize"));
    if (fn) {
        fn(ESPEAK_AUDIO_OUTPUT_SYNCHRONOUS, 0, data_path, 0);
    }
    // dlclose decrements our ref-count; the lib stays loaded because it is
    // also a required LOAD_DYLIB of the binary itself.
    dlclose(lib);
}
#endif // __APPLE__

struct PiperState {
    piper::PiperConfig config;
    piper::Voice voice;
    bool voice_loaded = false;
};

extern "C" {

PiperState *piper_init(const char *espeak_data_path) {
    if (!espeak_data_path) return nullptr;
    auto *state = new (std::nothrow) PiperState();
    if (!state) return nullptr;
    state->config.eSpeakDataPath = std::string(espeak_data_path);
    state->config.useESpeak = true;
    try {
        piper::initialize(state->config);
    } catch (...) {
        delete state;
        return nullptr;
    }
#ifdef __APPLE__
    // Initialize the dynamic espeak-ng instance used by piper_phonemize.dylib.
    // piper::initialize() already called espeak_Initialize on the static
    // libespeak-ng.a; we must also call it on the dynamic .dylib so that
    // piper_phonemize's two-level-namespace-bound espeak calls succeed.
    init_dynamic_espeak(espeak_data_path);
#endif
    return state;
}

int piper_load_voice(PiperState *state,
                     const char *model_path,
                     const char *config_path) {
    if (!state || !model_path || !config_path) return 1;
    try {
        std::optional<piper::SpeakerId> speaker_id;
        piper::loadVoice(state->config,
                         std::string(model_path),
                         std::string(config_path),
                         state->voice,
                         speaker_id,
                         /*useCuda=*/false);
        state->voice_loaded = true;
        return 0;
    } catch (const std::exception &e) {
        return 1;
    } catch (...) {
        return 1;
    }
}

int16_t *piper_synthesize(PiperState *state,
                          const char *text,
                          float length_scale,
                          float noise_scale,
                          float noise_w,
                          uint32_t *out_num_samples,
                          uint32_t *out_sample_rate) {
    if (!state || !state->voice_loaded || !text ||
        !out_num_samples || !out_sample_rate)
        return nullptr;

    // Apply synthesis parameters
    state->voice.synthesisConfig.lengthScale = length_scale;
    state->voice.synthesisConfig.noiseScale  = noise_scale;
    state->voice.synthesisConfig.noiseW      = noise_w;

    std::vector<int16_t> audio_buffer;
    piper::SynthesisResult result;

    try {
        piper::textToAudio(state->config,
                           state->voice,
                           std::string(text),
                           audio_buffer,
                           result,
                           nullptr /* no streaming callback */);
    } catch (const std::exception &e) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }

    if (audio_buffer.empty()) return nullptr;

    int16_t *out = static_cast<int16_t *>(
        malloc(audio_buffer.size() * sizeof(int16_t)));
    if (!out) return nullptr;

    memcpy(out, audio_buffer.data(), audio_buffer.size() * sizeof(int16_t));
    *out_num_samples = static_cast<uint32_t>(audio_buffer.size());
    *out_sample_rate = static_cast<uint32_t>(
        state->voice.synthesisConfig.sampleRate);
    return out;
}

void piper_free_audio(int16_t *audio) {
    free(audio);
}

void piper_destroy(PiperState *state) {
    if (!state) return;
    try {
        piper::terminate(state->config);
    } catch (...) {}
    delete state;
}

const char *piper_version(void) {
    static std::string version = piper::getVersion();
    return version.c_str();
}

} // extern "C"
