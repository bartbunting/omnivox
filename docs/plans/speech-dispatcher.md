# Speech Dispatcher integration feasibility

Status: unimplemented proposal. [The roadmap](../ROADMAP.md#explicit-future-proposals)
tracks priority; [ADR 0001](../adr/0001-engine-isolation-and-distribution.md)
governs native isolation. This is a feasibility investigation, not a ready-to-build
backend specification. No dependency or public protocol change is accepted here.

## Purpose and architectural constraint

Expose configured Speech Dispatcher output modules on Linux while preserving
Omnivox fallback, cancellation and truthful completion. Speech Dispatcher normally
owns playback instead of returning PCM. A successful `spd_say()` submission or an
empty `AudioBuffer` cannot represent completed Omnivox playback.

An integration must explicitly represent external playback and its reduced
capabilities. Speech would bypass Omnivox's PCM mixing, trimming, effects and
precise frame clock. Tones and icons would still use Omnivox output. Their ordering,
notification routing and completion need a defined contract before implementation.
The current engine result cannot express this simply by returning empty PCM.

## Questions to resolve

- Determine whether the supported daemon/module path can supply PCM, or whether
  an external-playback result and corresponding architectural decision are needed.
- Establish stable module/voice identities and request-local language, voice,
  rate, pitch and volume. Apply Omnivox's `0.5` normal-rate point and `0.0..2.0`
  range using verified native limits. Do not restore process-global language state.
- Correlate END/CANCEL callbacks by client and message IDs. Prove connection-loss,
  timeout and late-callback behavior so an old event cannot complete new speech.
- Verify the supported concurrency contract: a thread waiting for completion
  must not hold a lock that prevents cancellation or callback progress.
- Determine whether one serialized connection isolates request settings, or
  independent connections are required. Do not override the user's output module
  implicitly or infer sink routing from process environment alone.
- Define compatibility with tickets, keyed replacement, timeline actions,
  fallback after externally committed audio and independent notification lanes.
- Select dependency/feature placement and native CI coverage. A Linux-only sys
  crate in workspace members would affect the locked whole-workspace gate.

## Investigation inputs

Inspect the exact supported upstream version's
[`libspeechd.h`](https://github.com/brailcom/speechd/blob/master/src/api/c/libspeechd.h)
and [`spd-say` implementation](https://github.com/brailcom/speechd/blob/master/src/clients/say/say.c).
The earlier investigation identified threaded connections with END and CANCEL
notifications as the completion path; `SPD_MODE_SINGLE` is not synchronous
playback and there is no assumed `spd_say_sync()` interface. Reverify these
observations against the selected version before writing bindings.

Use a private test connection and an explicitly configured daemon/module.
Measure actual callback sequences and output cancellation, including daemon
loss and reconnect. Check voice/language discovery and empty input handling.
A local wakeup or successful cancel request alone does not prove output stopped.

## Acceptance before implementation

Produce the capability/result contract and compatibility decision first. A useful
prototype must demonstrate correlated completion, bounded cancellation and loss
handling, truthful degraded features, two-lane isolation and no stale output.
Retain native reports and distinguish callback timing from audible completion.
Keep Omnivox's existing eSpeak Linux fallback available. Historical FFI and file
modification sketches remain in Git; they are not current implementation guidance.
