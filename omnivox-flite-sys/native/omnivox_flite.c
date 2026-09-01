/*
 * Omnivox Flite adapter, Copyright (c) 2026 Robert Melton.
 *
 * This file is an Omnivox modification layer, licensed under MIT. Flite's
 * separately licensed sources are compiled without modification.
 */

#include <stddef.h>
#include <string.h>

#include "flite.h"

extern void usenglish_init(cst_voice *voice);
extern cst_lexicon *cmu_lex_init(void);
extern cst_voice *register_cmu_us_slt(const char *voice_directory);

typedef struct omnivox_flite_word_marker_struct {
    int frame_offset;
    int text_start;
    int text_length;
} omnivox_flite_word_marker;

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

cst_utterance *omnivox_flite_synthesize(
    cst_voice *voice,
    const char *text,
    float duration_stretch,
    float f0_shift)
{
    if (voice == NULL || text == NULL)
        return NULL;
    flite_feat_set_float(voice->features, "duration_stretch", duration_stretch);
    flite_feat_set_float(voice->features, "f0_shift", f0_shift);
    return flite_synth_text(text, voice);
}

static cst_wave *omnivox_flite_synthesis_wave(cst_utterance *synthesis)
{
    return synthesis == NULL ? NULL : utt_wave(synthesis);
}

int omnivox_flite_synthesis_sample_rate(cst_utterance *synthesis)
{
    cst_wave *wave = omnivox_flite_synthesis_wave(synthesis);
    return wave == NULL ? 0 : wave->sample_rate;
}

int omnivox_flite_synthesis_sample_count(cst_utterance *synthesis)
{
    cst_wave *wave = omnivox_flite_synthesis_wave(synthesis);
    return wave == NULL ? 0 : wave->num_samples;
}

int omnivox_flite_synthesis_channel_count(cst_utterance *synthesis)
{
    cst_wave *wave = omnivox_flite_synthesis_wave(synthesis);
    return wave == NULL ? 0 : wave->num_channels;
}

const short *omnivox_flite_synthesis_samples(cst_utterance *synthesis)
{
    cst_wave *wave = omnivox_flite_synthesis_wave(synthesis);
    return wave == NULL ? NULL : wave->samples;
}

int omnivox_flite_synthesis_word_markers(
    cst_utterance *synthesis,
    const char *text,
    omnivox_flite_word_marker *markers,
    int capacity)
{
    cst_item *token;
    cst_relation *tokens;
    cst_wave *wave;
    const char *cursor;
    int count = 0;

    if (synthesis == NULL || text == NULL || markers == NULL || capacity < 0)
        return -1;
    wave = omnivox_flite_synthesis_wave(synthesis);
    tokens = utt_relation(synthesis, "Token");
    if (wave == NULL || tokens == NULL)
        return -1;

    cursor = text;
    for (token = relation_head(tokens); token != NULL; token = item_next(token))
    {
        cst_item *segment;
        const char *name;
        const char *source;
        size_t length;
        float start_seconds;
        double frame_offset;

        name = item_feat_string(token, "name");
        if (name == NULL || name[0] == '\0')
            continue;
        source = strstr(cursor, name);
        if (source == NULL)
            continue;
        length = strlen(name);
        cursor = source + length;

        segment = path_to_item(
            token,
            "R:Token.daughter1.R:SylStructure.daughter1.daughter1.R:Segment");
        if (segment == NULL)
            continue;
        start_seconds = ffeature_float(segment, "p.end");
        if (!(start_seconds >= 0.0f))
            continue;
        if (count >= capacity)
            return -1;

        frame_offset = (double)start_seconds * (double)wave->sample_rate;
        if (frame_offset >= (double)wave->num_samples)
            markers[count].frame_offset = wave->num_samples;
        else
            markers[count].frame_offset = (int)(frame_offset + 0.5);
        markers[count].text_start = (int)(source - text);
        markers[count].text_length = (int)length;
        count++;
    }
    return count;
}

void omnivox_flite_delete_synthesis(cst_utterance *synthesis)
{
    if (synthesis != NULL)
        delete_utterance(synthesis);
}
