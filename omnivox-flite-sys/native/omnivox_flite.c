/*
 * Omnivox Flite adapter, Copyright (c) 2026 Robert Melton.
 *
 * This file is an Omnivox modification layer, licensed under MIT. Flite's
 * separately licensed sources are compiled without modification.
 */

#include <stddef.h>

#include "flite.h"

extern void usenglish_init(cst_voice *voice);
extern cst_lexicon *cmu_lex_init(void);
extern cst_voice *register_cmu_us_slt(const char *voice_directory);

int omnivox_flite_initialize(void)
{
    int result = flite_init();
    if (result != 0)
        return result;
    flite_add_lang("eng", usenglish_init, cmu_lex_init);
    flite_add_lang("usenglish", usenglish_init, cmu_lex_init);
    return 0;
}

cst_voice *omnivox_flite_register_slt(void)
{
    return register_cmu_us_slt(NULL);
}

cst_voice *omnivox_flite_load_voice(const char *path)
{
    if (path == NULL)
        return NULL;
    return flite_voice_load(path);
}

void omnivox_flite_delete_voice(cst_voice *voice)
{
    if (voice != NULL)
        delete_voice(voice);
}

const char *omnivox_flite_voice_name(const cst_voice *voice)
{
    if (voice == NULL)
        return NULL;
    return flite_get_param_string(voice->features, "name", voice->name);
}

cst_wave *omnivox_flite_synthesize(
    cst_voice *voice,
    const char *text,
    float duration_stretch,
    float f0_shift)
{
    if (voice == NULL || text == NULL)
        return NULL;
    flite_feat_set_float(voice->features, "duration_stretch", duration_stretch);
    flite_feat_set_float(voice->features, "f0_shift", f0_shift);
    return flite_text_to_wave(text, voice);
}

int omnivox_flite_wave_sample_rate(const cst_wave *wave)
{
    return wave == NULL ? 0 : wave->sample_rate;
}

int omnivox_flite_wave_sample_count(const cst_wave *wave)
{
    return wave == NULL ? 0 : wave->num_samples;
}

int omnivox_flite_wave_channel_count(const cst_wave *wave)
{
    return wave == NULL ? 0 : wave->num_channels;
}

const short *omnivox_flite_wave_samples(const cst_wave *wave)
{
    return wave == NULL ? NULL : wave->samples;
}

void omnivox_flite_delete_wave(cst_wave *wave)
{
    if (wave != NULL)
        delete_wave(wave);
}
