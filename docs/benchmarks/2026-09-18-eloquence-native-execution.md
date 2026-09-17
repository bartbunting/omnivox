# Eloquence native parameter execution, 2026-09-18

This follows the [optional binding audit](2026-09-17-native-parameter-bindings.md)
and [Windows cancellation fix](2026-09-17-windows-helper-cancellation.md).
It implements an internal execution path, not helper 6 or an Emacs editor.

## Implementation boundary

The Eloquence adapter accepts immutable, sparse integer/default edits for all
eight ECI voice parameters. Missing keys inherit; null in this internal typed
API requests the selected preset's default. Zero is an explicit value. The
future wire decoder must translate the accepted tagged operations into this
internal form; the wire format has not changed.

Unknown IDs, out-of-range values, unknown/duplicate context dimensions and
invalid mapped values are rejected before native mutation. Invalid edits are
rejected even when context would mask them. Explicit context masks its mapped
outputs: richness affects both breathiness and volume, and rate/rate-offset
mask speed. Common mapping formulas and clamps remain adapter-owned and
unchanged. Native breathiness does not recompute common volume compensation.

Execution stays on the adapter's serialized owner thread. That thread now
explicitly selects STA before starting; the native acceptance check found that
the prior code documented STA but inherited the .NET default apartment instead.
The capture layer copies the selected preset into the active voice, reads its
pristine fields, overlays the common values and unmasked native edits, and
applies all eight resulting fields. It verifies complete readback before
calling the application callback or synthesizing text. That callback receives
a copy, not mutable native state. Native commands or pointers never leave the
capture layer.

A finally path stops/clears native input, restores the selected preset, and
verifies its fields after successful speech, cancellation during application,
PCM cancellation or failed delivery. A request cancelled before dispatch leaves
existing state untouched. Every later request still begins from its own voice.
Ordinary requests keep their existing annotation path and need no optional
parameter APIs.

The initial profile is `eloquence.windows.6_1.en_us.v1`, schema
`eloquence.eci-units.v1`. It requires optional parameter bindings, verified ECI
units, version `6.1.0.0` and the exact runtime DLL qualified by the previous
audit (`da99080288cdca14a7effba20274af1d6d5878840e32be5a315bd8691124703b`).
The DLL is hashed lazily on the first native request. An unqualified runtime
keeps ordinary speech; it cannot execute this profile. Other DLL builds and
dialects need independent qualification rather than a version-string guess.

## Verification

[`EloquenceExecutionAudit.cs`](../../tools/EloquenceExecutionAudit.cs) loads the
exact helper bytes in a disposable x86 STA process. It does not invoke the
helper entry point or open an audio device. Its local interface proxy forwards
progressive sink callbacks to the probe without rebuilding the helper or
substituting a native engine. Run the PowerShell wrapper with a bounded
180-second outer timeout and the user-installed ECI DLL.

The current matrix passes:

- both independent Eloquence composition fixtures, including coupled context
  masking, plus frozen inputs, omission/default/zero and invalid masked values;
- both endpoints for all eight controls on all eight voices: 128 cases;
- combined eight-control edits and all-default requests on every voice;
- 40 cancellations, including before dispatch, partway through setters, after
  readback, and after PCM in buffered and progressive paths;
- eight failed application callbacks and eight failed progressive PCM callbacks;
- 24 progressive cases, checking nonempty aligned PCM, marker ordering,
  pre-PCM readback and reset after completion/cancellation/failure;
- 200 ordinary-speech reset checks, and 360 successful PCM captures in total;
- missing optional bindings preserving ordinary speech, unsupported runtime
  identity rejection, and both fixtures through the adapter's actual STA thread.

The [pinned-helper execution report](data/2026-09-18-eloquence-native-execution.json)
records the complete matrix and exact helper, DLL and probe SHA-256 identities.
Helper `2658d9644523d175aff60417e7d4e0f5dec865614b372c7b594b62fdc5d385d4`
was built using the Emacsvox target's pinned compiler and reference assemblies.
The initial execution matrix also passed with the default Framework compiler's
helper; the final strengthened partial-application assertion used the pinned
helper recorded above.

The [ordinary protocol-v5 report](data/2026-09-18-eloquence-native-execution-stream.json)
records 50 normal progressive syntheses and 50 cancellation/recovery probes on
that pinned helper, with markers, requested anchors, six common controls,
health pings and clean shutdown. Its invocation uses `--iterations 50
--cancel-every 1 --health-every 5 --resource-sample-every 0 --require-streaming`
and all six `--require-acss` controls. Source-contract and missing-runtime
startup/discovery checks also passed. Rust and Lisp implementation code did not
change in this slice.

Full Windows development staging, checksum verification and live inventory
checks passed for build `d4fd04189a38e08a`, staged under
`/tmp/emacsvox-eci-execution-runtime`. The Eloquence helper matches the audited
pinned binary byte for byte; DECtalk retains its previously qualified helper.
The guarded build used a self-contained source snapshot including the
then-current tracked changes and concurrent RHVoice work, with development
provenance. It is not a clean release build. The normal desktop launcher remains
on `e1ecdb481ee08fd0`; no live Emacs session was restarted.

## Remaining work

DECtalk native execution, helper 6 decoding/catalogues/receipts, runtime identity
transport, ordinary/strict-preview policy, public registration and timelines,
and the Emacs editor remain pending. No new wire capability is advertised.
The probe does not prove audible quality or complete cross-engine fallback,
and does not claim support for untested ECI runtimes or other platforms. It
injects delivery failures; it does not force every vendor API failure mode.
No desktop runtime or live Emacs session is changed by this acceptance run.
