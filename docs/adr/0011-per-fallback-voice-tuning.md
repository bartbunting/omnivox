# ADR 0011: Per-fallback tuning before contextual voice rules

Status: Accepted
Date: 2026-09-09

## Context

Emacsvox's maintainer selected shared settings, then the actual choice's
adjustments, then contextual overrides. This preserves heading and emphasis
rules while allowing different physical voices to use different base settings.
The existing shared-style registry and flattened timeline cannot express that
order after a real synthesis fallback. Client inventory prediction is insufficient.

## Decision

Implement the [paired wire contract](../per-fallback-voice-tuning.org) and
[independent examples](../protocol-fixtures/voice-choice-tuning.json). The source
contract records its Emacsvox commit. The complete bundle is
`voice_choice_tuning_v1`, requiring `presentation_timeline_v4` and
`playback_marker_events_v3`. Do not advertise it until registration, ordinary
speech, contextual spans, private previews and playback evidence all work.

Keep control envelope 1. Add separate `register_logical_voices_v2` and
`preview_voice_v2` operations with their versioned acknowledgements. Preserve
existing operations, remote envelope 1 and timeline versions 1–3. New registry
content explicitly distinguishes legacy and layered definitions while retaining
one generation domain. Layered definitions contain shared settings and ordered
choice records with stable IDs and sparse set/default patches.

At admission, freeze the definition generation, policy, administrative
disablement, host rate and span context. At each actual eligible synthesis
attempt, resolve the record identity, compose shared/choice/context settings,
apply existing placement, and adapt capabilities. Policy fallback has no choice
patch. Explicit values replace; relative offsets are not added together. Preserve
zero, explicit adapter default and inheritance. Emacsvox projects its historical
contextual nil semantics before transmission; Omnivox receives explicit operations.

Build native settings and effects from the original admitted request and the
selected adapter's defaults. Failed-attempt state must not leak into the next
route. Retain ADR 0004's rate calibration, ADR 0006's four-attempt and PCM
commitment behavior, and the existing engine/voice health distinction. Output
failures and failures after PCM commitment never cause cross-engine replay.
Retain separate speech lanes and ADR 0008's reconnect isolation and bounds.
No helper protocol, process boundary or dependency extension is included.

Version-4 spans preserve sparse context and named identity; mixed legacy runs
retain their existing semantics within a run and reset effect state at a mode
boundary. Version-3 playback events identify the selected choice and actual
physical voice at consumed first frames. Preview results distinguish committed
PCM from audio whose playback actually started, with bounded metadata and
explicit truncation. Neither resolution predictions nor committed-but-unconsumed
PCM establish last actual playback.

## Compatibility and consequences

Old clients keep old semantics on a new server. New clients use a shared-only
projection on older servers and report that saved individual settings were not
applied. Customized full previews require the complete bundle; exact auditions
remain separate and must be faithfully representable. No implicit upgrade of
helper protocols or required release version follows from this decision.

Implementation proceeds in tested commits: typed values/composition, registry
and negotiated codecs, routed execution and previews, then client integration.
Paired client/server fixtures and fake-engine tests must capture final settings,
PCM commitment, consumed frames, retries, cancellation, both lanes and remote
framing. Audible acceptance and deployment remain separate boundaries.

## Implementation status

The contract is accepted. No new server capability is advertised yet. Emacsvox's
storage slice is committed as `3abe311ca`. Typed shared settings, sparse patches,
strict choice records, and pure shared/choice/context composition are implemented.
Nine focused tests cover the paired examples, defaults, bounds and row identity;
all 655 workspace tests and workspace Clippy pass with the pinned Rust toolchain.
Registration, routed execution and client integration remain in progress in the
isolated `voice-choice-tuning` worktree.
