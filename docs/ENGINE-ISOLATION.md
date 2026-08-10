# Uncancellable Engine Isolation

Omnivox keeps its sole synthesis worker responsive when an engine cannot stop
an in-progress native call. The server wraps WinRT and helper-backed engines in
a generation-aware execution boundary. Synthesis runs on a detached task while
the worker polls the same generation counter used to reject stale playback.

When a stop or replacement advances the generation, the wrapper asks the
engine to stop, marks the native task quarantined, and returns without exposing
its eventual PCM. A later request sees that engine as temporarily unavailable,
so normal logical-voice routing selects its configured fallback. Once the old
task exits, the slot is released and the engine can participate again. There is
never concurrent access to a serialized engine instance.

The resource policy is deliberately conservative:

- one active or quarantined isolated call per engine;
- two active or quarantined isolated calls across the server process;
- at either limit, no task is spawned and the engine reports unavailable to
  normal fallback routing;
- detached tasks are never joined during shutdown, so an unreturning native
  call cannot keep the speech server alive;
- stale results are discarded before routing, effects, or playback.

Counting active calls against the quarantine budget is stricter than counting
only abandoned work. It closes the race where the global limit could be filled
after a native call starts but before that call needs quarantine.

WinRT remains truthful as `playback_only`: Omnivox does not claim to cancel the
Windows native operation, only to isolate its stale result. Piper is different.
`make build-piper` produces `omnivox` and `omnivox-piper-helper` beside one
another. The main server launches the helper through the versioned JSON
protocol, and the helper advertises `synthesis_and_playback` because the host
can terminate its process. A cancel is acknowledged on the helper protocol
thread; if Piper's synchronous call has not returned within 250 ms, the generic
helper watchdog kills and reaps the process. The next usable request negotiates
a fresh helper and reloads the model.

By default the server resolves `omnivox-piper-helper` beside its own executable.
`OMNIVOX_PIPER_HELPER` may provide an explicit path, and
`OMNIVOX_PIPER_MODEL` or `--piper-model` supplies the model. Paths are passed as
separate process arguments, so spaces do not require shell quoting. Deployment
must copy both executables and the helper's native dynamic libraries together;
the main `omnivox` binary no longer carries Piper rpaths or native linkage.
