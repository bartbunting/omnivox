# ADR 0018: Minimal owned emergency speech

- Status: Accepted for implementation on 2026-11-10. Platform qualification and
  release remain separate.
- Extends: [0007](0007-managed-voice-lifecycle.md),
  [0009](0009-local-speech-preferences.md) and
  [0017](0017-independent-host-preference-reload.md).
- Refines: the recovery admission dependency on management history in
  [0013](../benchmarks/data/2026-11-10-lightweight-reload/0013-private-sessions-and-multi-lane-supervision.txt),
  [0014](../benchmarks/data/2026-11-10-lightweight-reload/0014-unix-session-cleanup-guard.txt),
  [0015](../benchmarks/data/2026-11-10-lightweight-reload/0015-windows-session-cleanup-guard.txt)
  and [0016](../benchmarks/data/2026-11-10-lightweight-reload/0016-session-managed-voice-activation.txt).
  Their native cleanup and managed-asset exclusion requirements remain in force.
- Extended by: [0019](0019-windows-native-creation-ownership.md) for Windows
  guard-owned native creation and actual descendant process completion.

## Context

Ordinary retained recovery can reuse a valid startup despite broken current
preferences. It still requires readable management records and writable owner
storage. Neither damaged history nor a failed disk write should prevent all
speech when a trusted bundled engine and usable output remain available.
Conversely, ignoring uncertain native ownership can leave obsolete speech or
unbounded processes running. Those are different problems.

## Decision

Provide an explicit local emergency startup using only bundled eSpeak, built-in
speech defaults and a selected ordinary output backend. Do not read preferences,
managed generations, active pointers or management history. Do not discover
optional engines, load managed assets or require management writes. Use the
selected executable's bundled data and the existing bounded speech, cancellation
and playback paths. The emergency owner establishes native ownership before
opening the worker's launch gate.

Emergency selection explicitly sets aside saved voice exclusions and other
preferences for this minimal engine. It is never a silent fallback from invalid
configuration or a successful managed Apply. Temporary client controls remain
possible; preference reload and managed activation do not operate on the emergency
worker. Ordinary startup remains an explicit subsequent operation.

Before replacing known old speech, require verified native tree and I/O retirement.
An authoritative live owner may acknowledge that retirement even if persisting
its receipt fails. Distinguish this transient observation from durable evidence:
it permits minimal replacement in that live client, but does not release old
asset claims, repair history or authorize conflicting management. A lost reply
requires another correlated observation from that same surviving authority, or
existing valid durable evidence. Missing evidence remains missing.

Preserve old records, claims, transactions and pins. Never signal processes from
saved PIDs, delete claims to unlock speech or infer cleanup from exit, EOF, a free
lock or elapsed time. Emergency mode is independent of damaged management state,
not independent of actual native ownership. Loss of every authority can still
require a separately qualified OS boundary, such as a confirmed host reboot;
this record does not establish automated reboot reconciliation.

Reuse qualified native ownership and cleanup guards where needed, with independent
lifecycle communication when blocked speech I/O would defeat recovery. Avoid a
required system service, a new speech forwarding hop or coordinated preference
transactions. Qualify Linux, Windows/WSL and macOS separately. Do not advertise
owner/guard combined-failure recovery until its native proof exists.

## Consequences and alternatives

A small emergency path can restore speech without repairing the entire management
system. Preferences may be lost, interrupted speech is not replayed, and retained
asset claims may still require separate reconciliation. A functioning host, trusted
core payload, usable output and retained cleanup authority remain prerequisites.

Requiring durable configuration recovery for every restart preserves bookkeeping
at an unacceptable availability cost. Deleting damaged ownership state would
discard necessary safety evidence. A complete session manager remains an option
for managed activation and independent native guards; it is unnecessary merely to
start a minimal owned engine. The [delivery plan](../plans/lightweight-preference-reload.md)
tracks implementation and evidence separately from this decision.
