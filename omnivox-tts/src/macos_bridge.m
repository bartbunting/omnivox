// Objective-C bridge for AVSpeechSynthesizer buffer capture.
// Exposes a C function that synthesizes text and returns PCM float data.

#import <AVFoundation/AVFoundation.h>
#import <Foundation/Foundation.h>

// Result struct returned to Rust
typedef struct {
    float *samples;
    uint32_t sample_count;
    uint32_t sample_rate;
    uint16_t channels;
} SynthResult;

SynthResult omnivox_synthesize(
    const char *text,
    const char *voice_lang,
    const char *voice_name,
    float rate,
    float pitch,
    float volume
) {
    SynthResult result = {NULL, 0, 0, 0};

    @autoreleasepool {
        AVSpeechSynthesizer *synth = [[AVSpeechSynthesizer alloc] init];
        NSString *nsText = [NSString stringWithUTF8String:text];
        AVSpeechUtterance *utterance = [AVSpeechUtterance speechUtteranceWithString:nsText];

        // Set voice
        if (voice_lang != NULL) {
            NSString *lang = [NSString stringWithUTF8String:voice_lang];
            if (voice_name != NULL) {
                // Find voice by language + name
                NSString *name = [NSString stringWithUTF8String:voice_name];
                NSArray<AVSpeechSynthesisVoice *> *voices = [AVSpeechSynthesisVoice speechVoices];
                for (AVSpeechSynthesisVoice *v in voices) {
                    if ([v.language isEqualToString:lang] && [v.name isEqualToString:name]) {
                        utterance.voice = v;
                        break;
                    }
                }
            } else {
                utterance.voice = [AVSpeechSynthesisVoice voiceWithLanguage:lang];
            }
        }

        utterance.rate = rate;
        utterance.pitchMultiplier = pitch;
        utterance.volume = volume;

        // Collect PCM chunks
        NSMutableData *audioData = [NSMutableData data];
        __block uint32_t sampleRate = 0;
        __block uint16_t channelCount = 0;
        __block BOOL synthesisComplete = NO;

        [synth writeUtterance:utterance toBufferCallback:^(AVAudioBuffer * _Nonnull buffer) {
            AVAudioPCMBuffer *pcm = (AVAudioPCMBuffer *)buffer;

            if (pcm.frameLength == 0) {
                synthesisComplete = YES;
                return;
            }

            sampleRate = (uint32_t)pcm.format.sampleRate;
            channelCount = (uint16_t)pcm.format.channelCount;

            // Get float channel data and interleave
            const float * const *floatData = pcm.floatChannelData;
            if (floatData == NULL) return;

            for (uint32_t frame = 0; frame < pcm.frameLength; frame++) {
                for (uint16_t ch = 0; ch < channelCount; ch++) {
                    float sample = floatData[ch][frame];
                    [audioData appendBytes:&sample length:sizeof(float)];
                }
            }
        }];

        // Pump RunLoop until done or timeout (30s)
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:30.0];
        while (!synthesisComplete && [[NSDate date] compare:deadline] == NSOrderedAscending) {
            [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                     beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
        }

        // If we got no completion signal but have data, consider it done
        // (some macOS versions don't send frameLength==0)
        if (audioData.length > 0) {
            uint32_t totalSamples = (uint32_t)(audioData.length / sizeof(float));
            result.samples = (float *)malloc(audioData.length);
            memcpy(result.samples, audioData.bytes, audioData.length);
            result.sample_count = totalSamples;
            result.sample_rate = sampleRate;
            result.channels = channelCount;
        }
    }

    return result;
}

void omnivox_free_samples(float *samples) {
    if (samples != NULL) {
        free(samples);
    }
}

// Voice listing
typedef struct {
    char *identifier;
    char *name;
    char *language;
} VoiceEntry;

typedef struct {
    VoiceEntry *entries;
    uint32_t count;
} VoiceList;

VoiceList omnivox_list_voices(void) {
    VoiceList list = {NULL, 0};

    @autoreleasepool {
        NSArray<AVSpeechSynthesisVoice *> *voices = [AVSpeechSynthesisVoice speechVoices];
        list.count = (uint32_t)voices.count;
        list.entries = (VoiceEntry *)malloc(sizeof(VoiceEntry) * list.count);

        for (uint32_t i = 0; i < list.count; i++) {
            AVSpeechSynthesisVoice *v = voices[i];
            list.entries[i].identifier = strdup(v.identifier.UTF8String);
            list.entries[i].name = strdup(v.name.UTF8String);
            list.entries[i].language = strdup(v.language.UTF8String);
        }
    }

    return list;
}

void omnivox_free_voice_list(VoiceList list) {
    for (uint32_t i = 0; i < list.count; i++) {
        free(list.entries[i].identifier);
        free(list.entries[i].name);
        free(list.entries[i].language);
    }
    free(list.entries);
}
