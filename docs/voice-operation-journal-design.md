# Validation operation journals

Status: Development implementation boundary, 2026-09-16.

This is the first persistent-operation slice under
[ADR 0012](adr/0012-voice-library-and-model-lifecycle.md) and the
[voice-library contract](voice-library-contract.org). It adds preparation,
exclusive ownership and inspection for one validation operation. It does not
connect the native validator to a profile admission gate yet, publish packages,
change an index/active pointer, or restart speech.

## Ownership and frozen input

Omnivox owns these management primitives, alongside its shared library formats;
the CLI exposes development preparation and inspection. Helpers do not call the
storage primitives. The caller supplies an existing operations directory and a
locally generated operation UUID. Creating an operation adds only its new UUID
subdirectory; existing paths and partially created operations are retained.
Target-root provisioning and profile-wide locking remain admission-provider work.

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
terminal result or restart it. There is no force-clear/retry command in this
slice. Automatic PID-based termination, report-based completion, journal reset
and speech restart are deliberately absent from recovery inspection.

## Development commands

```sh
omnivox --prepare-voice-validation /absolute/request.json /absolute/operations
omnivox --inspect-voice-operation /absolute/operations/OPERATION_UUID
```

Preparation writes `prepared` and exits; it does not execute the saved request.
Inspection obtains a temporary lease for stable reads and changes no file
contents. Both commands run before audio/engine startup. Their text output is
for development, not a negotiated management API.

The [command probe](../tools/verify_voice_operations.py) checks exact input
preservation, refusal to overwrite, damaged-journal inspection and absence of
native loading. Shared component tests kill a real writer in prepared,
validating, torn-completion and staged states. They check lock release separately
from cleanup/reuse, as well as changed/replaced files and incomplete initialization.
A Unix regression test also holds a forked descriptor before exec and verifies
that normal owner retirement releases the lease without waiting for that child.

## Verification

At source `f10a32f3b32c72ad9b79accd3019f49b7bee65b2`, Linux passed all 11 shared
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

## Next admission slice

Before this journal can govern actual native validation, add the target/profile
admission owner so a new operation UUID cannot bypass an interrupted operation.
Persist worker ownership before opening each native startup gate, with explicit
handling of a crash between spawn and identity recording. Reconciliation needs
boot/process-tree identity and confirmed absence, not a reusable numeric PID.
Bind the validation report to this operation and exact request; a matching report
from a different attempt cannot finish an interrupted operation automatically.

Only after those boundaries work should installation transactions and full
candidate startup/activation use the journal. The voice-library capability
remains unadvertised, and ordinary native validation retains its prior behavior.
