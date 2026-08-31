# Omnivox Project Status

**Last reviewed:** 2026-08-31
**Workspace version:** 1.4.1

This file records present behavior and limitations. Protocol guarantees belong
in the linked protocol specifications; future work belongs in
[NEXT_STEPS.md](plans/NEXT_STEPS.md).

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
  framing, bounded schema/cross-reference/action-window validation before
  admission, resource preparation before new-span synthesis, and terminal
  status for decodable invalid or stale submissions.

### Routing and synthesis

- macOS AVSpeechSynthesizer, Windows WinRT, and eSpeak NG.
- Optional out-of-process Piper, Eloquence, and DECtalk engines.
- Structured engine/voice inventory and deterministic per-span logical routing.
- Server registration retains WinRT and eSpeak on Windows,
  AVSpeechSynthesizer and eSpeak on macOS, and eSpeak on Linux. Configured
  Piper helpers join that registry in Piper-enabled builds on every platform.
- Ordered fallback for missing voices, unsupported text repertoires, engine
  failure, and transient engine pressure.
- Persistent health circuits, bounded cooldowns, one recovery probe, and
  generation-stamped inventory updates.
- Immediate speech and letter commands follow the current global engine policy;
  they do not select a named logical voice.
- Requested synthesis anchors and engine markers, with truthful exact,
  word-boundary, span-boundary, or omitted resolution.
- Caller-supplied capitalization cues in timelines and pitch-rise presentation
  for isolated capital letters.

### Replacement and cancellation

- Ordered and urgent timelines bypass the reader's coalescing delay and are
  never coalesced or evicted.
- Replaceable timelines coalesce only within the same protocol-version and
  replacement-key domain.
- Successful worker-queue admission of a newer keyed timeline atomically
  cancels synthesis and tagged playback in that same domain. Failed admission
  leaves the older queued or active work intact.
- Domain cancellation does not clear ordered, urgent, legacy, or unrelated
  keyed playback. Active speech fades for three milliseconds to avoid a click.
- Hard stop remains stream-wide, invalidates older generations, requests stop
  from every registered engine, and cancels queued and buffered work.
- Uncancellable native calls are quarantined; helper processes can be killed
  after the cancellation grace period where the helper contract permits it.

### Presentation and audio

- Canonical stereo 44.1 kHz PCM conversion with bounded sample-rate conversion.
- Silence trimming with marker/anchor remapping, volume adjustment, and channel
  routing.
- Independent speech, tone, and sound streams with bounded queues.
- Bounded OGG/WAV resource loading, a 128-entry/64-MiB decoded LRU cache,
  immutable shared timeline PCM, and a 64-MiB retained-PCM preparation budget
  per presentation.
- Inserted and overlaid timeline audio/tone actions, inserted silence,
  semantic events, stable cue order, and tracked overlay tails.
- Persistent post-synthesis gain, filtering, pan, reverb, and echo state.
- Privacy-conscious persistent logs and optional sensitive full-text
  diagnostics.

## Current limitations

- Linux has no Speech Dispatcher backend; eSpeak NG is the current built-in
  Linux engine. [SPEECHD-PLAN.md](plans/SPEECHD-PLAN.md) is a proposal only.
- There is no TCP/network server mode and no authenticated remote protocol.
- Audio routing selects left, right, or both channels within one output device;
  arbitrary multi-device routing is not implemented.
- Immediate `tts_say` and letter commands use the global engine order rather
  than a named logical voice.
- Native cancellation strength differs by engine. WinRT work may continue in a
  quarantined task after its stale output has been suppressed.
- Marker precision differs by engine. Markerless engines retain speech and
  boundary-level presentation but cannot claim exact in-span action timing.
- Piper uses the maintained vendored libpiper v1.7 C API. Linux x64, Windows
  x64, macOS ARM64, and macOS x64 native runners verify checksum-locked inputs,
  relocated deterministic companion archives, and real synthesis with a
  locked CI-only model. Missing and corrupt model fallback is also verified.
  The companion is not published: corresponding-source review, the test
  model's deferred licensing review, and publishing-workflow integration
  remain open.
- The common effects set does not include a chorus effect.
- Logical-language routing is implemented, but live multilingual coverage is
  not comprehensive across all backends.

## Platform and CI coverage

| Platform | Runtime status | GitHub release artifact |
|---|---|---|
| macOS ARM64 | AVSpeechSynthesizer and eSpeak NG; optional Piper candidate verified | Yes |
| macOS x64 | AVSpeechSynthesizer and eSpeak NG; optional Piper candidate verified | Yes |
| Windows x64 | WinRT and eSpeak NG; optional Piper and proprietary helpers | Yes |
| Windows ARM64 | WinRT and eSpeak NG | Yes |
| Linux x64 | eSpeak NG; optional Piper candidate verified | Yes (Ubuntu 24.04 ABI baseline) |
| Linux ARM64 | Not CI-built or runtime-verified | No current workflow artifact |

The checked-in workflow builds, tests, and runs Clippy on all five release
targets using native runners. Linux x64 is built on Ubuntu 24.04; compatibility
with older glibc distributions is not claimed. Linux ARM64 has no workflow
build, release artifact, or runtime test job, so source-build compatibility is
not currently claimed. All five release archives stage the matching
`espeak-ng-data` beside the executable. CI validates the packaged data and
eSpeak voice discovery on every build runner, with native macOS WAV synthesis
also exercised during the build. Tag builds upload a draft release and then
verify each exact downloaded archive after relocation on its native platform.
The release verifier exercises eSpeak on all five targets and the native speech
engine on both macOS and both Windows targets. The draft is published only when
all checks pass.

The separate manual Piper workflow builds and verifies model-free companion
candidates on Linux x64, Windows x64, and both macOS architectures. Those
workflow artifacts are engineering candidates, not published release assets.

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
tracked in [NEXT_STEPS.md](plans/NEXT_STEPS.md).
