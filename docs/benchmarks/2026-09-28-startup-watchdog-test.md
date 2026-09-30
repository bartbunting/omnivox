# Windows startup watchdog test correction

Functional investigation on 2026-09-28. The initial
[1.13.0 tag run](https://github.com/bartbunting/omnivox/actions/runs/36366520107/attempts/1)
failed one Windows ARM64 unit test; the other build/test/package-source jobs
passed. No release was published by that attempt.

## Cause and correction

The test expected `Timeout("external startup budget")`, but its mock connection
could return `Timeout("mock response")` instead. A native Windows x64 GNU test
binary reproduced this on the fourth repetition, returning after 100.0182 ms.
Temporary instrumentation reproduced it again and showed the condition-variable
wait reporting timeout with 140.5 microseconds still left before the mock's
`Instant` deadline.

The production response reader already checks the clock after a timed-out wait
and continues waiting while time remains. The mock returned immediately without
that check. Its early error could therefore arrive before the outer startup
watchdog selected the expected timeout result. This was a test-model mismatch,
not evidence of an unbounded production startup.

Commit `b6ce206` makes the mock recheck its deadline. The watchdog test retains
its exact error assertion, elapsed-time bound, termination check, no-publication
check and confirmed-retirement check. It now joins the attempt before asserting
the result, and includes the failing case and elapsed time in diagnostics.
Both modified sections are test-only; production code is unchanged.

## Verification

Using Rust 1.97.1 and locked dependency resolution:

| Check | Result |
| --- | --- |
| Native Windows x64 GNU, corrected watchdog test | 60 repetitions passed: 240 hello/describe read/write stall cases. |
| Native Windows x64 GNU, complete TTS unit binary without the eSpeak feature | 437 passed. |
| Linux locked workspace tests | 1,099 passing test-result entries, including a nested subprocess test; two ignored, no failures. |
| Workspace/all-target Clippy with the CLI Piper feature | Passed, warnings denied. |
| Formatting and whitespace checks | Passed. |

Windows tests ran through WSL interop on Windows 10.0.26200, with the final test
executable on the Windows filesystem. An earlier repeat series launched from
the WSL share passed 37 times, then hit the harness's 15-second process timeout.
That timeout's captured output was not retained, so its exact phase is unknown.
The complete Windows-local rerun above passed. This report does not qualify
operation from WSL filesystem shares or establish audio-device behavior.

## Release treatment and evidence

The existing `v1.13.0` tag at `54aee84` is unchanged. The test correction is
committed after that tag and is not claimed as part of its source archive.
After identifying the test-only cause and completing the checks above, the
failed tag job was submitted for a retry; publication still requires the normal
release gates. This report does not claim that retry has passed.

The [raw-log manifest](data/2026-09-28-startup-watchdog-test/SHA256SUMS) covers
the original CI failure, local reproductions, successful checks and
[provenance](data/2026-09-28-startup-watchdog-test/provenance.json). No physical
listening or proprietary-runtime qualification was performed for this change.
