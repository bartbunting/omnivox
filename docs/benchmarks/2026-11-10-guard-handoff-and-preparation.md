# Windows creation handoff and concurrent emergency preparation

Retain the lightweight preference path and the smaller lifecycle guard. The
bounded Windows experiment establishes a workable ownership primitive for one
real Flite helper: retain each actual process handle before execution, then give
the worker direct helper pipes. It does **not** qualify the current Windows guard.
Adopting it requires a bounded creation service and a compatibility decision about
helpers that themselves create children. Do not enable Windows guarded owners yet.

The independent client improvement is implemented: start the disposable control
bridge while preparing the emergency owner. It reduces matched WSL-to-Windows
emergency-call time by about 181 ms, without another speech hop. It does not
establish a shorter source gap or acoustic blackout.

This follows the [Windows qualification failure](2026-11-10-windows-guard-qualification.md).
[Raw evidence](data/2026-11-10-guard-handoff/raw.tar.gz),
[summary](data/2026-11-10-guard-handoff/summary.json),
[provenance](data/2026-11-10-guard-handoff/provenance.json) and
[checksums](data/2026-11-10-guard-handoff/SHA256SUMS) retain successes, failed
harness attempts, exact fixtures, compiled-client inputs and verification logs.
No installed runtime, main branch, release tag or accepted decision changed.

## What the Windows experiment proves

The test controller creates a private kill-on-close job and uses an explicit
handle-inheritance list, atomic job assignment, suspended creation and a child
creation restriction. It retains the returned process and thread handles before
acknowledgement or execution. The worker receives duplicated helper pipe handles,
acknowledges them, and only then receives permission to use the resumed helper.
Flite returns protocol-5 PCM directly to the worker. The controller forwards no
speech. Failed or incomplete handoffs leave the actual handles with the controller.

Creation is serialized and bounded to four registered members. Retirement seals
creation, terminates the job and waits for **every retained process handle** to
signal, as well as empty job accounting. Lifetime process accounting is an
additional refusal check for missing registration, never a substitute for a
retained handle. No saved PID supplies termination authority, and no sleep supplies
completion evidence. Read-only PID diagnostics in early fixtures are separate
from the retained handles used to terminate processes.

Microsoft documents creation-time job assignment, explicit inherited handles and
child restrictions in [UpdateProcThreadAttribute](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute).
These attributes require Windows 10 / Server 2016 for the job and child-policy
parts. Process termination is asynchronous; the process object provides its
completion wait, as described in [TerminateProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess).
Ordinary job completion-port messages do not guarantee complete descendant
observation; see [job completion ports](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_associate_completion_port).

The final native run passes 12 trials of each case, 132 total:

| Case | Required observation |
| --- | --- |
| Worker dies before creation | No helper exists; actual worker completion. |
| Worker dies concurrently with creation | The controller retains the suspended helper despite the worker's death. |
| Concurrent helper creation | Both creations are serialized and registered within the bound. |
| Worker dies before pipe duplication | Suspended helper remains owned and can be retired. |
| Worker dies after one pipe duplicate | Partial handoff cannot release execution or lose ownership. |
| Lost handoff acknowledgement | Suspended helper remains owned; no invented acknowledgement. |
| Worker dies after acknowledgement, before release | Both actual handles complete during retirement. |
| Running Flite | Direct helper protocol produces 40 PCM chunks before retirement. |
| Worker dead, Flite suspended and surviving | Actual helper completion is still required. |
| Worker attempts an unregistered child | The OS rejects creation. |
| Controller deliberately introduces an unaccounted member | Completion is refused even after cleanup of the fixture's retained handles. |

In 120 of 132 final trials, empty job accounting was observed before all retained
process handles signalled. The wait loop withheld proof. These functional stress
observations are not retirement-latency benchmarks or evidence of audible stale
speech. The earlier 108-case run is retained too.

The fixture uses C# for native API orchestration and a small C worker; it does not
add either language to Omnivox. The actual Flite helper is the existing pinned
Rust/native payload. Early console-subsystem workers did not reach readiness with
the tested restriction/creation flags; an unrestricted .NET worker added an extra
job member. The final native worker uses the GUI subsystem, and detached creation
also lets the restricted, unchanged Flite helper execute. This is an observed
startup constraint, not a complete diagnosis of Windows console internals.
A protocol-fixture error omitting `anchors` was corrected before real synthesis
passed. All failed attempts remain in the archive.

## Why this is not a drop-in product fix

The current guard creates the root worker; ordinary helper creation still happens
inside `ProcessHelperConnection::spawn`. A worker-created child can be lost between
creation and registration if the worker dies. Enumeration or eventual job
notifications cannot close that interval. The successful fixture instead makes
the surviving controller the creator.

A production implementation therefore needs bounded, correlated creation requests,
verified executable/environment selection, cancellation during creation, actual
handle transfer, and retirement coordinated with outstanding requests. The worker
also needs its existing cancellation/wait authority over each helper. Direct
helper PCM pipes should remain direct. No persistent service or settings
transaction is needed.

Child restrictions would affect legitimate descendants: the MBROLA helper creates
its native runtime process, and user-supplied helper wrappers may create children.
Silently disabling those engines or allowing unregistered descendants is not an
acceptable compatibility shortcut. Start any implementation with an explicit
experimental eSpeak/Flite scope; broader engine coverage remains a separate gate.
Guard loss, a process stuck inside OS termination, full transfer channels, resource
exhaustion and complete multi-engine ownership are not qualified by this fixture.

The exact accepted decision needing review is retained
[ADR 0015](data/2026-11-10-lightweight-reload/0015-windows-session-cleanup-guard.txt):
its root-child completion plus empty-job criterion does not establish the tested
descendant boundary. Its restricted inheritance, independent guard, direct speech
pipes and partial-handoff retention should survive. Preserve
[0001](../adr/0001-engine-isolation-and-distribution.md)'s helper isolation,
[0007](../adr/0007-managed-voice-lifecycle.md)'s native ownership and asset protection,
[0008](../adr/0008-extensible-engine-registration.md)'s helper compatibility, and
[0018](../adr/0018-minimal-emergency-speech.md)'s requirement for actual tree and I/O
retirement. This report neither changes those decisions nor creates a new ADR.

## Implemented client change and matched timings

Emacsvox `b83c43aad` changes only control-bridge preparation. The one unpublished
emergency owner may have one waiting bridge. That bridge receives no descriptor
or native authority until the owner's descriptor passes validation. Attachment
consumes it once. Failed preparation or quit closes it; failed attachment retains
the exact unresolved authority. Reconnection still targets the same guard.
Speech remains on stdio anonymous pipes. Linux lifecycle control uses an abstract
Unix socket; the Windows prototype uses a local named pipe. There is no TCP.

Four alternating baseline/candidate blocks run on each platform, using fresh Emacs
31.0.90 processes and verified byte-code paths. The baseline client is
`ac338dde7`; the server is identical within each comparison. Each run covers
paused owner/connector, killed owner, healthy repeated recovery, lost reply with
retry, and disconnected control with reattachment. There are 20 successful
emergency calls per platform/variant, plus the deliberately failed lost-reply
attempts. The Windows SIGSTOP targets the WSL connector, not the native owner;
actual native suspension is covered separately in the earlier qualification
report. These matched runs do not overlap builds or native fault stress.

| Measurement, median milliseconds | Windows baseline | Windows candidate | Linux baseline | Linux candidate |
| --- | ---: | ---: | ---: | ---: |
| Successful emergency call | 876.21 | 695.55 | 102.56 | 98.04 |
| Owner preparation including bridge attach | 700.65 | 493.30 | 54.53 | 46.07 |
| Attach call after owner preparation | 207.62 | 1.55 | 7.48 | 0.44 |
| Healthy-recovery source-gap bound | 197.23 | 217.91 | 60.64 | 61.25 |

Rows are nested intervals, not additive costs. Windows run medians are
841.6–883.5 ms before and 666.9–717.0 ms after; the paired run-median savings are
153.5–177.5 ms. Linux run medians overlap substantially. Four healthy source-gap
bounds per variant range from 187.2–216.7 / 196.1–262.0 ms on Windows and
57.5–69.2 / 53.2–66.2 ms on Linux. There is no demonstrated blackout improvement;
Windows' small gap sample is also insufficient to establish non-regression.

The source-gap bound runs from the beginning of old retirement to the first
reported mixer-source consumption of new speech. It includes client scheduling
and subsequent speech submission. It is neither first PCM nor acoustic onset.
The timing change overlaps process startup while old speech may continue; it
cannot remove engine initialization or output readiness after native release.
No tail-latency claim follows from these sample sizes.

A focused follow-up addresses the uncertain gap: four alternating blocks of five
healthy restarts per variant, 20 samples each, with the same executable and client
pair. Median emergency-call time is 791.58 / 611.70 ms; the source-gap bound is
181.18 / 184.45 ms. Gap ranges are 167.37–193.52 / 168.32–211.48 ms, and paired
run-median differences range from -0.72 to +10.93 ms. The earlier 21 ms pooled
gap difference does not persist, but a small increase remains possible. Retain
this as an operation-time improvement with a roughly unchanged source-gap bound,
not an acoustic or tail non-regression claim. The follow-up is separate from the
mixed-failure comparison; its raw rows and summary are retained in the archive.

The frozen Windows timing runtime remains the **unqualified** `c56e4a39e052332a`
prototype. The final Windows runtime `1c1833281316787d` still rejects guarded
startup. Successful client timings cannot override the native completion failure.
The unchanged Linux payload is `840d7ae9…`; full executable hashes and runtime
identities are in provenance.

## Availability, verification and remaining acceptance

A fresh compiled-client probe repeatedly prepares and discards an unpublished
owner while both existing Flite lanes speak short utterances and reload their
unchanged preferences. All four Linux preparations preserve both lanes: 16
completed utterances and eight independent reload results. The four Windows
prototype preparations preserve both lanes: 133 completed utterances and eight
reload results. Each preparation observes source speech on both old lanes before
preparation returns. This establishes overlap and continued source activity,
not acoustic responsiveness under every workload.

All 34 recovery regressions pass from explicit source and fresh compiled loads.
New checks cover overlap ordering, rejected preparation/quit cleanup, failed
attachment without lost authority, and one replacement for a dead prepared bridge.
Incremental byte-code, freshness, Emacsvox documentation, Omnivox documentation
and paired-link checks pass. No Rust product code changed, so Rust builds and
workspace tests were not repeated. The final native snapshot contains no surviving
process from either the handoff fixture or its private Windows timing runtime.

The earlier [Windows workload comparison](2026-11-10-windows-guard-qualification.md)
retains character navigation during reload, concurrent notifications and
stop/replacement with real eSpeak, Flite and WinRT. The
[resident reload report](2026-11-10-resident-reload.md) retains the matched 1.18.0
comparison, and [Linux readiness report](2026-11-10-guard-readiness.md) retains
impaired-lane/forwarding coverage. Those unchanged server paths were not retested
as a new release claim. The present change affects preparation, not steady speech.

One preliminary baseline client run exited unsuccessfully without a Lisp
backtrace. Its original runner did not retain the exit status; logs remain and
the cause is undetermined. All 16 subsequent matched runs pass with recorded exit
status. Four additional alternating baseline/candidate runs also pass with
recorded exit status; they are follow-up functional checks, excluded from the
timing comparison. They do not explain the original unsuccessful exit.
The first availability fixture also failed JSON serialization after successful
observations; the corrected fresh run is the reported result. The first Windows
availability attempt bypassed the configured launcher and failed preparation;
the final run uses the same resolved launcher as the product. Failed artifacts
are retained, not reclassified as acceptance.

Native macOS, independent Windows helper ownership in product Rust, and actual
listening/acoustic onset remain outstanding. The user's working speech and
installed runtimes have not been changed. Do not promote the guard to the default
or declare the portable recovery goal complete.

The next decision is whether to implement the demonstrated creation boundary as a
small Windows-only Rust lifecycle facility. Review its helper compatibility scope
first, then implement one real Flite handoff and its failure gates before adding
other engines. Keep preference reload and the measured preparation optimization
independent of that work. Avoid expanding managed Apply to solve native creation.

That review should also state exactly which user-visible failure the stricter
completion boundary prevents. The retained-handle counterexample proves an early
completion claim, not stale playback or an unbounded leak. Any smaller alternative
must establish safe native and I/O ownership, suppression of obsolete audio and
resource bounds through documented OS behavior and failure tests; it cannot merely
rename missing evidence. The creation experiment supports a candidate mechanism,
not an obligation to add it regardless of product benefit.
