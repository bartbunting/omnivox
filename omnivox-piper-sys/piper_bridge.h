/**
 * C bridge over piper's C++ API.
 *
 * Piper uses C++ types (std::string, std::vector, std::optional, callbacks)
 * that bindgen cannot handle directly. This header exposes a plain-C interface
 * that Rust can call via FFI.
 *
 * Lifecycle:
 *   state = piper_init(espeak_data_path);
 *   piper_load_voice(state, model_path, config_path);
 *   audio = piper_synthesize(state, text, &len, &rate);
 *   piper_free_audio(audio);
 *   piper_destroy(state);
 */

#ifndef PIPER_BRIDGE_H
#define PIPER_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PiperState PiperState;

/** Initialise piper with the path to espeak-ng voice data.
 *  Returns NULL on failure. */
PiperState *piper_init(const char *espeak_data_path);

/** Load a voice from an .onnx model file and its companion .json config.
 *  Returns 0 on success, non-zero on failure. */
int piper_load_voice(PiperState *state,
                     const char *model_path,
                     const char *config_path);

/** Synthesise text to 16-bit mono PCM.
 *  On success sets *out_num_samples and *out_sample_rate and returns a
 *  heap-allocated int16_t array that the caller must free with piper_free_audio.
 *  Returns NULL on failure. */
int16_t *piper_synthesize(PiperState *state,
                          const char *text,
                          float length_scale,
                          float noise_scale,
                          float noise_w,
                          uint32_t *out_num_samples,
                          uint32_t *out_sample_rate);

/** Free an audio buffer returned by piper_synthesize. */
void piper_free_audio(int16_t *audio);

/** Destroy a PiperState created by piper_init. */
void piper_destroy(PiperState *state);

/** Return the piper library version string. */
const char *piper_version(void);

#ifdef __cplusplus
}
#endif

#endif /* PIPER_BRIDGE_H */
