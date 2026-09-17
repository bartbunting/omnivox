// Objective-C bridge for AVSpeechSynthesizer buffer capture.
// Uses a persistent synthesizer so stop() can interrupt ongoing speech.
//
// Threading: AVSpeechSynthesizer.writeUtterance:toBufferCallback: requires a
// thread with a live Cocoa RunLoop. Raw POSIX threads (std::thread in Rust)
// don't qualify. All synthesis is dispatched through a private serial GCD
// queue whose worker thread is a proper Cocoa-managed thread with a RunLoop.
// Rust polls a bounded request-owned queue while native synthesis remains active.

#import <AVFoundation/AVFoundation.h>
#import <Foundation/Foundation.h>
#include <time.h>
#include <math.h>
#include <stdatomic.h>
#include <stdbool.h>

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

// No callback borrows Rust memory. The native owner and any in-flight callback
// retain this object; late callbacks hold only a weak reference and see a closed
// capture. Queue entries are bounded independently of Apple's callback sizes.
enum { StreamWindows = 8, WindowFrames = 512, WindowSamples = 1024 };
enum {
    StreamFinished = 1, StreamDelegateFinished = 4, StreamCancelled = 5,
    StreamInvalidPcm = 6, StreamException = 7, StreamVoiceMissing = 8,
    StreamLimit = 9,
};
static const uint64_t StreamIdleNanos = 30ULL * 1000 * 1000 * 1000;

@interface OmnivoxCapture : NSObject <AVSpeechSynthesizerDelegate> {
@public
    NSCondition *condition;
    NSLock *producerLock;
    float windows[StreamWindows][WindowSamples];
    uint32_t counts[StreamWindows];
    unsigned head, count;
    uint32_t sampleRate;
    uint16_t channels;
    uint64_t totalSamples, startedAt, lastProgress;
    BOOL finished, cancelled, retired;
    uint32_t reason;
    SynthTimings timings;
    AVSpeechUtterance *expectedUtterance;
}
- (void)finish:(uint32_t)completion;
- (void)consume:(AVAudioBuffer *)buffer;
@end

@implementation OmnivoxCapture
- (instancetype)init {
    if ((self = [super init])) {
        condition = [[NSCondition alloc] init];
        producerLock = [[NSLock alloc] init];
        startedAt = lastProgress = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
        timings.first_buffer_us = timings.last_buffer_us = UINT64_MAX;
        timings.completion_signal_us = UINT64_MAX;
    }
    return self;
}
- (void)finish:(uint32_t)completion {
    [condition lock];
    if (!finished) {
        finished = YES;
        reason = completion;
        timings.completion_reason = completion;
        timings.completion_signal_us = elapsedMicroseconds(startedAt);
        if (completion != StreamFinished && completion != StreamDelegateFinished) count = 0;
    }
    [condition broadcast];
    [condition unlock];
}
- (void)speechSynthesizer:(AVSpeechSynthesizer *)synth didFinishSpeechUtterance:(AVSpeechUtterance *)utterance {
    if (utterance != expectedUtterance) return;
    // Serialize completion with a callback currently publishing its last window.
    [producerLock lock];
    [self finish:StreamDelegateFinished];
    [producerLock unlock];
}
- (void)speechSynthesizer:(AVSpeechSynthesizer *)synth didCancelSpeechUtterance:(AVSpeechUtterance *)utterance {
    if (utterance == expectedUtterance) [self finish:StreamCancelled];
}
- (void)consume:(AVAudioBuffer *)buffer {
    [producerLock lock];
    @try {
        if (![buffer isKindOfClass:[AVAudioPCMBuffer class]]) {
            [self finish:StreamInvalidPcm];
            return;
        }
        AVAudioPCMBuffer *pcm = (AVAudioPCMBuffer *)buffer;
        if (pcm.frameLength == 0) {
            [self finish:StreamFinished];
            return;
        }
        double rate = pcm.format.sampleRate;
        uint32_t channelCount = pcm.format.channelCount;
        float * const *data = pcm.floatChannelData;
        if (!isfinite(rate) || rate < 1 || rate > 384000 || floor(rate) != rate ||
            channelCount < 1 || channelCount > 2 || data == NULL) {
            [self finish:StreamInvalidPcm];
            return;
        }
        [condition lock];
        if (finished || cancelled) { [condition unlock]; return; }
        if (sampleRate != 0 && (sampleRate != (uint32_t)rate || channels != channelCount)) {
            [condition unlock];
            [self finish:StreamInvalidPcm];
            return;
        }
        sampleRate = (uint32_t)rate;
        channels = (uint16_t)channelCount;
        uint64_t samples = (uint64_t)pcm.frameLength * channels;
        if (samples > (128ULL * 1024 * 1024 / sizeof(float)) - totalSamples) {
            [condition unlock];
            [self finish:StreamLimit];
            return;
        }
        totalSamples += samples;
        uint64_t arrived = elapsedMicroseconds(startedAt);
        if (timings.first_buffer_us == UINT64_MAX) timings.first_buffer_us = arrived;
        timings.last_buffer_us = arrived;
        timings.buffers_received++;
        [condition unlock];

        for (uint32_t first = 0; first < pcm.frameLength;) {
            [condition lock];
            while (count == StreamWindows && !finished && !cancelled) {
                if (clock_gettime_nsec_np(CLOCK_UPTIME_RAW) - lastProgress > StreamIdleNanos) break;
                @autoreleasepool {
                    [condition waitUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
                }
            }
            if (finished || cancelled) { [condition unlock]; return; }
            if (count == StreamWindows) {
                [condition unlock];
                [self finish:SynthCompletionDeadline];
                return;
            }
            uint32_t frames = MIN(WindowFrames, pcm.frameLength - first);
            unsigned tail = (head + count) % StreamWindows;
            for (uint32_t frame = 0; frame < frames; frame++) {
                for (uint16_t ch = 0; ch < channels; ch++) {
                    windows[tail][frame * channels + ch] = pcm.format.isInterleaved
                        ? data[0][(first + frame) * channels + ch] : data[ch][first + frame];
                }
            }
            counts[tail] = frames * channels;
            count++;
            lastProgress = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
            [condition broadcast];
            [condition unlock];
            first += frames;
        }
    } @catch (NSException *exception) {
        [self finish:StreamException];
    } @finally {
        [producerLock unlock];
    }
}
@end

void *omnivox_stream_open(const char *text, const char *voice_lang,
                          const char *voice_name, const char *voice_identifier,
                          float rate, float pitch, float volume) {
    @autoreleasepool {
        OmnivoxCapture *capture = [[OmnivoxCapture alloc] init];
        NSString *nsText = [NSString stringWithUTF8String:text];
        NSString *lang = voice_lang ? [NSString stringWithUTF8String:voice_lang] : nil;
        NSString *name = voice_name ? [NSString stringWithUTF8String:voice_name] : nil;
        NSString *identifier = voice_identifier ? [NSString stringWithUTF8String:voice_identifier] : nil;
        dispatch_async(synthQueue(), ^{
            @autoreleasepool {
                AVSpeechSynthesizer *synth = nil;
                @try {
                    [capture->condition lock];
                    BOOL cancelled = capture->cancelled;
                    capture->timings.queue_wait_us = elapsedMicroseconds(capture->startedAt);
                    [capture->condition unlock];
                    if (!cancelled) {
                        synth = sharedSynthesizer();
                        AVSpeechUtterance *utterance = [AVSpeechUtterance speechUtteranceWithString:nsText];
                        AVSpeechSynthesisVoice *voice = identifier
                            ? [AVSpeechSynthesisVoice voiceWithIdentifier:identifier] : cachedVoice(lang, name);
                        if ((voice == nil && (identifier != nil || lang != nil)) ||
                            (identifier != nil && ![voice.identifier isEqualToString:identifier])) {
                            [capture finish:StreamVoiceMissing];
                        } else {
                            utterance.voice = voice;
                            utterance.rate = rate;
                            utterance.pitchMultiplier = pitch;
                            utterance.volume = volume;
                            capture->expectedUtterance = utterance;
                            synth.delegate = capture;
                            [capture->condition lock];
                            capture->timings.write_started_us = elapsedMicroseconds(capture->startedAt);
                            capture->lastProgress = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
                            [capture->condition unlock];
                            __weak OmnivoxCapture *weakCapture = capture;
                            [synth writeUtterance:utterance toBufferCallback:^(AVAudioBuffer *buffer) {
                                OmnivoxCapture *active = weakCapture;
                                if (active != nil) [active consume:buffer];
                            }];
                            for (;;) {
                                [capture->condition lock];
                                BOOL done = capture->finished || capture->cancelled;
                                BOOL timeout = clock_gettime_nsec_np(CLOCK_UPTIME_RAW) - capture->lastProgress > StreamIdleNanos;
                                [capture->condition unlock];
                                if (done) break;
                                if (timeout) { [capture finish:SynthCompletionDeadline]; break; }
                                @autoreleasepool {
                                    [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                        beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.005]];
                                }
                            }
                        }
                    }
                } @catch (NSException *exception) {
                    [capture finish:StreamException];
                } @finally {
                    // Closing this request precedes native retirement. The next
                    // request cannot reuse the synthesizer until Rust observes it.
                    [capture->condition lock];
                    BOOL succeeded = capture->finished && !capture->cancelled &&
                        (capture->reason == StreamFinished || capture->reason == StreamDelegateFinished);
                    [capture->condition unlock];
                    if (!succeeded) [capture finish:StreamCancelled];
                    @try {
                        if (synth != nil) {
                            if (!succeeded) [synth stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
                            synth.delegate = nil;
                        }
                    } @catch (NSException *exception) {
                        // Do not acknowledge retirement after a native stop failure.
                        [capture finish:StreamException];
                        return;
                    }
                    [capture->condition lock];
                    capture->timings.capture_completed_us = elapsedMicroseconds(capture->startedAt);
                    capture->retired = YES;
                    [capture->condition broadcast];
                    [capture->condition unlock];
                }
            }
        });
        return (__bridge_retained void *)capture;
    }
}

// 1: audio, 0: pending, 2: explicit native completion, -1: failure.
int omnivox_stream_next(void *handle, float *samples, uint32_t *sample_count,
                       uint32_t *sample_rate, uint16_t *channels, uint32_t *reason) {
    @autoreleasepool {
        OmnivoxCapture *capture = (__bridge OmnivoxCapture *)handle;
        [capture->condition lock];
        if (capture->count == 0 && !capture->finished && !capture->cancelled)
            [capture->condition waitUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
        int status = 0;
        *reason = capture->reason;
        if (capture->cancelled) status = -1;
        else if (capture->count > 0) {
            *sample_count = capture->counts[capture->head];
            *sample_rate = capture->sampleRate;
            *channels = capture->channels;
            memcpy(samples, capture->windows[capture->head], *sample_count * sizeof(float));
            capture->head = (capture->head + 1) % StreamWindows;
            capture->count--;
            capture->lastProgress = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
            [capture->condition broadcast];
            status = 1;
        } else if (capture->finished) {
            status = (capture->reason == StreamFinished || capture->reason == StreamDelegateFinished) ? 2 : -1;
        }
        [capture->condition unlock];
        return status;
    }
}

void omnivox_stream_cancel(void *handle) {
    OmnivoxCapture *capture = (__bridge OmnivoxCapture *)handle;
    [capture->condition lock];
    capture->cancelled = YES;
    capture->count = 0;
    [capture->condition broadcast];
    [capture->condition unlock];
}
int omnivox_stream_retired(void *handle) {
    OmnivoxCapture *capture = (__bridge OmnivoxCapture *)handle;
    [capture->condition lock];
    int retired = capture->retired;
    [capture->condition unlock];
    return retired;
}
SynthTimings omnivox_stream_timings(void *handle) {
    OmnivoxCapture *capture = (__bridge OmnivoxCapture *)handle;
    [capture->condition lock];
    SynthTimings timings = capture->timings;
    timings.bridge_elapsed_us = elapsedMicroseconds(capture->startedAt);
    [capture->condition unlock];
    return timings;
}
void omnivox_stream_release(void *handle) {
    // In-flight native work retains its own owner, never a pointer into Rust.
    (void)CFBridgingRelease(handle);
}

// Run the main NSRunLoop until omnivox_stop_main_runloop() is called.
// AVSpeechSynthesizer.writeUtterance:toBufferCallback: internally dispatches
// work via the main queue; the main thread must be running its RunLoop for
// those dispatches to be processed. Call this from main() after spawning the
// reader thread, so synthesis (on the worker thread) doesn't deadlock.
static atomic_bool _runloopShouldStop = false;

void omnivox_run_main_runloop(void) {
    while (!atomic_load_explicit(&_runloopShouldStop, memory_order_acquire)) {
        @autoreleasepool {
            [[NSRunLoop mainRunLoop] runMode:NSDefaultRunLoopMode
                                 beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        }
    }
}

void omnivox_stop_main_runloop(void) {
    atomic_store_explicit(&_runloopShouldStop, true, memory_order_release);
    CFRunLoopStop(CFRunLoopGetMain());
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
