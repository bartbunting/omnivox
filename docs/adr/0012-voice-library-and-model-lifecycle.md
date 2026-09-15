# ADR 0012: Voice library and model lifecycle

Status: Proposed — awaiting maintainer acceptance
Date: 2026-09-15

## Context

Piper currently constructs one model before helper startup and advertises its
filename-derived voice. Flite can load external files, but there is no shared
installed/eligible voice contract. Downloading more files without controlling
loading would make memory use unpredictable and could break palette identity.

## Proposed decision

Adopt the [version-1 voice-library contract](../voice-library-contract.org)
with Emacsvox ADR 0019. Omnivox owns the shared formats and native validation;
Emacsvox owns the initial reviewed catalogue and interaction. Separate the
installed index from immutable runtime generations. Keep assets outside
versioned executable installations and preserve imported-file ownership.

Use stable model/speaker identities, retaining legacy IDs through explicit
adoption. Enforce voice eligibility through selection, fallback, defaults and
preview. Each lane's Piper helper holds at most one model, loaded on demand;
speakers of that model share it. Isolate model load failures from engine
failures. Initial disablement takes effect through confirmed helper retirement
at an explicit coordinated restart, with pair rollback on partial failure.

The contract proposes `--voice-library`, `OMNIVOX_VOICE_LIBRARY`, managed helper
startup and the negotiated `voice_library_v1` status operation. It preserves
existing startup behavior, physical voice fields, control envelope 1 and helper
protocol versions 1–5. Installed metadata does not require native model loading.
Do not advertise the capability until the complete eligibility contract works.

## Consequences and boundaries

Both lanes share eligibility but retain independent residency and cancellation.
Lazy model switching trades first-utterance latency for bounded residency.
Explicit overrides remain visible and take precedence; no silent adoption or
palette rewriting. A generation acknowledgement does not prove audible output.

Preserve ADRs 0001–0011: engine process boundaries, measured rates, bounded PCM
commitment, remote ownership, output/engine failure separation and exact voice
preview/tuning. This proposal adds no provider, network management transport,
dependency, redistributable model or release artifact. MBROLA production
integration requires its own engine-boundary decision.

## Implementation status

Proposal only. No new flags, schema reader, capability or loading behavior is
implemented by this documentation commit. Acceptance precedes implementation
of the public contract; native and two-lane acceptance checks are in the
contract. The paired Emacsvox record is
`docs/adr/0019-voice-library-and-activation.org` in that repository.
