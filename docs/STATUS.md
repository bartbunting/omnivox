# Omnivox Project Status

**Last reviewed:** 2026-09-02
**Workspace version:** 1.6.2

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
- Optional out-of-process Piper, RHVoice, Flite, RuTTS, Eloquence, and DECtalk
  engines.
- Structured engine/voice inventory and deterministic per-span logical routing.
- Server registration retains WinRT and eSpeak on Windows,
  AVSpeechSynthesizer and eSpeak on macOS, and eSpeak on Linux. Configured
  Piper helpers join that registry in Piper-enabled builds. Staged or
  explicitly configured RHVoice, Flite, and RuTTS helpers are discovered on
  every desktop platform. Independent helpers initialize concurrently with
  built-in discovery, then join the complete initial inventory in deterministic
  order before the command loop opens.
- Verified content-addressed eSpeak data can reuse a bounded cached voice
  inventory; unverified, custom, stale, or malformed cache state falls back to
  live discovery without changing the complete first-inventory contract.
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
  The exact model revision is approved only for CI acceptance, based on its
  model card's public-domain LibriVox and trained-from-scratch declarations,
  and remains excluded from release artifacts. A deterministic,
  platform-neutral corresponding-source and build-input candidate is
  implemented and passes exhaustive manifest, Git-tree, input, model-exclusion,
  and offline Cargo verification. Piper companion and corresponding-source
  archives are published beginning with v1.6.2 after the gated tag workflow
  verifies the draft assets on their native platforms.
- RHVoice uses a user-installed 1.14-or-later compatible 1.x C API runtime.
  Linux x64 and Windows x64 have passed real synthesis, marker, ACSS,
  cancellation, and shutdown acceptance with 1.14.0; Windows uses an explicit
  C API DLL path. Linux ARM64 has helper compile coverage, Windows ARM64 has no
  accepted compatible runtime, and macOS remains compile-only because upstream
  does not claim macOS support.
- Flite uses checksum-locked v2.2 source, has only `cmu_us_slt` compiled in,
  and accepts optional local English Clustergen `.flitevox` files. Native
  release runners on Linux x64/ARM64, macOS Intel/Apple Silicon, and Windows
  x64/ARM64 each verify relocation, ACSS, 25 real SLT syntheses, cancellation,
  and clean shutdown. Flite has an ASCII input guarantee, native word-start
  markers, and word-boundary requested-anchor resolution, but no sentence,
  phoneme, or exact user-defined markers; eSpeak remains the Unicode fallback.
- RuTTS uses checksum-locked v6.3.3 source and exposes its built-in male and
  female Russian voices without RuLex. Linux x64 local acceptance covers both
  voices, ACSS, bounded PCM, cancellation, relocation, and clean shutdown. The
  complete Windows x64 GNU payload also builds and installs from WSL. Both
  voices pass persistent 25-synthesis helper sessions, repeated cancellation,
  exact full-server routing, mixed queue and hard-stop stress, Windows resource
  sampling, and dispatch-time helper death with eSpeak fallback and explicit
  recovery. The release matrix targets Linux x64/ARM64, macOS Intel/Apple
  Silicon, and Windows x64/ARM64 with MSVC; those release targets are not
  labelled runtime-accepted until their native workflow gates pass. The
  [Windows evidence pack](benchmarks/2026-09-01-windows-x64-rutts-23baa0a64c9cf117.md)
  records the bounded GNU-target results. The helper converts its KOI8-R
  repertoire losslessly and routes unsupported Unicode text to fallback; it
  provides no synchronization markers.
- The common effects set does not include a chorus effect.
- Logical-language routing is implemented, but live multilingual coverage is
  not comprehensive across all backends.
- WinRT, eSpeak NG, Piper, RHVoice, Flite, RuTTS, and DECtalk have measured
  monotonic rate curves anchored to the established Eloquence behavior.
  Individual voices still vary and several engines saturate before Eloquence's
  extended high-rate range. AVSpeechSynthesizer retains its system-native rate
  mapping until the repeatable audit is run on macOS.

## Platform and CI coverage

| Platform | Runtime status | GitHub release artifact |
|---|---|---|
| macOS ARM64 | AVSpeechSynthesizer and eSpeak NG; optional Piper and Flite companions verified | Yes |
| macOS x64 | AVSpeechSynthesizer and eSpeak NG; optional Piper and Flite companions verified | Yes |
| Windows x64 | WinRT and eSpeak NG; RHVoice and Flite accepted; optional Piper and proprietary helpers | Yes |
| Windows ARM64 | WinRT and eSpeak NG; Flite companion verified | Yes |
| Linux x64 | eSpeak NG; RHVoice accepted; optional Piper and Flite companions verified | Yes (Ubuntu 24.04 ABI baseline) |
| Linux ARM64 | Flite companion verified; generic server artifact pending | No current generic workflow artifact |

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

The separate manual Piper workflow still builds model-free engineering
candidates on Linux x64, Windows x64, and both macOS architectures. Tag builds
instead require the corresponding-source artifact and exact draft-asset
verification before publication. Release binaries remain unsigned.

The Flite workflow has native build and release gates for Linux x64/ARM64,
macOS Intel/Apple Silicon, and Windows x64/ARM64. Each gate compiles and lints
the same pinned C/Rust boundary, verifies the relocated archive, performs 25
SLT syntheses, exercises cancellation and shutdown, and uploads no external
voice file. Publication also requires the exact Flite source artifact to pass
its manifest, Git-tree, source-lock, and offline-preparation checks.

RuTTS has deterministic binary and corresponding-source packaging with local
Linux x64 verification. Its checked-in native gates use the same six companion
targets as Flite and require real synthesis with both built-in voices,
cancellation, relocation, provenance, and offline source preparation before a
tag can publish the assets. Those new non-Linux-x64 gates remain pending until
their hosted workflows pass.

## Validation

The supported local gates are:

```sh
make fmt-check
cargo test --locked --workspace
make lint
```

Real helper, cancellation, and audio-device behavior also require the relevant
platform runtimes. A passing unit suite is not evidence of acceptable audible
onset latency. Normal diagnostics now correlate protocol admission, queue
wait, engine synthesis attempts, audio queueing, first mixer-source
consumption, and terminal playback by dispatch ID using monotonic elapsed
microseconds. Physical device onset remains unmeasured; methodology and the
remaining performance work are tracked in
[NEXT_STEPS.md](plans/NEXT_STEPS.md).

`tools/benchmark_server.py` runs the tracked protocol against a selected native
server or launcher and reports raw cold/warm character, word, line,
dense-action, multipart, and rapid-replacement samples with nearest-rank
p50/p95/p99 summaries. New reports preserve and can strictly enforce the exact
physical voice, and a KOI8-R-compatible Russian profile covers RuTTS. The
source boundary is the first mixer-consumption marker; it does not turn those
results into an acoustic-onset claim.

The [2026-09-01 Windows x64 development baseline](benchmarks/2026-09-01-windows-x64-c9458361eb57b94a.md)
preserves 1,440 raw samples across WinRT, eSpeak NG, RHVoice, Flite, DECtalk,
and Eloquence with build provenance and checksums. It was collected from a
post-v1.5.1 development build and is not evidence about the published v1.5.1
artifact or physical acoustic onset.

`tools/stress_server.py` verifies interleaved replacement domains, ordered and
urgent survival, repeated hard-stop recovery, contiguous marker and semantic
event history, and exactly-once terminal status. Its optional fault mode kills
only one uniquely resolved child of its dedicated server, then verifies the
configured fallback and explicit helper recovery probe. Schema-v2 reports
retain physical voices, and strict exact-voice plus Russian-profile controls
cover both RuTTS voices without weakening fallback validation.
Dispatch-time fault mode can repeat a bounded crash cycle: it submits work,
terminates the pre-resolved current helper, cancels the outstanding dispatch,
and independently verifies fallback and exact-voice recovery. Idle fault mode
remains available for compatibility.

`tools/stress_helper.py` can now place repeated synthesis, health checks, and
in-flight cancellations in one persistent helper session while recording
machine-readable process-resource samples. It observes native Windows helper
counters from WSL only after resolving one unique new process; ambiguity is
reported as unavailable rather than attributed to the wrong helper.
Server stress can also sample the complete server/helper process tree, with
aggregate and per-executable steady-state summaries. This is opt-in so process
inspection overhead does not contaminate ordinary latency measurements.
