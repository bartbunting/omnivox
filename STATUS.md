# Omnivox Project Status

**Last Updated:** 2026-08-06
**Version:** 1.3.0

## Current State

### Working

- Full Emacspeak protocol parser (32 commands)
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
- Mandatory self-description for built-in engines and structured active-engine inventory
- Atomic, generation-safe logical-voice registration with resolution diagnostics
- Emacsvox capability negotiation, inventory discovery, and portable voice registration
- Deterministic engine registry backing server inventory and logical resolution
- Per-span queued logical-voice routing across registered engines
- Safe preferred-engine fallback for missing or unresolved logical routes
- Batch-local runtime voice/engine failure exclusion and deterministic re-resolution
- Same-chunk routed synthesis retry with a four-attempt cap and stop checks
- Persistent engine failure circuits with bounded cooldowns and single recovery probes
- Runtime health and recovery transitions projected into structured inventory
- Generic versioned helper-process engine with bounded framing, negotiation,
  cancellation, timeout handling, and restart/reconnect recovery probes
- Optional buffered Eloquence and DECtalk engines through separate 32-bit helpers
- Per-engine rate, average-pitch, and volume degradation for logical ACSS routes
- Canonical PCM conversion before helper audio enters the effects/playback pipeline
- Structured synthesis requests and results with realized engine/voice metadata
- Validated helper markers retained through canonical sample-rate conversion
- Hard-stop cancellation requests fan out across all registered engines
- Bounded, capability-advertised `emacsvox_tx` presentation framing with atomic validation
- Generation coalescing, stale-frame rejection, and stop-barrier semantics
- Capability-advertised tracked dispatch with completed, cancelled, and failed terminal results
- Playback acknowledgements covering queued speech, tones, silence, and audio icons
- English-US eSpeak voice selected as the portable engine default when available
- Diagnostic self-test (--check)
- GitHub Actions CI/CD for 6 platforms

### Not Yet Implemented

- Logical routing for immediate `tts_say` and letter commands
- Marker adjustment through silence trimming and downstream audio effects
- Eloquence and DECtalk native pitch-range, stress, richness, and marker support
- Linux Speech Dispatcher TTS backend
- Network mode (-p TCP flag)
- Multi-device audio routing
- Sox-style effects (reverb, echo, chorus)
- Language switching tables

See [NEXT_STEPS.md](NEXT_STEPS.md) for the ordered roadmap, degradation
contract, acceptance criteria, and additional backlog items.

## Test Results

```
Total: 276 tests, all passing

omnivox-audio:  63 unit + 31 integration = 94
omnivox-core:   41 unit + 1 doc = 42
omnivox-tts:    90 unit
omnivox-cli:    50 unit
```

## Platform Support

| Platform | Native TTS | espeak-ng Fallback | Status |
|----------|-----------|-------------------|--------|
| macOS | AVSpeechSynthesizer | Yes | Working |
| Linux | Speech Dispatcher (planned) | Yes | espeak-ng works |
| Windows | WinRT plus optional Eloquence/DECtalk helpers | Yes | Working |

## Commands Working

| Command | Status | Notes |
|---------|--------|-------|
| `q {text}` | Working | Queue speech |
| `c [{voice ...}]` | Working | Legacy physical voice switching |
| `c {[[logical_voice ID]]}` | Working | Registered engine/voice routing for queued spans |
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
| `omnivox_control` | Working | Capabilities, inventory, and logical-voice registration |
| `emacsvox_tx` | Working | Bounded replaceable presentation transaction after capability negotiation |
| `emacsvox_tracked_dispatch` | Working | One terminal result after queued playback completes, cancels, or fails |

## Next Priority

1. Preserve marker timestamps through silence trimming and queued playback.
2. Populate WinRT, Eloquence, and DECtalk native markers.
3. Add user-facing realized-route, degradation, and health diagnostics.
4. Make language commands functional and include language in synchronized state.

The complete plan is maintained in [NEXT_STEPS.md](NEXT_STEPS.md).
