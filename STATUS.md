# Omnivox Project Status

**Last reviewed:** 2026-08-16
**Workspace version:** 1.3.0

This file records present behavior and limitations. Protocol guarantees belong
in the linked protocol specifications; future work belongs in
[NEXT_STEPS.md](NEXT_STEPS.md).

## Implemented

### Protocol and admission

- Legacy Emacspeak command parsing and queue/state handling.
- A 512 KiB line limit, a bounded 32-line stdin handoff, atomic legacy
  transaction limits, and a bounded nonblocking synthesis queue.
- Versioned Base64-JSON control negotiation, inventory, logical-voice
  registration, runtime routing policy, recovery probes, and non-mutating
  preview.
- Tracked terminal status and marker protocols v1 and v2.
- Structured presentation timelines v1 through v3, including v3 multipart
  framing, complete validation before admission, and terminal status for
  decodable invalid or stale submissions.

### Routing and synthesis

- macOS AVSpeechSynthesizer, Windows WinRT, and eSpeak NG.
- Optional out-of-process Piper, Eloquence, and DECtalk engines.
- Structured engine/voice inventory and deterministic per-span logical routing.
- Ordered fallback for missing voices, unsupported text repertoires, engine
  failure, and transient engine pressure.
- Persistent health circuits, bounded cooldowns, one recovery probe, and
  generation-stamped inventory updates.
- Immediate speech and letter commands follow the current global engine policy;
  they do not select a named logical voice.
- Requested synthesis anchors and engine markers, with truthful exact,
  word-boundary, span-boundary, or omitted resolution.
- Capitalization presentation for timelines and isolated letters.

### Replacement and cancellation

- Ordered and urgent timelines bypass the reader's coalescing delay and are
  never coalesced or evicted.
- Replaceable timelines coalesce only within the same protocol-version and
  replacement-key domain.
- Receipt of a valid newer keyed timeline immediately cancels synthesis and
  tagged playback in that same domain; only its synthesis admission waits for
  the bounded reader coalescing window.
- Domain cancellation does not clear ordered, urgent, legacy, or unrelated
  keyed playback. Active speech fades for three milliseconds to avoid a click.
- Hard stop remains stream-wide, invalidates older generations, requests stop
  from every registered engine, and cancels queued and buffered work.
- Uncancellable native calls are quarantined; helper processes can be killed
  after the cancellation grace period where the helper contract permits it.

### Presentation and audio

- Canonical stereo 44.1 kHz PCM conversion and sinc resampling.
- Silence trimming with marker/anchor remapping, volume adjustment, and channel
  routing.
- Independent speech, tone, and sound streams with bounded queues.
- Bounded OGG/WAV resource loading, decoded LRU caching, and immutable shared
  timeline PCM.
- Inserted and overlaid timeline audio/tone actions, inserted silence,
  semantic events, stable cue order, and tracked overlay tails.
- Persistent post-synthesis gain, filtering, pan, reverb, and echo state.
- Privacy-safe persistent logs and optional sensitive full-text diagnostics.

## Current limitations

- Linux has no Speech Dispatcher backend; eSpeak NG is the current built-in
  Linux engine. [SPEECHD-PLAN.md](SPEECHD-PLAN.md) is a proposal only.
- There is no TCP/network server mode and no authenticated remote protocol.
- Audio routing selects left, right, or both channels within one output device;
  arbitrary multi-device routing is not implemented.
- Immediate `tts_say` and letter commands use the global engine order rather
  than a named logical voice.
- Native cancellation strength differs by engine. WinRT work may continue in a
  quarantined task after its stale output has been suppressed.
- Marker precision differs by engine. Markerless engines retain speech and
  boundary-level presentation but cannot claim exact in-span action timing.
- Piper packaging, model distribution, and broad real-platform latency testing
  are not release-complete.
- The common effects set does not include a chorus effect.
- Logical-language routing is implemented, but live multilingual coverage is
  not comprehensive across all backends.

## Platform and CI coverage

| Platform | Runtime status | GitHub release artifact |
|---|---|---|
| macOS ARM64 | AVSpeechSynthesizer and eSpeak NG | Yes |
| macOS x64 | AVSpeechSynthesizer and eSpeak NG | Yes |
| Windows x64 | WinRT and eSpeak NG; optional helpers | Yes |
| Windows ARM64 | WinRT and eSpeak NG | Yes |
| Linux x64/ARM64 | eSpeak NG source builds | No current workflow artifact |

The checked-in workflow formats on Linux, builds four macOS/Windows targets,
and tests macOS ARM64 plus Windows x64 and ARM64. Linux remains usable from a
source build but is not currently a release artifact or runtime test job.

## Validation

The supported local gates are:

```sh
make fmt-check
cargo test --locked --workspace
make lint
```

Real helper, cancellation, and audio-device behavior also require the relevant
platform runtimes. A passing unit suite is not evidence of acceptable audible
onset latency; measurement methodology and remaining performance work are
tracked in [NEXT_STEPS.md](NEXT_STEPS.md).
