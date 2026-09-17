# Helper-6 parent integration, 2026-09-18

The Rust parent now connects the qualified
[Eloquence](2026-09-18-eloquence-helper6.md) and
[DECtalk](2026-09-18-dectalk-helper6.md) handlers. Public native speech operations,
route/preview evidence and the Emacs editor remain separate delivery slices.

## Implementation

A separate strict session decoder owns helper-6 shared envelopes and the required
native start receipt. Legacy codecs and helper-side defaults retain versions 1–5.
The parent offers 6 first, handles unsupported-version replies and negotiates an
older helper without changing its synthesis shape. Ordinary helper-6 synthesis
sends an explicit null native block and requires a null receipt.

Explicit native APIs forward settings, sparse operations, context dimensions,
expected identity and unavailable policy intact. A receipt is checked against the
exact submitted request before native evidence, sink start or PCM can escape.
Existing buffered/progressive collectors retain PCM bounds, marker ordering,
canonical conversion and cancellation ownership. Strict native use of an older
helper fails before synthesis; deliberate common-only use returns degradation.
Native application callbacks are tentative adapter evidence, never proof of
committed PCM or playback. Future routed callers must keep that distinction.

Catalogue and explanation queries try the existing lifecycle lock. Speech,
startup and recovery produce busy without waiting or submitting a helper query.
An absent/deferred connection is unavailable and is not started by browsing.
Admitted writes and replies share a 200 ms deadline, further limited by the
configured request timeout. A joined watchdog retires a helper that stops reading
stdin, so pipe backpressure cannot bypass the deadline. Malformed, mismatched or late replies invalidate the connection; the
existing cleanup deadline remains separate. Cancellation acknowledgements retain
their target ownership and cannot extend a reply deadline indefinitely. No retry
loop, extra worker, background resynthesis or installation action is added.

Each parent retains at most 64 compact applied-plan references. Detail lookup
checks runtime identity and realized voice, while reconnect clears references.
The helper retains the independently bounded full detail. Later public operations
must map these helper-local references into connection-scoped opaque IDs.

## Acceptance

The retained [Rust probe](../../omnivox-tts/examples/helper_parameters_probe.rs)
launches a configured real helper through `HelperTtsEngine`; it does not implement
an alternate protocol client and never opens an audio device. Its arguments are
engine, helper path, voice, parameter, integer value, followed by helper arguments.
The [observations](data/2026-09-18-helper6-parent.json) record the installed qualified
ECI 6.1 and DECtalk 4.99 runtimes. Runtime libraries were not installed or changed.

It checks catalogue assembly, planned/applied explanations, buffered and streaming
native speech, receipt-before-start/PCM, busy queries during speech, cancellation,
ordinary follow-up, strict stale rejection, explicit common-only degradation,
reconnect generation changes, expired detail references and fresh native recovery.

The Linux debug parent launching the Windows helpers retires both engines after
first-PCM cancellation of the long probe, for native and ordinary speech alike.
Recovery starts a fresh helper; it cannot silently reuse the previous identity.
This is distinct from the earlier direct-wire cancellation at synthesis start,
which recovered within the same process. Neither result proves acoustic stop
latency or listening quality. The existing 250 ms watchdog was not changed.

Focused tests cover the strict session decoder, ordinary/legacy compatibility,
receipt validation before output, busy queries and cancellation, stalled-query
retirement, reconnect, bounded history, wrong-runtime explanation rejection and
no metadata-triggered deferred startup. All 85 focused helper tests passed, as
did 913 locked workspace tests; the existing long-session eSpeak stress test
remains ignored. Workspace Clippy, formatting, local documentation links and the
Emacsvox documentation gate passed.

The final probe also passed as a native Windows x64 GNU executable, built from
the same source with the pinned release compiler. It used a separate temporary
Windows directory with the matching GCC runtime files. DECtalk recovered from
both first-PCM cancellations without replacing its helper. Eloquence required
watchdog retirement for both ordinary and native cancellation; fresh connection,
catalogue identity and synthesis recovery passed. Eloquence
long-utterance cancellation/restart remains a measured limitation for follow-up. Neither engine's helper source changed in this slice.

Full Windows development staging passed as `c9ff5b92bc933fe1` from the isolated
final source snapshot. Deterministic helper checks, package checksums, live voice
inventories and Flite, RuTTS and TGSpeechBox synthesis passed. Both packaged
Windows helper hashes match those qualified by the native parent probes. The
staged Omnivox executable separately produced finite 44.1 kHz stereo WAV output
through Eloquence and DECtalk, with matching helper completion events. Dump-WAV
mode does not initialize Rust tracing; the native parent probes establish helper-6
operation, while these packaged checks establish ordinary main-server synthesis.
Piper is omitted by the existing development staging policy.

The desktop launcher remains on `e1ecdb481ee08fd0`; no live Emacs was restarted.
The earlier staging attempt was stopped when review found the blocked-write
deadline gap. Only the complete final build above is acceptance evidence.
