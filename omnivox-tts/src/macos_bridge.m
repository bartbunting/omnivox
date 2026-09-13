// Objective-C bridge for AVSpeechSynthesizer buffer capture.
// Uses a persistent synthesizer so stop() can interrupt ongoing speech.
//
// Threading: AVSpeechSynthesizer.writeUtterance:toBufferCallback: requires a
// thread with a live Cocoa RunLoop. Raw POSIX threads (std::thread in Rust)
// don't qualify. All synthesis is dispatched through a private serial GCD
// queue whose worker thread is a proper Cocoa-managed thread with a RunLoop.
// Callers block via a semaphore until synthesis completes.

#import <AVFoundation/AVFoundation.h>
#import <Foundation/Foundation.h>
#include <time.h>

// Persistent synthesizer instance
static AVSpeechSynthesizer *_sharedSynth = nil;
static dispatch_once_t _synthOnce;

static AVSpeechSynthesizer *sharedSynthesizer(void) {
    dispatch_once(&_synthOnce, ^{
        _sharedSynth = [[AVSpeechSynthesizer alloc] init];
    });
    return _sharedSynth;
}

// Serial queue for all synthesis work.
// GCD-managed threads have proper Cocoa RunLoops; std::thread workers do not.
static dispatch_queue_t _synthQueue = nil;
static dispatch_once_t _queueOnce;

static dispatch_queue_t synthQueue(void) {
    dispatch_once(&_queueOnce, ^{
        _synthQueue = dispatch_queue_create("com.omnivox.synthesis", DISPATCH_QUEUE_SERIAL);
    });
    return _synthQueue;
}

// Keep the same voice inventory for the lifetime of this process, including
// across Rust engine instances. Restart Omnivox after installing new voices.
static NSArray<AVSpeechSynthesisVoice *> *sharedVoices(void) {
    static NSArray<AVSpeechSynthesisVoice *> *voices = nil;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        voices = [[AVSpeechSynthesisVoice speechVoices] copy];
    });
    return voices;
}

// Called only on synthQueue. Cache native selection too: neither enumerating
// voices nor asking Apple for the default language voice belongs on every
// utterance. NSNull retains failed lookups without changing their fallback.
static AVSpeechSynthesisVoice *cachedVoice(NSString *lang, NSString *name) {
    static NSMutableDictionary<NSArray *, id> *selected = nil;
    if (selected == nil) selected = [NSMutableDictionary dictionary];
    NSArray *key = @[lang, name ?: (id)[NSNull null]];
    id voice = selected[key];
    if (voice == nil) {
        if (name != nil) {
            for (AVSpeechSynthesisVoice *candidate in sharedVoices()) {
                if ([candidate.language isEqualToString:lang] &&
                    [candidate.name isEqualToString:name]) {
                    voice = candidate;
                    break;
                }
            }
        } else {
            voice = [AVSpeechSynthesisVoice voiceWithLanguage:lang];
        }
        selected[key] = voice ?: [NSNull null];
    }
    return voice == [NSNull null] ? nil : voice;
}

// Monotonic offsets from entry to omnivox_synthesize, in microseconds.
// UINT64_MAX means that the corresponding callback was not observed.
// Keep this layout and the completion codes in sync with macos.rs.
typedef struct {
    uint64_t queue_wait_us;
    uint64_t write_started_us;
    uint64_t first_buffer_us;
    uint64_t last_buffer_us;
    uint64_t completion_signal_us;
    uint64_t capture_completed_us;
    uint64_t bridge_elapsed_us;
    uint32_t buffers_received;
    uint32_t completion_reason;
} SynthTimings;

enum {
    SynthCompletionEmptyBuffer = 1,
    SynthCompletionInactivity = 2,
    SynthCompletionDeadline = 3,
};

static uint64_t elapsedMicroseconds(uint64_t startedAt) {
    return (clock_gettime_nsec_np(CLOCK_UPTIME_RAW) - startedAt) / 1000;
}

// Result struct returned to Rust
typedef struct {
    float *samples;
    uint32_t sample_count;
    uint32_t sample_rate;
    uint16_t channels;
    SynthTimings timings;
} SynthResult;

// Core synthesis — must be called on a GCD thread (synthQueue) so the RunLoop
// pump picks up AVSpeechSynthesizer callbacks.
static SynthResult do_synthesize(
    NSString *nsText,
    NSString *lang,
    NSString *name,
    float rate,
    float pitch,
    float volume,
    uint64_t startedAt
) {
    SynthResult result = {0};
    result.timings.queue_wait_us = elapsedMicroseconds(startedAt);

    @autoreleasepool {
        AVSpeechSynthesizer *synth = sharedSynthesizer();

        // Stop any ongoing speech first
        if (synth.isSpeaking) {
            [synth stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
            [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                     beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
        }

        AVSpeechUtterance *utterance = [AVSpeechUtterance speechUtteranceWithString:nsText];

        // Set voice
        if (lang != nil) {
            utterance.voice = cachedVoice(lang, name);
        }

        utterance.rate = rate;
        utterance.pitchMultiplier = pitch;
        utterance.volume = volume;

        // Collect PCM chunks
        NSMutableData *audioData = [NSMutableData data];
        __block uint32_t sampleRate = 0;
        __block uint16_t channelCount = 0;
        __block BOOL synthesisComplete = NO;
        __block uint32_t chunksReceived = 0;
        __block uint64_t firstBufferUs = UINT64_MAX;
        __block uint64_t lastBufferUs = UINT64_MAX;
        __block uint64_t completionSignalUs = UINT64_MAX;

        result.timings.write_started_us = elapsedMicroseconds(startedAt);
        [synth writeUtterance:utterance toBufferCallback:^(AVAudioBuffer * _Nonnull buffer) {
            uint64_t arrivedUs = elapsedMicroseconds(startedAt);
            AVAudioPCMBuffer *pcm = (AVAudioPCMBuffer *)buffer;

            if (pcm.frameLength == 0) {
                if (completionSignalUs == UINT64_MAX) completionSignalUs = arrivedUs;
                synthesisComplete = YES;
                return;
            }

            sampleRate = (uint32_t)pcm.format.sampleRate;
            channelCount = (uint16_t)pcm.format.channelCount;

            float * const *floatData = pcm.floatChannelData;
            if (floatData == NULL) return;

            if (firstBufferUs == UINT64_MAX) firstBufferUs = arrivedUs;
            lastBufferUs = arrivedUs;
            for (uint32_t frame = 0; frame < pcm.frameLength; frame++) {
                for (uint16_t ch = 0; ch < channelCount; ch++) {
                    float sample = floatData[ch][frame];
                    [audioData appendBytes:&sample length:sizeof(float)];
                }
            }
            chunksReceived++;
        }];

        // Pump this thread's RunLoop until callbacks arrive and synthesis finishes.
        // On a GCD thread the RunLoop is properly initialized, so this works.
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:30.0];
        uint32_t lastChunkCount = 0;
        NSDate *lastChunkTime = [NSDate date];
        result.timings.completion_reason = SynthCompletionDeadline;

        while ([[NSDate date] compare:deadline] == NSOrderedAscending) {
            [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                     beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];

            if (synthesisComplete) {
                result.timings.completion_reason = SynthCompletionEmptyBuffer;
                break;
            }

            // If chunks have stopped arriving for 200ms, consider synthesis done.
            // (Some macOS versions omit the frameLength==0 completion signal.)
            if (chunksReceived > 0) {
                if (chunksReceived != lastChunkCount) {
                    lastChunkCount = chunksReceived;
                    lastChunkTime = [NSDate date];
                } else if ([[NSDate date] timeIntervalSinceDate:lastChunkTime] > 0.2) {
                    result.timings.completion_reason = SynthCompletionInactivity;
                    break;
                }
            }
        }

        result.timings.capture_completed_us = elapsedMicroseconds(startedAt);
        result.timings.first_buffer_us = firstBufferUs;
        result.timings.last_buffer_us = lastBufferUs;
        result.timings.completion_signal_us = completionSignalUs;
        result.timings.buffers_received = chunksReceived;

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

SynthResult omnivox_synthesize(
    const char *text,
    const char *voice_lang,
    const char *voice_name,
    float rate,
    float pitch,
    float volume
) {
    uint64_t startedAt = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
    // Convert C strings to NSStrings on the calling thread before dispatching.
    NSString *nsText     = [NSString stringWithUTF8String:text];
    NSString *nsLang     = voice_lang ? [NSString stringWithUTF8String:voice_lang] : nil;
    NSString *nsName     = voice_name ? [NSString stringWithUTF8String:voice_name] : nil;

    __block SynthResult result = {0};
    dispatch_semaphore_t done = dispatch_semaphore_create(0);

    dispatch_async(synthQueue(), ^{
        result = do_synthesize(nsText, nsLang, nsName, rate, pitch, volume, startedAt);
        dispatch_semaphore_signal(done);
    });

    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
    result.timings.bridge_elapsed_us = elapsedMicroseconds(startedAt);
    return result;
}

void omnivox_stop(void) {
    AVSpeechSynthesizer *synth = sharedSynthesizer();
    if (synth.isSpeaking) {
        [synth stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
    }
}

// Run the main NSRunLoop until omnivox_stop_main_runloop() is called.
// AVSpeechSynthesizer.writeUtterance:toBufferCallback: internally dispatches
// work via the main queue; the main thread must be running its RunLoop for
// those dispatches to be processed. Call this from main() after spawning the
// reader thread, so synthesis (on the worker thread) doesn't deadlock.
static volatile BOOL _runloopShouldStop = NO;

void omnivox_run_main_runloop(void) {
    while (!_runloopShouldStop) {
        [[NSRunLoop mainRunLoop] runMode:NSDefaultRunLoopMode
                             beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
    }
}

void omnivox_stop_main_runloop(void) {
    _runloopShouldStop = YES;
    CFRunLoopStop(CFRunLoopGetMain());
}

BOOL omnivox_is_speaking(void) {
    return sharedSynthesizer().isSpeaking;
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
        NSArray<AVSpeechSynthesisVoice *> *voices = sharedVoices();
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
