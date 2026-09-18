# Routed native voice execution, 2026-09-18

This slice follows [native choice admission](2026-09-18-native-choice-admission.md)
and connects its prepared settings to the CLI's existing actual-attempt synthesis
loop. It implements an internal integration boundary; public registration,
native timelines, preview operations, playback receipts and Emacs editing are
still pending. The native capability remains unadvertised.

## Behavior

Routing snapshots retain complete engine-layered definitions and count native
maps and values toward their existing queue byte budget. Each eligible attempt
composes the original common style, actual choice and context against immutable
metadata supplied by the caller. Routing performs no catalogue queries or native
parameter mapping. Current helper identity validation remains authoritative if
metadata becomes stale, including after recovery.

Buffered and progressive attempts use the shared native synthesis methods.
Strict preparation fails before synthesis when metadata cannot qualify the block.
Explicit common-only preparation retains common settings and records degradation;
a helper may also report a permitted common-only outcome. Policy fallback has no
choice-native block or native receipt. Runtime failure recomposes the next actual
choice instead of carrying the failed choice's native settings forward.

Native application receipts are validated before stream metadata, markers and
PCM can cross the routing boundary. They remain attached to the tentative
attempt until its first nonempty audio handoff. Receipt state is checked for
every event, but evidence is copied only once. Empty streams publish no native
attempt handoff. Missing, duplicate, late, mismatched or disallowed degraded
receipts fail the attempt; completed frame counts must agree with accepted PCM.
Buffered results carry their validated application with the complete attempt.
Neither handoff nor a returned buffer proves consumption by an audio device.

The existing four-attempt limit and transactional preamble remain authoritative.
Pre-audio synthesis failures can select another engine; consumer failures and
failures after output commitment cannot replay through a fallback. Cancellation
or generation supersession suppresses pending evidence and results. Consumer
failure does not quarantine a healthy engine.

Exact private selection retains the original occurrence ID, including two
choices naming the same physical voice, and removes policy substitutes. Older
routing entry points, styles and output consumers reject native definitions or
evidence. Existing marker/preview consumers must explicitly support the new
contract before this path can reach them.

## Verification

Thirteen new automated regressions cover all buffered/progressive fallback
combinations, native zero/default operations, explicit equal-value context,
choice-specific common settings and effects, same-voice exact selection, strict
and common-only behavior, policy fallback, malformed evidence, empty streams,
frame-count mismatches, no replay after commitment, output health, cancellation,
generation supersession, compatibility guards and native queue accounting.

The locked workspace passes 958 distinct tests plus one child-process rerun.
Two tests are skipped by the ordinary suite: the existing long eSpeak stress
test and the new qualified-runtime probe. The latter was explicitly run and
passed separately. Workspace Clippy across all targets, pinned formatting,
whitespace and documentation checks passed.

The retained opt-in test in
[routing_native_tests.rs](../../omnivox-cli/src/routing_native_tests.rs) reads an
explicit JSON configuration from `OMNIVOX_NATIVE_ROUTING_CASES`. Each entry names
`engine`, `program`, `arguments`, `voice`, `parameter` and integer `value`.
Run it with:

```sh
cargo test --locked -p omnivox-cli qualified_helpers_route_native_choices_without_playback -- --ignored --nocapture
```

The [recorded observations](data/2026-09-18-native-routed-execution.json) cover
qualified Windows Eloquence and DECtalk helpers launched by the Linux test
parent. Calls pass through voice eligibility and cancellation isolation. Each
engine produces progressive PCM and the requested native readback after a
synthetic first-engine failure, then again through exact private selection.
No audio device was opened. These checks do not establish listening quality,
public transport behavior, actual first-frame consumption or Windows-parent
acceptance of the new routing code. Cancellation regressions in this slice use
simulated engines; earlier reports retain real-helper cancellation qualification.

## Remaining work

Connect bounded connection-owned metadata to public registration and native
requests. Add timeline 5 and marker 4 transport, preserve compact native receipts
through accepted and actually consumed audio, and connect strict preview and
explanation operations. Then expose engine-described controls in Emacs.

This internal slice changes no helper protocol, executable distribution,
desktop launcher or live Emacs session. No Windows package was rebuilt; the
probe reuses unchanged qualified helpers. Full development staging remains
required when the public transport integration is ready.
