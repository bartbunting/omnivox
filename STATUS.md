# Omnivox Project Status

**Last Updated:** 2026-08-06
**Version:** 1.3.0

## Current State

### Working

- Full Emacspeak protocol parser (27 commands)
- Command queue system with dispatch (depth: speech 100, tone 10, sound 10)
- State management (voice, rate, pitch, volume, punctuation, split caps)
- macOS native TTS (AVSpeechSynthesizer via ObjC bridge, buffer capture)
- Windows native TTS (WinRT SpeechSynthesizer via windows-rs)
- espeak-ng TTS (always compiled in, cross-platform fallback)
- Optional Piper neural TTS backend
- Audio pipeline with extensible effects (SilenceTrimmer, VolumeAdjust, ChannelRouter)
- Rubato sinc resampler (256-tap BlackmanHarris2, replaces linear interpolation)
- Tone generation (pure Rust sine wave, fade envelopes, stereo spatial)
- Audio icon playback (OGG/WAV via rodio, LRU cache, tilde expansion)
- Text chunking (~15 word chunks for single-buffer utterances)
- Punctuation replacement (none/some/all levels)
- Split caps (insert spaces before capitals)
- Letter speaking with pitch raise for capitals
- Voice switching (intra-sentence via queue)
- Stop with persistent singleton synthesizer
- Engine selection via OMNIVOX_ENGINE env var or --engine flag
- Concurrent audio streams (speech/tones/sounds overlap, serialized within each stream)
- Channel routing (left/right/both) for dual-server notification mode
- Per-stream volume control (voice, tone, sound)
- CLI flags for all settings (--voice, --rate, --pitch, --voice-volume, etc.)
- Self-registering Emacs voice module (elisp/omnivox-voices.el)
- Emacs defcustoms with live protocol command sending
- Voice querying from server (--list-voices, --list-voices-alist)
- Structured engine, capability, physical voice, and logical voice contracts
- Pure late-binding voice resolver with ordered fallbacks and diagnostics
- Versioned, bounded Base64-JSON control channel with capability negotiation
- Diagnostic self-test (--check)
- GitHub Actions CI/CD for 6 platforms

### Not Yet Implemented

- Simultaneous engine registry and per-voice engine selection
- Runtime registration and routing of logical voices (the model and resolver exist)
- Full Emacsvox framing and tracked-dispatch extensions
- Eloquence and DECtalk engines through the planned x86 helper
- Linux Speech Dispatcher TTS backend
- Network mode (-p TCP flag)
- Multi-device audio routing
- Sox-style effects (reverb, echo, chorus)
- Language switching tables

See [NEXT_STEPS.md](NEXT_STEPS.md) for the ordered roadmap, degradation
contract, acceptance criteria, and additional backlog items.

## Test Results

```
Total: 192 tests, all passing

omnivox-audio:  60 unit + 31 integration = 91
omnivox-core:   39 unit + 1 doc = 40
omnivox-tts:    40 unit
omnivox-cli:    21 unit
```

## Platform Support

| Platform | Native TTS | espeak-ng Fallback | Status |
|----------|-----------|-------------------|--------|
| macOS | AVSpeechSynthesizer | Yes | Working |
| Linux | Speech Dispatcher (planned) | Yes | espeak-ng works |
| Windows | WinRT SpeechSynthesizer | Yes | Working |

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
| `tts_say {text}` | Working | Immediate speech (with chunking) |
| `tts_set_speech_rate` | Working | Rate control |
| `tts_set_voice` | Working | Voice selection |
| `tts_set_pitch_multiplier` | Working | Pitch control |
| `tts_set_*_volume` | Working | Volume per stream type |
| `tts_set_punctuations` | Working | none/some/all |
| `tts_split_caps` | Working | camelCase spacing |
| `tts_sync_state` | Working | Atomic state update |
| `tts_reset` | Working | Reset defaults |
| `version` | Working | Version announcement |
| `tts_exit` | Working | Clean exit |
| `omnivox_control` | Working | Versioned capability request; further message types planned |

## Next Priority

1. Populate descriptors from the existing WinRT and eSpeak engines.
2. Add structured engine inventory responses to the control channel.
3. Introduce logical-voice registration, then the engine registry and
   per-speech-span engine/voice routing.

The complete plan is maintained in [NEXT_STEPS.md](NEXT_STEPS.md).
