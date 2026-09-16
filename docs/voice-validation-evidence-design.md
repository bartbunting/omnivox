# Saved native-validation evidence: design review

Status: Draft. No report format or new command is implemented by this note.

The next delivery slice is saved validation evidence for the existing disposable
Piper/Flite validator. It follows [ADR 0012](adr/0012-voice-library-and-model-lifecycle.md)
and the [voice-library contract](voice-library-contract.org). Installation
transactions, restart recovery and coordinated activation remain later work.

## Existing constraints and evidence

- The installed index already has `NativeValidation`: `validator_version`,
  `target_id`, `validated_at` and `file_set_sha256`. Preserve that accepted
  schema. Detailed operation evidence can live separately from this summary.
- `--validate-voice-library` checks private per-load projections and confirms
  their process-tree and reader cleanup. It currently returns development text,
  with no persistent success record. A projection digest is distinct from the
  original generation digest.
- Both companion staging tools produce `SHA256SUMS` covering all companion
  files and `SOURCE-PROVENANCE.json`. Piper's inventory includes its native
  libraries and phonemizer data; Flite statically includes its native runtime.
  Checksums identify bytes, not a trusted publisher or the actual loaded modules.
- Piper's data selection permits `OMNIVOX_PIPER_ESPEAK_DATA` ahead of bundled
  data. It also has fallback data locations. Native-loader environment and
  platform lookup rules can affect dependency selection. Hashing just the
  helper executable or its staged directory would not cover these choices.
- Validation budgets have platform-specific meanings. The effective budget,
  deadline and probe policy must accompany the result; current defaults are
  documented in the [validator guide](VOICE-VALIDATION.md).

## Proposed direction

Use a separate, bounded report for one complete managed native-validation run.
It should record the exact original generation identity and bytes/digest,
validated model/speaker or Flite load identities, the validator identity,
effective helper/runtime/data identities, policy and limits, completion time,
and confirmed cleanup. Reports are local operational evidence, not authenticated
attestations or activation acknowledgements.

Validate each staged companion's complete checksum inventory. Reject missing,
extra, repeated or unsafe paths and mismatched bytes; include the inventory and
source-provenance file identities in the observed snapshot. Keep parsing bounded
and do not follow arbitrary paths or execute programs supplied by a saved report.
The caller supplies the current intended configuration for any comparison.

Create a success report only after every intended native load and cleanup has
succeeded, cancellation has been checked, and the relevant inputs have been
rechecked. Retain the distinction between observed file stability and immutable
inputs: before/after hashing cannot exclude an external edit followed by a revert.

Keep the existing index summary unchanged. Do not manufacture package-wide
validation from a report covering only a subset of its intended voices. The
manager must establish the package/file-set and validated-load correspondence
before attaching a summary to an installed revision.

## Questions to settle before implementation

1. **Effective runtime identity.** Define which packaged files, selected data
   roots, loader settings and platform information are required for a reusable
   result. The initial implementation could restrict reports to staged
   companions and reject unsupported overrides, or capture explicitly selected
   external data as well. Do not silently validate a different configuration.
   Define what remains outside the report's guarantee, including system
   dependencies and already-loaded executable mappings.
2. **Freshness and reuse.** Define the exact comparison against current inputs,
   and whether this first slice only reports matching observations or permits
   skipping a future native check. A current report alone must never authorize
   activation or bypass startup preflight. Binding the detailed report to the
   existing `validator_version` summary needs an explicit rule.
3. **Publication and interruption.** Choose a bounded, non-overwriting publication
   path with clear completion semantics on Windows, Linux and macOS. A partial
   record must not parse as success. Demonstrate the Windows filesystem behavior
   before claiming crash durability. Publishing a result does not implement
   operation ownership or reconciliation across manager restarts.

## Required verification

- Changed validator/helper bytes, runtime libraries, phonemizer data, selected
  data locations and policy invalidate the applicable previous evidence.
- Updated checksum files do not conceal a changed payload identity; missing or
  unlisted files, duplicate entries, path escapes and unsupported file types
  fail before native loading.
- Original generation bytes, projection identities, enabled speakers and
  package file-set identity remain distinct and correctly associated.
- Native failure, cancellation, timeout, changed inputs and unconfirmed cleanup
  cannot publish a success record or admit work through an old record.
- Malformed, duplicate-key, oversized and truncated reports are rejected.
  Existing destination files are preserved on publication failure.
- Native platform checks exercise creation, comparison, cancellation and
  interruption using the supported staged payloads and actual filesystems.

The voice-library capability remains unadvertised until the full eligibility,
status and activation contract is implemented and accepted.
