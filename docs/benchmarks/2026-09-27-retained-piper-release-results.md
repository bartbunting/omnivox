# Retained Piper release qualification

Extracted on 2026-09-27 from documentation at source `ca38781`. These are
retained observations, not new test runs or current universal qualification.
Each section identifies its original document; the Git revision preserves that
account. Original raw files and CI links remain where recorded. Missing local
logs, samples or provenance have not been reconstructed. Historical outstanding
work describes that observation, not necessarily the current implementation.

## Native packaging and validation

Source: `docs/plans/PIPER-RELEASE.md` at `ca38781`.

The moving libpiper fetch, target-selection defect, and custom bridge have now
been removed. The native build consumes vendored libpiper v1.7.0, checks
Cargo's requested native target, and fails closed for unsupported or
cross-compiled helper targets. The Rust adapter consumes every returned
float-audio chunk, including the final `PIPER_DONE` chunk, and observes
cancellation between chunks. Linux x64 real synthesis passes with the existing
local test model. All four initial native targets now prepare checksum-locked
eSpeak NG, Sonic, and ONNX Runtime inputs before CMake runs. Completed native
jobs recheck the prepared cache without network access after staging.
Relocatable staging and deterministic packaging are implemented for the native
`.tar.gz` and `.zip` layouts; Linux archive verification, including real
synthesis from the extracted candidate, passes locally. Linux x64, Windows
x64, and both macOS architectures build and stage on native runners. The
English CI model is revision- and checksum-locked, and its model card
declares a public-domain LibriVox dataset and training from scratch. The lock
approves that exact revision for CI-only acceptance and explicitly excludes it
from release artifacts. All four native jobs now verify their relocated
archive and synthesize real audio through the Piper-enabled main server.
A platform-neutral, deterministic source artifact now includes the exact
committed Omnivox/libpiper tree, locked Cargo sources, eSpeak NG and Sonic
sources, all four ONNX Runtime build inputs, and the corresponding ONNX Runtime
source. Its exhaustive manifest, Git tree, locked inputs, model exclusion, and
offline Cargo graph pass verification. The tag workflow packages all five
Piper artifacts, downloads them back from a draft, and gates publication on
native real synthesis and source verification. Windows Authenticode and macOS
Developer ID/notarization are deferred because the required external signing
services and credentials are unavailable; releases must state that binaries
are unsigned and continue to publish exhaustive SHA-256 checksums.

## Corresponding-source reproducibility

Source: `docs/plans/PIPER-RELEASE.md` at `ca38781`.

The artifact is normalized to a fixed timestamp, uid, gid, and mode policy.
Its manifest covers every other file by path, mode, size, and SHA-256 digest.
The verifier safely extracts the archive, compares the Omnivox tree with the
recorded Git commit, verifies the archived input locks and payloads, rejects
the CI voice payload, and resolves `Cargo.lock` offline using an empty Cargo
home. The first complete 1.5.0 candidate was 434.6 MiB and reproduced the same
SHA-256 digest across two builds.

## Recorded implementation acceptance

Source: `docs/plans/PIPER-RELEASE.md` at `ca38781`.

1. **Completed:** replace the archived source fetch and custom bridge with the
   selected, immutable `libpiper` source path. Make host and Cargo target
   selection explicit and fail closed for unsupported target triples.
2. **Completed on Linux x64:** adapt the Rust wrapper to the versioned C API
   while keeping the existing helper protocol stable. Observe cancellation
   between returned audio chunks; retain helper retirement for calls that do
   not return.
3. **Completed on Linux x64:** add a staging command that produces one complete
   relocatable companion payload and records source-input and payload digests.
   Its native-input preparation verifies every implicit upstream download and
   supports an offline repeat build.
4. **Completed on all four initial native layouts:**
   construct a deterministic platform-named `.tar.gz` or `.zip` and reject
   missing, unexpected, host-architecture, dynamically unresolved, or
   incorrectly hashed files. The verifier relocates the archive into a path
   with spaces and optionally exercises a separately supplied,
   licence-reviewed model end to end.
5. **Completed as a manual non-publishing workflow:** Linux x64, Windows x64,
   and macOS ARM64/x64 build and stage successfully, verify the relocated
   native archive, synthesize real audio with the locked CI-only model, and
   exercise 25 requests plus in-flight cancellation through one persistent
   helper session. The replacement CI model is approved for that exact CI-only
   purpose and remains excluded from release artifacts.
6. **Completed:** create and independently verify the deterministic
   corresponding-source and locked-build-input artifact for all four native
   companions.
7. **Completed for source builds, candidates, and tag releases:** document
   installation, model/config discovery, engine inventory, fallback,
   diagnostics, upgrade, and removal in the
   [Piper companion guide](../engines/piper.md), including the unsigned-binary boundary
   and checksum-verification requirement.
