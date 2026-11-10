# ADR 0019: Windows native creation ownership

- Status: Accepted for implementation on 2026-11-10. Native qualification,
  everyday-engine coverage and release remain separate.
- Refines: the native completion criterion and helper creation ownership in
  retained [0015](../benchmarks/data/2026-11-10-lightweight-reload/0015-windows-session-cleanup-guard.txt)
  for the independent local guard introduced under
  [0018](0018-minimal-emergency-speech.md).
- Preserves: [0001](0001-engine-isolation-and-distribution.md),
  [0003](0003-progressive-audio-and-markers.md),
  [0007](0007-managed-voice-lifecycle.md),
  [0008](0008-extensible-engine-registration.md) and
  [0017](0017-independent-host-preference-reload.md).

## Context

The Windows guard's root-process completion and empty job accounting can precede
completion of a surviving helper's actual process object. The
[qualification report](../benchmarks/2026-11-10-windows-guard-qualification.md)
preserves that counterexample. It does not demonstrate stale audible speech.
The missing completion evidence nevertheless prevents the guard from establishing
that every retained native process has finished before replacement or reuse.

A [bounded creation experiment](../benchmarks/2026-11-10-guard-handoff-and-preparation.md)
retains process handles before execution and waits for actual completion through
worker death and incomplete handoff. Merely asking a worker to register a process
it has already created leaves an ownership gap if the worker dies first.

## Decision

For the opt-in Windows local guard, the guard owns native worker and helper
creation. Establish the private kill-on-close job and retain each actual process
handle before releasing execution. Keep the guard outside that job. Restrict
handle inheritance and uncontrolled child creation. Saved PIDs, observed job
members, exit codes, elapsed time and completion-port messages do not replace
retained process-handle completion.

A private bounded creation channel carries lifecycle requests only. Select helper
programs, arguments and environment from the retained, validated launch snapshot;
speech clients gain no executable-selection authority. Keep executable and asset
verification at their existing required boundaries. Worker helper initialization
and recovery must use the same verified launch identity.

Transfer direct helper input/output pipes and the worker's required cancellation
and wait authority. Retain the guard's own process handle through completion.
A complete acknowledged handoff precedes execution. Partial transfer, lost replies,
worker death and cancellation cannot release an obsolete gate or lose native
ownership. Bound pending creation, unreaped members, transfer buffers and retained
native resources. Creation work must not block the guard's independent retirement
and status handling. An unconfirmed transfer may require retiring that worker to
close handles whose receipt is uncertain; its peer remains independent.

Close creation admission before retirement. Require completion of pending creation,
all actual retained worker/helper process handles, and the guard's own native I/O
before reporting native cleanup. Empty job accounting remains a consistency check
on the owned set, not the completion barrier. Preserve the distinct forwarding-I/O
lease and the requirement for both native and forwarding completion before
replacement. If the OS cannot complete termination, retain unavailable status.

Start qualification with explicit eSpeak/Flite test configurations. Reject
unsupported guarded configurations before disrupting working speech; do not
silently disable preferred engines or change saved settings. Existing ordinary
launch remains available with independent preference reload. Helpers that create
children, such as MBROLA or external wrappers, need an explicit supported creation
path and native qualification before guarded enablement. Everyday use requires
qualification of the actual selected engines, not merely the first test pair.

Speech, helper PCM, interruption and ordinary preference reload remain outside the
guard's transport. Do not add a required Windows service, a TCP speech hop, a shared
engine pool or coordinated preference transactions. Preserve managed claims,
asset pins and durable receipt rules. Minimal emergency speech still follows
0018; this decision does not repair damaged history, establish reboot
reconciliation or promise recovery after losing every cleanup authority.

## Consequences and alternatives

This adds a bounded native creation path while preserving existing speech and
helper protocols. The first implementation is Rust and private to guarded
Windows startup. Linux and macOS keep their existing ownership mechanisms and
require their own qualification. Existing ordinary, remote, validation and session
paths are not silently reclassified as qualified by this change.

Keeping only job accounting is smaller but cannot establish the observed process
completion boundary. Adding a sleep guesses at it. Worker-side creation followed
by registration leaves a failure interval; enumerating job PIDs cannot reconstruct
handles for already-disappeared members. A global service would add deployment and
lifetime requirements without being necessary for the selected local boundary.

If a smaller completion contract is later supported by documented OS guarantees
and tests establishing audio suppression, safe ownership and resource bounds,
review that alternative explicitly. The retained counterexample alone is not a
claim of audible harm. Implementation success, measured operation time, acoustic
acceptance and release remain separate gates in the
[delivery plan](../plans/lightweight-preference-reload.md).
