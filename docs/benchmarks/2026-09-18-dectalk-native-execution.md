# DECtalk native parameter execution, 2026-09-18

This follows the [binding qualification](2026-09-17-native-parameter-bindings.md)
and [Eloquence execution slice](2026-09-18-eloquence-native-execution.md).
It adds an internal typed execution path, not helper 6 or an Emacs editor.

## Implementation

The adapter freezes sparse integer/default edits for the 28 qualified design
voice controls. Omission inherits, null requests the selected preset's pristine
value, and zero is a value when permitted by the control's range. Unknown IDs,
invalid values, and unknown/duplicate contextual dimensions fail before native
mutation, including invalid edits that would be masked by context.

The initial profile is `dectalk.windows.4_99.v1`, schema
`dectalk.design-voice.v1`. It requires the optional speaker-parameter API, the
qualified DLL's exact SHA-256 and version, and matching runtime limits. Another
runtime retains ordinary speech but cannot execute this profile. The DLL hash
is checked lazily on the first native request. The matching American English
dictionary remains user supplied; this is not qualification of other dialects.

Under the synthesis lock, command-only batches select the pristine preset,
apply the existing common prefix, and apply unmasked native edits. Each batch
is synchronized. Full readback must match the composed plan before the callback
reports application and before speech or markers can be delivered. The qualified
runtime emits no PCM for these command-only batches. Commands and native pointers
stay inside the helper.

Common mapping formulas and commands remain unchanged. DECtalk itself clamps
some common values: the established 500 Hz common pitch command reads back as
350 Hz on this runtime; common stress minima read back as `hr=2`, `sr=1`.
Composition starts from this actual mapped state. Native edits retain their
separately qualified bounds, and explicit context masks every output of its
mapping, even when the contextual value equals the base.

A reset gate coordinates the cancelling thread with preparation and cleanup.
Native reset never holds the callback state lock. Cleanup prevents a concurrent
Stop from resetting after pristine restoration, drains pending native work,
restores the selected preset and verifies all 28 fields. Cancelled output stays
suppressed until the next capture. Successful ordinary requests retain their
single common prefix and do not gain an extra cleanup reset; early exits drain
pending work before capture state is released. Ordinary requests need no optional
parameter API and advertise the same protocol capabilities.

## Acceptance

[`DectalkExecutionAudit.cs`](../../tools/DectalkExecutionAudit.cs) loads the actual
helper bytes in a disposable x86 process and captures PCM silently. Its
[PowerShell wrapper](../../tools/check_dectalk_execution.ps1) accepts `-Helper`,
`-RuntimeDll`, and `-PlanningOnly`. Allow a bounded six-minute outer timeout for
the full runtime matrix. Planning-only checks do not load a speech DLL.

The matrix covers both endpoints for 28 controls across nine voices (504 cases),
combined edits, explicit defaults, seven independent composition fixtures,
invalid masked edits, frozen inputs, and preservation of legacy common clamps.
Cancellation covers before dispatch, after the common preamble, after native
readback, after progressive PCM, and a held native reset overlapping cleanup.
Receipt and PCM delivery failures must restore the preset before subsequent
ordinary speech. Progressive checks require readback before audio/markers,
nonempty aligned PCM, marker ordering, and reset after interruption.

The reset-overlap test wraps the real native Reset delegate, blocks the
cancelling thread before that call, and prevents the PCM callback from returning
until the cancelling thread has reached the barrier. The request cannot complete
while reset is held; after release, both threads must finish and pristine
readback and ordinary speech must pass. No vendor engine is substituted.
A disposable helper with only Stop's reset gate removed failed the extracted
reset-overlap test with "request completed while native reset was held".
The same extracted test passed all nine voices on the pinned helper. This
negative control used a temporary reduced copy of the retained audit, not a
second implementation of the reset-overlap checks.

The [pinned-helper execution report](data/2026-09-18-dectalk-native-execution.json)
records 1,135 successful captures, 585 ordinary-speech reset checks, 45
cancellations, 18 delivery failures, 27 progressive cases and nine held-reset
cases. It identifies the exact helper, runtime and probe hashes. Helper
`e7d7e6d015b5f9070ce5b01fcdb8c317cd350123d99d572e2331847f06c95a46`
uses the deployment target's pinned compiler and reference assemblies.

The [ordinary protocol-v5 report](data/2026-09-18-dectalk-native-execution-stream.json)
records 50 syntheses and 50 cancellation/recovery probes, with progressive PCM,
markers, requested anchors, six common controls, health checks and clean
shutdown. The command uses `--iterations 50 --cancel-every 1 --health-every 5
--resource-sample-every 0 --require-streaming` and all six `--require-acss`
controls. Source-contract, planning-only without a runtime, and missing-runtime
startup/discovery tests passed. Rust and Lisp implementation code did not change
in this slice.

Full Windows development staging, checksum verification and live inventory
and companion synthesis checks passed for build `c5fc6d3ff8960930` under
`/tmp/emacsvox-dectalk-execution-runtime`. Its DECtalk helper matches the audited
pinned binary byte for byte; Eloquence matches its previous qualified build.
The guarded target used a self-contained source snapshot including the
then-current tracked changes and concurrent RHVoice work, with development
provenance. This is not a clean release. The desktop launcher remains on
`e1ecdb481ee08fd0`, and no live Emacs session was restarted. Both projects'
documentation gates passed.

## Limits and next steps

The tests do not establish acoustic quality, every vendor API failure mode,
other DECtalk runtimes, or other platforms. Runtime metadata, helper 6 decoding
and receipts, public transport and the Emacs editor remain pending. No new wire
capability is advertised, and no live Emacs session is restarted by the tests.
