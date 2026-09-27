# Managed voices: installation, activation and removal

These local commands implement the installed-state part of
[ADR 0007](../adr/0007-managed-voice-lifecycle.md) and the
[voice-library contract](../reference/voice-library.org). They register validated
local Piper models and external Flite voices, persist desired enablement, and
prepare immutable generations for the client's explicit Apply operation. The
local service also acquires reviewed downloads for Piper, Flite, MBROLA and
RHVoice; runtime availability and platform support remain provider-specific.

Installation does not restart speech. New imports start disabled. Enabling a
voice changes desired state; the existing active pointer and speech processes
retain their previous configuration. Emacsvox supplies reviewed download
catalogues; Omnivox handles acquisition, managed storage and native validation.
Package updates and implicit legacy voice-ID adoption remain separate work.
The local provider supplies owned speech workers, retained Apply
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
[validation operations](../reference/voice-operation-journals.md). Installation retains
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
durability and interrupted-Apply reconciliation are follow-up
hardening, rather than a prerequisite for ordinary installation and Apply.
Explicit [managed voice uninstallation](voice-management.md#managed-voice-uninstallation) reviews package
ownership and retained references before removing downloaded files.

## Verification scope

Component tests cover index replacement and retained revisions, competing owners,
stale edits, changed assets, immutable generations, initial/previous active
pointers, invalidated candidates and enabled-only projection. The native voice
probe additionally installs admitted Piper and external Flite fixtures, verifies
that both start disabled, rejects changed evidence/assets, saves enablement,
and validates the resulting candidate with actual native helpers without playback.
These checks do not establish live Emacs activation or audible acceptance.

Dated installation checks are preserved in the
[voice-management evidence report](../benchmarks/2026-09-27-retained-voice-management-results.md).
See [STATUS.md](../STATUS.md) for current qualification and remaining acceptance.

## Managed voice uninstallation

The development local service removes reviewed Piper, Flite, MBROLA and RHVoice
downloads under the ownership rules in [ADR 0007](../adr/0007-managed-voice-lifecycle.md).
Emacsvox supplies the accessible
review and confirmation. The remote speech socket has no removal operation.

Removal is package-wide: all speakers sharing a Piper model appear in the
review, and all must be disabled. Apply that disablement before uninstalling.
Built-in Flite SLT, MBROLA en1, system voices, imported files, engine runtimes
and the MBROLA frontend are outside removal's ownership. Saved palette choices
and disabled physical IDs remain; downloading the same voice again preserves
its physical identity and starts disabled.

### Reference and ownership checks

The review binds an immutable operation, original index revision and digest,
package/revision UUIDs, catalogue and file hashes. Execution repeats the checks
under native storage and profile leases. Other profiles' references, active
generations, incomplete validation or Apply (including rollback), and startup
snapshots lacking retirement evidence retain the files. Unknown or malformed
references block cleanup. Native startup snapshot publication and removal share
a permanent storage lock, so new managed owners cannot appear between the
reference check and deletion.

An owned speech process records retirement only after its entire worker tree
and output readers have finished. An idle process, disconnected client or
released OS lock is insufficient. Prepared startup snapshots are separately
identified; using one starts a new independently recorded owner. Snapshots from
older binaries have no retirement receipt and conservatively retain their
referenced files. There is no PID-based force cleanup or automatic restart.

Package paths must be the exact installer-owned location and contain only the
catalogue's fixed filenames plus its retained catalogue. Content hashes and
sizes are rechecked; unexpected files, links and changed content retain the
package. These checks protect normal managed operations, not arbitrary external
programs concurrently rewriting a user's private storage.

### Interruption and reporting

Execution first publishes an index without the package and its voice rows,
then unlinks verified files individually. Original index revisions, review and
deletion receipts remain. Interruption before index publication preserves the
installed package; interruption afterward leaves an explicit resumable cleanup.
Retry uses the same retained plan and rechecks current references, including
any new installation sharing files. It never recursively deletes unexpected
content or deletes files outside the managed revision.

Results distinguish blocked, partial and complete cleanup, confirmed removed
file bytes, remaining bytes and absent bytes without a confirmed deletion
receipt. A lost receipt after unlinking does not become claimed savings on
retry. Logical file bytes are not a measurement of filesystem free space,
compression, retained external file handles or RAM. Interrupted or incomplete
metadata writes may require inspection; stronger power-loss guarantees remain
separate work.

### Private local interface and checks

`host` advertises `removal_version: 1`. Older services remain usable for their
existing commands; clients require this capability before offering removal.
The private stdio commands are:

- `uninstall-preview`: engine, physical voice and expected index digest; returns
  the frozen review and current blockers without deleting or detaching anything.
- `uninstall`: operation UUID and reviewed plan digest; repeats reference and
  ownership checks and returns the cleanup outcome.
- `uninstall-pending`: retained operations whose index detachment began and
  whose cleanup has no completion receipt, with fresh blocker information.

Run the focused removal tests with
`cargo test --locked -p omnivox-tts voice_library::installation::removal`.
`tools/verify_voice_removal.py` uses reviewed catalogues in private native roots
to check actual acquisition, two independently owned speech processes, blocked
cleanup until both retire, removal and verified reinstallation. Supply the
explicit development MBROLA helper for MBROLA checks. On Windows it also holds
a file open against deletion, checks the partial result and detached index,
then releases the handle and resumes the same operation. Its speech output is
null; these are storage/lifecycle checks, not listening acceptance.

Dated removal observations are preserved in the
[voice-management evidence report](../benchmarks/2026-09-27-retained-voice-management-results.md).
See [STATUS.md](../STATUS.md) for current platform qualification.
