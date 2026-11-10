# Windows guard transport and native qualification boundary

Keep independent preference reload and the opt-in Linux guard. Retain the Windows
transport work, but do not enable Windows guarded owners yet: a repeated native
failure test disproves the current descendant-completion criterion. The final
candidate rejects Windows `--guarded` before reading management state or creating
native ownership. This is a qualification failure, not a completed portable
recovery feature. Ordinary speech and reload do not acquire another transport hop.

This follows the [Linux readiness investigation](2026-11-10-guard-readiness.md).
[Provenance](data/2026-11-10-windows-guard-qualification/provenance.json),
[raw observations and fixtures](data/2026-11-10-windows-guard-qualification/raw.tar.gz),
[the native counterexample](data/2026-11-10-windows-guard-qualification/native-counterexample.json.gz)
and [checksums](data/2026-11-10-windows-guard-qualification/SHA256SUMS) distinguish
the tested prototype from the final gated candidate. The private prototype is
runtime `c56e4a39e052332a`, executable SHA-256
`4b7d5bd9d61a5f40a2d2a045277c69505acb1275d7b2899ca82d949517dcf4f9`.
It was built with Rust 1.97.1 and the pinned Windows GNU image using the full
Emacsvox development staging path, redirected into Windows Temp. Installed
speech, main, release tags and historical evidence remain untouched.

## What the port establishes

The shared Rust guard uses platform-specific lifetime and control objects. Linux
retains pidfds, anonymous speech pipes and abstract Unix sockets. Windows uses
retained actual process handles, private jobs, anonymous speech pipes and protected
local named pipes. There is no TCP connection. Only lifecycle frames traverse the
guard bridge; speech remains on the ordinary owner/worker stdio path.

The Windows launch explicitly inherits six private objects plus a NUL standard
stream, with no job-handle inheritance. The owner retains the actual speech-I/O
completion lease; the guard receives an observer that cannot signal it. A signalled
actual owner handle can establish retirement of that owner's I/O. Native job
retirement and I/O retirement remain separate facts. Overlapped control owns
bounded buffers and stable native allocations through cancellation completion.
The event loop observes parent death even when status output is full. The private
pipe namespace is retained continuously and accepts only local clients with the
creating user's protected ACL and identification-only impersonation level.

Twelve native Windows pipe tests pass, including ACL rejection, non-inheritance,
pending accept/read/write cancellation, coalesced input, peer exit and reconnect.
The handle stress check stabilizes at 140 handles across 40 post-warmup cycles;
its earlier warmup rises from 118 to 139 to 140. Partial bootstrap expires, and
partial START reports tree cleanup while the actual I/O lease remains live.
Those checks do not establish descendant completion after a helper has started.

## Failure matrix

Native PowerShell fixtures retain actual creation identities, validate the observed
private executable and creation time, and signal only those retained handles.
They use real Flite helpers, corrupt configuration/history and deny session writes
with a native Windows ACL. Successful cases preserve old claims, a healthy Flite
notification lane, and explicit minimal eSpeak after observed cleanup.

| Case | Result | Completion condition / limitation |
| --- | --- | --- |
| Killed or suspended actual owner | Passed individual trials | Correlated tree/I/O reply plus retained worker/helper handles signalled. |
| Full speech input or output | Passed individual trials | Actual native cleanup remains independent of forwarding. |
| Disconnected bridge, owner alive | Passed | Speech continues; reconnect authenticates the same retained guard. |
| Full status output, owner killed | Passed | Worker/helper handles signal before replies are drained; one trial took 7.798 ms. Both proof fields arrive afterward. |
| Guard killed | Passed refusal | No cleanup proof is manufactured; replacement remains blocked, peer speech survives. |
| Worker killed, helper suspended and still alive | **Failed qualification** | Guard can report `tree_retired: true` before the helper process object signals. |
| Partial bootstrap / START | Passed native executable checks | No speech before launch gate; I/O remains false until actual endpoint/lease retirement. |

The surviving-helper failure repeats in separate runs. In the focused atomic
observation, the root process wait returns `WAIT_OBJECT_0`, the retained helper
wait returns `WAIT_TIMEOUT` (258), and `GetExitCodeProcess` already returns exit
code 1. The same helper handle signals 0.640 ms later. This is not a PID-reuse
observation, a missing reply or evidence that stale audio was heard. It shows that
the reported tree proof can precede actual descendant process completion.

The reused `remote_windows::Job::is_empty` queries `ActiveProcesses == 0` after
`TerminateJobObject`; the guard also waits for the root `Child` to end. Those
conditions are insufficient for the stricter retained-descendant test. Microsoft
describes job termination as applying process termination to each member;
process termination is asynchronous and a retained process-handle wait establishes
completion. See [TerminateJobObject](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-terminatejobobject)
and [TerminateProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess).
The accounting structure documents membership, but supplies no stronger process-handle
barrier that resolves this observed counterexample; see
[job accounting](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information).

The same primitive is reused elsewhere, so this is not evidence that the new named
pipe caused the race. Older passing recovery trials do not prove its absence in
ordinary Windows owners, validation or session cleanup. Audit those callers when
repairing the primitive; this report does not establish a reproduced release bug
in each caller. Do not add a sleep, treat an exit code as a signalled handle,
reopen saved PIDs, delete claims, or invent completion evidence.

## Matched speech and preparation measurements

The WSL Python client alternates direct and guarded owners in four blocks per
engine, with the same frozen prototype, null output, 80 character requests after
reload submission, simultaneous notification characters and 50 capability
exchanges per block. There are 320 foreground source samples, 320 successful
reloads and 200 protocol exchanges per engine/variant. No builds or fault stress
runs overlap these blocks. Raw per-request samples and per-run medians are retained.

| Engine | Direct / guarded source median | Direct / guarded protocol median | Direct / guarded two-lane preparation median |
| --- | ---: | ---: | ---: |
| eSpeak | 3.357 / 3.287 ms | 1.116 / 1.025 ms | 911.9 / 1555.1 ms |
| Flite | 5.577 / 5.327 ms | 1.104 / 1.134 ms | 912.4 / 1593.9 ms |
| WinRT | 121.913 / 126.769 ms | 1.069 / 1.014 ms | 1182.9 / 1771.1 ms |

These results do not demonstrate a fixed guard penalty for ordinary eSpeak/Flite
speech. WinRT run medians vary from 104.2–142.3 ms direct and 113.6–135.6 ms guarded;
the pooled difference is inconclusive. Do not infer tail equivalence or compare
these engine source times with the roughly 1 ms protocol-response figure.
Reload medians are 8.214/7.431 ms for eSpeak, 9.672/7.177 ms for Flite and
7.292/6.084 ms for WinRT, direct/guarded. Speech continues during reload.
Two-lane preparation adds approximately 588–682 ms, much more than the Linux
preparation difference. Four preparation samples per variant do not establish tails.

Separate stop/replacement checks during sustained speech and reload pass 20 trials
of each operation for each engine/owner combination, with concurrent notifications.
Old requests cancel and replacement requests complete. These are source/control
checks, not acoustic stop-to-silence measurements. The Linux follow-up also covers
foreground requests while a secondary worker is paused; the Windows fault matrix
covers healthy notification speech while foreground ownership is impaired.

## WSL compiled-client recovery profile

Fresh compiled Emacs files from `ac338dde7` restore minimal speech after paused,
killed, repeated, lost-reply and disconnected-client scenarios in the prototype.
These are functional observations under the failed native proof criterion, not
independent qualification of that criterion. WSL SIGSTOP affects the interoperability
connector; the separate native fixture tests actual Windows owner suspension.
The native ACL/unchanged-claim checks belong to the native fixture, not this client
fixture. A native Windows voice root is required: the first attempt used a WSL
filesystem path and failed native management locking with `Incorrect function`.

The first compiled run takes 826–999 ms for the ordinary recovery scenarios and
1447 ms for lost reply plus retry. A separate profiled run takes 690–754 ms inside
successful emergency-start calls. Its owner preparation/describe takes 327–358 ms,
bridge connection/first reply 181–214 ms, release 81–89 ms and capability/startup
acknowledgement roughly 52–67 ms. Observed old retirement takes 0.1–5.8 ms.
These nested spans must not be added to their containing operations. Bridge
startup includes launcher, WSL/native process creation and first request; this
profile does not separately attribute that whole interval to a Windows API or
polling. Native emergency snapshot preparation itself is around 25–26 ms in the
same diagnostic log.

Preparation precedes intentional old retirement, so total operation time is not
speech blackout. An already-dead owner cannot provide speech during preparation.
The client source timestamps include subsequent synthesis and delivery; none of
these measurements is acoustic onset, exact blackout or a fresh 1.18.0 comparison.
The retained earlier 393 ms ordinary-owner comparison used a different run;
this profile identifies current costs but is not a matched before/after speedup.

## Decision and bounded next work

Complete the descendant proof before enabling Windows guards. The exact accepted
constraint needing review is retained [ADR 0015](data/2026-11-10-lightweight-reload/0015-windows-session-cleanup-guard.txt),
which requires the actual child to end and job accounting to reach zero. The
observed implementation meets those checks while a helper handle remains unsignalled.
Do not silently reinterpret them as full descendant completion. ADR 0018 still
requires verified tree/I/O retirement; ADR 0017's independent preference boundary
needs no change. No ADR is created or rewritten by this slice.

The next focused experiment should establish a complete set of actual descendant
handles before native execution, including creator death during handoff, and wait
for their completion independently of speech/status I/O. Test one real Flite
helper first. Account for concurrent child creation and unseen descendants;
a PID snapshot or an assumed fixed helper count does not establish completeness.
Keep the private job for containment and bounded retention. Stop this experiment
if a complete creation/ownership handoff cannot be established with documented
APIs and a small bounded protocol; return the concrete boundary change for review
instead of expanding the manager or weakening proof.

After that proof succeeds, measure the smallest preparation-cost experiment:
reuse an existing client bridge or prepare the lifecycle connection while old
speech remains available. Require the same loss/backpressure checks and matched
source timing. A persistent bridge needs bounded authority lifetime and a reviewed
ownership contract; reducing process count alone is not evidence of improvement.
Ordinary preference reload remains independent of both experiments and managed Apply.

## Verification and limits

Pinned workspace tests and workspace/all-target Piper Clippy pass for the shared
refactor. Windows all-target Clippy and native pipe/bootstrap checks pass. The
final source is `5f39677`. Full private Windows staging of runtime
`1c1833281316787d` passes archive verification and native eSpeak, Flite, RuTTS and
TGSpeechBox smoke. Three native tests then pass against that explicit payload:
ordinary/emergency guarded startup rejects before management access, partial
bootstrap expires, and partial START retains its I/O proof boundary. These tests
do not bypass the failed helper qualification.

The final Linux payload SHA-256 is
`840d7ae934c3c12235187d1347147fd00f9a72a3f007003131df8d77cf344ccd`;
every copied executable/data/companion file is verified against the `make build`
output. Its six native faults, additional disconnect/partial-control/held-status
checks, partial bootstrap/START and five compiled-client recovery scenarios pass.
These final functional checks overlap build activity; their timing is not used
as performance evidence. One earlier Linux fixture copy retained the preceding
readiness executable; its raw results are preserved and are not credited as
validation of the refactor.

Native macOS, acoustic/listening checks, Windows Piper, and universal descendant
completion remain unqualified. Keep Linux guard opt-in, Windows guarded startup
unavailable and the existing low-latency preference path independent of this work.
