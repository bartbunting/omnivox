# ADR 0007: Managed voices and explicit activation

- Status: Accepted
- Consolidated: 2026-09-27 from voice-library, MBROLA, RHVoice and startup-fallback decisions.
- Related: [Runtime supply](0001-engine-isolation-and-distribution.md),
  [worker ownership](0004-workstation-service-and-worker-ownership.md),
  [voice eligibility](0006-voice-selection-and-customization.md).
- Proposed extension: [Omnivox session coordination](0011-session-configuration-and-reload.md)
  would transfer client-driven activation coordination to Omnivox.

## Context

Installed assets, desired enablement and the configuration used by running speech
workers are different states. Downloading or finding a model must not silently
load it, change saved identities, consume unbounded memory or interrupt speech.
Optional provider failure must preserve ordinary fallback without weakening
administrative exclusions or falsely acknowledging a successful activation.

## Decision

### Separate ownership and state

Omnivox provides shared formats, acquisition, managed storage, native validation and
the local service. Emacsvox supplies the initial reviewed catalogue, accessible
interaction and coordination of foreground/notification workers. Downloaded
files belong to the user; imported files keep external ownership. Assets and
native target/profile identities live outside versioned executable installations.

Separate the installed/desired index from immutable runtime generations and the
active pointer. Preserve stable model/speaker IDs; legacy-ID adoption is explicit
and cannot rewrite palettes silently. New downloads/imports start disabled.
Installation and enablement do not restart workers. On first MBROLA installation,
retain its existing en1 voice as an enabled built-in row unless explicitly
disabled, preserving its physical ID.

Enforce engine/physical-voice exclusions in defaults, exact/property selection,
fallback, previews, direct synthesis and recovery. Explicit file overrides remain
visible and replace only their provider's load set; global exclusions still apply.
Each Piper helper loads at most one model on demand, shared by its speakers.
Flite loads only its selected external voices. Keep model-specific load failure
separate from engine failure and contain partial native construction/cleanup.

### Preserve provider boundaries

| Provider | Managed contract |
| --- | --- |
| Piper and Flite | Schema 1 defines bounded model/speaker and external-voice load sets. Keep earlier physical IDs through explicit adoption and verify selected assets before native use. |
| MBROLA | Schema 2 adds built-in en1 and reviewed US1/US2/US3 English databases from pinned MBROLA-voices inputs. IDs bind database hash, sample rate and reviewed frontend profile. Catalogue entries cannot introduce arbitrary aliases, executable arguments or downloads. Keep per-database terms separate from runtime/frontend terms. |
| RHVoice | Schema 3 adds reviewed voice plus English-language packages, initially Alan, Bdl, Clb and Ksp. Use pinned HTTPS file inputs, checksums and a bounded resource allowlist. Each package owns its language files; enabled packages must agree on that data. No implicit runtime acquisition or archive extraction. |

Earlier schema bytes retain their semantics and serialization when new provider
sections are absent. Older readers reject unsupported schemas before activation.
Runtime policy remains in [ADR 0001](0001-engine-isolation-and-distribution.md).

MBROLA helpers retain metadata, then verify the small common frontend bundle and
selected database for each serialized request. A short-lived native child opens
that database and returns bounded buffered PCM before retirement. Other databases
are not scanned or loaded per utterance. Preserve text bytes, cancellation,
no-marker/no-streaming claims and the original en1 latency acceptance boundary.

RHVoice ordinary startup preserves external discovery and rejects managed/external
physical-ID collisions. Managed acquisition needs an explicit compatible library.
Disposable validation suppresses unrelated external discovery and checks exactly
the selected package and real PCM. Runtime libraries stay user-installed; native
Windows/Linux qualification does not imply macOS support.

### Verify before native use and retain cleanup ownership

Strict bounded metadata parsing is separate from asset verification. Bind exact
generation bytes and file identities; verify sizes, hashes, configuration and
expected physical inventory before using a load. Recheck assets on managed native
loads where required; stale generations cannot validate repaired or replaced
models. A hash is neither a licence decision nor loaded-module attestation.

Disposable validation uses private per-load projections and the exact staged
helper. Require expected inventory, nonempty PCM and actual voice identity, then
confirmed process-tree and reader cleanup before admitting another native load.
Windows private jobs and Unix ownership/supervision retain deadlines and bounded
resource accounting. macOS sampled footprint limits are not hard allocation caps.
Unconfirmed retirement prevents replacement. No audio device is opened merely
to validate an installation.

Before/after observations may publish an immutable bounded evidence report only
after successful checks and cleanup. Bind it to the generation, native load,
staged inputs, search settings and policy. Such a report is not a reusable native
validation cache or proof of all system libraries loaded by the process.

Persistent operation plans, profile leases, journals and ownership claims prevent
concurrent conflicting work. A separate live supervisor retains native authority
after manager loss. Saved PIDs do not grant cross-invocation signalling authority.
Incomplete or damaged history stays blocked unless explicit reconciliation proves
recorded cleanup under the existing contract; abandonment never promotes a failed
attempt to success or repairs its original bytes. Stronger crash/power-loss
recovery remains additional hardening, not permission to bypass blocked claims.

### Activate both workers explicitly

Apply captures reviewed candidate and rollback startup configurations for both
lanes. Establish native tree ownership before helper initialization. Preflight
and independently verify both workers' actual eligibility before publishing the
active pointer. Partial failure retains ownership and rolls back under the saved
configuration; uncertain publication or cleanup remains blocking. Recovery cannot
reconstruct old settings from newly edited client preferences. Each lane retains
independent runtime residency and cancellation.

Ordinary startup with a valid generation can retain unavailable providers and
choose eligible fallback when features, executables, runtimes, assets or expected
inventories are missing. Failed providers contribute no invented voices. Rescans
repeat asset and inventory checks; a missing build feature needs repair/restart.
Exact diagnostics and native validation remain strict. Apply cannot call degraded
startup successful when the candidate's expected voices are missing. Malformed or
unreadable generation metadata fails startup because dropping it could re-enable
excluded voices. Startup also fails when no eligible engine can synthesize.

Removal checks ownership and active, rollback and unretired-session references.
It preserves imported data, external runtimes and saved palette references. No
force-cleanup shortcut or implicit speech restart follows from this contract.

## Consequences and alternatives

Users can install and manage assets without immediately changing working speech.
On-demand model loading trades first-use latency for bounded residency. Explicit
two-worker activation, validation and cleanup add state and recovery obligations.
Eagerly loading all installed models, using download success as native validation,
or treating one worker's acknowledgement as successful Apply would violate these
boundaries. Rejecting all ordinary speech for an optional failed provider would
defeat engine isolation; silently discarding exclusion metadata is equally invalid.

Exact formats live in the [voice-library contract](../reference/voice-library.org).
[Installation](../guides/voice-management.md), [validation](../guides/native-voice-validation.md) and
[removal](../guides/voice-management.md#managed-voice-uninstallation) document current commands and qualification.
[Retained results](../benchmarks/2026-09-27-retained-validation-results.md) preserve
historical tests without embedding an implementation diary in this decision.
