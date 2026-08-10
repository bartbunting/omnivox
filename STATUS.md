# Omnivox Project Status

**Last Updated:** 2026-08-09
**Version:** 1.3.0

## Current State

### Working

- Full Emacspeak protocol parser plus bounded Emacsvox extensions
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
- Bounded audio-resource loading (16 MiB/30 seconds) and decoded LRU caching
- Text chunking (~15 word chunks for single-buffer utterances)
- Punctuation replacement (none/some/all levels)
- Split caps (insert spaces before capitals)
- Letter speaking with anchored capitalization tone or pitch-raise fallback
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
- Ordinary unnamed speech uses the same engine circuit and same-chunk fallback
- Helper cancellation watchdog terminates native calls that remain stuck after stop
- Persistent engine failure circuits with bounded cooldowns and single recovery probes
- Runtime health and recovery transitions projected into structured inventory
- Independent generation-safe preferred, fallback, and disabled engine policy
- Structured circuit, last-failure, cooldown, and policy-disable diagnostics
- Explicit recovery-probe arming for failed in-process and helper engines
- Generic versioned helper-process engine with bounded framing, negotiation,
  cancellation, timeout handling, and restart/reconnect recovery probes
- Optional buffered Eloquence and DECtalk engines through separate 32-bit helpers
- Eloquence ECI index callbacks mapped to bounded word markers and UTF-8 ranges
- DECtalk private indexes mapped to bounded word markers and UTF-8 ranges
- DECtalk native phoneme changes and caller indexes mapped to audio-frame markers
- Bounded requested synthesis anchors with exact, word-boundary, span-boundary,
  and omitted resolution grades
- Helper protocol v2 exact Eloquence anchors and v3 extended ACSS with automatic
  fallback through older protocol versions
- Anchor frames retained through canonical resampling and silence trimming
- Speech-overlaid 440 Hz capital and 1300 Hz all-caps tones resolved through
  requested anchors with word-boundary and chunk-boundary degradation
- Per-engine degradation for all six logical ACSS dimensions
- Eloquence pitch-range, stress, and richness mapped to ECI `vf`, `vr`, and
  paired `vy`/`vv` controls
- DECtalk pitch-range, stress, and richness mapped to native `pr/as`,
  `hr/sr/qu/bf`, and `ri/sm` controls
- Capability-gated signed per-voice rate offsets relative to the live global
  zero-to-100 rate, shared by timelines and exact previews
- Canonical PCM conversion before helper audio enters the effects/playback pipeline
- Structured synthesis requests and results with realized engine/voice metadata
- WinRT word and sentence boundaries mapped to playback-synchronized markers
- Validated helper markers retained through canonical sample-rate conversion
- Silence-trim reports keep synthesis marker offsets aligned with retained audio
- Tracked playback sources with ordered, cancellation-safe frame cue delivery
- Capability-gated marker dispatch with versioned, bounded playback events
- Emacsvox marker-event negotiation, bounded decoding, and callback dispatch
- Hard-stop cancellation requests fan out across all registered engines
- Bounded, capability-advertised `emacsvox_tx` presentation framing with atomic validation
- Generation coalescing, stale-frame rejection, and stop-barrier semantics
- Capability-advertised tracked dispatch with completed, cancelled, and failed terminal results
- Playback acknowledgements covering queued speech, tones, silence, and audio icons
- Pure engine-neutral presentation timeline vocabulary and scheduler with
  bounded IDs, source/span positions, stable action order, insert/overlay
  projection, persistent effect-state snapshots, checked frame mapping, and
  cancellation projections
- Queue-boundary auditory-icon overlays with same-boundary mixing, complete-tail
  tracked playback, and stop-safe deferred scheduling
- Bounded timeline PCM rendering with serial insertion, sample-aligned overlay,
  cross-chunk tails, and insertion-aware marker/anchor remapping
- Playback-bound semantic action cues with bounded opaque IDs, stable marker
  ordering, cancellation of unreached events, and a versioned v2 wire record
- Non-mutating exact/portable voice preview with playback completion, realized
  route metadata, and ACSS degradation reporting
- English-US eSpeak voice selected as the portable engine default when available
- Diagnostic self-test (--check)
- GitHub Actions CI/CD for 6 platforms

### Not Yet Implemented

- Logical routing for immediate `tts_say` and letter commands
- Structured timeline transport, semantic actions, and timeline-aware effects
- Linux Speech Dispatcher TTS backend
- Network mode (-p TCP flag)
- Multi-device audio routing
- Sox-style effects (reverb, echo, chorus)
- Language switching tables

See [NEXT_STEPS.md](NEXT_STEPS.md) for the ordered roadmap, degradation
contract, acceptance criteria, and additional backlog items.

## Test Results

```
Total: 326 tests, all passing

omnivox-audio:  76 unit + 31 integration = 107
omnivox-core:   48 unit + 1 doc = 49
omnivox-tts:   111 unit
omnivox-cli:    59 unit
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
| `l {letter}` | Working | Anchored cap tone when enabled; pitch raise otherwise |
| `t {freq} {dur}` | Working | Independent tone generation |
| `emacsvox_tone 1 {mode} {freq} {dur}` | Working | Capability-gated insert/overlay tone on the presentation clock |
| `a {path}` | Working | Queue-boundary overlay; complete tail is tracked |
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
| `omnivox_control` | Working | Capabilities, inventory, logical voices, runtime engine policy, recovery probes, and one-shot preview |
| `emacsvox_tx` | Working | Bounded replaceable presentation transaction after capability negotiation |
| `emacsvox_tracked_dispatch` | Working | One terminal result after queued playback completes, cancels, or fails |
| `emacsvox_marker_dispatch` | Working | Versioned marker events plus the tracked terminal result |

## Next Priority

1. Add persistent post-synthesis effects.
2. Add structured Emacsvox timeline transport and degradation reporting.
3. Make language commands functional and include language in synchronized state.

The complete plan is maintained in [NEXT_STEPS.md](NEXT_STEPS.md).
