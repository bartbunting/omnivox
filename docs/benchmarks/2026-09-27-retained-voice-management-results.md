# Retained voice-management verification

Extracted on 2026-09-27 from documentation at source `ca38781`. These are
retained observations, not new test runs or current universal qualification.
Each section identifies its original document; the Git revision preserves that
account. Original raw files and CI links remain where recorded. Missing local
logs, samples or provenance have not been reconstructed. Historical outstanding
work describes that observation, not necessarily the current implementation.

## Operation ownership and recovery

Source: `docs/voice-operation-journal-design.md` at `ca38781`.

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
[the validator guide](../VOICE-VALIDATION.md).

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

## Installed-state acceptance

Source: `docs/VOICE-INSTALLATION.md` at `ca38781`.

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

## Native validation and saved evidence

Source: `docs/VOICE-VALIDATION.md` at `ca38781`.

Native Intel and Apple Silicon checks passed in
[verification run 35035356056](https://github.com/bartbunting/omnivox/actions/runs/35035356056)
at source commit `9107e6f578094c6dfba90a75d4a62a6a390c2179`. Each host passed six
supervision tests five times, including a deterministic zombie-group regression,
then the full Piper/Flite probe and its ownership fault checks. Native Clippy
also passed for the validator and its prepared dependencies. These are silent
validation checks, not acoustic or coordinated-activation acceptance.

Saved-evidence checks subsequently passed on both native Mac architectures in
[run 35042369039](https://github.com/bartbunting/omnivox/actions/runs/35042369039)
at source commit `7ff386693701d8a7a50cb10be615f455062516ca`. Each host passed seven
shared evidence tests, repeated supervision tests, native Piper/Flite report
creation and comparison, stale-input rejection, and refusal to publish after
cancellation or supervisor death. Linux passed the same integration probe and
the workspace/Clippy gates. Native Windows x64 GNU component tests passed six
shared evidence checks and eight supervision/command checks, including report
creation, non-overwriting publication and cancellation on its native filesystem.

## Managed removal

Source: `docs/VOICE-UNINSTALLATION.md` at `ca38781`.

Development checks on 2026-09-17 passed for the reviewed Piper Kristin, Flite
AWB/RMS and MBROLA us1/us2/us3 downloads on Linux, and Flite AWB/RMS and all
three MBROLA downloads on native Windows. Shared Piper speakers, active and
rollback generations, other profiles, ownership failures and interrupted
metadata/unlink handling have regression coverage. Native macOS uninstallation
and spoken Emacsvox acceptance remain unverified.
