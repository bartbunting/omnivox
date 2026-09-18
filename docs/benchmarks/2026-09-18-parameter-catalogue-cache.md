# Connection-owned parameter catalogue cache

This internal integration retains complete catalogues from existing public
queries so native admission can validate settings without asking an engine again.
Public query replies and advertised capabilities remain unchanged.

## Behavior

The connection owns at most 32 complete immutable catalogues, with a 1 MiB
aggregate encoded-content budget and one partial assembly capped at 256 KiB.
These are encoded-content budgets, not measured process memory limits. Existing
field and collection bounds also apply. Oldest complete entries are evicted
first. Larger valid public catalogues can still be queried but are not cached.

Pages must form a complete, validated sequence with consistent runtime, profile,
schema, revision, voice and mappings. Partial, discontinuous, timed-out or late
results never become usable metadata. Busy replies preserve an existing assembly;
terminal failures invalidate the queried key. Starting a new first page replaces
the one partial assembly. Cache publication is opportunistic: contention skips it
without delaying a public reply.

A memory-only token identifies each helper connection incarnation within its
engine object. Replacement changes the token even if a helper reuses its wire
runtime generation. Cache reads check both object identity and token; disabled,
unavailable, replaced and pending-recovery engines cannot contribute metadata.
Ordinary speech does not invalidate the token. No cache read waits for speech,
queries an engine, starts a helper or loads a voice. Weak engine references do
not keep a runtime resident.

Returned catalogues are immutable observations, not leases on a runtime. A
replacement can race with a reader; native execution must still validate the
frozen wire identity against the actual helper. Parent-local tokens are never
saved in voice definitions or substituted for that wire identity.

## Verification

Eleven new automated tests cover complete-page publication, discontinuities,
duplicate controls, mismatched mappings and identities, orphan continuations,
entry/byte bounds, weak ownership, replacement, contention, late replies and
registration without further engine calls. Existing tests additionally check
that cached metadata remains usable during ordinary progressive speech while
live catalogue queries return busy, and that pending recovery excludes it.

The locked workspace passed 969 distinct tests plus one child-process rerun,
with two ordinary skips. The strengthened active-speech regression also passed
separately. Workspace Clippy across all targets, pinned formatting and whitespace
checks passed. The retained qualified-runtime test, one of the ordinary skips,
passed when explicitly run:

```sh
OMNIVOX_NATIVE_ROUTING_CASES=/path/to/cases.json cargo test --locked -p omnivox-cli qualified_helpers_route_native_choices_without_playback -- --ignored --nocapture
```

The configuration format is described in the
[preceding routing report](2026-09-18-native-routed-execution.md).
[Recorded observations](data/2026-09-18-parameter-catalogue-cache.json) show all
eight Eloquence and 28 DECtalk controls passing through public query handling
into this cache. Routed fallback and exact private selection then used the
cached catalogues and produced PCM with the requested native readback. Recovery
made each old catalogue unavailable. The probe used the qualified unchanged
Windows helpers with a Linux parent and opened no audio device. It does not
establish listening quality, actual playback or Windows-parent acceptance.

## Remaining work

Connect the cache to public native registration and requests, then carry
application evidence through accepted and actually consumed audio. Add strict
preview/explanation operations and the Emacs parameter editor. The internal
cache accessor is deliberately not yet consumed by production native admission;
the integration tests exercise that next boundary.

No helper protocol, Windows package, desktop launcher or live Emacs session was
changed. Full development staging remains required for public transport
integration. Documentation links were checked alongside this report.
