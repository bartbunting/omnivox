# Omnivox Project Status

**Last Updated:** 2026-01-09
**Version:** 0.1.0
**Status:** 🟢 Working on macOS

## Quick Start

```bash
# Build
make build

# Test basic speech
echo "tts_say {Hello world}" | ./target/release/omnivox

# List available voices
./target/release/list-voices | head -50

# Run comprehensive tests
./test-all-features.sh
```

## Current State

### ✅ Fully Working

**Core Infrastructure:**
- Emacspeak protocol parser (27 commands)
- Command queue system
- State management
- Logging infrastructure

**macOS TTS:**
- AVSpeechSynthesizer integration
- 216 voices available
- Voice switching (intra-sentence)
- Rate/pitch/volume control
- Split caps
- Letter speaking with pitch raise
- Stop/reset functionality

**Commands Working:**
- `q {text}` - Queue speech
- `c [{voice ...}]` - Voice switching
- `d` - Dispatch
- `s` - Stop
- `l {letter}` - Speak letter
- `tts_say {text}` - Immediate speech
- `tts_set_voice` - Change voice
- `tts_set_speech_rate` - Change rate
- `tts_set_pitch_multiplier` - Change pitch
- `tts_set_voice_volume` - Change volume
- `tts_split_caps` - Split camelCase
- `tts_reset` - Reset state
- `version` - Speak version
- `tts_exit` - Exit

### ⏳ Partially Implemented

- `t {freq} {dur}` - Tone (queued but not generated)
- `sh {duration}` - Silence (implemented as sleep, works)
- `a {path}` - Audio icon (queued but not played)
- Punctuation levels (state tracked, not applied to text)
- Caps beep (pitch raise works, beep not implemented)

### ❌ Not Yet Implemented

**Phase 3 - Audio Pipeline:**
- PCM buffer capture from TTS
- Silence trimming
- Channel panning
- Effects (reverb, echo, etc.)

**Phase 5-7 - Other Platforms:**
- Linux (Speech Dispatcher)
- Windows (SAPI)
- eSpeak-ng fallback

**Phase 8 - Advanced:**
- Multi-device routing
- Network mode (-p flag)
- Runtime device switching

**Phase 9 - Effects:**
- Sox-style effects
- Reverb/echo/chorus/etc.

## Test Results

```
Total Tests: 38
Passing: 38 (100%)
Failing: 0

omnivox-core: 35 tests ✅
omnivox-cli: 3 tests ✅
```

## Code Metrics

```
Total Lines: ~1,600
  omnivox-core: 740 lines
  omnivox-tts: 386 lines
  omnivox-cli: 453 lines

Binary Size: 2.8MB (release)
Clippy Warnings: 0 (clean)
```

## Platform Support

| Platform | TTS Engine | Status | Quality |
|----------|------------|--------|---------|
| macOS | AVSpeechSynthesizer | ✅ Working | Excellent |
| Linux | Speech Dispatcher | ❌ Not started | - |
| Windows | SAPI | ❌ Not started | - |
| Fallback | eSpeak-ng | ❌ Not started | - |

## Performance

| Metric | Target | Current | Status |
|--------|--------|---------|--------|
| Latency | < 100ms | ~50-100ms | ✅ Excellent |
| Memory | < 50MB | ~20MB | ✅ Excellent |
| CPU (idle) | < 5% | ~0% | ✅ Excellent |
| Voices | 100+ | 216 | ✅ Exceeded |

## Known Issues

1. **Deprecation warnings:** Using deprecated `NSString::UTF8String` method (47 warnings)
   - *Impact:* None (still works)
   - *Fix:* Migrate to objc2-foundation methods

2. **No buffer capture:** TTS speaks directly, not through pipeline
   - *Impact:* Can't apply effects yet
   - *Fix:* Implement AVSpeechSynthesizer write-to-buffer mode (Phase 3)

3. **No tone generation:** Tones are queued but not generated
   - *Impact:* Beeps don't work
   - *Fix:* Implement sine wave generator

4. **No audio file playback:** Audio icons queued but not played
   - *Impact:* No auditory icons
   - *Fix:* Add audio file player

## Roadmap Progress

| Phase | Goal | Status | Completion |
|-------|------|--------|------------|
| 1 | Foundation | ✅ Done | 100% |
| 2 | macOS TTS | ✅ Done | 100% |
| 3 | Audio Pipeline | ⏳ Ready to start | 0% |
| 4 | Full Protocol | ⏳ In progress | 75% |
| 5 | Linux Support | ❌ Not started | 0% |
| 6 | Windows Support | ❌ Not started | 0% |
| 7 | eSpeak Fallback | ❌ Not started | 0% |
| 8 | Advanced Features | ❌ Not started | 0% |

**Overall Progress:** Week 4 of 21 (19% complete, but core features working!)

## Next Steps

See SESSION_02_SUMMARY.md for detailed next steps.

**Priority 1:** Implement audio processing pipeline
**Priority 2:** Add tone generation
**Priority 3:** Add audio icon playback
**Priority 4:** Apply punctuation replacement
