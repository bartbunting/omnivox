# Saved native-validation evidence

Status: Accepted implementation boundary, 2026-09-16. Development format 1.

This follows [ADR 0012](adr/0012-voice-library-and-model-lifecycle.md) and the
[voice-library contract](voice-library-contract.org). Omnivox owns capture,
comparison and publication. Helpers still only load and synthesize voices;
they do not own reports, catalogues or downloads.

## Meaning and scope

A report records a successful silent native-validation run and the inputs
observed before and after it. Matching observations help diagnose whether those
inputs have changed. They do **not** permit skipping another native check,
authorize activation, acknowledge a running generation, or establish recovery
ownership after an interrupted manager operation. Reports are unauthenticated
local metadata, not publisher signatures or attestations of loaded modules.

Keep the accepted installation-index `NativeValidation` schema unchanged.
This slice neither populates it nor defines a reusable `validator_version`
token. A later manager must establish the package/file-set and validated-load
correspondence before attaching that summary. Testing enabled speakers of one
Piper model does not validate every voice of its installed package.

## Observed identities

The shared TTS library contains strict metadata readers and explicit capture.
Each snapshot includes:

- Exact original generation JSON bytes and their SHA-256. The embedded generation
  retains target, profile, generation, asset and model identities.
- Each private native-load projection's digest and every tested physical voice.
  One Piper projection covers all its enabled speakers; Flite loads are separate.
- Canonical validator path, size and SHA-256 of the executable on disk.
- Canonical helper paths and every file's relative path, size and SHA-256 in each
  selected staged companion, including `SHA256SUMS`, `SOURCE-PROVENANCE.json`,
  native libraries and Piper's bundled phonemizer data.
- OS, architecture, probe/cleanup policy version, platform memory-policy version,
  memory limit and per-worker deadline. Working directory and `PATH`, `SystemRoot`
  and `WINDIR`, when present, record ordinary search configuration.

A completed report additionally records schema/kind, completion time as Unix
seconds, and confirmed cleanup. Comparison requires equality of the whole input
snapshot. Paths and search configuration are deliberately part of the identity:
relocation requires a new observation even when file bytes are identical.
Canonical path labels may include Windows' verbatim-path prefix. They are
observations, not asset-path inputs for the runtime or installation index.

Reports are limited to 8 MiB on read and serialization. Embedded generations
retain their 1 MiB limit. Companion manifests and provenance are limited to
2 MiB each; directory traversal is limited to 8,192 entries and 16 path
components. Reads hash with a fixed 64 KiB buffer. Parsing rejects duplicate JSON
keys, unknown fields, incomplete success records and inconsistent generation or
load identities. Reading a report never opens or executes its stored paths;
the caller supplies the current generation and helper locations for capture.

## Supported runtime context

Initial capture requires the supported staged companion layout. Check the
complete checksum inventory against the actual file tree, then verify each
file. Reject missing, unlisted, repeated, case-aliased or unsafe entries,
symlinks/reparse points and unsupported file types. Check source-provenance
schema, engine family and native target OS/architecture. Separate helpers may
use a different compiler ABI, including Windows GNU/MSVC. Updating a checksum file cannot
hide a payload change from comparison with an earlier report.

Piper must have its bundled native libraries and `espeak-ng-data/phontab`, with
no competing adjacent `phontab`. Saving or comparing rejects nonempty
`OMNIVOX_PIPER_ESPEAK_DATA`, `ESPEAK_NG_DATA`, `LD_*`, `DYLD_*`, `_RLD*`, `LIBPATH`
and `SHLIB_PATH`. Do not silently sanitize an unsupported runtime selection.
Ordinary native validation remains available without saving evidence.

These checks identify observed packaged bytes and selected configuration, not
all dependencies actually loaded by the OS. System libraries, loader caches,
OS patch levels, injected modules and already-mapped executable images are
outside this guarantee. In particular, hashing the current executable path
cannot attest the supervisor's mapped image after a concurrent replacement.
Companion checksums and provenance can themselves be edited; their inclusion
provides identity, not trust. These limits are why matching observations cannot
be a validation cache or activation prerequisite on their own.

## Supervision and publication

Run both input observations in disposable owned workers with the validator's
memory budget, deadline, cancellation and confirmed-cleanup rules. The first
observation precedes native loading. After every native probe succeeds, repeat
asset verification and capture, require equality, and remove owned scratch
files before publishing. Native failure, timeout, cancellation, mismatched
inputs or unconfirmed cleanup cannot reach publication. Before/after reads
cannot exclude an external edit followed by a revert, or later mutations.

Write a uniquely named temporary file beside the requested destination, flush
and synchronize it, close it, then check cancellation. Create the destination
using a same-directory hard link; this is the publication commit point and
never replaces an existing name. Unsupported filesystems fail closed. On Unix,
new report files have mode 0600. Remove the temporary name on ordinary exit;
a process killed before cleanup may leave it behind.

Cancellation observed before the commit point prevents publication. Cancellation
or supervisor death after that point does not revoke the report. Readers can
see only the fully written published record, but the report is **not a
power-loss-durable transaction**: directory-entry persistence and restart
reconciliation are not established. File publication may block on the selected
filesystem; it is outside the owned-worker deadline. Never interpret a missing
command response, stray temporary file or saved report as proof of operation
ownership or cleanup in another invocation.

## Delivery and verification

The shared evidence layer is the first implementation slice. The development
validator then exposes optional save and compare operations, documented in the
[validator guide](VOICE-VALIDATION.md). Component tests cover content/policy
changes, unsafe/incomplete inventories, exact speaker/generation identities,
bounded parsing and non-overwriting publication. Native integration checks
exercise real Piper and Flite validation, saving, comparison and refusal after
changed inputs or native failure.

Native Windows filesystem/component checks and both native Mac architectures
must be distinguished from full Windows server/companion acceptance and
power-loss recovery. Storage transactions, interrupted-operation ownership,
full candidate startup/status and coordinated two-lane rollback remain later
work. The voice-library capability remains unadvertised.
