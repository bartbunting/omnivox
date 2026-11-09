# ADR 0017: Independent host preference reload

- Status: Accepted for implementation on 2026-11-10. Qualification and release
  remain separate and are tracked in the [delivery plan](../plans/lightweight-preference-reload.md).
- Extends: [0008](0008-extensible-engine-registration.md) and
  [0009](0009-local-speech-preferences.md).
- Partially supersedes: the preference coordination/commit requirements of
  [0011 at session revision 7879419](../benchmarks/data/2026-11-10-lightweight-reload/0011-session-configuration-and-reload.txt)
  and [0013 at that revision](../benchmarks/data/2026-11-10-lightweight-reload/0013-private-sessions-and-multi-lane-supervision.txt).
- Retains: [0001](0001-engine-isolation-and-distribution.md),
  [0003](0003-progressive-audio-and-markers.md),
  [0004](0004-workstation-service-and-worker-ownership.md),
  [0006](0006-voice-selection-and-customization.md),
  [0007](0007-managed-voice-lifecycle.md), and the native cleanup/managed activation
  protections of [0014](../benchmarks/data/2026-11-10-lightweight-reload/0014-unix-session-cleanup-guard.txt),
  [0015](../benchmarks/data/2026-11-10-lightweight-reload/0015-windows-session-cleanup-guard.txt) and
  [0016](../benchmarks/data/2026-11-10-lightweight-reload/0016-session-managed-voice-activation.txt).

## Context

The [lightweight investigation](../benchmarks/2026-11-10-lightweight-reload.md)
preserved release-like speech response with independent policy installation.
Cross-stream preference convergence and exact preference restoration are less
valuable than continued foreground speech, low latency and dependable recovery.
The previous coordinated durable commit is unnecessary for changing a host
punctuation table. Native process ownership and safe asset use remain necessary.

## Decision

Allow ordinary locally owned workers to reload host preferences independently.
Use their existing stdio control connection, including WSL interoperability, with
explicit capability negotiation. A resident bounded preparation task captures and
validates the worker's selected files using its frozen native launch context and
pinned asset selection. Remote and unowned speech connections gain no local
file-reading authority. Clients do not upload paths or executable definitions.

Each admitted speech request keeps one coherent immutable policy. Installation
occurs at a safe command boundary, without draining ongoing playback or restarting
engines/output. Keep validation, hashing, large policy destruction and blocked
management replies off admission. Preserve transient client settings and update
only the relevant saved baseline used by Reset. Initially only punctuation tables
are reloadable; other effective changes reject the whole operation.

Each worker acknowledges actual live policy and operation state separately from
its immutable native startup identity. Bound pending work and retained results;
make pending, rejected, stale and unknown outcomes honest. Lost replies cannot
justify rollback or replay of obsolete operations. Exact version/identity/retry
rules belong in the protocol reference.

Successful foreground installation does not depend on notifications. Workers may
capture different file versions if files change during the operation. Report
per-worker outcomes and policy identities; never claim atomic cross-stream success.
Named profile selection may follow each supporting worker's local descriptor;
missing profiles use the client's declared fallback. Differing policy hashes do
not themselves disable a working foreground profile.

Preference installation requires no durable transaction journal or exact persisted
live-policy recovery record. Recovery may restore retained startup preferences;
explicit fresh restart may capture saved settings. Neither replays interrupted
speech. Reset and recovery do not implicitly reread edited files. Keep managed
asset activation, native replacement and cleanup evidence separate from ordinary
reload. No claim may be erased or assumed retired to make recovery succeed.

Retain independent synthesis, cancellation, bounded queues and helper isolation.
This decision does not replace native cleanup guards, permit unsafe asset reuse,
change remote management policy, or establish a new emergency-start mechanism.
A smaller lifecycle manager may be considered for a demonstrated ownership or
recovery benefit, without putting ordinary speech behind management availability.

## Consequences and alternatives

The initial delivery can remain small and mostly Rust. Clients request reload and
present independent outcomes; they do not implement native rollback or lifecycle
policy. Preference divergence and reversion on recovery are explicit tradeoffs.
Existing clients keep ordinary speech and explicit restart compatibility.

Completing coordinated sessions remains useful for managed activation and as a
source of tested guards, validation and failure cases. It is no longer a prerequisite
for inexpensive preferences. Rolling engine replacement remains separate, subject
to memory, output exclusivity, cancellation and verified native retirement.

Adoption requires measurements during reload and recovery, representative engines,
native platform checks and honest acoustic limits. This record accepts a contract;
it does not establish that every platform or recovery failure has been qualified.
