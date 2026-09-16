# Installed voices and local activation

These development commands implement the installed-state part of
[ADR 0012](adr/0012-voice-library-and-model-lifecycle.md) and the
[voice-library contract](voice-library-contract.org). They register validated
local Piper models and external Flite voices, persist desired enablement, and
prepare immutable generations for the client's explicit Apply operation.

Installation does not restart speech. New imports start disabled. Enabling a
voice changes desired state; the existing active pointer and speech processes
retain their previous configuration. Download catalogues, managed asset copying,
package updates and legacy voice-ID adoption remain separate implementation
work. The local provider now supplies owned speech workers, retained Apply
leases and active-pointer publication for Emacsvox's two-lane controller.
Speech connections advertise `voice_library_v1` when status is available.

## Local provider

The bundled Emacsvox launcher uses `--voice-library-owner` when the selected
binary's ordinary help advertises it. Each invocation owns one speech worker
and all its helpers. The native child waits behind START until Windows job or
Unix process-group ownership is established. The owner forwards speech and
control traffic and accepts private `OMNIVOX-LOCAL` records on the same local
stdin. Retirement acknowledges the exact native owner UUID only after the
worker, descendants and output reader have exited. Startup failure retains
the attempt, including a failure before any child was created, so the client
can obtain cleanup evidence before rolling back. Closing stdin also retires
the owned tree. No network listener or remote management command is added.

Local children use a distinct ownership flag for the START barrier and EOF
cancellation. They retain local file-earcon access. The remote broker's
bundled-icon restriction applies only to remote workers; otherwise a local
speech-and-earcon timeline would fail as a whole during resource preparation.
Check this against a complete staged server with
`python3 omnivox-cli/tests/check_local_earcons.py PATH_TO_OMNIVOX`; it also
verifies that remote workers continue to reject local file paths.

`--voice-library-service` accepts bounded JSON records on private local stdin.
It provisions persistent native target and default-profile UUIDs, independently
of executable releases. Windows uses `%LOCALAPPDATA%\Emacsvox\Omnivox\voices`;
POSIX uses `${XDG_DATA_HOME:-$HOME/.local/share}/emacsvox/omnivox/voices`.
An explicit `OMNIVOX_VOICE_ROOT` is a native absolute path, useful for isolated
tests. It is never inferred from the Emacs or WSL home directory. Existing or
partial initialization is retained rather than overwritten.

Private operations cover inspection, validated imports, desired enablement,
explicit inclusion of built-in Flite SLT, candidate preparation, retained Apply
ownership, activation, rollback and completion. `begin` holds the same OS lease
used by validation through the final result. Unresolved validation or Apply
records block another activation. Enabling a voice remains a separate desired
edit and does not restart any speech process.

Each owner saves a private native startup record below `sessions/`, containing
the exact executable identity, arguments, working directory, environment and
generation. The client retains only its path and hash. Restarts recheck the
record and executable; rollback does not reconstruct settings from current
Customize values or from an active pointer advanced by another session.
Candidate snapshots are resolved separately through each lane's current
launcher environment before review and admission. In particular, clearing a
file override affects the candidate without changing the old worker's rollback
record. A failed preflight leaves that old worker and its settings intact.
The launcher distinguishes its Piper fallback from explicit file settings;
a managed candidate may replace that fallback, while explicit overrides remain
visible and must be resolved before the client's preflight permits retirement.

## Native profile ownership

The provider selects the native root and target/profile identities. Reuse the
profile initialized by `--prepare-voice-admission`, as described in
[validation operations](voice-operation-journal-design.md). Installation retains
that same OS lock, so it cannot race a cooperating native validator or another
profile writer. It does not infer the speech host from the manager's home path.

The installer adds these files under `profiles/PROFILE_UUID/`:

| Path | Meaning |
| --- | --- |
| `index.json` | Current desired installed and enabled state. |
| `index-revisions/REVISION_UUID.json` | Exact immutable index snapshots. |
| `imports/INDEX_REVISION_UUID.json` | Import receipt binding the validation operation, package and before/after index digests. |
| `generations/GENERATION_UUID.json` | Immutable runtime projection for selected providers. |
| `candidates/GENERATION_UUID.json` | Preparation binding that generation, desired index and previous active-pointer bytes. |
| `activations/OPERATION_UUID/` | Frozen reviewed plan, candidate, immutable state records, verified pair and completion receipt. |
| `active.json` | Atomically published configuration after successful verification of both lanes. |

Every edit names the expected SHA-256 of the exact current index and a fresh
revision UUID. Stale edits fail without replacing current state. The new revision
and a temporary index file are written and synchronized before a same-directory
rename publishes `index.json`. Earlier revisions remain intact. A failed write
or interrupted publication retains partial/unreferenced files; it never repairs
them by overwriting or silently retries using the same revision ID. Reopen and
inspect the current index after an uncertain result. Initial creation also uses
non-overwriting writes. Unix synchronizes the containing directories.

These are process-level publication primitives. Stronger power-loss durability,
automatic reconciliation of partial receipts and retention/garbage collection are
additional hardening. They do not block ordinary installation and activation
development. The existing refusal to reuse unresolved native validation still
applies; installation never signals saved PIDs or clears validation claims.

## Import requirements

An import names an admitted, successfully staged validation operation on the same
target/profile and native platform. The installer rechecks the exact journal,
bound evidence and worker-file digests, then verifies asset sizes and hashes.
A prepared, cancelled, abandoned, damaged or unfinished operation cannot install
a voice. A matching standalone report is insufficient.

The operation must select exactly one external load: one Piper model with its
validated speakers, or one external Flite file. Piper uses an explicit import
UUID and canonical speaker IDs; catalogue identities and implicit legacy
adoption are rejected by this local-import command. Flite retains the validated
native voice ID, and its package UUID becomes its persistent import identity.
The package and revision UUIDs must be new for this profile. All imported voice
rows start disabled, including every speaker of a multi-speaker model.

Imports retain their original external paths and files. The index records the
original validation time and exact observed validator digest as its validator
identifier, rather than claiming validation by a later installing executable.
The file-set digest remains bound to the declared paths and bytes. Imported files
can change afterwards; candidate preparation and native loading recheck them.

## Development commands

All UUID arguments below are canonical lowercase UUIDs generated by the caller.
`ROOT` and asset paths use the native speech host's filesystem conventions.

```sh
omnivox --initialize-voice-library ROOT PROFILE_UUID INDEX_REVISION_UUID
omnivox --inspect-voice-library ROOT PROFILE_UUID
omnivox --import-validated-voice ROOT PROFILE_UUID VALIDATION_OPERATION_UUID PACKAGE_UUID PACKAGE_REVISION_UUID NEW_INDEX_REVISION_UUID EXPECTED_INDEX_SHA256
omnivox --set-library-voice-enabled ROOT PROFILE_UUID ENGINE_ID VOICE_ID true NEW_INDEX_REVISION_UUID EXPECTED_INDEX_SHA256
omnivox --stage-voice-library-activation ROOT PROFILE_UUID GENERATION_UUID piper EXPECTED_INDEX_SHA256
omnivox --inspect-voice-library-activation ROOT PROFILE_UUID GENERATION_UUID
```

Inspection returns the exact index bytes, without an added newline, so a client
can calculate the next expected digest from its response. Enablement accepts
`true` or `false`; unknown voices are rejected. Installation does not rewrite
palettes or make a disabled voice eligible on existing speech processes.

Activation preparation accepts `piper`, `flite`, or `both` for the providers to
manage. Other providers keep legacy startup. A selected provider with no enabled
voices gets an explicit empty load set. Enabled Piper speakers sharing one model
produce one model entry; selecting different revisions of that model is rejected.
Flite includes only enabled indexed voices, including SLT only when explicitly
represented by an enabled built-in row. Missing or stale enabled inputs fail
preparation; disabled inputs need not be opened.

The candidate records the exact generation identity/digest, desired index
revision/digest and previous `active.json` bytes, or null when no active pointer
exists. Reinspection rejects changes to any of them and rechecks enabled assets.
Preparation never creates or replaces `active.json`. The retained local Apply
transaction rechecks it, the index and generation before retirement and again
before publication. It requires both lane identities and correlated readiness,
generation and eligibility receipts. The client binds these records to actual
owned connections; JSON alone is not native process authority. The pointer is
published by same-directory replacement only after both lanes verify. A failure
after publication begins is uncertain until inspected, never an instruction to
restart the old pair. Unfinished or failed-recovery journals remain blocking.

`tools/verify_local_voice_owner.py SERVER` checks native identity persistence,
profile exclusion, desired edits, owner identity, startup refusal and confirmed
retirement without playback. Emacsvox's isolated live acceptance also exercises
paired Apply and rollback through its ordinary routing/registration path. Native
Windows staging and the native macOS CI jobs remain the platform acceptance
paths; a Linux-only run does not establish those results. Additional power-loss
durability, interrupted-Apply reconciliation and retention cleanup are follow-up
hardening, rather than a prerequisite for ordinary installation and Apply.

## Verification scope

Component tests cover index replacement and retained revisions, competing owners,
stale edits, changed assets, immutable generations, initial/previous active
pointers, invalidated candidates and enabled-only projection. The native voice
probe additionally installs admitted Piper and external Flite fixtures, verifies
that both start disabled, rejects changed evidence/assets, saves enablement,
and validates the resulting candidate with actual native helpers without playback.
These checks do not establish live Emacs activation or audible acceptance.

### Installation acceptance

At source commit `f04184c8ac57194cb58c182a2989d5a995c96666`, Linux passed
73 voice-library tests and the full locked workspace run (816 passed, one
existing ignored test). Supported server staging, the silent Piper/Flite import
and candidate-validation probe, the operation-command probe and workspace Clippy
with the Piper features also passed.

Native Windows x64 GNU passed all 70 applicable voice-library tests from its
native temporary filesystem, including index replacement and candidate checks.
Windows-target Clippy passed. This is shared storage/projection acceptance;
full Windows server/companion installation validation and MSVC acceptance remain
separate work.

Native Intel and Apple Silicon macOS passed at the same source commit in
[run 35056626404](https://github.com/bartbunting/omnivox/actions/runs/35056626404).
Each host passed six installed-state tests, four projection tests, eight evidence
tests, 26 operation tests and 13 supervision tests repeated five times. Supported
native staging, Clippy, the full silent Piper/Flite import and candidate-validation
probe and the operation-command probe also passed. The native voice probe retained
the existing failure, cancellation and ownership-recovery checks on both hosts.
