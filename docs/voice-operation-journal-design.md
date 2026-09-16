# Validation operation journals

Status: Development implementation boundary, 2026-09-16.

This implements development validation operations under
[ADR 0012](adr/0012-voice-library-and-model-lifecycle.md) and the
[voice-library contract](voice-library-contract.org). It provides preparation,
profile admission, native execution, exclusive operation ownership and recovery
inspection and abandonment using recorded cleanup. It does not publish packages,
change an index/active pointer, or restart speech. Interrupted native work without
complete worker cleanup records remains blocked.

## Ownership and frozen input

Omnivox owns these management primitives, alongside its shared library formats;
the CLI exposes development preparation and inspection. Helpers do not call the
storage primitives. The caller supplies an existing operations directory and a
locally generated operation UUID. Creating an operation adds only its new UUID
subdirectory; existing paths and partially created operations are retained.
Target-root provisioning and host-identity resolution remain provider work.

The immutable `plan.json` is a strict `native_validation` request containing:

- schema version and canonical operation UUID;
- host platform and exact generation JSON, retaining target/profile/generation
  IDs, selected models/speakers and declared asset hashes;
- the intended validator and exact helper paths for nonempty native load sets;
- deadline, memory budget and the `bundled-companions-v1` runtime policy.

Paths are ordinary absolute native paths under the shared contract. Preparation
parses metadata without opening these paths. Admission must still verify the
actual target/profile, runtime bytes, environment and assets. This is a validation
request, not the larger activation plan that freezes index revision, previous
active pointer, session workers, overrides and rollback configurations.

The directory also contains a permanent `owner.lock` and `journal.frames`.
Omnivox uses a nonblocking OS file lock and keeps its descriptor private to the
owner. Contention reports busy. Never delete, rename or replace the lock file,
or deliberately duplicate or pass its descriptor to children: ownership must
remain in one lock domain.
[The standard library's lock contract](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)
provides cross-process exclusion. Normal retirement explicitly unlocks before
closing the file. On Unix a concurrent fork can briefly duplicate the descriptor
until exec closes it; after a crash, inspection may remain busy until those
copies close too. Acquiring the lease still does not establish native cleanup.
Locks coordinate cooperating managers; they do not protect against the user
rewriting or replacing state files outside the service.

## Records and bounds

Each frame is one compact UTF-8 JSON record, newline, the lowercase SHA-256 of
those exact JSON bytes, and a final newline. The record includes schema,
sequence, previous digest, exact plan digest, diagnostic writer PID/time and a
typed state transition. The first previous digest is the plan digest; subsequent
records chain to the previous record's digest. Digests establish consistency,
not authentication. A stored PID is never authority to signal a process.

Plans are limited to 8 MiB, with the embedded generation retaining its 1 MiB
limit. A record is limited to 8 KiB and a journal to 1,024 frames. Strict readers
reject duplicate JSON keys, unknown fields and inconsistent identities, ordering,
states or cleanup claims. Inspection retains the valid prefix and identifies a
damaged suffix. It never truncates or repairs bytes. Missing/malformed initial
files produce an inspection error and remain untouched.

Before an append, reread the selected plan and journal and require the previously
observed bytes. Append the complete frame and call `sync_all` before returning
success; any write/sync failure poisons that writer. A complete frame may remain
visible after a lost response or sync error; inspection does not prove that the
caller received a successful acknowledgement. Initial Unix creation also
synchronizes directory entries. These are process-crash recovery primitives,
not a demonstrated power-loss-safe multi-file transaction on every provider.
Windows directory publication, filesystem/device failure recovery and durable
active-pointer replacement remain separate acceptance work. File I/O may block;
only lock acquisition itself is nonblocking.

## Recovery meanings

| Last complete state | Cleanup statement | Inspection after the owner exits |
| --- | --- | --- |
| `prepared` | Native work not started | May reopen for preparation; admission still required. |
| `validating` | Unconfirmed | Interrupted; append/reuse blocked. |
| `staged` | Confirmed, with evidence digest | Validation result recorded; no further writes or activation implied. |
| `cancelled` or `failed` | Not started, or confirmed after validation | Terminal; no further writes. |
| `recovery_failed` | Unconfirmed | Cleanup failure retained; append/reuse blocked. |
| Any incomplete/damaged suffix | Unknown | Damaged; append/reuse blocked even after an otherwise terminal prefix. |

The initial transition is `prepared`. Only a current owner can record
`validating`, and it must do so before native work. That same owner can record
`staged` after successful validation/cleanup, or a bounded failure/cancellation
outcome with the appropriate cleanup statement. `staged` requires an evidence
digest, but this journal layer does not independently verify a report or its
operation correspondence. The integrating supervisor must establish both.

Losing the lock proves only that no owner holds that lease. It does not prove
that helpers, descendants or readers finished. Therefore a new process that
acquires a `validating` or damaged operation can inspect it but cannot append a
terminal result or restart it. Explicit recovery may add a separate abandonment
receipt when all retained worker cleanup records are complete, as described below.
Automatic PID-based termination, report-based completion, journal reset and speech
restart are deliberately absent from recovery inspection.

## Development commands

```sh
omnivox --prepare-voice-validation /absolute/request.json /absolute/operations
omnivox --inspect-voice-operation /absolute/operations/OPERATION_UUID
omnivox --prepare-voice-admission ROOT TARGET_UUID PROFILE_UUID
omnivox --inspect-voice-admission ROOT PROFILE_UUID
omnivox --run-voice-validation-operation ROOT PROFILE_UUID OPERATION_UUID
omnivox --recover-voice-validation-operation ROOT PROFILE_UUID OPERATION_UUID
```

Preparation writes `prepared` and exits; it does not execute the saved request.
Inspection obtains a temporary lease for stable reads and changes no file
contents. All commands run before audio/engine startup. Their text output is
for development, not a negotiated management API.

## Profile admission and native execution

The provider selects one root containing `operations/` and
`profiles/PROFILE_UUID/`. These directories must already exist. Explicit gate
preparation creates only `profiles/PROFILE_UUID/validation-admission/`, containing
an immutable target/profile `identity.json`, permanent `owner.lock`, and `claims/`.
It never repairs or replaces an existing or incomplete gate. This pairs a profile
with the provider-supplied target identity; it does not discover or authenticate
the speech host. Cooperating managers must use the same provider-selected root.

An admitted run retains the profile lease and the operation lease. Before
starting, it reads every retained claim and its operation. Claims contain the
operation UUID and exact plan digest, with a checksum of their original bytes.
Creating a new claim uses non-overwriting creation and file synchronization;
Unix also synchronizes the directory. History is limited to 4,096 claims and is
never pruned automatically. Full history needs an explicit retention review.

Only intact `staged`, `cancelled`, `failed` or verified `abandoned` operations allow
a different operation to proceed. Claimed `prepared` work may resume only under that same
operation ID. Busy, validating, recovery-failed, damaged, missing or mismatched
history blocks admission. A different UUID cannot bypass such history. A complete
report beside an interrupted journal does not release the claim. Inspection
acquires no cleanup authority and never edits history or signals stored PIDs.

The run command checks the host platform and the planned validator against the
current executable, resolves the planned helpers and rejects unsupported runtime
overrides. It records `validating` before native work and retains the original
generation under the operation. The existing bounded supervisor performs input
observations and native Piper/Flite probes, one load at a time, without playback.

For every observation or native worker, retained files under `workers/` record:

1. Spawn intent, including exact program and arguments, before process creation.
2. Assignment to the supervisor's private process group or job, before `START`.
3. Confirmed process-tree and reader cleanup, before admitting another worker.

These are bounded, non-overwriting files synchronized before acknowledgement.
A failed ownership write leaves `START` closed. Failed or incomplete recording
keeps work unresolved even if an unrecorded cleanup attempt later succeeds.
Each event is limited to 128 KiB, with at most 256 workers. The bound report names
and hashes every event in intent/owned/cleaned order and requires exactly the
selected native loads plus the two input-observation workers.

The saved `validation-evidence.json` binds the operation UUID, exact plan digest,
original native evidence and complete worker-record manifest. Its 18 MiB bound
allows the existing 8 MiB native report to be embedded without losing exact bytes.
It is written and synchronized before the terminal journal frame records its
digest. Same-generation evidence from another attempt fails operation binding.
These records remain unauthenticated observations of the trusted supervisor.

Normal failures or cancellation record terminal state only after confirmed
cleanup. An interrupted manager leaves its claim blocked even if all workers
subsequently exit. Worker PIDs describe the live supervisor's observed assignment;
they are explicitly marked `live-supervisor-only`, not reusable boot/birth
identities. There is no automatic restart, PID signalling or force-clear command.
Missing cleanup observations still require provider-specific recovery evidence.

## Abandonment using recorded cleanup

The explicit recovery command acquires the profile and operation leases, checks
the existing claim and exact plan binding, then examines an intact `validating`
journal and its initialized `workers/` directory. Each worker must have a complete
intent, ownership and cleanup triple in sequence. Every event must match the
operation, plan, platform ownership and phase. Missing, partial, unknown or
inconsistent events block recovery. The same 256-worker and 128 KiB per-event
bounds apply; the count cannot exceed the planned native loads plus the two input
observations. Strict readers reject duplicate keys and omitted nullable fields.

An empty initialized worker directory also qualifies: the supervisor must persist
intent before any process creation. A missing directory does not qualify. A
complete prefix qualifies only when there is no outstanding next-worker intent.
This trusts the original supervisor's recorded process-tree and reader cleanup;
it does not discover live processes, signal stored PIDs or execute saved paths.

Successful recovery creates `cleanup-recovery.frames`, leaving the journal, report,
claim and worker files untouched. The receipt is one checksummed JSON frame, at
most 128 KiB. It records schema 1, outcome `abandoned`, basis
`completed-worker-records-v1`, the operation ID, exact plan and original journal
digests, and the ordered names and digests of every worker event. Creation never
overwrites a file and synchronizes it, plus its directory on Unix. The same
process-crash versus power-loss limitations described above apply.

Every subsequent opening checks the receipt against the original journal and the
complete current worker history. A valid receipt yields inspection state
`Abandoned`; the original journal still says `validating` and cannot be appended
to. Repeating recovery verifies the existing receipt without rewriting it. This
handles a lost acknowledgement. A partial receipt or changed evidence blocks
admission and is retained for further recovery; there is no overwrite or force
clear. Other unresolved claims still block new work independently.

Abandonment allows admission of a new operation ID. It does not authorize reuse of
the old attempt, native-check skipping, report promotion, installation or
activation. A missing or partial native report is retained but is irrelevant to
cleanup reconciliation. Damaged journals, `recovery_failed` operations and workers
without recorded cleanup remain blocked even if their old processes have exited.

The standalone `--validate-voice-library` diagnostic retains its existing behavior
and does not participate in profile admission. The operation command is the path
for admitted management work. The voice-library capability remains unadvertised.

The [command probe](../tools/verify_voice_operations.py) checks exact input
preservation, refusal to overwrite, damaged-journal inspection and absence of
native loading. Shared component tests kill a real writer in prepared,
validating, torn-completion and staged states. They check lock release separately
from cleanup/reuse, as well as changed/replaced files and incomplete initialization.
A Unix regression test also holds a forked descriptor before exec and verifies
that normal owner retirement releases the lease without waiting for that child.

## Verification

### Original per-operation foundation

For the original per-operation foundation at source
`f10a32f3b32c72ad9b79accd3019f49b7bee65b2`, Linux passed all 11 shared
operation tests, the staged command probe and the locked workspace suite
(785 passed, one existing ignored test). Workspace Clippy with Piper features,
formatting and local documentation-link checks also passed. Native Windows x64
GNU passed all nine applicable operation tests on its native temporary filesystem,
including real owner termination and interrupted/torn-journal inspection.
The Unix-only tests cover symbolic links and descriptor inheritance across fork.

Native Intel and Apple Silicon macOS passed at the same source in
[verification run 35045813419](https://github.com/bartbunting/omnivox/actions/runs/35045813419).
Each host passed all 11 operation tests and the staged command probe, together
with the workflow's repeated supervisor tests, saved-evidence checks, full native
Piper/Flite validation probe and Clippy gate. The deterministic fork regression
passed on both architectures.

These checks establish this storage slice's behavior, not persistent ownership
of the native validator or full installation/activation recovery. Full Windows
server/companion and MSVC acceptance remain separate, as recorded in
[the validator guide](VOICE-VALIDATION.md).

### Admitted native execution

At `cdffab6533a8bfdce51d2b0137bb9d1787109984`, Linux passed the locked workspace
suite (796 passed, one existing ignored test), including 20 operation/admission
tests and seven supervisor/command tests. The staged native probe passed with
Piper speakers, compiled-in SLT and an exported external Flite voice. It verifies
sequential admitted runs, per-attempt evidence binding, confirmed cancellation,
blocked admission after manager death and refusal to treat a report as completion
when the final journal append is missing. Workspace Clippy with Piper features,
formatting and local documentation-link checks passed.

Native Windows x64 GNU passed all 18 applicable operation/admission tests and ten
supervisor/command tests on its native temporary filesystem. These include killed
profile owners and refusal to open START after an ownership-record failure.
The subsequent import-only portability cleanup at `347684c` passed Windows-target
Clippy for the shared library and CLI with Piper discovery enabled, plus a Linux
shared-library compile check. Full Windows server/companion and MSVC acceptance,
power-loss recovery, speech playback and activation remain separate.

Native Intel and Apple Silicon macOS passed at `cdffab6` in
[verification run 35048372336](https://github.com/bartbunting/omnivox/actions/runs/35048372336).
Each host passed all 20 operation/admission tests, the repeated supervisor tests
including refusal to open START after a recording failure, native Clippy, the
full Piper/Flite probe including external Flite, and the preparation/inspection
command checks. The probe verified both profile release after confirmed
cancellation and blocked admission after manager death or a lost terminal append.

## Remaining recovery work

Profile admission now blocks the spawn/record crash gap, and reports are bound to
the operation and request. Complete saved cleanup can release an interrupted
claim through explicit abandonment. Work with missing cleanup records still needs
provider-specific boot/process-tree identity and confirmed absence, not a reusable
numeric PID or a matching report from another attempt. That reconciliation must
also account for incomplete worker records and damaged journals/receipts without
inventing success.

Only after those boundaries work should installation transactions and full
candidate startup/activation use the journal. The voice-library capability
remains unadvertised.
