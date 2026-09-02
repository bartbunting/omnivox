/*
 * Omnivox boundary for the MIT-licensed TGSpeechBox DSP and frontend.
 *
 * SPDX-License-Identifier: MIT
 */

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <exception>
#include <string>

#include "frontend/nvspFrontend.h"
#include "speechPlayer.h"

namespace {

thread_local std::string create_error;

enum class builtin_voice {
  none,
  adam,
  benjamin,
  caleb,
  david,
  robert,
};

struct engine {
  nvspFrontend_handle_t frontend = nullptr;
  speechPlayer_handle_t player = nullptr;
  int sample_rate = 0;
  double volume = 1.0;
  builtin_voice builtin = builtin_voice::adam;
  std::string last_error;
};

std::string lowercase(const char *value) {
  std::string result = value ? value : "";
  std::transform(result.begin(), result.end(), result.begin(), [](unsigned char c) {
    return static_cast<char>(std::tolower(c));
  });
  return result;
}

builtin_voice parse_builtin(const char *profile) {
  const std::string name = lowercase(profile);
  if (name.empty() || name == "adam") return builtin_voice::adam;
  if (name == "benjamin") return builtin_voice::benjamin;
  if (name == "caleb") return builtin_voice::caleb;
  if (name == "david") return builtin_voice::david;
  if (name == "robert") return builtin_voice::robert;
  return builtin_voice::none;
}

void apply_builtin(speechPlayer_frame_t &frame, builtin_voice voice) {
  switch (voice) {
  case builtin_voice::adam:
    frame.voicePitch *= 0.92;
    frame.endVoicePitch *= 0.92;
    frame.cb1 *= 1.3;
    frame.pa6 *= 1.3;
    frame.fricationAmplitude *= 0.85;
    break;
  case builtin_voice::benjamin:
    frame.cf1 *= 1.01;
    frame.cf2 *= 1.02;
    frame.cf4 = 3770.0;
    frame.cf5 = 4100.0;
    frame.cf6 = 5000.0;
    frame.cfNP *= 0.9;
    frame.cb1 *= 1.3;
    frame.fricationAmplitude *= 0.7;
    frame.pa6 *= 1.3;
    break;
  case builtin_voice::caleb:
    frame.aspirationAmplitude = 1.0;
    frame.voiceAmplitude = 0.0;
    break;
  case builtin_voice::david:
    frame.voicePitch *= 0.75;
    frame.endVoicePitch *= 0.75;
    frame.cf1 *= 0.90;
    frame.cf2 *= 0.93;
    frame.cf3 *= 0.95;
    break;
  case builtin_voice::robert:
    frame.voicePitch *= 1.10;
    frame.endVoicePitch *= 1.10;
    frame.cf1 *= 1.02;
    frame.cf2 *= 1.06;
    frame.cf3 *= 1.08;
    frame.cf4 *= 1.08;
    frame.cf5 *= 1.10;
    frame.cf6 *= 1.05;
    frame.cb1 *= 0.65;
    frame.cb2 *= 0.68;
    frame.cb3 *= 0.72;
    frame.cb4 *= 0.75;
    frame.cb5 *= 0.78;
    frame.cb6 *= 0.80;
    frame.glottalOpenQuotient = 0.30;
    frame.voiceTurbulenceAmplitude *= 0.20;
    frame.fricationAmplitude *= 0.75;
    frame.parallelBypass *= 0.70;
    frame.pa3 *= 1.08;
    frame.pa4 *= 1.15;
    frame.pa5 *= 1.20;
    frame.pa6 *= 1.25;
    frame.pb1 *= 0.72;
    frame.pb2 *= 0.75;
    frame.pb3 *= 0.78;
    frame.pb4 *= 0.80;
    frame.pb5 *= 0.82;
    frame.pb6 *= 0.85;
    frame.pf3 *= 1.06;
    frame.pf4 *= 1.08;
    frame.pf5 *= 1.10;
    frame.vibratoPitchOffset = 0.0;
    frame.vibratoSpeed = 0.0;
    break;
  case builtin_voice::none:
    break;
  }
}

unsigned int milliseconds_to_samples(const engine &state, double milliseconds) {
  if (milliseconds <= 0.0) return 0;
  const double samples = milliseconds * static_cast<double>(state.sample_rate) / 1000.0;
  return samples <= 0.0 ? 0 : static_cast<unsigned int>(std::ceil(samples));
}

void frame_callback(void *user_data, const nvspFrontend_Frame *frame_or_null,
                    const nvspFrontend_FrameEx *frame_ex_or_null,
                    double duration_ms, double fade_ms, int user_index) {
  auto *state = static_cast<engine *>(user_data);
  if (!state || !state->player) return;
  const unsigned int duration = std::max(1u, milliseconds_to_samples(*state, duration_ms));
  const unsigned int fade = std::max(1u, milliseconds_to_samples(*state, fade_ms));
  if (!frame_or_null) {
    speechPlayer_queueFrameEx(state->player, nullptr, nullptr, 0, duration, fade,
                              user_index, false);
    return;
  }

  static_assert(sizeof(nvspFrontend_Frame) == sizeof(speechPlayer_frame_t),
                "TGSpeechBox frame layouts differ");
  speechPlayer_frame_t frame{};
  std::memcpy(&frame, frame_or_null, sizeof(frame));
  apply_builtin(frame, state->builtin);
  frame.outputGain *= state->volume;

  if (frame_ex_or_null) {
    static_assert(sizeof(nvspFrontend_FrameEx) == sizeof(speechPlayer_frameEx_t),
                  "TGSpeechBox FrameEx layouts differ");
    speechPlayer_queueFrameEx(
        state->player, &frame,
        reinterpret_cast<const speechPlayer_frameEx_t *>(frame_ex_or_null),
        static_cast<unsigned int>(sizeof(speechPlayer_frameEx_t)), duration, fade,
        user_index, false);
  } else {
    speechPlayer_queueFrame(state->player, &frame, duration, fade, user_index, false);
  }
}

void set_profile_tone(engine &state, bool yaml_profile) {
  speechPlayer_voicingTone_t tone = speechPlayer_getDefaultVoicingTone();
  if (yaml_profile) {
    nvspFrontend_VoicingTone source{};
    if (nvspFrontend_getVoicingTone(state.frontend, &source)) {
      tone.voicingPeakPos = source.voicingPeakPos;
      tone.voicedPreEmphA = source.voicedPreEmphA;
      tone.voicedPreEmphMix = source.voicedPreEmphMix;
      tone.highShelfGainDb = source.highShelfGainDb;
      tone.highShelfFcHz = source.highShelfFcHz;
      tone.highShelfQ = source.highShelfQ;
      tone.voicedTiltDbPerOct = source.voicedTiltDbPerOct;
      tone.noiseGlottalModDepth = source.noiseGlottalModDepth;
      tone.pitchSyncF1DeltaHz = source.pitchSyncF1DeltaHz;
      tone.pitchSyncB1DeltaHz = source.pitchSyncB1DeltaHz;
      tone.speedQuotient = source.speedQuotient;
      tone.aspirationTiltDbPerOct = source.aspirationTiltDbPerOct;
      tone.cascadeBwScale = source.cascadeBwScale;
      tone.tremorDepth = source.tremorDepth;
      tone.nasalBwScale = source.nasalBwScale;
      tone.f4FreqScale = source.f4FreqScale;
      tone.nasalGainScale = source.nasalGainScale;
      tone.chorusDepth = source.chorusDepth;
      tone.chorusDetuneHz = source.chorusDetuneHz;
    }
  }
  if (state.builtin == builtin_voice::robert) tone.voicedTiltDbPerOct = -6.0;
  speechPlayer_setVoicingTone(state.player, &tone);
}

bool initialize_player(engine &state) {
  state.player = speechPlayer_initialize(state.sample_rate);
  if (!state.player) {
    state.last_error = "TGSpeechBox could not initialize its DSP player";
    return false;
  }
  return true;
}

char clause_type(const char *text) {
  if (!text) return '.';
  const std::string value(text);
  const auto position = value.find_last_not_of(" \t\r\n");
  if (position == std::string::npos) return '.';
  const char last = value[position];
  return last == '?' || last == '!' || last == ',' ? last : '.';
}

} // namespace

extern "C" {

void *omnivox_tgspeechbox_create(const char *pack_root, int sample_rate) {
  create_error.clear();
  if (!pack_root || !*pack_root || sample_rate <= 0) {
    create_error = "TGSpeechBox requires a pack root and positive sample rate";
    return nullptr;
  }
  try {
    auto *state = new engine();
    state->sample_rate = sample_rate;
    state->frontend = nvspFrontend_create(pack_root);
    if (!state->frontend) {
      create_error = "TGSpeechBox could not load its language packs";
      delete state;
      return nullptr;
    }
    if (!initialize_player(*state)) {
      create_error = state->last_error;
      nvspFrontend_destroy(state->frontend);
      delete state;
      return nullptr;
    }
    return state;
  } catch (const std::exception &error) {
    create_error = error.what();
  } catch (...) {
    create_error = "unknown exception while creating TGSpeechBox";
  }
  return nullptr;
}

const char *omnivox_tgspeechbox_create_error() { return create_error.c_str(); }

void omnivox_tgspeechbox_destroy(void *handle) {
  auto *state = static_cast<engine *>(handle);
  if (!state) return;
  if (state->player) speechPlayer_terminate(state->player);
  if (state->frontend) nvspFrontend_destroy(state->frontend);
  delete state;
}

const char *omnivox_tgspeechbox_last_error(void *handle) {
  auto *state = static_cast<engine *>(handle);
  return state ? state->last_error.c_str() : "invalid TGSpeechBox handle";
}

uint32_t omnivox_tgspeechbox_dsp_version() {
  return speechPlayer_getDspVersion();
}

int omnivox_tgspeechbox_frontend_abi_version() {
  return nvspFrontend_getABIVersion();
}

char *omnivox_tgspeechbox_languages(void *handle) {
  auto *state = static_cast<engine *>(handle);
  return state ? nvspFrontend_getAvailableLanguages(state->frontend) : nullptr;
}

const char *omnivox_tgspeechbox_profile_names(void *handle) {
  auto *state = static_cast<engine *>(handle);
  return state ? nvspFrontend_getVoiceProfileNames(state->frontend) : "";
}

void omnivox_tgspeechbox_free_string(char *value) {
  if (value) nvspFrontend_freeString(value);
}

int omnivox_tgspeechbox_configure(void *handle, const char *language,
                                  const char *profile) {
  auto *state = static_cast<engine *>(handle);
  if (!state || !language || !*language || !profile) return 0;
  try {
    state->last_error.clear();
    if (!nvspFrontend_setLanguage(state->frontend, language)) {
      const char *error = nvspFrontend_getLastError(state->frontend);
      state->last_error = error && *error ? error : "TGSpeechBox rejected the language";
      return 0;
    }
    state->builtin = parse_builtin(profile);
    const bool yaml_profile = state->builtin == builtin_voice::none;
    if (!nvspFrontend_setVoiceProfile(state->frontend,
                                      yaml_profile ? profile : "")) {
      state->last_error = "TGSpeechBox could not select the voice profile";
      return 0;
    }
    nvspFrontend_setFrameExDefaults(state->frontend, 0.0, 0.0, 0.0, 0.0, 1.0);
    set_profile_tone(*state, yaml_profile);
    return 1;
  } catch (const std::exception &error) {
    state->last_error = error.what();
  } catch (...) {
    state->last_error = "unknown exception while configuring TGSpeechBox";
  }
  return 0;
}

char *omnivox_tgspeechbox_prepare_text(void *handle, const char *text) {
  auto *state = static_cast<engine *>(handle);
  return state && text ? nvspFrontend_prepareText(state->frontend, text) : nullptr;
}

int omnivox_tgspeechbox_begin(void *handle, const char *text, const char *ipa,
                              double speed, double base_pitch_hz,
                              double inflection, double volume) {
  auto *state = static_cast<engine *>(handle);
  if (!state || !text || !ipa || !*ipa) return 0;
  try {
    state->last_error.clear();
    state->volume = std::clamp(volume, 0.0, 1.0);
    double synthesis_speed = std::clamp(speed, 0.25, 4.0);
    double time_stretch = 1.0;
    if (synthesis_speed > 2.0) {
      time_stretch = synthesis_speed / 2.0;
      synthesis_speed = 2.0;
    }
    speechPlayer_setTimeStretch(state->player, time_stretch);
    const char clause[] = {clause_type(text), '\0'};
    const int queued = nvspFrontend_queueIPA_ExWithText(
        state->frontend, text, ipa, synthesis_speed,
        std::clamp(base_pitch_hz, 25.0, 300.0),
        std::clamp(inflection, 0.0, 1.0), clause, 0, frame_callback, state);
    if (!queued) {
      const char *error = nvspFrontend_getLastError(state->frontend);
      state->last_error = error && *error ? error : "TGSpeechBox rejected the IPA input";
      return 0;
    }
    return 1;
  } catch (const std::exception &error) {
    state->last_error = error.what();
  } catch (...) {
    state->last_error = "unknown exception while queueing TGSpeechBox frames";
  }
  return 0;
}

int omnivox_tgspeechbox_next(void *handle, int16_t *samples, size_t capacity) {
  auto *state = static_cast<engine *>(handle);
  if (!state || !samples || capacity == 0 || capacity > UINT32_MAX) return -1;
  try {
    static_assert(sizeof(sample) == sizeof(int16_t), "TGSpeechBox sample layout differs");
    return speechPlayer_synthesize(state->player, static_cast<unsigned int>(capacity),
                                   reinterpret_cast<sample *>(samples));
  } catch (const std::exception &error) {
    state->last_error = error.what();
  } catch (...) {
    state->last_error = "unknown exception while synthesizing TGSpeechBox PCM";
  }
  return -1;
}

int omnivox_tgspeechbox_reset(void *handle) {
  auto *state = static_cast<engine *>(handle);
  if (!state) return 0;
  try {
    if (state->player) speechPlayer_terminate(state->player);
    state->player = nullptr;
    return initialize_player(*state) ? 1 : 0;
  } catch (...) {
    state->last_error = "exception while resetting TGSpeechBox";
    return 0;
  }
}

} // extern "C"
