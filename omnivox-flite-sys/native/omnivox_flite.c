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

typedef int (*omnivox_flite_stream_callback)(
    const short *samples,
    int sample_count,
    int sample_rate,
    int channel_count,
    int last,
    const omnivox_flite_word_marker *markers,
    int marker_count,
    void *user_data);

typedef struct omnivox_flite_stream_context_struct {
    omnivox_flite_stream_callback callback;
    void *user_data;
    const char *text;
    int marker_capacity;
    int markers_sent;
} omnivox_flite_stream_context;

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

static int omnivox_flite_collect_word_markers(
    const cst_utterance *synthesis,
    const cst_wave *wave,
    const char *text,
    omnivox_flite_word_marker *markers,
    int capacity)
{
    cst_item *token;
    cst_relation *tokens;
    const char *cursor;
    int count = 0;

    if (synthesis == NULL || wave == NULL || text == NULL ||
        markers == NULL || capacity < 0)
        return -1;
    tokens = utt_relation(synthesis, "Token");
    if (tokens == NULL)
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

static int omnivox_flite_stream_audio(
    const cst_wave *wave,
    int start,
    int size,
    int last,
    cst_audio_streaming_info *streaming)
{
    omnivox_flite_stream_context *context;
    omnivox_flite_word_marker *markers = NULL;
    int marker_count = 0;
    int result;

    if (wave == NULL || streaming == NULL || start < 0 || size < 0 ||
        start > wave->num_samples || size > wave->num_samples - start)
        return CST_AUDIO_STREAM_STOP;
    context = (omnivox_flite_stream_context *)streaming->userdata;
    if (context == NULL || context->callback == NULL)
        return CST_AUDIO_STREAM_STOP;

    if (!context->markers_sent)
    {
        if (context->marker_capacity > 0)
            markers = cst_alloc(omnivox_flite_word_marker,
                                context->marker_capacity);
        marker_count = omnivox_flite_collect_word_markers(
            streaming->utt,
            wave,
            context->text,
            markers,
            context->marker_capacity);
        context->markers_sent = 1;
    }

    result = context->callback(
        size == 0 ? NULL : &wave->samples[start],
        size,
        wave->sample_rate,
        wave->num_channels,
        last,
        markers,
        marker_count,
        context->user_data);
    if (markers != NULL)
        cst_free(markers);
    return result == 0 ? CST_AUDIO_STREAM_STOP : CST_AUDIO_STREAM_CONT;
}

cst_utterance *omnivox_flite_synthesize_stream(
    cst_voice *voice,
    const char *text,
    float duration_stretch,
    float f0_shift,
    int marker_capacity,
    omnivox_flite_stream_callback callback,
    void *user_data)
{
    cst_audio_streaming_info *streaming;
    omnivox_flite_stream_context context;
    cst_utterance *synthesis;

    if (voice == NULL || text == NULL || callback == NULL ||
        marker_capacity < 0)
        return NULL;
    streaming = new_audio_streaming_info();
    if (streaming == NULL)
        return NULL;
    context.callback = callback;
    context.user_data = user_data;
    context.text = text;
    context.marker_capacity = marker_capacity;
    context.markers_sent = 0;
    streaming->asc = omnivox_flite_stream_audio;
    streaming->userdata = &context;

    flite_feat_set_float(voice->features, "duration_stretch", duration_stretch);
    flite_feat_set_float(voice->features, "f0_shift", f0_shift);
    feat_set(voice->features, "streaming_info",
             audio_streaming_info_val(streaming));
    synthesis = flite_synth_text(text, voice);
    feat_remove(voice->features, "streaming_info");
    return synthesis;
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
    cst_wave *wave;

    if (synthesis == NULL || text == NULL || markers == NULL || capacity < 0)
        return -1;
    wave = omnivox_flite_synthesis_wave(synthesis);
    return omnivox_flite_collect_word_markers(
        synthesis, wave, text, markers, capacity);
}

void omnivox_flite_delete_synthesis(cst_utterance *synthesis)
{
    if (synthesis != NULL)
        delete_utterance(synthesis);
}
