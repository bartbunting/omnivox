# Omnivox failure diagnostics

The Emacsvox WSL launcher writes each OmniVox session to a separate file under
`$XDG_STATE_HOME/emacsvox/omnivox`, or
`~/.local/state/emacsvox/omnivox` when `XDG_STATE_HOME` is unset. Set the
launcher-only `OMNIVOX_LOG_DIRECTORY` variable to use a different Linux
directory.

The log correlates the Rust synthesis worker and 32-bit helper processes using
UTC timestamps, process and thread IDs, helper request IDs, logical and
physical voices, text byte counts, frame and marker counts, elapsed time,
fallback decisions, recovery probes, native-call boundaries, and panic
backtraces. It deliberately does not record synthesized text. Ordinary
OmniVox logging remains at info level because existing debug messages can
contain protocol text.

If speech stops, collect evidence before manually restarting it:

```sh
cd ~/src/omnivox
tools/collect_diagnostics.sh
```

The command prints the generated archive path. It includes bounded excerpts
from logs written during the previous 24 hours, source and runtime versions,
relevant WSL and Windows process inventory, Windows Application events, and a
listing of available crash dumps. It does not include dump contents. Inspect
the archive before sharing it.

## Native helper crashes

Managed exceptions and protocol failures appear in the session log. A native
failure inside `ECI.DLL` or `DECtalk.dll` can terminate the 32-bit helper
without running managed exception handlers. Windows Error Reporting can retain
a full dump for that case.

From an elevated PowerShell (a PowerShell started with **Run as
administrator**), run:

```powershell
& "\\wsl.localhost\Ubuntu-26.04\home\bart\src\omnivox\tools\configure_windows_crash_dumps.ps1"
```

Use the actual WSL distribution and source path if they differ. The script
keeps at most five dumps per helper in
`%LOCALAPPDATA%\Emacsvox\Omnivox\dumps`. Add `-IncludeServer` to cover the
64-bit Rust process too. Preview registry changes with `-WhatIf`.

Full dumps can contain spoken text, voice settings, paths, and unrelated
process memory. Keep them private. After reproducing the failure, disable the
per-application policy with:

```powershell
& "\\wsl.localhost\Ubuntu-26.04\home\bart\src\omnivox\tools\configure_windows_crash_dumps.ps1" -Disable
```

Pass `-IncludeServer` during removal if it was supplied during setup. Windows
requires administrator privileges and `HKEY_LOCAL_MACHINE` configuration for
WER LocalDumps; per-process settings override global settings. See Microsoft’s
[WER settings documentation](https://learn.microsoft.com/windows/win32/wer/wer-settings).

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
