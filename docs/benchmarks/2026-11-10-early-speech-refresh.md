# Automatic discovery observation and private startup qualification

This continues the [early-speech experiment](2026-11-10-early-speech.md) with
Emacsvox `0c72147cc` and the unchanged private Windows runtime
`4e1c1561f51908a6` (Omnivox implementation `a40a930`). Checks began on
2026-11-10 UTC and continued on 2026-11-11 Sydney time. This qualifies the private
listening prototype, not a release, a new public protocol or acoustic latency.

## Client behavior

The experimental launcher enables bounded inventory observation using the same
`OMNIVOX_EXPERIMENTAL_EARLY_SPEECH=1` opt-in as the server. Each lane observes its
own unavailable/in-progress entries. It issues at most one background inventory
request, waits a second after a reply before another request, and stops when
engines reach final availability/failure or its 180-second budget expires.
A lost reply can retry within the original budget. Manual refresh remains
available after expiry. No worker, native helper or audio queue is held by this
observer.

Unchanged observations neither redraw views nor register voices again. Changed
engine observations refresh existing views and re-register current preferences
on that lane. They never restore a captured startup selection over a newer
choice. Saved exact selectors and native adjustments remain stored while an
engine is unavailable. Existing views expose the availability reason and publish
voices only when the server actually supplies them.

Retirement and process exit end observation. A prepared foreground replacement
starts observing after publication. Notification delivery temporarily rebinds
Emacs's foreground-process variable; the observer follows the retained actual
foreground owner through that binding instead of abandoning its timer or
publishing notification state as foreground state.

The private observer recognizes the paired runtime's existing reason strings.
That is deliberately bounded experimental compatibility, not an advertised
pending-discovery capability for arbitrary servers. Ordinary eager startup,
managed activation, routing-ready acknowledgement, native ownership and
cancellation contracts are unchanged. Public adoption still needs a readiness
and discovery-contract review.

## Checks and failures

The [compiled GUI fixture](data/2026-11-10-early-speech-refresh/listening/check-late.el)
uses real Flite and a synthetic optional RuTTS-labelled helper with a sixteen-second
handshake delay. It tests four cases without manually requesting late inventory:

- Saved exact choice: both lanes speak fallback while pending, then automatically
  use the newly available selected helper.
- Changed choice: selecting Flite before completion survives later discovery.
- Failed optional helper: its startup deadline expires, observation stops with
  failure, and both lanes continue speaking Flite. Saved intent is retained.
- Restart while pending: the prepared replacement resumes observation, speaks
  fallback and later uses the saved choice on both lanes.

Eight source regressions cover these observation boundaries, request deduplication,
fixed retry budget, dead/retiring/replaced owners, independent lanes, notification
bindings and preservation of serialized saved native choices. The focused speech
and startup selection passes 428 tests, with 12 graphical/native checks skipped
in batch. Separate compiled GUI checks exercise Flite, Eloquence, DECtalk and
RHVoice with speech and sound icons on both lanes. Batch Flite checks cover
reload, restart, retained recovery, emergency eSpeak and return to saved settings.
All use isolated settings and null output; human listening remains separate.

The broader suite ran 4,701 tests: 4,446 expected results, 17 previously observed
unrelated failures and 238 skips. Those failures concern the existing advice audit,
Magit tests and documentation expectations. The full run preceded the last two
new regressions; the final eight-test and focused runs cover the completed client
code. Byte-code build/preflight, header checks and documentation review pass.
No Rust source or payload changed in this slice, so its earlier Rust and Windows
payload qualification is reused.

Two real-engine GUI attempts timed out waiting for client playback observations,
although their server logs show completed speech. The failed captures are retained,
not counted as passing runs. Profiling found synchronous native advice compilation
in fresh Emacs caches. A control with early speech disabled also took 12.43 seconds
loading `emacsvox-advice`. An early-speech cache-prime run took 11.53 seconds there;
reusing that private cache reduced the observed load to 103 ms. The profiled
subprocess arguments identify native subroutine trampoline compilation.
A later fixture run also timed out because its notification check directly
rebound the foreground variable instead of using `tts-notify-apply`. The retained
harness now exercises the actual notification wrapper and its recovery binding.
These are individual diagnostic observations, not a benchmark distribution;
instrumentation itself also installs advice.

The new launcher therefore reuses only Emacs-generated native code in
`/home/bart/.cache/emacsvox-lightweight-test/eln-cache`. Each launch still has its
own settings, resources, logs, native owners and application state. No advice is
disabled. First use after a cache/compiler change can still incur compilation.
The warm compiled check observed the first source event in about 58 ms after
its request; this is client observation of source playback, not acoustic onset
or complete desktop startup time.

Background registration remains measurable work: one instrumented changed-inventory
callback took about 128 ms. This slice removes repeated unchanged registration,
but does not establish a tail-latency bound during changed-inventory registration.
Further optimization should measure that client work independently of native
startup and preserve coherent request snapshots.

## Evidence and reproduction

The [run index](data/2026-11-10-early-speech-refresh/runs.json) retains profile paths,
pass/failure artifacts and intermediate diagnostic attempts. The
[checksums](data/2026-11-10-early-speech-refresh/SHA256SUMS) cover synthetic server
logs, client diagnostics, test reports, input files and launcher source. Native
owner credentials and private control records are excluded. The helper executable
and descriptor are the previously retained early-speech fixture; `late-start.py`
records its exact local path. It produces synthetic PCM, not real RuTTS speech.

Run the new [launcher](data/2026-11-10-early-speech-refresh/listening/start.py) with
`--gui-check --engine eloquence`, or `--batch-check --engine flite` for recovery.
The delayed launcher accepts `--gui-check --case saved`, `changed`, `failed` or
`restart`. Diagnostic controls are `OMNIVOX_PROFILE_DISCOVERY=1` and
`OMNIVOX_EARLY_CONTROL=0`; the normal private launch keeps early speech enabled.
All launchers verify the pinned executable hash and compiled client preflight.

## Handoff

The Development shortcut now reaches the qualified launcher through the existing
Windows `evox.bat` and WSL wrapper. A check through that actual batch entry point
passes reload, restart, retained recovery, emergency eSpeak and return to saved
Flite. The [deployment record](data/2026-11-10-early-speech-refresh/deployment.json)
identifies the wrapper backup and launch log. The existing shortcut executable,
installed release and other shortcuts were not changed. No user Emacs was restarted.
The expected
improvement is earlier usable speech during startup/restart when optional helpers
are slow; ordinary navigation is expected to retain its existing responsiveness.
The previous matched server experiment remains the evidence for a warm first-source
median improvement from 1,137 to 864 ms. This slice does not replace that benchmark
or claim a universal startup speedup. Preferred-engine initialization, native
compilation after a cache change, and other Emacs startup work can still dominate.
