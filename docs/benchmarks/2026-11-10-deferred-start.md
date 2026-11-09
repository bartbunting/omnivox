# Prepared emergency startup and Windows launch costs, 2026-11-10

## Result

Keep the lightweight speech and reload path. Prepare the actual emergency owner
while old speech remains available, then release its single native attempt after
verified old retirement. This removes the separate capability process without a
potentially stale capability cache. Native initialization still occurs only after
old output has retired. Executable/startup verification at native launch remains.

In eight matched, unprofiled pairs per platform, median emergency operation time
fell from 579 to 393 ms on WSL-to-Windows and from 84 to 76 ms on Linux. The bound
from retirement request to the first subsequently requested source event fell
from 415 to 190 ms and from 92 to 63 ms respectively. This is useful progress,
not acoustic qualification or recovery after loss of native ownership.

The implementation follows [ADR 0018](../adr/0018-minimal-emergency-speech.md).
The [exact contract](../reference/engine-startup-snapshot.md#explicit-minimal-emergency-startup)
describes deferred preparation, retry, cancellation and older-runtime rejection.
[Fixtures, provenance and raw evidence](data/2026-11-10-deferred-start/README.md)
retain this comparison separately from all previous reports.

## Matched client comparison

Both variants use the same new native payload on each platform. The legacy
variant explicitly loads the two saved, compiled client modules from Emacsvox
`25cc0e24b`; the candidate loads the newly compiled modules from the isolated
lightweight branch. Loaded paths are asserted. Emacs is 31.0.90. Neither variant
uses a live profile. Native output is null; ordinary old speech uses real Flite,
replacement uses bundled eSpeak, and the notification owner remains Flite.

Each platform runs one warmup pair, eight measured pairs with alternating order,
then two separately profiled pairs. Platforms run sequentially after compilation
and core tests complete. Every process has fresh private configuration and state.
The first Flite-to-eSpeak operation supplies each measured row; additional minimal
restarts are functional checks, not additional independent comparison samples.

| Platform and client | Operation, median [min–max] ms | Retirement-request to requested source, median [min–max] ms |
| --- | --- | --- |
| Linux, legacy | 83.725 [76.294–112.101] | 92.308 [86.509–121.782] |
| Linux, prepared | 76.473 [69.130–90.715] | 62.558 [57.076–73.530] |
| WSL-to-Windows, legacy | 579.006 [555.569–604.815] | 414.604 [383.743–437.882] |
| WSL-to-Windows, prepared | 392.556 [352.032–404.675] | 189.825 [183.454–223.320] |

The operation ends after the replacement's own registration and client setup.
The later source probe also includes deliberate fixture waiting and request
dispatch, so its interval is an upper bound on a source-level gap. It is not
first PCM, audio-device output or acoustic silence. Eight pairs do not establish
tail latency. The raw observations retain individual phase and notification times.

Old retirement itself has similar median duration: about 11 ms on Linux and
12 ms on Windows. Before retirement, the prepared client now spends about 29 ms
on Linux and 225 ms on Windows constructing its real candidate while old speech
is still available. After confirmed retirement, its median remaining operation
time falls from 67 to 36 ms and from 376 to 153 ms respectively. Medians of phases
need not sum to the median of their total; profiled nested waits also overlap.

Every pair deliberately loses the first actual retirement reply, prevents its
durable receipt write and corrupts current preferences/host data. Notification
speech is submitted during recovery, completes on its unchanged owner and has
median client-observed source delay around 17 ms for both variants/platforms.
Management bytes remain unchanged. Linux also crashes the exact replacement
worker and restores minimal speech on the next character. Repeated explicit
minimal restarts pass on both platforms.

## What the Windows difference contains

The retained launch-only probe executes the early, engine-free
`--emergency-version` entry. There are 24 measured starts and one warmup per case:

| Launch route | Median [min–max] ms |
| --- | --- |
| Linux direct in WSL | 2.202 [1.887–4.153] |
| Linux through the launcher | 5.963 [4.573–7.930] |
| Windows direct from WSL | 196.855 [180.689–222.346] |
| Windows through the WSL launcher | 202.930 [191.273–219.394] |
| Windows from a native PowerShell/.NET parent | 87.058 [82.636–92.704] |

This establishes a large process-start/exit cost before engine work. The native
parent uses a different API and runs separately; subtracting these medians does
not isolate WSL overhead precisely. These observations do not identify antivirus,
loader, filesystem or scheduler costs individually. The shell wrapper adds little
compared with the native executable invocation in this setup.

Two extra Windows profiling runs explicitly forward the diagnostic flag through
`WSLENV`; the earlier profiled pairs instrumented only the Emacs side on Windows.
Across four prepared starts in one corrected run, native worker `Command::spawn`
takes median 53.582 ms [51.755–57.020], versus 0.209 ms [0.176–0.244] across ten
Linux prepared starts. Windows minimal snapshot preparation is about 22 ms and
immediate prelaunch verification about 11 ms, close to the Linux measurements.
Sending the owned START frame is about 0.2 ms on both. These small profiled samples
are attribution evidence, not stable platform performance claims.

The previously retained engine logs independently show eSpeak initialization
through readiness around 52 ms on Windows versus 4 ms on Linux in this setup.
Thus removing the extra capability launch has a measured benefit, while Windows
worker creation and engine initialization still account for substantial remaining
time. No poll interval change, integrity bypass or assumption that fewer processes
always means faster speech is needed for this conclusion.

## Failure checks and scope

Native Linux and Windows fixtures verify no child before release, wrong-identity
rejection, cancellation before release, no second worker after duplicate release,
invalid output/missing data while old speech works, and failed initialization
after successful preparation. Linux additionally observes actual child identities;
Windows checks the native protocol and successful tree retirement. Losing a
release reply retries only the same attempt. After possible release, an uncertain
attempt remains retained and blocks another preparation.

Fresh compiled client checks against the previous native payload reject the new
option while preserving actual main and notification Flite speech on both
platforms. A separate Linux Piper-to-minimal compiled recovery passes. There is
no native Windows Piper payload in this build. Native macOS, native Windows Emacs,
listening, acoustic onset and Stop-to-silence remain unqualified.

Locked workspace tests pass: 1,213 tests, two ignored. Locked CLI/TTS Clippy with
Piper and all targets passes. The core client suite passes 1,066 tests with 18
environmental skips; all 20 focused recovery tests also pass against fresh
byte-code. Full private Windows development staging and native runtime checks
pass. Early fixture/build errors, including a missing dynamic-variable declaration
caught by the compiled suite, are retained with their corrected checks.

No ordinary speech admission or reload code changes in this increment. The
previous matched release comparisons remain applicable to that unchanged path;
this report makes no new steady-state or release-publication claim. Installed
speech, maintenance main and the original session experiment remain untouched.

## Next reliability boundary

The [owner-failure counterexamples](2026-11-10-owner-authority.md) still apply:
an owner blocked writing speech cannot service retirement, and a dead owner may
leave a paused worker without reachable cleanup authority. Faster preparation
does not solve either case. The next increment must preserve an independent
native guard, independent lifecycle communication, exact tree and I/O proof,
bounded handoff and safe failure when the guard itself is lost. Keep that work
outside ordinary preference coordination and retain the existing speech hop count.
