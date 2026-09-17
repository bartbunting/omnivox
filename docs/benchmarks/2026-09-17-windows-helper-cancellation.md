# Windows helper cancellation ordering, 2026-09-17

## Failure and correction

The [native-binding audit](2026-09-17-native-parameter-bindings.md) exposed a
pre-existing race in the common Eloquence/DECtalk helper host. It set the
cancellation flag, called native stop, then acknowledged cancellation. The
worker could finish during native stop and emit `synthesis_cancelled` first.

The host now publishes the flag and `cancel_accepted` under the same guard used
by every synthesis audio, marker and terminal frame. An accepted cancellation
suppresses subsequent PCM and markers and replaces a pending success or failure
terminal with `synthesis_cancelled`. Terminal publication also retires the active
request under that guard: completion winning the race makes a later cancellation
an inactive-target error, rather than acknowledging an already completed request.
Native stop runs outside the guard so callbacks can finish. A native stop error
is logged without emitting a second response to the acknowledged cancel request;
the worker and the existing parent cancellation watchdog remain responsible for
termination. Shutdown can still cancel an active worker without a cancel request.

This preserves helper protocol versions 1–5 and their existing wire shapes. It
does not introduce helper 6 or execute the new native parameter bindings.

## Deterministic regression coverage

Run `make windows-helpers-cancellation-test` on WSL, or
[`tools/test_windows_helper_cancellation.ps1`](../../tools/test_windows_helper_cancellation.ps1)
in Windows PowerShell. It compiles the actual host with a fake capture engine
and controlled text streams; no proprietary DLL or audio device is needed.

All 37 cases pass. The unfixed host, with only the internal stream-injection
constructor added, fails the first case because native stop runs before the
acknowledgement. Event barriers, rather than timing sleeps, exercise:

- native stop waiting for the worker's cancelled terminal, proving the output
  guard is not held across stop;
- success, failure and late audio/marker callbacks after cancellation;
- native stop throwing after cancellation has already been acknowledged;
- a native call returning while the acknowledgement writer is blocked;
- completion already publishing when cancellation arrives;
- shutdown of active speech without an explicit cancellation acknowledgement;
- inactive-target cancellation, subsequent speech and health ping recovery;
- all five protocol versions, including both buffered and progressive version 5.

Each wait is bounded, and the test runner kills its own test process after
60 seconds if it cannot finish. The stream-injection constructor is internal;
production helpers continue to use their standard input and output.

## Native acceptance and deployment

Each reproducibly compiled helper passed 50 ordinary synthesis requests and
50 cancellation/recovery probes under protocol 5. The checks require streaming
PCM, validate markers and requested anchors, exercise all six common controls,
and interleave health pings before clean shutdown:

- [Eloquence ECI 6.1.0.0 report](data/2026-09-17-eloquence-cancellation.json).
- [DECtalk v4.99 GitHub NORMAL ACCESS32 report](data/2026-09-17-dectalk-cancellation.json).

Both reports use `--iterations 50 --cancel-every 1 --health-every 5
--resource-sample-every 0 --require-streaming` with all six `--require-acss`
controls and the user-installed native DLL. The helper binaries were built by
the guarded Emacsvox development target using its pinned Roslyn compiler and
.NET reference assemblies. Their identities are:

| Input | SHA-256 |
| --- | --- |
| Eloquence helper | `12029ea4ccd31ff245a5660f1cdf7adea7a14333b1e784a9107917d5a0e50623` |
| ECI DLL | `da99080288cdca14a7effba20274af1d6d5878840e32be5a315bd8691124703b` |
| DECtalk helper | `3819fe218488f80d508d01e66038ced0463decd9eba8cb12ada1dba8b61ad22c` |
| DECtalk DLL | `af25879d858846aaaa80b8f9626b1cbf4e57a1ab0467e8e2d82990558033c852` |

The default .NET Framework compiler's helpers also passed the same native
matrix before the pinned build. Twelve source-contract checks, five stress-tool
regressions and the native helpers' missing-runtime startup/discovery suite
passed. No Rust or Lisp implementation changed in this slice.

Full `make windows-omnivox-dev` staging passed, including deterministic helper
builds, checksum verification and live inventory checks. Build `0912514c66489200`
in `/tmp/emacsvox-cancellation-runtime` contains the exact helper hashes above.
The source copy was self-contained and included the then-current tracked diff,
including separate RHVoice work, recorded by the normal development provenance.
This is a development payload, not a clean release. The desktop launcher remains
on `e1ecdb481ee08fd0`; no live Emacs session was restarted.

## Limits

These are silent protocol and captured-PCM checks. They do not prove acoustic
quality, long-term resource stability, native parameter reset after cancellation,
or cancellation behavior for other engine helpers. A blocked output pipe still
relies on the parent's existing watchdog as the hard process boundary. No live
Emacs session or desktop runtime is changed by these checks.
