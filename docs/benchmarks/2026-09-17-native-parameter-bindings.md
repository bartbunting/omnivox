# Windows native parameter bindings and limits, 2026-09-17

This extends the [initial interior-value audit](2026-09-17-native-voice-parameters.md)
with optional native bindings in the Windows helpers. It does not enable a new
protocol, advertise writable controls, or add an Emacs editor.

## Implementation

- ECI optionally binds `eciGetParam`, `eciGetVoiceParam`, `eciSetVoiceParam` and
  `eciCopyVoice` as one feature group. Missing any member leaves ordinary speech
  available. Parameter writes target the active voice only; preset copies can
  only select existing presets 1–8 into that active voice. IDs and ECI-unit
  bounds are checked before native writes.
- Native ECI access requires verified ECI units. The installed 6.1 ABI uses
  environment parameter 8 for unit mode. The abbreviated enum in the archived
  IBM manual omits ABI entries; the maintained IBM ECI driver's explicit enum
  agrees with this runtime. The audit switches mode on its disposable instance
  to verify rejection in real-world units, then restores ECI units.
- DECtalk optionally binds `TextToSpeechGetSpeakerParams`. Its readback copies
  only the 28 initialized, documented fields, then frees all four successful
  query allocations with the Windows allocator. It never serializes reserved
  structure slots or exposes native pointers. The runtime owns cleanup when its
  query returns an error, as confirmed by the inspected native source.

All calls remain inside the isolated helper. These low-level methods require
an idle instance on its native owner thread. Existing synthesis, mapping and
protocol code does not call the new methods yet. Binding availability is not
runtime qualification or a capability advertisement.

## Acceptance

The [updated probe](../../tools/NativeVoiceParametersAudit.cs) uses the helper's
actual bindings instead of binding independent copies of the APIs. It checks
one representative value and both endpoints for each control/voice pair, followed by
an ordinary utterance and complete field comparison against its baseline.
ECI also checks direct setter readback and pristine preset-copy restoration.
Every capture stays in memory; no audio device is opened.

The final evidence files identify the exact helper, native DLL and probe by
SHA-256:

- [ECI 6.1.0.0](data/2026-09-17-eloquence-native-bindings.json): 64 control/voice
  pairs, 128 endpoint cases, and 424 nonempty PCM captures.
- [DECtalk v4.99 GitHub NORMAL ACCESS32](data/2026-09-17-dectalk-native-bindings.json):
  252 control/voice pairs, 504 endpoint cases, and 1,530 nonempty PCM captures.

All 948 representative/endpoint cases and ordinary resets passed, using 1,954 captures.
ECI rejected values outside each declared range without changing the readback.
It also rejected invalid parameter/preset indices and native access in the
wrong unit mode. DECtalk's qualified `ap` endpoint remains 350 Hz; this does not
change the established common adapter clamp of 500.

Missing-export lookup is exercised against a deliberately absent name in the
loaded library. Each optional delegate is then fault-injected as missing in
turn, only in the disposable audit instance. Availability becomes false and
native operations reject, while an ordinary capture still completes. This is
missing-binding fault injection, not acceptance of another vendor DLL version.

The existing protocol-v5 path also passed 20 ordinary requests per helper,
with progressive PCM, markers, all six common controls, health pings and clean
shutdown. The [ECI streaming report](data/2026-09-17-eloquence-native-bindings-stream.json)
and [DECtalk streaming report](data/2026-09-17-dectalk-native-bindings-stream.json)
retain those checks. They deliberately report zero cancellation probes; the
separate cancellation test failed as described below. Source-contract checks
and missing-runtime startup/discovery checks also passed.

Full `make windows-omnivox-dev` staging passed in an isolated source copy,
including deterministic helper builds, payload verification and live inventory
checks. Build `1ce20ab373624c79` contains the exact helper hashes in the audit.
The temporary runtime root is `/tmp/emacsvox-native-bindings-runtime`; the
normal desktop launcher still selects its original runtime. No live Emacs
session was restarted. The isolated copy includes the then-current tracked
worktree diff, including separate RHVoice development work, recorded by the
standard development provenance. This is not a clean release artifact.

## Existing cancellation ordering failure

The protocol-v5 stress check with `--iterations 20 --cancel-every 5` failed for
both rebuilt helpers: `synthesis_cancelled` arrived before `cancel_accepted`.
The native-binding changes do not modify the shared host or synthesis methods.
The same error reproduced with the previously staged Eloquence helper from
payload `e1ecdb481ee08fd0` (Omnivox `f10962b`) using `--iterations 50
--cancel-every 1`. An initial baseline run with four cancellations passed on
both engines, demonstrating why a short successful run cannot exclude the race.

`OmnivoxHelperHost.HandleCancel` sets the shared cancellation flag, calls native
Stop, and only then writes the acknowledgement. The worker can observe the flag
and publish its terminal event first. This is a pre-existing ordering failure,
not evidence that the new native parameter state resets after cancellation.
The [cancellation follow-up](2026-09-17-windows-helper-cancellation.md) fixes and
tests acknowledgement/output ordering before wiring helper 6. Keep that work
separate from the successful field/readback and ordinary-reset matrix.

## Reproduction and limits

Build the helpers, then run `tools/check_windows_voice_parameters.ps1` with the
selected helper and native DLL in a fresh x86 STA PowerShell process, as in the
initial audit. Allow a bounded 180-second outer timeout per engine for the
expanded matrix. Loading the selected helper bytes lets the probe run against
a WSL checkout without changing Windows assembly-loader policy or installing
that helper. Reproduce the original interior-only evidence using the probe at
commit `338356a`.

Cancellation/failure after native application, multi-parameter dependency
execution, request-level atomic validation, helper 6, ordinary/preview
interleaving, Linux profiles and other dialects remain pending. These tests do
not establish acoustic quality or make the full native-control feature usable.
