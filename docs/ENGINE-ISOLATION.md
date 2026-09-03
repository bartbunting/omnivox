# Uncancellable Engine Isolation

Omnivox keeps its sole synthesis worker responsive when an engine cannot stop
an in-progress native call. The server wraps WinRT and helper-backed engines in
a cancellation-aware execution boundary. Synthesis runs on a detached task
while the wrapper polls the hard-interrupt generation, an engine stop epoch,
and any keyed replacement token attached to the request.

A hard stop asks the engine to stop and quarantines the task immediately. A
soft supersession (including a keyed replacement) gives the call 75
milliseconds to return cooperatively before requesting engine stop and
quarantining it. In both cases its eventual PCM is suppressed. A later request
waits at most 350 milliseconds for an occupied engine/process slot, with
cancellation checks during the wait, and then normal routing selects a
configured fallback. Once the old task exits, the slot is released and the
engine can participate again. There is never concurrent access to a serialized
engine instance.

The resource policy is deliberately conservative:

- one active or quarantined isolated call per engine;
- two active or quarantined isolated calls across the server process;
- at either limit, no new task is spawned; after the bounded wait the engine
  reports unavailable to normal fallback routing;
- detached tasks are never joined during shutdown, so an unreturning native
  call cannot keep the speech server alive;
- stale results are discarded before routing, effects, or playback.

Counting active calls against the quarantine budget is stricter than counting
only abandoned work. It closes the race where the global limit could be filled
after a native call starts but before that call needs quarantine.

When a superseded progressive request has already filled the bounded playback
relay, closing that relay is an expected cancellation event rather than a
helper transport failure. The helper client stops forwarding stale PCM but
continues validating and draining that request through its protocol terminal.
This releases the serialized helper connection for the next request without
restarting a cooperative helper. A malformed response, unexpected consumer
failure, timeout, or missed cancellation deadline still retires the helper.

WinRT remains truthful as `playback_only`: Omnivox does not claim to cancel the
Windows native operation, only to isolate its stale result. Piper is different.
The main server launches `omnivox-piper-helper` through the versioned JSON
protocol, and the helper advertises `synthesis_and_playback` because the host
can terminate its process. A cancel is acknowledged on the helper protocol
thread. The maintained libpiper API returns sentence-level audio chunks, so the
worker observes a stop between chunks. If the current native inference call has
not returned within 250 ms, the generic helper watchdog kills and reaps the
process. The next usable request negotiates a fresh helper and reloads the
model. Cooperative cancellation instead preserves the loaded process through
the drain described above.

By default the server first resolves `piper/omnivox-piper-helper` beside its own
executable, then accepts the legacy directly adjacent helper.
`OMNIVOX_PIPER_HELPER` may provide an explicit path, and
`OMNIVOX_PIPER_MODEL` or `--piper-model` supplies the model. Paths are passed as
separate process arguments, so spaces do not require shell quoting. The
companion directory keeps the helper, its native libraries, and its generated
eSpeak data together; the main `omnivox` binary carries no Piper rpaths or
native linkage.
