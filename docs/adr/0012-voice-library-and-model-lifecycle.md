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

This slice assumes a trusted parent has verified the immutable generation's
assets. Hash/provenance verification, main-server eligibility and status,
dynamic host inventory, storage/download service and Emacsvox activation remain
pending. The helper host retains its startup descriptor; the adapter rejects
quarantined voices even though they remain in that snapshot. No main-server
library flag or `voice_library_v1` capability is advertised yet.
