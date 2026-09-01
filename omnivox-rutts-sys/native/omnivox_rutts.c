/*
 * Minimal Omnivox boundary for the MIT-licensed RuTTS library.
 *
 * SPDX-License-Identifier: MIT
 */

#include <stddef.h>
#include <stdint.h>

#include "ru_tts.h"

#define OMNIVOX_RUTTS_WAVE_BUFFER_SIZE 4096

typedef int (*omnivox_rutts_callback)(const int8_t *samples, size_t count,
                                      void *user_data);

typedef struct {
  omnivox_rutts_callback callback;
  void *user_data;
  int status;
} omnivox_rutts_sink;

static int omnivox_rutts_consume(void *buffer, size_t size, void *user_data) {
  omnivox_rutts_sink *sink = (omnivox_rutts_sink *)user_data;

  if (!sink->status) {
    sink->status = sink->callback((const int8_t *)buffer, size, sink->user_data);
  }
  return sink->status;
}

int omnivox_rutts_synthesize(const char *koi8r_text, int speech_rate,
                             int voice_pitch, int intonation,
                             int alternative_voice,
                             omnivox_rutts_callback callback,
                             void *user_data) {
  int8_t wave_buffer[OMNIVOX_RUTTS_WAVE_BUFFER_SIZE];
  omnivox_rutts_sink sink;
  ru_tts_conf_t config;

  if (!koi8r_text || !callback) {
    return -1;
  }

  sink.callback = callback;
  sink.user_data = user_data;
  sink.status = 0;
  ru_tts_config_init(&config);
  config.speech_rate = speech_rate;
  config.voice_pitch = voice_pitch;
  config.intonation = intonation;
  if (alternative_voice) {
    config.flags |= USE_ALTERNATIVE_VOICE;
  }

  ru_tts_transfer(&config, koi8r_text, wave_buffer, sizeof(wave_buffer),
                  omnivox_rutts_consume, &sink);
  return sink.status;
}
