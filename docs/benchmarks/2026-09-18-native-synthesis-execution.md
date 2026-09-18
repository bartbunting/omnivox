# Native synthesis execution, 2026-09-18

This slice connects the existing helper-native APIs to `TtsEngine` and its
cancellation isolation and voice eligibility wrappers. It follows
[public catalogue discovery](2026-09-18-public-parameter-catalogues.md).
Native choice registration, routing, preview evidence and the Emacs parameter
editor remain subsequent work. No additional public capability is advertised.

## Behavior

Buffered and progressive methods carry the complete qualified native block,
including explicit zero/default operations, common context dimensions, runtime
identity and unavailable policy. Existing ordinary request/result shapes and
methods retain their behavior. A native-capable helper dispatches through its
existing validated execution path. Unsupported adapters reject strict native
requests before synthesis. Explicit common-only requests retain the adapter's
ordinary streaming path and return a degradation receipt.

Voice eligibility applies before either native method can reach the adapter.
Cancellation isolation retains its one-call-per-engine and two-call-per-process
limits, bounded stream relay, deferred recovery and obsolete-result quarantine.
Native application evidence travels through the same relay before stream start,
PCM and markers. Audio cannot reach the sink without a valid receipt and start;
repeated, late, mismatched or disallowed degraded receipts terminate the stream. Cancellation is rechecked
after receiving an event, so cancellation during that wait also suppresses native evidence.
Requests freeze their settings when dispatched to the isolated worker.

Application receipts remain tentative adapter evidence. They do not establish
that PCM was accepted or playback began; the future routing and public evidence
layers must establish those separate facts.

## Verification

Six new regressions cover native trait dispatch through eligibility, disabled
voices, intact zero/default/context operations, buffered and progressive output,
strict unsupported rejection, explicit common-only streaming, invalid receipts,
cancellation before a late receipt, quarantined buffered results and clean
ordinary follow-up. Existing isolation tests cover bounded admission and stream
cancellation/backpressure through the same execution machinery.

The retained [helper probe](../../omnivox-tts/examples/helper_parameters_probe.rs)
now sends explicit native synthesis through `TtsEngine` and the eligibility
wrapper. Its [observations](data/2026-09-18-native-synthesis-execution.json) cover
both installed qualified Eloquence and DECtalk runtimes, with a Linux parent
and Windows helpers. Both produced nonempty buffered and progressive PCM,
rejected stale strict requests, explicitly degraded common-only requests and
recovered with fresh identities after cancellation. In these probes both helpers
were retired after first-PCM cancellation, for native and ordinary speech.
No audio device was opened, and these results establish no listening quality
or acoustic stop latency. Isolation is exercised by the synthetic regression
suite; the real-helper probe exercises native dispatch and eligibility.

Concurrent unrelated source edits were preserved. The final Windows build uses
an isolated checkout containing only this slice on top of `10e7dda`.

The isolated workspace passed 927 distinct tests plus one child-process rerun,
with no failures and one existing long-session eSpeak stress test ignored.
Locked workspace Clippy across all targets, formatting and whitespace checks
passed for that exact source.

## Windows staging

Full development staging passed as `7e38878e1051c7db`, including deterministic
Windows helper verification, package checksums, inventories, and live Flite,
RuTTS and TGSpeechBox companion synthesis. The native source diff hash is
`f36d8f2b207569532eee3a4fcd79d4e3aa7d7e1377e7638e5e7e7b3f67ed67bf`;
it exactly matches implementation commit `fbddf32` relative to `10e7dda`.
Eloquence and DECtalk helper hashes match the binaries used by the native probes.
The main Windows build compiles the new execution interface; native application
through that interface is covered by the separate helper probe and isolation
tests described above, since public native routing is still pending.

An initial packaging attempt failed because the temporary shared clone referred
to Git objects outside Docker's mounted source. The checkout was made
self-contained, verified with `git fsck`, and the supported full development
target was rerun successfully. The failed attempt is not acceptance evidence.

The runtime is staged under `servers/omnivox-bin/native-execution-check` in the
Emacsvox checkout. It omits Piper under the existing development build policy.
This slice did not select its package in the desktop launcher or restart a live
Emacs session. Final inspection found that the concurrent startup-fallback work
had separately selected runtime `3501b3f4b84bd467`; that selection was preserved.
Emacsvox's documentation gate passed, and commit `2e761a971` records
implementation progress and the remaining integration.
