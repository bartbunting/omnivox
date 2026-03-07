/**
 * C bridge implementation over piper's C++ API.
 */

#include "piper_bridge.h"
#include "piper/src/cpp/piper.hpp"

#include <cstdlib>
#include <cstring>
#include <memory>
#include <vector>

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
