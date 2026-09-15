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
operation ownership across manager restarts, full Windows acceptance, native macOS
acceptance, dynamic helper availability and coordinated activation remain
pending. The capability is still unadvertised.

Linux probes cover managed Piper speakers, compiled-in and external Flite,
failed hashes/native models, deadlines, cancellation, supervisor death,
descendant reaping, inherited limits and refusal to continue after unconfirmed
pipe cleanup. Native Windows x64 GNU component tests cover job termination,
descendant pipes, last-handle closure and an over-budget native memory commit.
These are not full Windows target/companion or acoustic acceptance claims.
The supervisor and native test sources compile for both Apple targets from Linux;
this does not establish native linking or execution. A manual Intel/Apple Silicon
workflow now covers native footprint limits, Piper/Flite validation and Unix
ownership faults, but has not yet been run for this change.
