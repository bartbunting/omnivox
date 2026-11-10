# Slow initial Windows startup: availability before complete discovery

Bounded investigation of the user's successful 23:05 Sydney listening session
on 2026-11-10. Runtime `ea5d6dfb93dd575d` includes the helper queue correction
from Omnivox `ac9ee0a`. No runtime, installed settings, desktop shortcut or live
speech session was changed by this investigation.

## Observed slow launch

The [startup event extraction](data/2026-11-10-startup-investigation/observed-startup.json)
contains only timing and engine identities from the user's foreground log.
Full logs, including personal speech, remain in the private listening profile.
From the worker's first startup message to command readiness took **9,274 ms**.
This excludes earlier Emacs and owner preparation, and is not an acoustic gap.

| Helper | Process-start acknowledgement after worker startup | Connection ready after worker startup |
| --- | ---: | ---: |
| RHVoice | 1,959 ms | 1,987 ms |
| DECtalk | 2,577 ms | 2,856 ms |
| Flite | 4,497 ms | 4,524 ms |
| Eloquence | 5,034 ms | 5,340 ms |
| TGSpeechBox | 6,733 ms | 7,447 ms |
| RuTTS | 9,193 ms | 9,220 ms |

RuTTS's initialization span was mostly waiting before process-start
acknowledgement; its remaining connection negotiation took about 27 ms.
The logs do not distinguish creation queue wait, native process creation,
handle transfer, scheduling or OS scanning inside that earlier interval.
Windows cold caches or security scanning are hypotheses, not established causes.
Do not sum the overlapping helper initialization spans.

Flite, the selected engine, was ready **4,750 ms before command admission**.
All eight engines eventually registered; there were no unavailable-helper,
registry-generation or sound-path errors. Audio setup after engine readiness
took about 53 ms. This is successful but unnecessarily delayed availability.

## Code explanation

`create_engines` in `omnivox-cli/src/engine.rs` starts helper initialization
concurrently, initializes native engines, then joins helper inventory before
choosing the startup engine and returning to the command loop. Other eligible
engines remain available as fallbacks. Even a ready selected engine therefore
waits for unrelated helper initialization.

The guarded Windows worker serializes helper creation and acknowledged handle
transfer; actual native initialization can overlap after release. TGSpeechBox
already has a validated packaged descriptor cache, but its background prewarm
competes for this same channel. In the observed run it started before RuTTS.
This is a plausible contributor, not a measured 1:1 attribution of launch time.

## Controlled experiment

The [probe](data/2026-11-10-startup-investigation/matched.py) uses the unchanged
private runtime through WSL, null output, fresh per-run configuration and owned
cleanup. Six alternating-order blocks compare ordinary and guarded owners for
three explicitly selected test configurations. Disabling engines here is a
diagnostic control, not a proposed default or a change to the listening profile.

Each run records capabilities readiness, client-observed first mixer-source
speech, five subsequent short utterances and successful native retirement.
There are **36 matched runs**, plus a retained 18-run guarded-only pilot.
All speech probes completed, all cleanup checks passed, and the matched stderr
captures contain no WARN or ERROR records. The experiment uses the direct
protocol fixture, not full Emacs initialization or acoustic output.

| Configuration | Guarded first source, median (range) | Ordinary first source, median (range) |
| --- | ---: | ---: |
| All eight engines | 1,023 ms (990–1,209) | 702 ms (619–726) |
| All except TGSpeechBox | 1,048 ms (907–1,113) | 693 ms (620–738) |
| Only eSpeak and selected Flite | 798 ms (779–815) | 474 ms (457–502) |

Subsequent request-to-source medians were 4.7–5.1 ms across these groups.
The guard adds about 321 ms to the all-engine warm startup median, including
its extra preparation and process exchanges. That does not explain the observed
nine-second worker startup. Removing TGSpeechBox saved 22 ms in the guarded
pilot but was 25 ms slower in the matched guarded comparison: no consistent
benefit. Reducing to the selected pair saves about 225 ms warm, at the unacceptable
cost of removing selectable voices if deployed as a general solution.

The [summary](data/2026-11-10-startup-investigation/summary.json),
[raw measurements and runtime identity](data/2026-11-10-startup-investigation/matched/result.json)
and [checksums](data/2026-11-10-startup-investigation/SHA256SUMS) are retained.
Per-run artifacts include exact test configurations, stderr and unmodified speech
event records. Full local control captures retain private guard descriptors and
are not copied into this pack. These are warm-cache screening samples; no host
cache was flushed or machine rebooted. Six samples per group support neither
tail-latency guarantees nor a causal cold-start comparison.

## Decision and next experiment

Keep the working runtime unchanged. The results do not justify removing the
guard, disabling selectable engines, reducing safety checks or claiming that
delaying TGSpeechBox prewarm alone fixes slow startup.

The next candidate should make selected speech usable before optional discovery
finishes. Preserve eSpeak fallback, truthful pending/unavailable inventory,
bounded discovery, live descriptor validation and native cleanup. A missing or
failed preferred engine must still yield working fallback speech. Client voice
registration must tolerate later inventory completion without stale generations
or an automatic change to the user's active voice. Existing complete-inventory
startup and managed preflight contracts need explicit review before changing
publication behavior; this investigation does not silently revise them.

Test a deliberately delayed optional helper while continuously requesting
selected speech, then verify later selection of that helper, failed discovery,
Stop and retirement during discovery, and independent notifications. Proposed
acceptance: the delay no longer gates selected speech, all configured engines
remain discoverable, and matched normal speech gains no material latency.
Add opt-in per-engine queue/create/handoff spans when profiling the next slow
native launch; current logs cannot identify the OS-level cause. The observed
4.75-second availability window motivates this experiment but is not a promised
improvement for an unimplemented candidate.
