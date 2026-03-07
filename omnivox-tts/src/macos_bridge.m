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

// Result struct returned to Rust
typedef struct {
    float *samples;
    uint32_t sample_count;
    uint32_t sample_rate;
    uint16_t channels;
} SynthResult;

// Core synthesis — must be called on a GCD thread (synthQueue) so the RunLoop
// pump picks up AVSpeechSynthesizer callbacks.
static SynthResult do_synthesize(
    NSString *nsText,
    NSString *lang,
    NSString *name,
    float rate,
    float pitch,
    float volume
) {
    SynthResult result = {NULL, 0, 0, 0};

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
            if (name != nil) {
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
        __block uint32_t chunksReceived = 0;

        [synth writeUtterance:utterance toBufferCallback:^(AVAudioBuffer * _Nonnull buffer) {
            AVAudioPCMBuffer *pcm = (AVAudioPCMBuffer *)buffer;

            if (pcm.frameLength == 0) {
                synthesisComplete = YES;
                return;
            }

            sampleRate = (uint32_t)pcm.format.sampleRate;
            channelCount = (uint16_t)pcm.format.channelCount;

            float * const *floatData = pcm.floatChannelData;
            if (floatData == NULL) return;

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

        while ([[NSDate date] compare:deadline] == NSOrderedAscending) {
            [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                     beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];

            if (synthesisComplete) break;

            // If chunks have stopped arriving for 200ms, consider synthesis done.
            // (Some macOS versions omit the frameLength==0 completion signal.)
            if (chunksReceived > 0) {
                if (chunksReceived != lastChunkCount) {
                    lastChunkCount = chunksReceived;
                    lastChunkTime = [NSDate date];
                } else if ([[NSDate date] timeIntervalSinceDate:lastChunkTime] > 0.2) {
                    break;
                }
            }
        }

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
    // Convert C strings to NSStrings on the calling thread before dispatching.
    NSString *nsText     = [NSString stringWithUTF8String:text];
    NSString *nsLang     = voice_lang ? [NSString stringWithUTF8String:voice_lang] : nil;
    NSString *nsName     = voice_name ? [NSString stringWithUTF8String:voice_name] : nil;

    __block SynthResult result = {NULL, 0, 0, 0};
    dispatch_semaphore_t done = dispatch_semaphore_create(0);

    dispatch_async(synthQueue(), ^{
        result = do_synthesize(nsText, nsLang, nsName, rate, pitch, volume);
        dispatch_semaphore_signal(done);
    });

    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
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
