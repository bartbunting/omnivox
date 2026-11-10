# Windows native creation: Rust component qualification

This is private implementation evidence under [ADR 0019](../adr/0019-windows-native-creation-ownership.md).
The Rust creation and retirement components pass native Windows tests. Product
guarded startup remains disabled pending integrated speech and recovery checks.
The earlier [C# handoff experiment](2026-11-10-guard-handoff-and-preparation.md)
is supporting evidence, not a substitute for these Rust checks.

## Implementation and observations

The guard creates workers and helpers suspended, with atomic private-job
assignment, restricted child creation and an explicit inheritance list. It retains
each process handle before execution. A single bounded background task performs
creation; status and retirement remain on the independent control loop. The worker
receives direct helper pipes and wait/cancel authority, acknowledges the handoff,
and then the guard releases the helper. Speech and PCM do not pass through it.

Retirement closes creation admission. While a launch is pending, the guard stops
the actual known processes individually. It collects any late suspended result
before terminating the job. Native completion requires the creation task, actual
worker/helper process objects and guard I/O to finish. Job accounting additionally
checks that the retained set is complete; forwarding-I/O completion stays separate.

The initial implementation incorrectly called `TerminateProcess` after an accepted
job termination and propagated an access-denied error before process completion.
Four of five cleanup tests failed. Removing that duplicate call passed four tests,
but worker loss during pending creation still produced access denied and no cleanup
proof. Serializing final job termination after creation completion passes all five.
The failed versions and outputs remain in the evidence pack. These were withheld
proofs, not observed false cleanup receipts or stale audible speech.

Microsoft documents asynchronous termination, the need to wait on the actual
process object, and access denied on a subsequent termination call.
[TerminateProcess reference](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess).
The concurrent-creation failure does not establish a general Windows guarantee
about every intermediate job-accounting update.

Launch matching covers the captured engine, native program path, arguments and
environment. Existing payload verification, managed-asset verification before
construction/rescan, and helper protocol/descriptor checks remain. An extra helper
file fingerprint in the first draft was removed: it added a synchronous disk read
before guard readiness and was not part of the ordinary helper launch contract.
Root executable verification still occurs immediately before native creation,
outside the control loop. No integrity or compatibility check from the existing
path was removed.

## Checks

| Check | Result and boundary |
| --- | --- |
| Windows Rust cleanup components | Five tests pass: lost offer/ack, changed identity/duplicate creation, pending launch, worker loss during creation, and unaccounted-member refusal. They use actual private process handles and the real Flite executable; they do not establish integrated audible speech. |
| Windows argument quoting | Empty arguments, spaces, quotes, trailing backslashes and NUL rejection pass. |
| Windows pipe components | All 12 pass, including cancellation, restricted access, peer death and backpressure. Retained handle counts stabilize at 140 through 40 additional iterations after warm-up. |
| Windows helper-engine suite | All 60 tests pass. Controlled panic messages in stderr are expected test inputs. |
| Windows locked Clippy/test compilation | CLI and TTS all-target checks pass on the pinned toolchain. |
| Linux locked workspace tests | Pass, with existing ignored native cases kept explicit in the raw log. |
| Linux locked workspace Clippy | Pass, including all targets and the CLI Piper feature. |
| Full private Windows development build | Ordinary eSpeak, Flite, RuTTS and TGSpeechBox smoke checks pass. This first build still contains the failed guard draft behind its startup gate; it is not a usable guarded candidate. |

The first native runner lacked its debug compiler DLLs and exited with
`0xc0000135` before tests. A second runner lost complete panic output because
PowerShell treated native stderr as a terminating error. The corrected runner
retains stdout, stderr and exit status separately. Both setup failures remain
recorded; neither is counted as passing product evidence.

## Provenance and limits

The [raw pack](data/2026-11-10-windows-owned-creation/SHA256SUMS) retains source
patches against `a4ac667`, commands, failed attempts, native outputs, checks and
runtime provenance. `source-initial.patch` matches the first full build's recorded
worktree digest `135a70ecc3319a6847224d8cf8195ace519ee35d9891a24865413961c169c5af`.
`source-unit-5.patch` identifies the corrected passing component implementation.
The paired client is unchanged at `b83c43aad`. Test executables and compiler DLLs
are identified by hashes; generated binaries are not duplicated into Git.

The first full runtime is private build `242ab510fb56456f`, executable SHA-256
`ddbe824433d2412c5acfc6bc88f6ce3fdfa7e28914d3d0dcdeeef06d9dfd4b39`.
It is under Windows Temp, not the installed speech cache. The native tests use
Windows x64 through WSL and explicit private program paths. No installed runtime,
release baseline, accepted decision or main branch was changed.

Remaining gates are integrated guarded eSpeak/Flite synthesis, cancellation,
owner/worker/helper failure, compiled Emacs recovery, matched response/source
timing during reload, and a listening build. Additional engines, native macOS,
acoustic timing and loss of every cleanup authority remain unqualified.
