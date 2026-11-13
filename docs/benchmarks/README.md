# Benchmark Evidence

This directory keeps dated performance measurements and functional acceptance
reports. Performance packs retain raw samples, provenance, reproduction inputs
and checksums where recorded; functional-only reports are labelled separately.
The maintained harnesses are documented in the
[tools reference](../../tools/README.md#server-lifecycle-benchmarks).
Cross-engine suite inputs live under `plans/`; unmodified measurements and logs
live under `data/`. A missing raw artifact or measurement is a coverage gap,
not an implied successful result.

## Performance comparisons and baselines

Choose a baseline matching the path being changed. The following are retained
observations, not universal performance thresholds or a claim of current
acceptance on every platform.

| Concern | Retained comparison / reproduction source | Measurement boundary |
| --- | --- | --- |
| Early selected speech | [Optional discovery prototype](2026-11-10-early-speech.md), [automatic client observation](2026-11-10-early-speech-refresh.md) | Controlled delayed-helper availability, fallback, active cancellation, native retirement, compiled client late selection and matched warm startup. |
| Speech during discovery acknowledgement | [Native early-start correction](2026-11-13-native-early-start.md) | Reproduced generation mismatch, source and native delayed-reply regressions, four-engine lifecycle checks, Windows personal profile and WSL checks; no acoustic timing claim. |
| Native desktop launch delay | [Personal launcher timing](2026-11-13-native-startup-latency.md) | Matched first-source timing after removing two launch-time Emacs validation processes; Eloquence and DECtalk checks, unchanged package loading and explicit build audit. |
| Personal Development profile | [Normal configuration handoff](2026-11-11-personal-development-launch.md) | Qualified early speech with the normal Emacsvox profile; launcher and guarded source checks, plus user-reported daily use without issues on 2026-11-13. |
| Native Windows client reload | [Native Emacs ownership and personal shortcut](2026-11-13-native-windows-client-reload.md) | Compiled native WinRT/Flite/Eloquence/DECtalk reload and guarded recovery, graphical personal-profile checks, and separate failed early-discovery observations; functional checks only. |
| Slow initial startup | [Selected speech versus complete discovery](2026-11-10-startup-investigation.md) | Real slow-launch events and 36 matched warm ordinary/guarded startup controls; no acoustic or cold-start causality claim. |
| Windows startup contention | [Independent helper queue deadline](2026-11-10-helper-queue.md) | Observed Eloquence/Flite exclusion, bounded queue regression, native ownership checks and isolated GUI source completion. |
| Foreground settings timeout | [Delayed registration acknowledgement](2026-11-10-foreground-registration.md) | Functional startup failure reproduction, bounded client repair and isolated desktop profile; source completion only. |
| Replacement and foreground handover | [Prompt Flite cancellation and prepared foreground](2026-11-10-replacement-and-handover.md) | Phase-controlled replacement, eight-engine reload response, continuous source gaps, slow notification startup and fresh compiled recovery profiles. |
| Windows multiple engines and startup | [Eight-engine ownership and client preparation](2026-11-10-windows-multi-engine.md) | Six retained helpers, independent recovery, matched speech during reload, compiled client startup and continuous source handover probes. |
| Windows creation integration | [Opt-in eSpeak/Flite prototype](2026-11-10-windows-creation-integration.md) | Native failures, helper fallback/recovery, compiled client, pipe correction and matched reload/interaction timing. |
| Windows native creation | [Rust guard creation components](2026-11-10-windows-owned-creation.md) | Actual process handles, interrupted handoffs and pending creation; product entry remains gated pending integration. |
| Lightweight preference reload | [Independent punctuation investigation](2026-11-10-lightweight-reload.md) | Release/maintenance/candidate source-start comparisons, active reload overlap, native helpers, source Emacs, failure checks and platform limits. Private prototype; no acoustic or public-release claim. |
| Resident preference delivery | [Owned-worker punctuation reload](2026-11-10-resident-reload.md) | Negotiated stdio control, resident preparation, independent Emacsvox results and matched release/native qualification. |
| Complete host preference reload | [Capital pitch, chunking and saved defaults](2026-11-10-host-preferences.md) | Same bounded path, coherent requests, preserved client choices and 8,192 matched requests. |
| Ordinary retained recovery | [Client owner-failure fix and retained restart](2026-11-10-retained-recovery.md) | Real Flite/Piper, broken saved configuration, exact worker crash and healthy peer; source timings, not acoustic or emergency qualification. |
| Minimal emergency speech | [Storage-independent owned eSpeak](2026-11-10-emergency-speech.md) | Corrupt preferences/host state, failed receipt writes, transient native proof, repeated minimal recovery and healthy peer; owner-loss and acoustic limits remain explicit. |
| Owner failure boundary | [Blocked forwarding and lost authority](2026-11-10-owner-authority.md) | Bounded Linux faults with actual retained pidfds and exact durable status; establishes the need for independent lifecycle authority, not successful product recovery. |
| Prepared emergency startup | [Launch cost and deferred native release](2026-11-10-deferred-start.md) | Matched compiled clients on Linux and WSL-to-Windows; lower recovery time and source-gap bound, cancellation/retry and older-runtime checks. |
| Independent guard extraction | [Real Linux worker/helper failure matrix](2026-11-10-independent-guard.md) | Rust native cleanup independent of speech and status backpressure; prototype only, with production integration and Windows/macOS qualification outstanding. |
| Guarded owner/client integration | [Linux local IPC and native fault checks](2026-11-10-guard-integration.md) | Opt-in actual Rust owner/guard and compiled Emacs client; matched speech during reload, separate startup cost, Windows/macOS still pending. |
| Guard readiness and recovery | [Event waiting and interrupted startup](2026-11-10-guard-readiness.md) | Repeated matched reload timing, real status backpressure, partial bootstrap/START and compiled-client reconnection; Linux only. |
| Windows guard qualification | [Local pipes, WSL timing and descendant completion](2026-11-10-windows-guard-qualification.md) | Native component successes, matched real-engine reload/interaction timing, and a retained-helper counterexample that keeps Windows guarded startup unavailable. |
| Native creation and preparation | [Retained-handle Flite handoff and concurrent bridge startup](2026-11-10-guard-handoff-and-preparation.md) | Bounded Windows ownership experiment; implemented client startup overlap, matched timings and speech during preparation. Windows product qualification remains blocked. |
| Lost retirement replies | [Existing native receipt verification](2026-11-10-retirement-receipts.md) | Linux Flite/Piper and WSL-to-Windows restoration, damaged-proof rejection and notifications completed inside foreground recovery. |
| Warm/cold synthesis and resampling | [Progressive resampler reuse](2026-09-20-progressive-resampler-reuse.md) | Windows GNU, null output; matched compiler control, raw timing and workload plans. |
| Legacy letter navigation | [Letter playback reserve](2026-09-20-letter-playback-reserve.md), [initial investigation](2026-09-20-dectalk-letter-navigation.md) | Muted Windows device runs; first/final mixer-source consumption, stalls and rapid replacement. |
| Native control overhead | [Parameter timing](2026-09-20-dectalk-parameter-timing.md), [batched parameters](2026-09-20-dectalk-batched-parameters.md), [reset placement experiment](2026-09-20-dectalk-reset-latency.md) | DECtalk runtime-specific measurements; distinguish private prototypes from shipped behavior. |
| End-to-end server responsiveness | [Responsiveness investigation](2026-09-20-responsiveness.md) | Preserve the reported compiler confound; use the later matched resampler comparison for that question. |
| Cancellation and runtime resources | [Windows helper cancellation](2026-09-17-windows-helper-cancellation.md), [RuTTS acceptance](2026-09-01-windows-x64-rutts-23baa0a64c9cf117.md) | Helper protocol/retirement and process counters; not acoustic stop-to-silence. |
| Speech-rate consistency | [Rate calibration](../reference/rate-calibration.md), [retained rate audits](../rate-audits/README.md) | Corpus/WAV duration and WPM, distinct from synthesis throughput. |
| WSL output experiments | [WSLg evidence](../experiments/2026-09-06-wslg-audio.md), [native PulseAudio trial](../experiments/2026-09-07-native-pulseaudio.md) | Local software observations and listening limits; no general acoustic parity claim. |

Earlier baselines remain available:

- [2026-09-20 Windows x64 progressive resampler reuse](2026-09-20-progressive-resampler-reuse.md)
  compares repeated speech before and after bounded filter reuse, and records
  the compiler difference that confounded the earlier release comparison.
- [2026-09-03 Windows x64 null-output pre-optimization baseline](2026-09-03-windows-x64-null-f7204ac69b6010f1.md)
  covers all eight configured physical engines with exact representative
  voices, randomized order, and no audible playback.
- [2026-09-03 Windows x64 TGSpeechBox state-reuse comparison](2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d.md)
  isolates the first non-streaming optimization at unchanged voice and sample
  rate, before the helper-protocol streaming work.
- [2026-09-03 Windows x64 TGSpeechBox streaming comparison](2026-09-03-windows-x64-null-tgspeechbox-streaming-75f1bf105ec2a65e.md)
  measures first-source latency after bounded progressive synthesis, while
  retaining dense anchored speech as an explicit buffered control.
- [2026-09-03 Windows x64 Eloquence and DECtalk streaming comparison](2026-09-03-windows-x64-null-eloquence-dectalk-streaming-09b89b3ff537d12b.md)
  measures both marker-capable Windows helpers after protocol-v5 callback
  streaming, with their dense anchored workloads retained as buffered controls.
- [2026-09-03 Windows x64 anchored-streaming follow-up](2026-09-03-windows-x64-null-anchored-streaming-1c6e5690e30b758b.md)
  verifies incremental Eloquence and DECtalk dense timelines after native PCM
  moved to the continuous sinc converter.
- [2026-09-01 Windows x64 development baseline](2026-09-01-windows-x64-c9458361eb57b94a.md)
  covers WinRT, eSpeak NG, RHVoice, Flite, DECtalk, and Eloquence.
- [2026-09-01 Windows x64 RuTTS development acceptance](2026-09-01-windows-x64-rutts-23baa0a64c9cf117.md)
  covers both built-in voices, exact routing, cancellation, resource sampling,
  mixed queues, hard stops, fallback, and repeated helper recovery.

## Functional acceptance and additional reports

- [2026-11-13 lightweight reload source integration](2026-11-13-lightweight-merge.md)
  records paired main/master merge checks, regenerated manuals, native Windows
  Eloquence/DECtalk coverage and reproduced pre-existing source-test failures.

### Omnivox 1.18.0 release evidence

The [candidate report](2026-10-16-1.18-candidate.md)
and [publication report](2026-10-16-1.18-publication.md)
are retained with their raw evidence from release commit `0a838874`.
They record Windows x64 development-runtime listening acceptance, all 74 passing
release jobs, public asset verification, the completed Homebrew update and
Emacsvox installer follow-up. Acoustic timing and broader listening coverage
remain separate.

### Earlier reports

- [2026-10-08 Omnivox 1.16.1 candidate](2026-10-08-1.16.1-candidate.md)
  records the eSpeak marker-order regression, patch-release scope and local
  candidate checks.

- [2026-10-01 Homebrew Omnivox 1.16.0 update](2026-10-01-homebrew-1.16.md)
  records the merged formula, both native Mac jobs and verification of the
  published archives against the merged formula.

- [2026-09-30 Omnivox 1.16.0 publication](2026-09-30-1.16-publication.md)
  records the main-branch release, all 64 passing jobs, two ARM64 helper-startup
  retries and verification of the 28 public assets.

- [2026-09-30 Omnivox 1.16.0 candidate](2026-09-30-1.16-candidate.md)
  records the published-history merge, palette character regression checks,
  Linux and full native Windows development acceptance.

- [2026-09-28 Omnivox 1.15.0 publication](2026-09-28-1.15-publication.md)
  records all 64 passing release jobs, the Windows ARM64 timeout and successful
  retry, and the exact public asset set and checksums.
- [2026-09-28 Omnivox 1.15.0 candidate](2026-09-28-1.15-candidate.md)
  combines the streaming onset fix and punctuation configuration/editor checks.
- [2026-09-28 streaming onset fix](2026-09-28-streaming-onset-fix.md)
  records failing/passing source regressions and a matched Windows comparison:
  all 50 fixed helper captures retain the onset.
- [2026-09-28 streaming onset loss](2026-09-28-streaming-onset-loss.md)
  reproduces the 1.14.0 follow-up with Windows process loopback and isolates
  a 256/352-frame progressive conversion discard.
- [2026-09-28 local punctuation editor](2026-09-28-punctuation-editor.md)
  records local-service saves, Emacs draft editing and graphical navigation.
- [2026-09-28 punctuation configuration](2026-09-28-punctuation-configuration.md)
  records strict parsing, Unicode/source-offset regressions, historical snapshots
  and captured pronunciations across Linux workers, reset and recovery.
- [2026-09-28 Omnivox 1.14.0 publication](2026-09-28-1.14-publication.md)
  records all 64 passing release jobs, the immutable tag, and the exact public
  asset set and checksums.
- [2026-09-28 Omnivox 1.14.0 candidate](2026-09-28-1.14-candidate.md)
  records release checks and the subsequent successful physical Windows
  headphone-switching test after correcting the saved launcher selection.
- [2026-09-28 Windows output recovery and onset fix](2026-09-28-device-recovery-and-onset-fix.md)
  records queue regression, recovery/cancellation, native Windows stream
  replacement and workspace checks before the physical follow-up above.
- [2026-09-28 idle playback onset](2026-09-28-idle-onset.md)
  reproduces the release queue's 256-frame onset stretch on Linux and native
  Windows using mixer samples; initial frame loss and lead-in collapse remain
  unconfirmed, and no live loopback recording was made.
- [2026-09-28 capital pitch and audio settings](2026-09-28-capital-pitch-and-audio-settings.md)
  records Linux and native Windows configuration, fallback, reset and recovery
  checks, WAV channel assertions and native PulseAudio startup requests.
- [2026-09-28 saved speech defaults](2026-09-28-saved-speech-defaults.md)
  records Linux and native Windows checks for saved preferences, overrides,
  reset, immutable recovery and historical startup compatibility.
- [2026-09-28 host chunk-size configuration](2026-09-28-host-chunk-configuration.md)
  records frozen startup/reset behavior, strict compatibility checks, exploratory
  real-engine timing and a separately reproduced pre-existing marker failure.

These reports qualify behavior and native integrations. A passing functional
check is not a performance baseline unless the report also records measured
samples and a reproducible comparison.

- [Extensible engine configuration and the original startup deadline gap](2026-09-27-engine-framework.md)
- [Engine startup deadline fix and retained ownership checks](2026-09-27-engine-startup-deadline.md)
- [Engine framework process acceptance and native Windows startup blocker](2026-09-27-engine-framework-acceptance.md)
- [Engine framework version-1 acceptance audit](2026-09-28-engine-framework-audit.md)
- [Windows engine snapshot capture and historical retention fixes](2026-09-28-windows-engine-snapshot.md)

- [Windows native voice default audit](2026-09-09-windows-native-defaults.md)
- [Windows native parameter bindings and limits, 2026-09-17](2026-09-17-native-parameter-bindings.md)
- [Windows native voice parameter audit, 2026-09-17](2026-09-17-native-voice-parameters.md)
- [DECtalk helper-6 handlers, 2026-09-18](2026-09-18-dectalk-helper6.md)
- [DECtalk native parameter execution, 2026-09-18](2026-09-18-dectalk-native-execution.md)
- [Eloquence helper-6 handlers, 2026-09-18](2026-09-18-eloquence-helper6.md)
- [Eloquence native parameter execution, 2026-09-18](2026-09-18-eloquence-native-execution.md)
- [Native parameter helper wire boundary, 2026-09-18](2026-09-18-helper-native-wire-boundary.md)
- [Reserved helper-6 parameter codecs, 2026-09-18](2026-09-18-helper6-parameter-codecs.md)
- [Helper-6 parent integration, 2026-09-18](2026-09-18-helper6-parent.md)
- [Native choice admission and preparation, 2026-09-18](2026-09-18-native-choice-admission.md)
- [Strict native private previews](2026-09-18-native-previews.md)
- [Routed native voice execution, 2026-09-18](2026-09-18-native-routed-execution.md)
- [Native synthesis execution, 2026-09-18](2026-09-18-native-synthesis-execution.md)
- [Native timelines and consumed-audio receipts](2026-09-18-native-timelines.md)
- [Connection-owned parameter catalogue cache](2026-09-18-parameter-catalogue-cache.md)
- [Public native parameter explanations](2026-09-18-public-native-explanations.md)
- [Public native voice registration](2026-09-18-public-native-registration.md)
- [Public engine parameter catalogues, 2026-09-18](2026-09-18-public-parameter-catalogues.md)
- [Native parameter client activation](2026-09-19-native-client-activation.md)
- [Retained native-validation and routing test results](2026-09-27-retained-validation-results.md)
- [Retained layered-routing acceptance results](2026-09-27-retained-routing-results.md)

- [Retained voice-management verification](2026-09-27-retained-voice-management-results.md)
- [Retained platform and companion observations](2026-09-27-retained-platform-results.md)
- [Retained Piper release qualification](2026-09-27-retained-piper-release-results.md)

- [Omnivox 1.13.0 candidate checks](2026-09-28-1.13-candidate.md)
- [Windows startup watchdog test correction](2026-09-28-startup-watchdog-test.md)
- [Omnivox 1.13.0 publication checks](2026-09-28-1.13-publication.md)

## Preservation policy

Treat a committed evidence pack as immutable. Do not replace its raw samples
or revise measurements in place. A rerun gets a new date/build identifier and
a new report, even when it supersedes an earlier result.

Every report must state the platform, build provenance, harness configuration,
sample count, clock, instrumentation point, and important limitations. In
particular, mixer-source consumption must not be described as physical audible
onset. Reports collected with null output must say so explicitly; their
terminal timings do not include waveform duration and are not comparable with
device-output terminal timings. Preserve the raw samples so reviewers can
recompute percentiles and inspect outliers instead of relying only on a summary
table.

From a data directory, verify one pack with:

```sh
sha256sum --check SHA256SUMS
```

## Comparing a future candidate

1. Select the affected workloads, baseline and acceptance criteria before the
   run. Match OS/architecture, compiler/optimization settings, engine/runtime,
   voice/model, speech rate, effects, audio backend, worker count and harness.
   Record unavoidable differences and do not attribute them to the code change.
2. Retain exact source/build identifiers, executable/helper/runtime/data hashes,
   hardware and environment, commands, warmups, repetitions, ordering, clock and
   measurement point. Preserve failed, cancelled and excluded samples with their
   reasons. Do not silently remove outliers or count replacement as throughput.
3. Run baseline and candidate under matched conditions, preferably interleaved
   blocks. Compare cold and warm behavior separately. Report absolute and
   relative changes, distributions and sample counts; insufficient tail samples
   cannot establish p95/p99 stability.
4. Pair timing with cancellation, correctness, PCM/marker and resource checks.
   Use `benchmark_server.py` / `benchmark_suite.py` for lifecycle timing,
   `stress_server.py` for mixed work and recovery, `stress_helper.py` for native
   cancellation and resource growth, and `audit_speech_rates.py` for duration.
   The tools reference gives invocation and scope for each.
5. Save a new dated report, complete raw results and reproduction inputs. State
   the comparison threshold and its rationale; an experiment's existing threshold
   does not become a project-wide release rule automatically. Keep the baseline.

Physical onset/stop-to-silence, audible underruns, intelligibility, long-session
resource growth and platforms without matched retained samples still require
appropriate new acceptance measurements. The documentation consolidation did
not run those measurements or establish new regression thresholds.
