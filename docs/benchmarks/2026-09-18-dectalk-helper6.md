# DECtalk helper-6 handlers, 2026-09-18

This connects the [qualified native execution](2026-09-18-dectalk-native-execution.md)
to the helper-6 boundary introduced by [Eloquence](2026-09-18-eloquence-helper6.md).
The Rust parent continues to negotiate helper 5; this is not yet an Emacs editor
or public native-parameter capability.

## Implementation

DECtalk now implements the existing optional helper-6 interface. It exposes 28
qualified design-voice controls, native set/default operations, draft explanations,
application receipts before PCM, and retained applied explanations. It uses the
existing strict wire reader, state/cancellation guards and native reset boundary.
No shared host or Eloquence handler code changed.

Metadata stays within the DECtalk adapter. Labels and units follow the runtime
source's `SPDEFS` and `define_options` tables; ranges are the already qualified
runtime limits. Catalogue defaults remain unknown with verified reset support.
Its single page neither selects a voice nor calls the native readback API.
Background DLL qualification returns busy without holding synthesis locks and
has the same ten-second reporting bound as Eloquence. Actual native execution
still verifies runtime identity, runtime limits and complete readback.

Planned explanations use the existing common tables, rounding and pitch mapping.
They account for the qualified runtime's 350 Hz pitch ceiling and stress minima
(`hr=2`, `sr=1`). Those predictions do not change the commands used by ordinary
speech. Native explicit values retain their separately checked bounds. Rate and
PCM volume have no outputs among these 28 controls and do not mask native edits.
Unmapped or explicitly reset preset values remain unknown until applied readback.

Plans retain at most 64 entries and 256 KiB. Metadata revisions are independent
of the new helper's random runtime generation. Stale strict requests fail;
explicit common-only degradation reports a reason. Malformed native data fails
even when context would mask the invalid value or common-only was requested.

## Acceptance

The [direct-wire suite](../../tools/test_helper6_dectalk.py) passed 121 cases
against the installed qualified DECtalk DLL and matching dictionary. It captures
PCM silently and never installs or changes a runtime. Coverage includes:

- Both limits for all 28 controls, with planned/applied comparison and readback.
- All nine voices, complete default operations, and common/context mappings at
  zero, midpoint and maximum; the known pitch and stress clamps are checked.
- Unknown preset defaults, explicit native overrides, omission of optional common
  fields, malformed/masked data, stale identities and common-only degradation.
- Bounded history eviction, connection restart identity, ordinary helper-5
  synthesis/recovery, requested markers and real progressive cancellation.
- Catalogue access during active synthesis without waiting for it to finish,
  suppression of speech frames after cancellation acknowledgement, and clean
  ordinary and native follow-up speech.

The [compact evidence](data/2026-09-18-dectalk-helper6.json) identifies the tested
helper and streaming observations. Ten
[captured exchanges](../protocol-fixtures/dectalk-helper6-runtime.json) exercise
Rust decoding, request correlation and catalogue assembly, including a planned
common-clamp example. The native audio checks establish framing and readback,
not listening quality or every native failure mode.

The final locked workspace passed 902 tests, with the existing long-session
eSpeak stress test ignored. All 14 parameter-codec tests, workspace Clippy,
formatting, 12 helper-source checks and three release-source tests passed.
The shared host suite passed six receipt-ordering, 39 cancellation and 110
malformed-wire/recovery cases. The changed DECtalk binary also passed 20 ordinary
helper-5 syntheses and four cancellation/recovery probes, including progressive
PCM, markers and health pings. Local documentation links passed.

Full Windows development staging passed as `7e111b38b3f5c8a3` from a separate
source snapshot and runtime root. Pinned helper determinism, artifact checksums,
live inventories and Flite, RuTTS and TGSpeechBox synthesis passed. The packaged
DECtalk executable passed the same 121 direct helper-6 cases; its hash matches
the locally qualified binary. The Eloquence helper hash is unchanged from the
previous qualified slice. The development payload omits Piper under the existing
staging policy.

The Emacsvox documentation gate passed. The desktop launcher remains on
`e1ecdb481ee08fd0`; no live Emacs was restarted. These checks capture PCM silently
and do not establish perceived sound quality.

## Remaining integration

Both Windows handlers are now connected. The parent dispatcher still needs
helper-6 stream-frame handling, query scheduling, parameter forwarding and
per-request evidence. Public ordinary-speech/preview transport, actual fallback,
multiple spans, independent speech lanes and the Emacs native editor follow.
Other native runtimes/platforms remain separately qualified profiles.
