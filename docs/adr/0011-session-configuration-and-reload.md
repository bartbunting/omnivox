# ADR 0011: Omnivox session configuration and reload

- Status: Proposed.
- Date: 2026-09-28.
- Direction: Omnivox coordination was selected; the lifecycle and staged reload
  contract below remain proposed and unimplemented.
- Extends: [Managed activation](0007-managed-voice-lifecycle.md#activate-both-workers-explicitly),
  [engine registration](0008-extensible-engine-registration.md) and
  [local preferences](0009-local-speech-preferences.md).
- Related: [Worker ownership](0004-workstation-service-and-worker-ownership.md),
  [engine isolation](0001-engine-isolation-and-distribution.md) and
  [progressive playback](0003-progressive-audio-and-markers.md).

## Context

Omnivox supplies configuration snapshots, native worker ownership, validation and
active-generation publication. Emacsvox currently sequences activation across
foreground and notification workers, checks their acknowledgements and drives
rollback. Configuration reload would otherwise add more coordination to Emacsvox
and require other clients to reproduce it.

Speech preferences now share configuration with engine launch definitions. A
punctuation-table edit can be useful without restarting engines, while changing a
helper's arguments requires a lifecycle operation. Both need a consistent active
configuration and recovery record. Separate workers also provide independent
synthesis and cancellation; centralizing configuration does not require merging
those workers.

## Proposed decision

### Coordinate the session in Omnivox

An Omnivox session controller coordinates configuration validation, change
classification, activation, publication, rollback and recovery for all participating
lanes. Clients request operations and present their progress and result. They
continue to provide accessible editing and session/context speech preferences;
they do not sequence native replacement or decide which candidate became active.

Reuse the existing local management and native ownership boundaries. Initially
retain separate foreground and notification workers, independent engine instances,
lane settings, output routing and cancellation. Session coordination is an Omnivox
responsibility without requiring a system-wide daemon or shared engine pool.

### Retain one authoritative configuration revision

Prepare one validated candidate from the speech host's selected configuration
root and retained launch context. Preserve each lane's explicit launcher choices
and current client settings. Serialize conflicting management operations, bind
them to the expected active revision, and retain the exact prior configuration
needed for rollback. A worker must acknowledge its actual installed configuration;
echoing a shared identifier is insufficient.

Use a coordinated admission boundary: previously admitted requests retain their
captured settings, and subsequent requests use the committed revision. Report
success only after every participating lane has adopted the candidate and the
controller has retained the configuration needed for recovery. A recovering or
later-started lane must use the session's committed revision, not reread files.

Validation failure changes nothing. Failure during activation restores the prior
configuration where recovery is confirmed; it never reports partial adoption as
success. Uncertain publication or native cleanup remains an explicit incomplete
operation with retained ownership. Client disconnection cannot abandon cleanup or
make a repeated request unknowingly apply the operation twice. Interrupted speech
is not replayed. Exact interruption, status and retry semantics must be specified
before implementation.

### Introduce reload in bounded stages

The first reload scope is host speech preferences: punctuation pronunciation
tables, capital-letter cues, chunk size and saved speech defaults. Engines and
audio outputs remain running. Reload updates host policies for subsequent requests
and the baseline used by explicit speech reset. It does not reset speech or
replace the current client-selected punctuation mode, voice, rate or gains.

Validate the complete candidate before applying it. Initially, a change outside
that reload scope rejects the whole operation with the fields requiring restart;
do not silently apply only part of the file. Invalid files retain the working
configuration. Reset and failure recovery continue using captured configuration,
without initiating a file reload.

Later configuration Apply can coordinate helper and output changes through the
existing worker replacement lifecycle. Selective helper replacement is a further
optimization requiring engine-specific cancellation, retirement and inventory
proofs. Changes to helper executables or launch arguments require replacement of
the affected native process; this proposal does not introduce in-process code
replacement or shared model residency.

### Preserve compatibility and access boundaries

Negotiate session management and reload explicitly, with correlated operation
results and observable configuration revisions. Preserve existing standalone and
legacy speech interfaces. Older peers retain the existing explicit restart/Apply
path; they must not be reported as supporting live reload.

Initial management is local to the speech host. This proposal does not add remote
executable definitions, configuration uploads or management commands to the
[remote protocol](../protocols/remote.md). Remote-session reload requires its own
review under ADR 0004. Existing helper isolation, eligibility, cleanup and
no-replay constraints remain applicable.

## Consequences and alternatives

Clients gain one configuration operation while Omnivox retains the coordination
and recovery rules needed to complete it. Two-worker verification and rollback
still exist internally. The controller adds a shared failure point whose lifetime
and recovery must be defined; it does not remove native validation costs or
duplicate model memory.

Keeping coordination in each client repeats lifecycle policy. Combining both
workers would change failure and synthesis isolation without settling the
configuration contract. A shared engine pool adds scheduling and cancellation
constraints and remains a separate choice.

The [roadmap](../ROADMAP.md#host-configuration-follow-up) tracks delivery. Exact
protocols, revision persistence, controller lifetime and failure acceptance belong
in the implementation specification and references. Approval to create this record
does not authorize implementation or establish platform qualification.
