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
| Any incomplete/damaged suffix | Unknown until independent cleanup verification | Damaged; append/reuse blocked. Explicit abandonment is possible only with a verified `validating` prefix and complete worker cleanup. |

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
receipt when the verified prefix ends at `validating` and all retained worker
cleanup records are complete, as described below. This can include a damaged final
append; the damage remains part of the retained evidence.
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

The run command starts a separate supervisor from the same executable. That
supervisor checks the host platform and the planned validator against the
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
cleanup. An interrupted supervisor leaves its claim blocked even if all workers
subsequently exit. Worker PIDs describe the live supervisor's observed assignment;
they are explicitly marked `live-supervisor-only`, not reusable boot/birth
identities. There is no automatic restart, PID signalling or force-clear command.
Missing cleanup observations still require provider-specific recovery evidence.

## Keep cleanup ownership through manager death

The development run command is a small manager. A separate invocation of the same
executable owns the profile and operation leases, native process handles, output
readers, deadlines and journal writes. It runs only the existing validation path,
before audio or engine startup. Helpers retain their existing engine boundary.

The manager starts that supervisor with a private stdin pipe and sends exactly
`START` plus a newline. The internal supervisor entry rejects missing or malformed
startup before acquiring admission or opening a saved request. Its existing
cancellation watcher then observes EOF, unexpected input or read failure. The
manager closes this pipe when its own input closes; process death closes it in
the kernel. It keeps the write end private, forwards no voice-library bytes over
the pipe, and waits for the supervisor's exit without forcibly terminating it.
Native workers still have their own separate startup/cancellation gates.

On Unix the supervisor starts in its own process group. On Windows it uses
[`CREATE_NO_WINDOW`](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)
so it is not attached to the manager's console. These isolate ordinary client
lifetime; they do not bypass an enclosing Windows job or survive a whole-session,
whole-tree or host shutdown. Progress and diagnostics inherit the manager's output
destinations. A closed output pipe can still prevent progress reporting; recorded
cleanup and retained history, rather than receipt of that text, govern admission.

When the manager dies during validation, the surviving supervisor cancels native
work, verifies tree and reader cleanup, records cancellation, then releases its
leases and exits. Until then another manager sees the profile as busy. New work
uses normal admission after confirmed cleanup. A caller that lost contact must
inspect the retained result; client exit alone is not a cleanup acknowledgement.

| Failure | Result |
| --- | --- |
| Manager disappears before `START` | Supervisor exits without admission or native work. |
| Manager disappears during native work | Supervisor attempts bounded native cleanup and records the outcome. |
| Native cleanup cannot be confirmed | Recovery-failed history continues to block admission. |
| Supervisor also dies before completion | Existing recorded-cleanup recovery applies; incomplete cleanup remains blocked. |

The internal supervisor switch is an implementation/test entry, not a negotiated
management API or an independently restartable operation. This adds no daemon,
installed service, new artifact, native process-identity authority or automatic
speech restart. Existing incomplete histories cannot gain cleanup evidence merely
because this manager/supervisor split was installed.

## Abandonment using recorded cleanup

The explicit recovery command acquires the profile and operation leases, checks
the existing claim and exact plan binding, then examines a journal whose verified
prefix ends at `validating` and its initialized `workers/` directory. Each worker
must have a complete intent, ownership and cleanup triple in sequence. Every event
must match the operation, plan, platform ownership and phase. Missing, partial,
unknown or inconsistent events block recovery. The same 256-worker and 128 KiB per-event
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
`completed-worker-records-v1` for an intact journal, the operation ID, exact plan
and original journal digests, and the ordered names and digests of every worker
event. Creation never overwrites a file and synchronizes it, plus its directory
on Unix. The same process-crash versus power-loss limitations described above apply.

Every subsequent opening checks the receipt against the original journal and the
complete current worker history. A valid receipt yields inspection state
`Abandoned`; the original journal's verified prefix still ends at `validating`
and cannot be appended to. Repeating recovery verifies the existing receipt
without rewriting it. This handles a lost acknowledgement. A partial receipt or
changed evidence blocks admission and is retained for further recovery; there is no overwrite or force
clear. Other unresolved claims still block new work independently.

Abandonment allows admission of a new operation ID. It does not authorize reuse of
the old attempt, native-check skipping, report promotion, installation or
activation. A missing or partial native report is retained but is irrelevant to
cleanup reconciliation. Journals without a verified `validating` prefix,
`recovery_failed` operations and workers without recorded cleanup remain blocked
even if their old processes have exited.

### Supervisor loss during the final append

After the last worker cleanup record is synchronized, the supervisor can die
while writing its terminal journal frame. The valid prefix still ends at
`validating`, but the suffix is damaged. Explicit recovery can abandon this
attempt using the same complete worker-history verification. It does not infer
success from the partial frame or a saved report.

For this case the receipt uses basis
`completed-worker-records-damaged-journal-v1`. It hashes the entire original
journal, including the damaged suffix. Recovery does not truncate, repair or
append to that journal. Later inspection reports `Abandoned` only after verifying
the receipt and all original bytes again; changing even the uninterpreted suffix
invalidates recovery. Older readers reject this basis rather than accepting
evidence they cannot verify.

Damage to the initial or validating frame still blocks recovery, as does a
damaged suffix after a terminal prefix. An outstanding worker intent or missing
cleanup record also blocks it. This handles a lost final write after recorded
cleanup; it provides no new process-tree authority after an active-worker crash.

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

### Recorded-cleanup recovery verification

The recovery implementation and native probe are committed in `a76d403` and
`39ecbe2`; `eb8b376` fixes only preservation-test portability. Linux passed the
locked workspace suite (800 passed, one existing ignored test), workspace Clippy
with Piper features, formatting, documentation links and the metadata command
probe. All 24 operation/admission tests passed again after the test-only fixes.
The staged silent Piper/Flite probe, including an exported external Flite voice,
verified explicit abandonment after a lost final journal append, preservation of
the original report/history, idempotent recovery, refusal to reuse the old attempt
and admission of fresh validation. Missing cleanup after manager death still
blocks recovery and later admission even after the test observes those processes
exit.

At `eb8b376`, native Windows x64 GNU passed all 22 applicable operation/admission
tests and ten supervisor/command tests. Windows-target and workspace Clippy passed.
The preservation checks release the Windows lock before reading its file and
compare canonical paths, including native extended path prefixes. Full Windows
server/companion and MSVC acceptance, power-loss recovery, playback and activation
remain separate.

Native Intel and Apple Silicon macOS passed at
`eb8b3768f11a54b054de4fd71ffed13b4c4d64e4` in
[verification run 35050429897](https://github.com/bartbunting/omnivox/actions/runs/35050429897).
Each host passed all 24 operation/admission tests, repeated supervisor tests,
evidence checks, native Clippy, the full silent Piper/Flite probe including
external Flite, and metadata command checks. Both verified recovery after a lost
terminal append and continued refusal when worker cleanup had not been recorded.
The canonical-path comparison also handles macOS temporary-directory aliases.

### Separate supervisor verification

The implementation and fault probes are committed in `431e716` and `c63e7c6`.
Linux passed the locked workspace suite (803 passed, one existing ignored test),
workspace Clippy with Piper features, formatting and documentation links. The
metadata command probe rejects closed and malformed supervisor startup gates
without changing the prepared journal or initializing admission. The staged
silent Piper/Flite probe, including an exported external Flite voice, distinguishes
manager death from supervisor death: the former records confirmed cancellation
and permits fresh admission; the latter keeps incomplete work blocked. It observes
the independent supervisor and all tested native descendants exit before checking
the retained outcome.

Native Windows x64 GNU passed all 13 supervisor/command tests and Windows-target
Clippy. The lifetime test uses a real manager process and observes supervisor exit
through a native wait handle after both explicit cancellation and manager death.
These are process-control and component checks; full Windows server/companion and
MSVC acceptance remain separate. They do not establish recovery after the
supervisor itself dies, filesystem power loss, or whole-job/host shutdown.

Native Intel and Apple Silicon macOS passed at
`c63e7c629bb7608ef1a2ab8ae0a2e36bd271788d` in
[verification run 35052109946](https://github.com/bartbunting/omnivox/actions/runs/35052109946).
Each host passed the supervisor suite five times, all 24 operation/admission tests,
evidence tests, native Clippy, the full silent Piper/Flite probe including external
Flite, and the metadata command probe. Both distinguished confirmed cleanup after
manager death from blocked recovery after supervisor death.

### Damaged completion verification

The recovery change and native probe are committed in `261f07a` and `6a35e57`.
Linux passed all 805 locked workspace tests (one existing ignored test), including
26 operation/admission tests. These cover a real writer killed after a partial
terminal append, explicit abandonment, preserved damaged bytes, refusal to reuse
the attempt, and rejection of later changes to the damaged suffix. Missing
cleanup or damage to the validating record still blocks new work.

The staged silent Piper/Flite probe, including an exported external Flite voice,
passed both missing and torn final-write scenarios after actual native cleanup.
It verifies idempotent recovery, inspection and fresh validation, while retaining
the original operation files. The metadata command probe, workspace Clippy with
Piper features, formatting, Python syntax and documentation-link checks passed.

Native Windows x64 GNU passed all 24 applicable operation/admission tests on its
native temporary filesystem, including the killed-writer case. Windows-target
Clippy passed. Full Windows server/companion and MSVC acceptance remain separate;
these results do not establish cleanup recovery for active workers, power-loss
durability, speech playback or installation/activation transactions.

Native Intel and Apple Silicon macOS passed at
`6a35e57a9e97efffd8d0472f34ab428425f49784` in
[verification run 35053686464](https://github.com/bartbunting/omnivox/actions/runs/35053686464).
Both passed all 26 operation/admission tests, five repetitions of the supervisor
suite, evidence tests, native Clippy, the full silent Piper/Flite probe including
external Flite, and metadata command checks. Both verified explicit recovery of
the torn final write and continued refusal when worker cleanup was missing.

## Additional recovery hardening

Profile admission now blocks the spawn/record crash gap, and reports are bound to
the operation and request. An independent supervisor now retains cleanup ownership
through manager death. Complete saved cleanup can release an interrupted claim
through explicit abandonment, including a damaged final journal write after a
verified validating prefix. Work with missing cleanup records after supervisor
death still needs provider-specific boot/process-tree identity and confirmed
absence, not a reusable numeric PID or a matching report from another attempt. That reconciliation must
also account for incomplete worker records, damaged receipts and journals without
a verified validating prefix without inventing success.

On 2026-09-16 the maintainer prioritized installation and activation over this
additional recovery hardening. Continue those features while retaining the
existing refusal to reuse unresolved native work. Do not add force-clearing or
treat missing cleanup as success. The voice-library capability remains
unadvertised until the complete runtime and client activation contract works.
