# Foreground speech after a delayed settings acknowledgement

Functional investigation and correction, 2026-11-10. This is private prototype
qualification, not a release or an acoustic latency measurement. The client
correction is committed as Emacsvox `5fc22ec71`.

## Incident

The desktop test session's foreground log contains 29 rejected dispatches from
09:39:20 to 09:39:46 UTC, all reporting `timeline registry generation is not
current`. The notification log has no corresponding rejections. Flite completed
an initial foreground utterance, and saved routing subsequently selected
Eloquence before the rejection period. Further Eloquence requests completed
after the engine selection was changed. The logs establish rejected foreground
speech; they do not establish a Flite synthesis or output-device failure.

Raw incident logs include personal speech and remain local under
`target/foreground-registry/incident/`. No personal utterances or saved palette
contents are published in this evidence pack.

## Cause and correction

Interactive Emacsvox startup restores saved voice and routing data after
starting its workers. A worker can install a new logical registry while Emacs
is still busy starting. The five-second configuration deadline then retires the
request callback before Emacs processes the acknowledgement. Speech continues
to reference the old client registry generation, and the worker correctly
rejects it. Selecting an engine applies a fresh registration and repairs the
mismatch.

Emacsvox now keeps an already sent version-2 or version-3 registration's reply
observer for up to 30 additional seconds after the apply timeout, within the
existing 64-request per-process bound. A valid acknowledgement updates only
that live worker's frozen speech snapshot. It cannot reopen the failed settings
operation, roll back a newer snapshot, accept mismatched native content or
change another worker. Late policy replies still cannot start more work.
Reply, expiry or process exit releases the observer. Legacy registration
continues to retire with its operation because its speech has no registry
number to repair. No Rust runtime, transport, native ownership or cancellation
change was needed.

This is bounded delayed-reply recovery. A permanently lost reply, or one beyond
the observer deadline, can still require explicit reapply or speech recovery.
It does not claim automatic convergence after every management failure.

## Reproduction and results

The [before capture](data/2026-11-10-foreground-registration/delayed-before.log)
and [after capture](data/2026-11-10-foreground-registration/delayed-after.log)
record control identities and test outcomes, without utterance contents. The
[probe](data/2026-11-10-foreground-registration/probe.el) temporarily reduces the
apply deadline to 0.2 seconds and delays registration replies by 0.5 seconds.
Startup work can extend the observed delay. It runs in a separate graphical
Emacs under Xvfb, with the qualified Windows runtime and null output.

| Check | Before | Corrected client |
| --- | --- | --- |
| Delayed foreground registry acknowledgement | Received after callback retirement; client stays on generation 1 | Accepted as generation 3 for the same worker |
| Settings operation result | Failed on timeout | Remains failed on timeout |
| Four subsequent tracked foreground utterances | Four failed | Four completed |

The [local reproduction launcher](data/2026-11-10-foreground-registration/probe.py)
uses the previous retained listening fixture and the session's saved startup
routing data, which are not copied into this report. Run it with
`OV_FORCE_TIMEOUT=1`, an absolute `OV_PROBE_LOG`, and `xvfb-run -a python3`.
The ERT regressions instead use self-contained native and layered fixtures and
exercise actual request tables, acknowledgement validation and speech encoding.

The [provenance](data/2026-11-10-foreground-registration/provenance.json) identifies
the unchanged Windows runtime and pre-fix client. This investigation does not
replace or edit the [earlier handover evidence](2026-11-10-replacement-and-handover.md).

## Desktop profile isolation

The earlier fixture set `emacsvox-user-directory` after loading the preamble.
Compiled defaults still resolved palette and routing files under the normal
home directory. A private Emacs directory and Windows configuration root alone
did not isolate those files. That explains why the nominal Flite test could
restore saved Eloquence routing.

The [corrected launcher](data/2026-11-10-foreground-registration/listening/start.py)
sets a fresh private `HOME` and cache directory before Emacs starts. It retains
the same verified Windows payload and the old startup fixture, whose historical
contents remain unchanged. The desktop development shortcut reaches this
launcher through `/home/bart/bin/emacsvox-lightweight-test`; the previous wrapper
was backed up before changing its runner path. Existing Emacs sessions were not
restarted or reconfigured.

The [graphical check](data/2026-11-10-foreground-registration/listening/check.el)
loads the full interactive startup and compiled client, checks both settings
paths are private, and requires completed Flite speech on both guarded lanes.
Startup announcements are suppressed only for this automated check so they
cannot legitimately replace its tracked utterance. It performs cleanup on the
UI thread. An initial timer-based check passed speech but failed cleanup because
the timer ran on a background startup thread; another check was cancelled by
startup announcements. These were corrected in the check harness before the
successful [capture](data/2026-11-10-foreground-registration/gui-check.json).

The Windows batch launcher also completed the retained sequence of Flite
speech, preference reload, restart, recovery, emergency eSpeak and return to
Flite; its [output](data/2026-11-10-foreground-registration/desktop-batch-check.log)
is retained. Source completion with null output does not establish listening
acceptance. Startup sound-file warnings and an optional-helper discovery warning
occurred in an intermediate run; this investigation does not qualify those
unrelated paths.

## Verification

- The final [261 focused source checks](data/2026-11-10-foreground-registration/focused-tests.log.gz)
  pass. They include timeout during the write, bounded observer expiry, malformed
  and older replies, replaced workers, independent lanes and ordinary speech
  using the corrected registry.
- The [full-suite run](data/2026-11-10-foreground-registration/full-tests.log.gz)
  before the final narrowing to layered registrations ran 4,694 tests: 4,439
  expected, the same 17 baseline failures, and 238 skips. Those failures are the
  existing Magit dependency, aural audit and two manual assertions. The final
  focused run additionally preserves legacy timeout behavior.
- Incremental byte-code build and preflight, reference generation, documentation
  review, paired documentation links and whitespace checks were run for these
  changes. No Rust build is needed for this Lisp and launcher correction.
