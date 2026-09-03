# ADR 0005: Experimental TGSpeechBox Companion

- Status: Accepted
- Date: 2026-09-02

## Context

TGSpeechBox is an MIT-licensed C++ formant synthesizer whose frontend converts
IPA into parameter frames for its DSP. It offers several built-in voice
profiles and direct speed, base-pitch, inflection, and output-gain controls.
Unlike eSpeak NG, it does not provide its own general text-to-phoneme path for
all of the language packs we want to expose.

The upstream `v-310` line is a beta. Its repository tests are not all green at
the selected revision, and the line has neither the retained rate
measurements required by ADR 0004 nor source-offset evidence for truthful
Omnivox markers. Linking its C++ implementation directly into the main server
would also contradict ADR 0001's rule that a new external native engine begins
behind a dedicated helper boundary.

TGSpeechBox's source is MIT-licensed, but an Omnivox helper needs eSpeak NG to
turn Unicode text into IPA. The resulting statically linked executable is a
combined GPLv3 program and needs a different distribution declaration from the
MIT-licensed native boundary source.

## Decision

Omnivox adds TGSpeechBox as an experimental, opt-in companion with these
boundaries:

- The source input is pinned to the exact upstream `v-310` branch snapshot
  `v-310@f5ec247`, commit `f5ec247bca50507ab1e2ed661136395538dc3e97`,
  with an archive digest and a complete extracted-tree digest. This is six
  commits after `v-310b802`; Omnivox does not represent it as a new upstream
  release tag. The supported preparer verifies both digests before compilation.
- TGSpeechBox and eSpeak NG run only inside `omnivox-tgspeechbox-helper`. The
  main process uses helper protocol v5 and receives only bounded
  PCM and truthful capability metadata. No helper-protocol or public synthesis
  schema is extended for the experiment.
- The helper uses the pinned Omnivox `espeak-rs-sys` dependency synchronously
  to produce IPA, then sends that IPA through the TGSpeechBox frontend and DSP.
  The C++, frontend, DSP, and eSpeak state are serialized inside one helper.
- Each language/profile combination accepted by both the pinned TGSpeechBox
  frontend and eSpeak is a physical voice. The accepted Windows x64 GNU build
  currently reports 22 languages, five built-in profiles (Adam, Benjamin,
  Caleb, David, and Robert), and two frontend-enumerated data profiles (Beth
  and Bobby). Source-defined profiles that the pinned frontend does not
  enumerate are not advertised.
- Portable ACSS rate, average pitch, pitch range, and volume map to native
  speed, base pitch, inflection, and output gain. The rate curve is explicitly
  provisional until the standard audit retains measurements; it is monotonic
  with landmarks `0.0 -> 0.5x`, `0.5 -> 1x`, `1.0 -> 2x`, and `2.0 -> 4x`.
- The helper advertises no markers. It does not infer offsets from generated
  IPA or frame indices.
- Cancellation is checked between bounded PCM pulls. The ordinary helper-host
  cancellation grace and process termination provide containment if native
  work does not return promptly.
- Companion staging asks the exact packaged helper for its descriptor at each
  supported native sample rate and records those responses in checksum-covered,
  source-identified voice inventories. Server mode selects the configured
  rate's bounded, schema-validated cache and defers native initialization out
  of the startup critical path. After cache validation, it starts one
  background connection pre-warm while other engines initialize and retains
  that process for synthesis. A concurrent first request joins the serialized
  connection lifecycle, and cancellation can prevent that waiting request
  from being dispatched. A missing or invalid cache falls back to eager live
  initialization; the first deferred connection must return a descriptor
  identical to the selected cache before synthesis is accepted.
- Companion staging includes the exact TGSpeechBox packs, generated eSpeak
  data, source lock, Cargo lock, licence texts, build provenance, and exhaustive
  payload checksums. Cross-compiling from WSL may use host-generated eSpeak data
  from the same locked dependency because that generated data is
  architecture-independent; the executable itself remains target-built.
- Windows x64 GNU is the first runtime-accepted target and, beginning with
  Omnivox v1.7.0, is published as a separate experimental companion archive.
  Linux x64 synthesis remains a development smoke check. TGSpeechBox stays
  outside generic archives and the Emacsvox release bundle.
- The tag workflow rebuilds the Windows x64 GNU helper from the verified source
  input, regenerates both voice inventories, lints the native crates, exercises
  repeated synthesis, streaming, cancellation, and every advertised ACSS
  dimension, then verifies exact TGSpeechBox routing with the relocated generic
  Windows archive. Publication also requires a deterministic source artifact
  containing the exact Omnivox Git tree, vendored Cargo and eSpeak NG source,
  the TGSpeechBox input archive, and exhaustive checksums and provenance.
- The helper package declares `GPL-3.0-or-later` for the combined binary. The
  Omnivox-authored narrow C++/Rust boundary remains MIT, and TGSpeechBox retains
  its upstream MIT licence. Redistribution must satisfy the combined helper's
  GPL obligations as well as retaining component notices.

## Consequences

Users can evaluate a DECtalk-like formant engine on Windows without placing a
beta C++ runtime in the main speech-server process. Existing logical routing,
fallback, health, cancellation, and portable ACSS behavior apply without a new
wire format.

The experiment does not expose arbitrary frame fields as public knobs. Such an
extension would require bounded semantics, capability negotiation, and an
explicit public-protocol decision. The current rate is useful for evaluation
but cannot be described as calibrated, and marker-dependent presentation
actions degrade according to the existing engine contract.

Cached registration removes TGSpeechBox's multi-second native initialization
from the server command-loop critical path. Background connection pre-warming
usually completes before the first TGSpeechBox utterance; speech arriving sooner
waits only for the remaining initialization work.

The published Windows x64 binary remains explicitly experimental despite
passing its release gates: the rate is provisional, markers are unavailable,
and no other native target is claimed. Adding another target still requires a
matching native gate and retained runtime evidence rather than inference from
the Windows result.
