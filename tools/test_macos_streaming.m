// Native lifecycle regressions, without speech services or an audio device.
#include "../omnivox-tts/src/macos_bridge.m"
#include <assert.h>
#include <stdio.h>

static AVAudioPCMBuffer *pcm(uint32_t frames, uint16_t channels, BOOL interleaved) {
    AVAudioFormat *format = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
        sampleRate:44100 channels:channels interleaved:interleaved];
    AVAudioPCMBuffer *buffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:format frameCapacity:frames];
    buffer.frameLength = frames;
    for (uint32_t frame = 0; frame < frames; frame++) {
        for (uint16_t channel = 0; channel < channels; channel++) {
            float value = (float)(frame % 99 + channel) / 100;
            if (interleaved) buffer.floatChannelData[0][frame * channels + channel] = value;
            else buffer.floatChannelData[channel][frame] = value;
        }
    }
    return buffer;
}

static int next(OmnivoxCapture *capture, float *samples, uint32_t *count) {
    uint32_t rate = 0, reason = 0;
    uint16_t channels = 0;
    return omnivox_stream_next((__bridge void *)capture, samples, count, &rate, &channels, &reason);
}

static void backpressure_cancel(void) {
    OmnivoxCapture *capture = [[OmnivoxCapture alloc] init];
    dispatch_group_t producer = dispatch_group_create();
    dispatch_group_async(producer, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        [capture consume:pcm(10000, 2, NO)];
    });
    uint64_t deadline = clock_gettime_nsec_np(CLOCK_UPTIME_RAW) + 2000000000ULL;
    for (;;) {
        [capture->condition lock];
        BOOL full = capture->count == StreamWindows;
        [capture->condition unlock];
        if (full) break;
        assert(clock_gettime_nsec_np(CLOCK_UPTIME_RAW) < deadline);
        [NSThread sleepForTimeInterval:0.001];
    }
    assert(dispatch_group_wait(producer, DISPATCH_TIME_NOW) != 0);
    omnivox_stream_cancel((__bridge void *)capture);
    assert(dispatch_group_wait(producer, dispatch_time(DISPATCH_TIME_NOW, NSEC_PER_SEC)) == 0);
    // Both queued PCM and a callback after retirement must remain invisible.
    [capture consume:pcm(100, 1, NO)];
    float samples[WindowSamples]; uint32_t count = 0;
    assert(next(capture, samples, &count) == -1);
    assert(count == 0);
}

static void gap_is_not_completion(void) {
    OmnivoxCapture *capture = [[OmnivoxCapture alloc] init];
    float samples[WindowSamples]; uint32_t count = 0;
    [capture consume:pcm(100, 1, NO)];
    assert(next(capture, samples, &count) == 1 && count == 100);
    [NSThread sleepForTimeInterval:0.3];
    assert(next(capture, samples, &count) == 0);
    [capture consume:pcm(200, 1, NO)];
    assert(next(capture, samples, &count) == 1 && count == 200);
    [capture finish:StreamFinished];
    assert(next(capture, samples, &count) == 2);
    [capture consume:pcm(100, 1, NO)];
    assert(next(capture, samples, &count) == 2);
}

static void stereo_layouts(void) {
    for (unsigned interleaved = 0; interleaved < 2; interleaved++) {
        OmnivoxCapture *capture = [[OmnivoxCapture alloc] init];
        [capture consume:pcm(700, 2, interleaved)];
        [capture finish:StreamFinished];
        float samples[WindowSamples]; uint32_t count = 0, frame = 0;
        int status;
        while ((status = next(capture, samples, &count)) == 1) {
            assert(count <= WindowSamples);
            for (uint32_t sample = 0; sample < count; sample += 2, frame++) {
                assert(samples[sample] == (float)(frame % 99) / 100);
                assert(samples[sample + 1] == (float)(frame % 99 + 1) / 100);
            }
        }
        assert(status == 2 && frame == 700);
    }
}

static void format_failure_discards_pending_audio(void) {
    OmnivoxCapture *capture = [[OmnivoxCapture alloc] init];
    [capture consume:pcm(100, 1, NO)];
    [capture consume:pcm(100, 2, NO)];
    float samples[WindowSamples]; uint32_t count = 0;
    assert(next(capture, samples, &count) == -1);
    assert(count == 0 && capture->reason == StreamInvalidPcm);
}

static void stale_delegate_cannot_finish_another_utterance(void) {
    OmnivoxCapture *capture = [[OmnivoxCapture alloc] init];
    capture->expectedUtterance = [AVSpeechUtterance speechUtteranceWithString:@"current"];
    AVSpeechUtterance *old = [AVSpeechUtterance speechUtteranceWithString:@"old"];
    [capture speechSynthesizer:nil didFinishSpeechUtterance:old];
    [capture speechSynthesizer:nil didCancelSpeechUtterance:old];
    assert(!capture->finished);
    [capture speechSynthesizer:nil didFinishSpeechUtterance:capture->expectedUtterance];
    assert(capture->finished && capture->reason == StreamDelegateFinished);
}

int main(void) {
    @autoreleasepool {
        backpressure_cancel();
        gap_is_not_completion();
        stereo_layouts();
        format_failure_discards_pending_audio();
        stale_delegate_cannot_finish_another_utterance();
        puts("PASS five native queue/callback lifecycle regressions");
    }
}
