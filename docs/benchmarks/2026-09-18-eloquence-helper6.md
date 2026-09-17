# Eloquence helper-6 handlers, 2026-09-18

This connects the [reserved codecs](2026-09-18-helper6-parameter-codecs.md) to
Eloquence's [qualified native execution](2026-09-18-eloquence-native-execution.md).
It implements direct helper-6 use, not public client or Emacs parameter editing.

## Implementation

The shared Windows host negotiates 6 only for adapters implementing its optional
parameter interface. Eloquence implements catalogue queries, draft/applied
explanations and native synthesis; DECtalk and missing-runtime hosts retain 1–5.
The Rust parent still offers 5 first and cannot yet dispatch native settings.

Eight ECI controls have typed integer ranges, labels, mapping relationships and
verified reset support. Preset default values remain unknown in the catalogue.
The protocol thread never selects a native preset to browse it. A single
background DLL qualification performs no native calls; queries return busy while
it runs and unavailable if still unfinished after ten seconds. Actual parameter
application independently checks the qualified runtime and ECI units on the STA
owner, composes the existing common mapping, verifies all eight values and
restores the pristine preset after success, cancellation or failure.

A v6 synthesis start carries required nullable application evidence. Native
application/readback precedes that frame and PCM. The host guards start, stream
frames and cancellation under the existing state lock and rejects early audio,
duplicate starts or a missing receipt. Versions 1–5 retain ordinary synthesis.

Draft rows reuse the exact mapping and frozen native edits, without claiming
readback. Applied rows carry verified native values and a process-scoped plan ID.
Retention is bounded to 64 plans and 256 KiB; expiration cannot start synthesis.
Catalogue revisions hash canonical metadata independently of connection identity.
A new helper has a fresh positive random runtime generation. Strict stale/native
mismatches fail; explicit common-only requests can degrade with a reason.
Malformed values, fields and identities cannot silently degrade.

## Verification

The silent [direct-wire acceptance tool](../../tools/test_helper6_eloquence.py)
passed 57 cases against the installed qualified ECI 6.1 DLL. It covers all eight
presets, set/default operations, context masking, planned versus applied values,
malformed data, stale runtime identity, common-only degradation, omitted common
fields, integer-token validation, 65 later applications evicting an older plan,
reconnect identity and ordinary version-5 synthesis/recovery. Real helper-6
streaming also delivered requested start/end markers, cancelled without later
speech frames after its acknowledgement, and accepted clean ordinary and native
follow-up requests. [Retained evidence](data/2026-09-18-eloquence-helper6.json)
records the accepted helper hash and streaming observations.

Repeated ordinary ECI utterances have different PCM hashes even before native
edits, but matching frame counts. The acceptance tool therefore checks stable
frame counts and native readback/reset guards, not byte-identical synthesis.
It captures audio silently and does not establish perceived sound quality.

The deterministic shared-host suite passed six native receipt-ordering cases,
39 cancellation cases and 110 legacy malformed-wire/recovery cases. It holds
preparation before receipt, queries the catalogue without releasing it, accepts
cancellation, and verifies no later receipt/PCM escapes. Both buffered and
progressive paths are covered. Early PCM and omitted receipts fail without audio;
ordinary speech remains usable afterward. The 12 helper source checks passed.

Both changed Windows helper binaries also passed 20 ordinary version-5
syntheses and four in-flight cancellations each through the existing soak tool,
with progressive PCM, marker bounds, health pings and clean shutdown checked.

Nine [captured exchanges](../protocol-fixtures/eloquence-helper6-runtime.json)
round-trip through Rust validation and catalogue assembly. This caught a unit
label that needed the schema's identifier form (`eci`); it was corrected before
qualification. All 13 focused codec tests pass. The final locked workspace
passed 901 tests with one long-session eSpeak stress test ignored. Workspace
Clippy, formatting, three release-source tests and local documentation links
also passed.

Full Windows development staging passed as `1092ac6518d5e78d` from a separate
source snapshot and runtime root. Pinned helper determinism, artifact checksums,
live inventories and Flite, RuTTS and TGSpeechBox synthesis passed. The staged
Eloquence executable passed the same 57 direct helper-6 cases. Both staged helper
hashes match the binaries used for the native and version-5 compatibility checks.
The development payload omits Piper under the existing staging policy.

Emacsvox's documentation gate passed. An earlier shared-worktree check found
concurrent manual edits whose generated files were not yet current; those edits
were left intact, and the gate passed after their separate documentation commit.
The desktop launcher remains on `e1ecdb481ee08fd0`; no live Emacs was restarted.

## Remaining integration

DECtalk still needs its helper-6 handler. Rust must connect the new operation
codecs to shared stream frames, scheduling and per-request evidence before public
speech/preview transport or the Emacs native editor can use them. End-to-end
fallback, multiple logical voices/spans and both speech lanes remain future
acceptance work. Longer malformed-stream/fault-injection runs can add hardening.
