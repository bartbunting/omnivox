# Native timelines and consumed-audio receipts

Timeline 5 now connects publicly registered native choices to ordinary speech.
Its strict bounded decoder preserves legacy, layered and engine-layered modes,
context and placement. Registry generation and exact definition mode are checked
before admission. Multipart identity, byte bounds and version-local replacement
reuse the existing transaction path. Older readers still reject native spans.

Admission freezes the connection's cached parameter catalogues with the routing
snapshot. Retained metadata is charged to queue memory accounting. No discovery
or helper query runs on this path. Buffered and streaming attempts use the
existing native preparation and transactional PCM handoff. An unavailable native
block produces explicit common-only evidence; ordinary common mappings remain
unchanged. Each span supplies its own context, including explicit equal values.

Marker 4 pairs the actual choice and native result with the utterance-start
record at the first consumed PCM frame. Both records share one playback cue and
remain adjacent before diagnostics. Native evidence is required but nullable;
marker 3 retains its previous shape. Full receipt serialization and reservation
still precede PCM commitment. Empty or unconsumed audio and failed pre-audio
attempts cannot publish applied evidence. Cancellation and fallback retain the
existing no-replay boundary.

Helper-local plan IDs become monotonic references owned by the connection's
synthesis worker. At most 64 references retain weak runtime ownership, the
observed epoch, voice and catalogue identity. The epoch is captured before
synthesis, preventing a replacement helper from inheriting a stale receipt by
reusing a plan ID. Eviction or runtime replacement expires detail lookup without
invalidating already emitted compact evidence. Public explanation is a later
slice; these references do not yet expose a public query operation.

## Verification

Eleven new regressions cover independent timeline/receipt fixtures, strict
nested parsing and required nullable members, old-format rejection, multipart
byte declarations, registry generation and mode, all four buffered/streaming
fallback combinations, per-span context, absent metadata, cancelled or malformed
native streams, empty audio, queue accounting and plan reference retirement.
The playback checks use null output; they establish consumed-source events,
not acoustic quality.

Strict native previews, draft/applied explanation operations, Emacs editing and
two-stream acceptance remain. The full native capability is still unadvertised.


The locked workspace passed 988 distinct tests plus one child-process rerun,
with two expected skips. Workspace Clippy across all targets, pinned formatting,
whitespace checks and backend documentation links passed. The Emacsvox
documentation release check also passed; no Lisp implementation changed.

## Packaged Windows verification

Full Windows development staging passed as `bc33370ac992544e`, including
checksums, deterministic helper builds and live companion synthesis. The
recorded Omnivox tracked-diff hash is
`d6f6d32a43599e3a7b763767e52e99e8525cf5f8babff42673817b7c3076e3fe`.
Final report observations were appended after staging. The
[retained observations](data/2026-09-18-native-timelines.json) record package
provenance and all six dispatches from the final probe.

The packaged Windows server produced applied receipts for DECtalk and Eloquence,
with distinct public plan references and explicit equal-context masking only on
the first span. Eloquence's request used multipart timeline 5. A version-4
layered request referencing the native definition failed without playback.
A choice without a native block returned null; a separate connection without
cached catalogue metadata returned common-only evidence. Native playback was
cancelled and an ordinary preview succeeded afterwards. The native capability
bundle remained unadvertised. These checks used null output and do not establish
listening quality.

The initial multipart probe reused a benchmark helper that hardcodes version 3
in its part headers. The server correctly rejected that mismatch; correcting
only the probe header to version 5 made the final probe pass.

The payload is staged separately under
`servers/omnivox-bin/native-timeline-check` in Emacsvox. The full development
package omits Piper, so it was not selected for the active voice-library profile.
No desktop launcher or live Emacs session was changed.
