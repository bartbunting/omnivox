# ADR 0012: Voice library and model lifecycle

Status: Accepted
Date: 2026-09-15

## Context

Piper currently constructs one model before helper startup and advertises its
filename-derived voice. Flite can load external files, but there is no shared
installed/eligible voice contract. Downloading more files without controlling
loading would make memory use unpredictable and could break palette identity.

## Decision

Adopt the [version-1 voice-library contract](../voice-library-contract.org)
with Emacsvox ADR 0019. Omnivox owns the shared formats and native validation;
Omnivox also owns the reusable download, installation and storage service;
the downloaded files belong to the user. Emacsvox owns the initial reviewed
catalogue, interaction and coordination of its two speech processes. Separate the
installed index from immutable runtime generations. Keep assets outside
versioned executable installations and preserve imported-file ownership.

Use stable model/speaker identities, retaining legacy IDs through explicit
adoption. Enforce voice eligibility through selection, fallback, defaults and
preview. Each lane's Piper helper holds at most one model, loaded on demand;
speakers of that model share it. Isolate model load failures from engine
failures. Initial disablement takes effect through confirmed helper retirement
at an explicit coordinated restart, with pair rollback on partial failure.

The contract defines `--voice-library`, `OMNIVOX_VOICE_LIBRARY`, managed helper
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
preview/tuning. This decision adds no provider, network management transport,
dependency, redistributable model or release artifact. MBROLA production
integration requires its own engine-boundary decision.

## Implementation status

The maintainer accepted the contract and ownership split on 2026-09-15 and
authorized implementation in slices. This acceptance commit introduces no
runtime behavior. Native and two-lane acceptance checks are in the contract.
The paired Emacsvox record is
`docs/adr/0019-voice-library-and-activation.org` in that repository.

The first implementation slice adds bounded, strict readers for runtime
generations, the installation index and active-pointer records. It validates
stable identities, ownership records, package references, exclusions and load
sets without touching asset files. It preserves exact generation bytes and
defines deterministic file-set hash inputs; actual hash verification and
native validation remain service responsibilities. No startup flag or control
capability is advertised, and existing synthesis paths are unchanged.

### Native loading prerequisite

Inspection and an isolated Linux helper probe on 2026-09-15 exposed a required
failure boundary before implementing model switching. With the existing staged
Piper helper, a file containing `not an ONNX model` and a copied valid Kristin
configuration terminated the process with SIGABRT (subprocess return code -6).
The diagnostic was an uncaught `Ort::Exception` during protobuf parsing.
No audio or live speech process was involved; the temporary fixture was removed.

In the vendored `libpiper/src/piper.cpp`, `piper_create_with_options` parses
configuration, initializes eSpeak, allocates a synthesizer and constructs an
ONNX session without an exception boundary. `piper_free` terminates the shared
eSpeak phonemizer. Rust's helper-host panic handler cannot contain a C++
exception crossing this boundary. Merely checking a null return value or
replacing the adapter's model pointer cannot meet the accepted model-specific
failure contract.

The next implementation must establish exception containment and cleanup of
partial native construction, then verify repeated create/free/create and
failed-load/recovery sequences in one owned helper. Preserve pristine vendored
source and make any native overlay explicit and reproducible. Review startup
cancellation as part of this boundary: a queued stop must not be cleared when
the synthesis worker begins model loading. Keep opaque-call deadlines and
forced helper retirement; do not claim native cooperative cancellation.

These are prerequisites for the accepted design, not a change to its one-model
residency limit. The probe establishes the current failure, not a working fix,
Windows behavior, audible acceptance or measured memory recovery.

The native prerequisite now has a checked overlay on the generated libpiper
build copy. It contains construction and inference exceptions inside C++,
owns partial construction through RAII, and reserves one native model slot
until destruction finishes. The pristine vendor is unchanged. Companion
provenance records the overlay and build-script hashes. An explicit native
lifecycle test covers repeated malformed-config/invalid-model/valid-model
loads, refusal of overlapping construction, inference failure and subsequent
successful synthesis in one process. This establishes Linux recovery, not
Windows acceptance or measured memory release. Cancellation and library
selection still follow in separate implementation slices.

The helper host now attaches its permanent request cancellation token before
publishing the active request. The same token gates protocol completion and
is visible to adapters, including a worker starting after cancellation. The
blocked-worker regression verifies that the adapter observes cancellation;
existing cancel acknowledgements and helper protocol versions are unchanged.

### Managed Piper helper selection

The Piper helper now accepts an immutable runtime generation through its own
`--voice-library` option. It advertises enabled voice metadata without native
loading, validates selected asset sizes/configuration/speaker bounds, and loads
one model on demand. Both synthesis paths set the selected speaker index. Model
changes drop the old native owner before constructing another; speaker changes
reuse it. Request cancellation remains visible before and after opaque loading.
Failed loads quarantine only that model until helper retirement. Legacy model
startup retains its physical ID and speaker zero.

Owned deterministic ONNX fixtures verify actual PCM routing through the native
runtime. Tests cover lifecycle, model-specific errors, cancellation, disabled
speakers, empty eligibility and all helper protocol versions 1–5 on Linux. These
fixtures contain no trained weights and establish no trained speech quality,
audible acceptance, Windows behavior or measured memory recovery.

This slice assumed a trusted parent had verified the immutable generation's
assets. Subsequent verification and startup work is recorded below. Dynamic
host inventory, storage/download services and Emacsvox activation remain
pending. The helper host retains its startup descriptor; the adapter rejects
quarantined voices even though they remain in that snapshot.

### Managed Flite helper selection

Flite now accepts the same helper-local generation option. It registers
compiled-in SLT only when enabled, loads exactly the projected external files,
checks their byte sizes and native physical IDs, and rejects incomplete load
sets. Earlier external loads are released on startup failure. Legacy environment
selection retains SLT, its default and warnings for unusable optional files.

Linux tests use a temporary export of the bundled SLT data, including buffered
and streamed external synthesis, native identity mismatch, changed file sizes,
partial failure and subsequent loading. A fresh owned process verifies SLT's
native registration remains absent for an empty managed selection. Helper
versions 1–5 pass enabled/disabled SLT checks. No additional model is downloaded
or included in an artifact. The same adapter and protocol checks also pass
with the native Windows x64 GNU executable through WSL. Tests remove the
temporary voice after dropping the last engine to check file-handle release.
MSVC release validation, two-lane activation and measured memory recovery
remain pending. Parent-side hash verification is described below.

### Shared administrative eligibility

The shared TTS library now derives immutable eligibility from a validated
generation and resolved Piper/Flite overrides. Overrides replace only the
provider load set; global physical-ID exclusions remain effective. Projection
keeps excluded inventory rows unavailable, preserves runtime failures, and
selects managed defaults in generation order. Administrative eligible IDs are
sorted independently of runtime health and exclude policy-disabled engines.

An opt-in engine registry pins this policy for its lifetime. Its engine handles
guard both buffered and streamed synthesis before native submission, require
exact physical IDs, and retain the guard after startup rescans and descriptor
refreshes. Empty managed providers cannot trigger a rescan. Legacy registries
keep their existing constructor and behavior. Native load sets must still be
configured before engine construction; filtering alone cannot establish that
a disabled model was never loaded.

Tests cover exact/default/property selectors, fallback, direct synthesis,
typed requests, saved references, native-default precedence, global exclusions
under explicit overrides, late discovery, empty providers and health separation.
The main-server integration is described below. No `voice_library_v1`
capability is advertised.

### Asset and generation verification

The maintainer approved adding RustCrypto `sha2` on 2026-09-16. Shared
verification now hashes original generation bytes and deterministic file-set
metadata. Asset reads use a fixed 64 KiB buffer, require regular files with
the declared length, and reject mismatched content or observed changes while
reading. Provider overrides skip only the assets they replace. Parsing remains
free of filesystem access; verification is explicit and does not establish
ownership, provenance or native compatibility.

Piper rechecks both managed assets on each model load, without loading models
during discovery. Flite rechecks managed external files before native loading.
A repaired same-size Piper model remains unavailable under an old generation;
a new matching generation is required. Tests exercise same-size edits in both
native adapters, interrupted and bounded reads, generation byte identity and
provider-specific verification. Native reopening by path cannot guarantee
immutability against concurrent writes to imported files.

Native adapter checks pass for Linux Piper and Linux/Windows x64 GNU Flite.
Staged helper protocols 1–5 pass on Linux and with generation files in the
Windows native temporary directory. The Windows protocol run intermittently
timed out awaiting its greeting when reading generation files through the WSL
share, including empty libraries which do no hashing. That startup limitation
remains unresolved; these checks do not establish MSVC or two-lane acceptance.

### Main-server startup and development status

The server now accepts the generation via CLI or native environment, with CLI
precedence. It verifies active provider assets before engine construction,
binds registry eligibility for the process lifetime, and omits empty managed
helpers. Required helpers must supply the complete projected voice set;
missing helpers, startup errors or mismatched inventories fail startup. The
parent passes the exact source digest to managed helpers, which reject changed
generation bytes before constructing an engine.

Explicit file overrides retain their legacy load behavior and defaults while
global physical-ID exclusions still govern selection. An override is reported
as such; this does not claim that the managed load set or its memory policy
was applied. Diagnostic engine selection uses the same configuration rules.

The development status operation uses one inventory/health snapshot and the
current reader-owned routing policy. Its administrative eligibility is
independent of runtime health. Initial inventory and status are checked against
the complete encoded control and remote line bounds, including unrelated
engines. No capability is advertised yet: disposable native validation, process
tree cleanup evidence, dynamic helper availability and coordinated activation
remain unfinished.

Helper retirement now checks direct-child exit and reader completion with a
five-second cleanup deadline. It kills before acquiring stdin, discards buffered
requests without flushing, and retains unfinished cleanup for retry. An existing
adapter cannot start a replacement after invalidation or failed negotiation
until that cleanup succeeds. Cancellation and drop paths report cleanup failures
instead of claiming success. Linux and native Windows GNU process tests cover
blocked stdin; Linux additionally covers an inherited stdout pipe. Fault-injection
tests on both platforms cover blocked replacement and retry.
This adapter retirement alone does not establish descendant cleanup. Disposable
validation now has a separate supervisor, described below.

Owned Linux main-server probes cover managed Flite/Piper previews, legacy
status, exact exclusions, default reselection, policy generations, input
precedence, required-helper failure, generation changes and model overrides.
Windows x64 GNU startup tests use a native temporary directory and matching GCC
runtime DLLs; the first raw Cargo launch lacked that DLL setup. Neither these
tests nor the development status response establishes MSVC or live Emacsvox
activation acceptance. The full Windows main-server probe remains pending:
the GNU cross-build produced no eSpeak data and the normal staging wrapper
rejected that incomplete runtime. No alternate data set was substituted.

### Disposable native validation

The development [native validation command](../VOICE-VALIDATION.md) uses private
per-load projections without changing installed or active state. One Piper load
checks all projected speakers of that model; external Flite voices are isolated
from each other and from compiled-in SLT. Projection digests remain distinct from
the original input digest. The existing helper protocol and startup arguments
suffice; no native code moves into the supervisor.

A gated worker owns the exact helper path supplied for its provider. The
supervisor requires exact inventory, nonempty PCM and actual voice identity,
then confirmed tree and reader cleanup before admitting another load. No audio
device is opened. Windows uses a private job with checked termination, active
process accounting, aggregate committed-memory limits and kill-on-close. Linux
uses a private group, a dedicated subreaper, inherited address-space limits and
a worker thread that kills its group when the supervisor pipe closes. Group
signalling precedes reaping and is never repeated after PID reuse becomes
possible. Normal speech-worker and remote-service lifecycle rules are retained.

macOS also uses a private group and parent-pipe watcher, with system reaping of
orphaned descendants and bounded observation of group absence. It samples the
aggregate native physical footprint instead of applying a virtual-address limit
that may be below the worker's existing mappings. Exceeding the budget, unavailable
accounting or a full bounded process snapshot aborts the load; this is a sampled
cutoff, not a hard allocation cap. Cleanup must still finish before another load.

This is development validation of managed loads, not the transaction service or
full candidate startup preflight. Durable executable/companion provenance,
operation ownership across manager restarts, full Windows acceptance,
dynamic helper availability and coordinated activation remain
pending. The capability is still unadvertised.

Linux probes cover managed Piper speakers, compiled-in and external Flite,
failed hashes/native models, deadlines, cancellation, supervisor death,
descendant reaping, inherited limits and refusal to continue after unconfirmed
pipe cleanup. Native Windows x64 GNU component tests cover job termination,
descendant pipes, last-handle closure and an over-budget native memory commit.
These are not full Windows target/companion or acoustic acceptance claims.

Native Intel and Apple Silicon validation passed in
[run 35035356056](https://github.com/bartbunting/omnivox/actions/runs/35035356056)
at commit `9107e6f578094c6dfba90a75d4a62a6a390c2179`. Each target passed six
supervision tests five times, staged native main/Piper/Flite payloads, relevant
Clippy checks and the complete silent validation/fault probe. Native testing
exposed Darwin's `EPERM` response for an all-zombie group. Retirement now waits
for confirmed group absence under the existing deadline; it never accepts that
response as cleanup success. A real unreaped-child regression covers both the
termination attempt and absence check. These results establish native validator
behavior on both Macs, not acoustic output, durable operation recovery or
two-lane activation.

### Saved validation observations

The [evidence design](../voice-validation-evidence-design.md) settles the next
boundary without changing the installed-index schema or advertising capability.
Separate bounded reports bind exact generation bytes and native load identities
to observed validator and complete staged companion files, search configuration,
probe policy and limits. Before/after capture runs under owned-worker supervision.
Only successful native checks, matching inputs and confirmed cleanup can publish
a report, using a complete file and non-overwriting hard link.

Saved metadata is unauthenticated evidence of observations, not a reusable native
validation cache, loaded-module attestation or activation acknowledgement.
Unsupported data/loader overrides fail explicitly. System dependencies and
concurrent edits followed by reverts remain outside the guarantee. Publication
does not establish power-loss durability or recovery ownership. The manager must
still establish package/file-set correspondence before writing the accepted
index summary; this slice does not populate `NativeValidation`.

Linux native report checks and Windows GNU component/filesystem checks passed.
Native Intel and Apple Silicon report acceptance then passed in
[run 35042369039](https://github.com/bartbunting/omnivox/actions/runs/35042369039)
at `7ff386693701d8a7a50cb10be615f455062516ca`, including actual Piper/Flite loads,
report creation/comparison, changed-input rejection and absence of a published
report after cancellation or supervisor death. This does not establish full
Windows server/MSVC acceptance, power-loss recovery or activation transactions.

### Persistent validation-operation foundation

The [operation-journal design](../voice-operation-journal-design.md) adds explicit
preparation, per-operation ownership and recovery inspection. A bounded immutable
validation request is paired with an append-only, checksummed state history and
a permanent OS lock file. Each append rechecks prior inputs and synchronizes the
new frame. Interrupted native work and damaged suffixes remain blocked after the
owner exits; inspection never repairs bytes, signals a saved PID or restarts speech.

This is a validation suboperation, not a complete installation/activation plan.
The native validator is not wired into it yet. Profile-wide admission, persisted
native-worker identities, report-to-operation binding and cleanup reconciliation
must precede claims of cross-invocation native ownership. The development prepare
and inspect commands change neither desired nor applied voice state. Durable
multi-file publication and full provider recovery remain pending.

### Profile admission and recorded native execution

Development managed validation now uses a permanent profile lease and bounded,
immutable claims referring to exact operation plans. Every claimed operation is
checked before admission. Unresolved, missing or damaged history blocks a new
operation ID; complete terminal cleanup permits the next operation. The provider
still owns root provisioning and target-identity resolution.

The separate operation execution command records spawn intent before creating
workers, live job/group assignment before opening their native gates, and cleanup
after tree and reader retirement. Its completion evidence binds the operation,
plan and every worker observation. Losing the final journal append does not allow
a saved report to clear an interrupted claim. The standalone diagnostic retains
its prior behavior and does not participate in these claims.

Saved PIDs explicitly have live-supervisor-only authority. They do not establish
boot/birth identity for cross-invocation signalling. Interrupted work stays blocked
pending provider-specific reconciliation; no force-clear, automatic speech
restart or activation is added. Filesystem power-loss guarantees remain pending.

### Reconciliation of recorded cleanup

Explicit recovery can now abandon an intact interrupted validation when every
retained worker has a complete intent/ownership/cleanup sequence, or its
initialized worker directory is empty. It retains both leases, the original
journal and any report. A separate checksummed abandonment receipt binds the
exact plan, journal and complete worker history; every later opening rechecks it.
A verified abandonment permits a new operation through ordinary profile admission
and never promotes the old attempt to successful validation. Lost recovery
acknowledgements are handled by verifying the existing receipt without rewriting.

This reuses the live supervisor's recorded tree and reader cleanup. It introduces
no native process-identity API or authority to signal a saved PID. Missing cleanup
events, malformed history and partial recovery receipts remain blocked and
preserved. Provider-specific cleanup proof for those cases, power-loss recovery,
installation transactions and activation remain separate work.

### Preserve native cleanup ownership after client loss

The development operation command now delegates validation to a separate
supervisor invocation of the same executable. That process holds both admission
leases and all live native ownership. A private startup/cancellation pipe lets it
observe manager death and finish cancellation, cleanup and journal persistence
before exiting. The initial gate prevents a manager lost before startup from
admitting native work. Unix uses a separate process group; Windows starts the
supervisor without the manager's console. Existing workers, helpers, engine
protocols and memory/deadline policies are unchanged.

This preserves the authority already held by a live supervisor. It does not
reconstruct authority from saved PIDs or kernel-object names. A supervisor crash,
whole-tree termination or unconfirmed cleanup still leaves history blocked;
old incomplete histories are not cleared. There is no persistent daemon, new
release artifact, automatic speech restart or voice-library capability added.

### Abandon recorded cleanup after a damaged final write

Explicit recovery also accepts a damaged journal suffix when its verified prefix
ends at `validating` and every retained worker has complete cleanup records. A
distinct receipt basis binds the entire original journal, including its damage.
Later admission verifies that receipt and all original evidence before accepting
the abandoned attempt. The journal cannot be repaired, resumed or promoted to
success. Missing cleanup, damage before the validating record, damaged terminal
prefixes and partial recovery receipts remain blocked. This extends recorded
cleanup reconciliation without adding authority over processes after supervisor
loss during native work.

### Installation and activation priority

On 2026-09-16 the maintainer explicitly moved implementation on to installation
and activation. Further recovery for supervisor loss during active native work,
damaged recovery receipts and stronger filesystem durability is additional
hardening, not a prerequisite for the next delivery slices. Existing unresolved
validation claims continue to block conflicting native work. This changes
delivery order, not the requirement for verified inputs, explicit activation,
matching evidence from both speech lanes or truthful failure reporting.
