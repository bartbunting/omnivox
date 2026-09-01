# Omnivox failure diagnostics

The Emacsvox WSL launcher writes each Omnivox session under
`$XDG_STATE_HOME/emacsvox/omnivox`, or
`~/.local/state/emacsvox/omnivox` when `XDG_STATE_HOME` is unset. A session
normally spans bounded files named `omnivox-...-partNNNNNN.log`; if the log
filter is unavailable, the launcher falls back to one unnumbered session log.
Set the launcher-only `OMNIVOX_LOG_DIRECTORY` variable to use a different Linux
directory. The launcher creates the directory with mode `0700` where possible
and log parts with mode `0600`.

Each part defaults to an approximate 16 MiB limit. Closed parts are retained
within both a 16-file and 256 MiB aggregate limit. Configure those values with
`OMNIVOX_LOG_MAX_FILE_BYTES`, `OMNIVOX_LOG_RETAINED_FILES`, and
`OMNIVOX_LOG_RETAINED_BYTES`; see [ENV-VARS.md](ENV-VARS.md) for exact
semantics. The active part of each live session is protected from pruning.

The log correlates launcher/session identity and server events using UTC
timestamps, helper request IDs, logical and physical voices, text byte counts,
frame and marker counts, elapsed time, fallback decisions, recovery probes,
native-call boundaries, and panic backtraces where those fields apply. It does
not record synthesized text by default. Set
`OMNIVOX_LOG_SYNTHESIS_TEXT=1` before launching Emacsvox to add an escaped
`synthesis_text` field for every routed engine attempt. The server emits a
startup warning whenever this sensitive mode is active. Ordinary Omnivox
logging remains at info level because existing debug messages can contain
protocol text.

Full synthesis-text logs can contain passwords, private messages, document
contents, and other sensitive material. Even without full text, a collected
archive includes usernames and source paths, command lines, process inventory,
Git status, runtime identity, and relevant Windows events. Keep logs private,
inspect diagnostic archives before sharing them, and unset
`OMNIVOX_LOG_SYNTHESIS_TEXT` when the capture is no longer needed.

## Latency and lifecycle records

The normal info-level log records monotonic elapsed microseconds for each
speech request. Tracked and marker-aware requests use their dispatch ID as
`request_identifier`; preview requests use their control request ID. Events
inside the `speech_request` tracing span inherit that identifier, request kind,
stop epoch, and the submitted protocol generation when the request is a
structured timeline.

The lifecycle is reported at these boundaries:

| `lifecycle_stage` | Meaning |
|---|---|
| `protocol_admitted` | The bounded synthesis queue accepted the request. |
| `worker_started` | The synthesis worker dequeued it; `queue_wait_us` measures time since admission. |
| `synthesis_started` | One engine attempt began. A fallback retry produces another attempt record. |
| `synthesis_completed` or `synthesis_failed` | That engine attempt ended; `synthesis_elapsed_us` covers the native or helper call and result validation. |
| `worker_finished` | Request processing finished; the record includes time since admission and, when audio was accepted, `admission_to_audio_queued_us`. |
| `playback_terminal` | All tracked mixer sources completed or were cancelled. It summarizes `admission_to_mixer_source_us` for the first consumed speech source and total `admission_to_terminal_us`. |
| `request_retired` or `request_rejected` | Work ended before the synthesis worker could complete it. |

These durations come from one monotonic clock inside Omnivox and can therefore
be compared safely even when Emacs runs in WSL and Omnivox runs as a native
Windows process. Match them with Emacsvox's opt-in aural diagnostic records by
dispatch ID. The first mixer-source measurement is not physical acoustic
onset: operating-system, device, and hardware buffers can add latency after
the mixer requests its first sample.

Use `tools/benchmark_server.py` to collect repeatable client-observed cold and
warm distributions through the public server protocol. Cold samples start a
fresh process; warm samples reuse one process after configurable warmups. The
default cases cover a character, word, ordinary line, dense semantic-action
timeline, multipart timeline, and rapid keyed replacement. Each summary uses
nearest-rank p50, p95, and p99 values and the optional JSON report retains every
raw monotonic sample and actual engine ID:

```sh
python3 tools/benchmark_server.py ../emacsvox/servers/omnivox \
  --engine flite --expected-engine-id flite \
  --mode both --iterations 20 --warmups 2 \
  --provenance ../emacsvox/servers/omnivox-bin/current/PROVENANCE \
  --json-output /path/to/private/flite-latency.json
```

Select individual workloads by repeating `--case`. The `multipart` workload
deliberately fragments a short presentation, so it measures assembly without
turning playback duration into a large-text benchmark. The replacement result
reports the slowest stale-dispatch terminal cancellation in each burst as well
as the winning dispatch's onset. These remain mixer-source observations, not
microphone or physical audible-onset measurements.

Windows Eloquence and DECtalk are runtime-routing inventory IDs, not accepted
startup selectors. Use `--engine native --preferred-engine-id ENGINE` for those
engines. The harness applies that preference through the public control
protocol to every cold process and once to the warm process; keep
`--expected-engine-id ENGINE` so any fallback fails the run instead of entering
the measured distribution.

Use `tools/stress_server.py` to repeat domain-scoped replacement with
interleaved ordered and urgent work. It periodically issues a hard stop and
then verifies recovery. Every dispatch must produce exactly one expected
terminal status; marker sequences must be contiguous; completed survivors must
reach their mixer-source and semantic callbacks; and no marker or callback may
arrive after terminal history:

```sh
python3 tools/stress_server.py ../emacsvox/servers/omnivox \
  --engine flite --expected-engine-id flite \
  --iterations 25 --stop-every 5 \
  --provenance ../emacsvox/servers/omnivox-bin/current/PROVENANCE \
  --json-output /path/to/private/flite-stress.json
```

Helper fault injection is opt-in. The tool snapshots processes before starting
its dedicated server and refuses to act unless it resolves exactly one new
helper with the requested executable name beneath that server. It kills only
that PID, verifies the explicitly configured fallback, requests an engine
recovery probe, and requires a later dispatch to return to the recovered
engine. For the staged Windows Flite runtime:

```sh
python3 tools/stress_server.py ../emacsvox/servers/omnivox \
  --engine flite --expected-engine-id flite --iterations 5 \
  --fault-helper-process omnivox-flite-helper.exe \
  --fault-engine-id flite --fallback-engine-id espeak
```

If speech stops, collect evidence before manually restarting it:

```sh
cd /path/to/omnivox
tools/collect_diagnostics.sh
```

The command prints the generated archive path. It includes bounded excerpts
from logs written during the previous 24 hours, source and runtime versions,
relevant WSL and Windows process inventory, Windows Application events, and a
listing of available crash dumps. It does not include dump contents. The
archive is created with mode `0600`; inspect it before sharing it.

Pass an output path as the first argument when `/tmp` is unsuitable:

```sh
tools/collect_diagnostics.sh /path/to/private/omnivox-diagnostics.tar.gz
```

If the sibling Emacsvox checkout is not at `../emacsvox`, point the collector
at it explicitly:

```sh
EMACSVOX_SOURCE_DIRECTORY=/path/to/emacsvox tools/collect_diagnostics.sh
```

## Native helper crashes

Managed exceptions and protocol failures appear in the session log. A native
failure inside `ECI.DLL` or `DECtalk.dll` can terminate the 32-bit helper
without running managed exception handlers. Windows Error Reporting can retain
a full dump for that case.

First obtain the script's Windows path from the Omnivox checkout in WSL:

```sh
cd /path/to/omnivox
wslpath -w "$PWD/tools/configure_windows_crash_dumps.ps1"
```

Then start PowerShell with **Run as administrator**, paste the printed path at
the prompt below, and run the script:

```powershell
$script = Read-Host "Windows path to configure_windows_crash_dumps.ps1"
& $script
```

The script keeps at most five dumps per helper in
`%LOCALAPPDATA%\Emacsvox\Omnivox\dumps`. Add `-IncludeServer` to cover the
64-bit Rust process too. Preview registry changes with `-WhatIf`.

Full dumps can contain spoken text, voice settings, paths, and unrelated
process memory. Keep them private. After reproducing the failure, disable the
per-application policy with:

```powershell
& $script -Disable
```

Pass `-IncludeServer` during removal if it was supplied during setup. Windows
requires administrator privileges and `HKEY_LOCAL_MACHINE` configuration for
WER LocalDumps; per-process settings override global settings. See Microsoft’s
[WER settings documentation](https://learn.microsoft.com/en-us/windows/win32/wer/wer-settings).

If administrator access is unavailable, skip LocalDumps. The ordinary session
log and `tools/collect_diagnostics.sh` still capture the request, native-call
boundary, helper exit or forced termination, fallback, recovery, process list,
and available Windows Application events. That is normally enough to identify
whether the Rust server, its helper transport, or a native engine failed; only
native stack and memory inspection require a dump.

## Expected failure behaviour

Helper transport failures invalidate the child, open the engine circuit, and
retry the same chunk through the configured fallback route. This includes
ordinary speech without a named logical voice. A retryable native synthesis
failure also retires the helper so a potentially damaged Eloquence or DECtalk
session is never reused.

If a helper does not finish a requested cancellation within 250 milliseconds,
Omnivox terminates it so the synthesis worker cannot remain blocked behind a
native call. Cancellation belongs to the superseded request and does not by
itself make the engine unhealthy: the next live request negotiates a fresh
helper immediately. A genuine runtime failure still uses the circuit breaker;
after cooldown, one request reconnects the helper and acts as a recovery
probe. Voice identifiers reported by eSpeak are matched case-insensitively
against its inventory so backend language-tag normalization cannot prevent
fallback.

If the Rust synthesis worker itself panics, Omnivox writes a forced backtrace
and exits with status 70. Emacs retires that failed process and initializes a
replacement when speech is next requested, rather than leaving a live control
channel with a dead synthesis queue.
