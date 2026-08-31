# Piper Release Plan

Status: option A selected; implementation in progress.

Piper is already represented by an optional out-of-process Omnivox helper, but
it is not a distributable release component. This plan defines the work needed
to make it reproducible on Windows, Linux, and macOS without weakening the
failure containment of the helper architecture or silently taking ownership of
third-party voice-model licences.

## Decisions already made

- Keep Piper out of the Omnivox server process. The helper remains the native
  library owner and process retirement remains the hard-cancellation and crash
  boundary.
- Retain every available platform engine in the server registry. A configured
  Piper helper joins that registry; it does not replace WinRT,
  AVSpeechSynthesizer, or eSpeak.
- Keep voice models user-supplied. Piper's
  [voice documentation](https://github.com/OHF-Voice/piper1-gpl/blob/v1.7.0/docs/VOICES.md)
  requires checking the `MODEL_CARD` for each voice because model licences and
  restrictions vary.
- Treat Piper as an optional release payload with its own native dependencies,
  notices, source provenance, and platform verification. Do not infer that a
  working developer build is a redistributable archive.
- Publish Piper as a separate optional companion archive rather than adding its
  native runtime to every generic Omnivox archive.
- Defer multiple live instances of one engine until long-session evidence
  shows that restart and fallback are insufficient.

## Pre-migration audit and current state

The original experimental `omnivox-piper-sys` build was not suitable for a
release:

- its Cargo build script cloned the archived `rhasspy/piper` repository at the
  current default-branch head when a local ignored checkout was absent;
- its CMake graph fetches additional native sources and binaries without a
  complete set of immutable revisions and verified digests;
- platform selection in the build script observed the build-script host rather
  than Cargo's requested target, so a cross-compiled result could contain host
  libraries;
- the custom bridge targeted the archived Piper C++ API rather than the
  maintained, versioned `libpiper` C API;
- `make build-piper` produces linked binaries in a developer tree but does not
  stage all native libraries, eSpeak data, notices, and provenance into a
  relocatable payload; and
- the ordinary release archive and archive verifier intentionally know nothing
  about the Piper helper or its runtime files.

The first two defects and the custom bridge have now been removed. The native
build consumes vendored libpiper v1.7.0, checks Cargo's requested native target,
and fails closed for unsupported or cross-compiled helper targets. The Rust
adapter consumes every returned float-audio chunk, including the final
`PIPER_DONE` chunk, and observes cancellation between chunks. Linux x64 real
synthesis passes with the existing local test model. Dependency verification,
staging, archive verification, and other native runners remain open.

The maintained upstream is
[`OHF-Voice/piper1-gpl`](https://github.com/OHF-Voice/piper1-gpl). Release
[`v1.7.0`](https://github.com/OHF-Voice/piper1-gpl/releases/tag/v1.7.0)
provides a versioned, chunked
[`libpiper` C API](https://github.com/OHF-Voice/piper1-gpl/blob/v1.7.0/libpiper/README.md)
and upstream native build coverage for Linux, macOS, and Windows. The project
is GPL-3.0-or-later. Its published release assets are Python packages rather
than standalone `libpiper` development/runtime bundles, so Omnivox cannot
merely copy a supported native archive into its release.

## Proposed release boundary

The main Omnivox executable may be built with helper discovery enabled on all
platforms; that feature does not itself link Piper. Publish Piper separately as
a platform-specific optional companion payload containing:

- `omnivox-engine-piper`;
- the matching `libpiper` and ONNX Runtime libraries;
- the eSpeak data and runtime files required by that exact libpiper build;
- third-party notices, GPL text, locked source identity, and corresponding
  source or durable corresponding-source instructions appropriate to the
  distribution method; and
- a manifest of every file and SHA-256 digest.

Do not include a voice model in that payload. The user supplies an `.onnx`
model and its adjacent configuration after reviewing its `MODEL_CARD`.

Separate companion archives keep a large GPL/native payload optional and let
the generic server remain useful without a model. They do not remove the GPL
and corresponding-source obligations of distributing the Piper companion.
The exact compliance bundle should be reviewed before publication; this plan
is an engineering component map, not legal advice.

## Source acquisition decision

Option A was selected. Option B remains documented so the trade-off is not
lost if repository size or source-publication requirements change.

### A. Vendor the maintained libpiper source subtree (selected)

Import the exact `v1.7.0` libpiper source, upstream licence, and required build
metadata under a clearly separately licensed third-party directory. Pin and
verify every downloaded native input, or vendor an input when its build cannot
be made checksum-verifiable.

Advantages:

- Cargo never clones a moving branch;
- the source needed to rebuild the native helper is visible and reviewable;
- normal builds can be made network-free after native binary inputs are
  cached; and
- release tags identify the complete build logic rather than depending on the
  future contents of a generated GitHub archive.

Costs:

- the repository gains a separately licensed source subtree and generated-size
  pressure; and
- upstream updates become explicit import/review work.

### B. Require a verified upstream source cache (not selected)

Add a preparation command that downloads an exact upstream tag or commit,
checks a recorded SHA-256 digest, and places it in a versioned cache. Cargo
builds must refuse network access and fail with an actionable preparation
instruction when the cache is absent.

Advantages:

- less third-party source is stored in the Git repository; and
- upstream source remains visibly separated from Omnivox-authored files.

Costs:

- clean builds require an explicit fetch step and durable source hosting;
- GitHub-generated archive stability must not be assumed without controlling
  and retaining the verified input; and
- corresponding-source publication becomes another release operation that
  must be tested.

Keeping and merely pinning the archived `rhasspy/piper` bridge is not proposed.
It saves an API migration but preserves an unmaintained native stack and gives
Omnivox sole ownership of platform repair.

## Initial platform proposal

Start only with native runner builds that can execute synthesis during CI or
physical acceptance:

| Companion artifact | Initial state | Reason |
| --- | --- | --- |
| Linux x64 | First release candidate | Matches the current generic Linux release architecture. |
| Windows x64 | First release candidate | Required for the primary Emacsvox deployment path. |
| macOS ARM64 | First release candidate | Current Apple hardware and an upstream native build target. |
| macOS x64 | First release candidate | Needed while Intel Macs remain supported. |
| Linux ARM64 | Hold | Add with a generic Linux ARM64 release and native runtime runner. |
| Windows ARM64 | Hold | Upstream's current packaged/runtime path is x64-oriented; do not claim untested support. |

Each released row requires its own locked build, staged-runtime inspection, and
real synthesis test. Compile-only cross-checks do not establish runtime
support.

## Implementation slices after the decision

1. **Completed:** replace the archived source fetch and custom bridge with the
   selected, immutable `libpiper` source path. Make host and Cargo target
   selection explicit and fail closed for unsupported target triples.
2. **Completed on Linux x64:** adapt the Rust wrapper to the versioned C API
   while keeping the existing helper protocol stable. Observe cancellation
   between returned audio chunks; retain helper retirement for calls that do
   not return.
3. Add a staging command that produces one complete relocatable companion
   payload and records all source and binary digests.
4. Teach archive verification to reject missing, unexpected, host-architecture,
   or dynamically unresolved files in a Piper companion archive.
5. Add native-runner CI one platform at a time. Do not add a platform to the
   published matrix until the runner exercises a real, licence-reviewed test
   model.
6. Document installation, model/config discovery, engine inventory, fallback,
   diagnostics, upgrade, and removal. Clearly distinguish helper availability
   from model availability.

## Release acceptance

A Piper companion is releasable only when all of the following are repeatable
from a clean checkout:

- every source and prebuilt native input has an immutable identity and verified
  digest, and a second build uses no undeclared network input;
- the staged archive relocates to a path containing spaces and starts without
  developer-tree library search paths;
- architecture and dynamic dependency inspection match the artifact label;
- helper hello, engine inventory, real synthesis, audio format, clean shutdown,
  crash replacement, timeout retirement, and missing/corrupt model behavior
  pass on the target platform;
- stop and keyed replacement prevent stale audio from reaching the mixer even
  when native synthesis is retired rather than cooperatively cancelled;
- cold start, first-utterance onset, long-utterance memory, and repeated restart
  measurements are recorded rather than described qualitatively;
- an unavailable Piper model leaves the platform-native/eSpeak registry and
  fallback route working; and
- the archive contains the audited notices and source-provenance material, but
  no unreviewed voice model.
