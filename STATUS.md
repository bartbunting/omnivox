# Omnivox Project Status

**Last Updated:** 2026-02-07
**Version:** 0.1.0

## Current State

### Working

- Full Emacspeak protocol parser (27 commands)
- Command queue system with dispatch
- State management (voice, rate, pitch, volume, punctuation, split caps)
- macOS native TTS (AVSpeechSynthesizer via ObjC bridge, buffer capture)
- espeak-ng TTS (always compiled in, cross-platform fallback)
- Audio pipeline with extensible effects (SilenceTrimmer, VolumeAdjust, ChannelRouter)
- Tone generation (pure Rust sine wave, fade envelopes, stereo spatial)
- Audio icon playback (OGG/WAV via rodio, LRU cache, tilde expansion)
- Punctuation replacement (none/some/all levels)
- Split caps (insert spaces before capitals)
- Letter speaking with pitch raise for capitals
- Voice switching (intra-sentence via queue)
- Stop with persistent singleton synthesizer
- Engine selection via OMNIVOX_ENGINE env var
- Concurrent audio streams (speech/tones/sounds overlap, serialized within each stream)
- Backlog depth limits per stream (speech:10, tones:3, sounds:5) with overflow drop

### Not Yet Implemented

- Windows SAPI TTS backend
- Linux Speech Dispatcher TTS backend
- Network mode (-p TCP flag)
- Multi-device audio routing
- Sox-style effects (reverb, echo, chorus)
- Language switching tables
- Caps beep (pitch raise works, beep not implemented)

## Test Results

```
Total: 161 tests, all passing

omnivox-audio:  60 unit + 31 integration = 91
omnivox-core:   34 unit + 1 doc = 35
omnivox-tts:    22 unit
omnivox-cli:    13 unit
```

## Code Metrics

```
Total Lines: ~5,100 Rust + 194 ObjC
  omnivox-cli:    851 lines (main.rs) + 43 (list-voices.rs)
  omnivox-audio:  1,776 lines (buffer, effects, loader, output, pipeline, tone, integration tests)
  omnivox-tts:    1,158 lines (lib, macos, espeak, build.rs) + 194 ObjC bridge
  omnivox-core:   741 lines (command, queue, state, lib)
```

## Platform Support

| Platform | Native TTS | espeak-ng Fallback | Status |
|----------|-----------|-------------------|--------|
| macOS | AVSpeechSynthesizer | Yes | Working |
| Linux | Speech Dispatcher (planned) | Yes | espeak-ng works |
| Windows | SAPI (planned) | Needs testing | Not started |

## Commands Working

| Command | Status | Notes |
|---------|--------|-------|
| `q {text}` | Working | Queue speech |
| `c [{voice ...}]` | Working | Voice switching |
| `d` | Working | Dispatch queue |
| `s` | Working | Stop (persistent synth) |
| `l {letter}` | Working | Pitch raise for caps |
| `t {freq} {dur}` | Working | Tone generation |
| `a {path}` | Working | Audio icon playback |
| `p {path}` | Working | Immediate sound |
| `sh {duration}` | Working | Silence |
| `tts_say {text}` | Working | Immediate speech |
| `tts_set_speech_rate` | Working | Rate control |
| `tts_set_voice` | Working | Voice selection |
| `tts_set_pitch_multiplier` | Working | Pitch control |
| `tts_set_*_volume` | Working | Volume per type |
| `tts_set_punctuations` | Working | none/some/all |
| `tts_split_caps` | Working | camelCase spacing |
| `tts_sync_state` | Working | Atomic state update |
| `tts_reset` | Working | Reset defaults |
| `version` | Working | Version announcement |
| `tts_exit` | Working | Clean exit |

## Next Priority

Windows SAPI TTS backend implementation. See CLAUDE.md for implementation guide.
