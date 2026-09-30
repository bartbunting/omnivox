# Retained layered-routing acceptance results

Extracted on 2026-09-27 from `docs/per-fallback-streaming-handoff.md` at local
Git revision `03d3285`. These are the original 2026-09-09 observations, not new
runs during documentation cleanup. Read the original with
`git show 03d3285:docs/per-fallback-streaming-handoff.md`.

The source reports passing pinned-toolchain locked workspace tests, workspace
Clippy, formatting and documentation checks at each checkpoint below. It does
not retain an individual server revision, exact invocation or raw log for every
count. Counts describe historical suite runs, not current coverage or performance.

| Tested boundary | Focused checks recorded | Historical passing workspace count |
| --- | --- | --- |
| Actual-attempt preparation | Five tests: all four buffered/streaming primary/fallback combinations, frozen registration, fresh defaults across chunks, duplicate-choice versus policy identity, invalid stream identity and no replay after commitment/output failure. | 667 |
| Route-owned effects | Four tests: continuity, duplicate physical targets, legacy boundaries, tail placement, speech-bus resources and rejected rendering. | 671 |
| Early ticket ownership | Three tests: held-consumer partial failure with/without markers, initial-cue failure, and successful send followed by attachment failure with accepted but unconsumed PCM. | 674 |
| Acceptance versus consumption | Seven tests: callback ordering, repeated identities, bounded truncation, cancellation and partial failure behind a held consumer. | 681 |
| Preview codecs | Four fixture/adversarial tests: round trips, missing/unknown/duplicate fields, semantic validation and bounded terminal encoding. | 685 |
| Private execution | Eight worker/reporter tests plus one codec test: buffered/streaming execution, strict duplicate choice, no substitution, frozen exclusions, defaults above host rate one, queue accounting and pre-synthesis response reservation. | 694 |
| Timeline 4 | Six tests: mixed fixture, malformed fields, trustworthy rejection identity, registry ownership, UTF-8 actions and transport bounds. | 700 |
| Marker 3 | Three tests: required identity, duplicates, old-version isolation, escaping, 32 KiB decoded receipts and 512 KiB encoded marker bounds. | 703 |
| First-frame receipt publication | Four tests: both output modes, encoding preflight, paired capacity release and rejection before engine calls. Held-consumer and empty-output tests checked receipt suppression and consumed failed prefixes. | 707 |
| Ordinary mixed renderer | Five tests: buffered/streaming fallback, context/default precedence, placement, neutral legacy runs, frozen registry, unresolved voices and rejection of invalid later spans/windows before synthesis. | 712 |
| Reader/queue/multipart integration | Four tests: version changes, incomplete/replayed assemblies, invalid new work preserving active speech, separate replacement domains and complete consumed-choice/terminal reporting in both output modes. | 716 |

## Native and remote observations

The [Windows native-default audit](2026-09-09-windows-native-defaults.md)
reported set/default/set restoration for all 17 advertised Windows voices,
with 85 captured syntheses and native parameter queries. No native reset change
was needed for those tested runtimes. The linked report and
[raw output](data/2026-09-09-windows-native-defaults.txt) preserve its evidence;
this is not a claim about every runtime version.

`tools/test_voice_choice_remote.py` exercised real isolated foreground and
notification workers with null audio: distinct registrations, unavailable-primary
fallback, strict duplicate-row previews, multipart delivery, reconnect clearing,
legacy replacement of mixed registries and old-protocol speech passed. The
inherited remote suite reported five executed passes and three optional
live/lease/workstation checks skipped. The development payload was built through
`make dev` using verified input caches, without installation. The source recorded
capability advertisement after this server acceptance, with client integration
and audible acceptance still outstanding at that historical point.

The [later client acceptance](2026-09-19-native-client-activation.md) and other
[native-control reports](README.md#functional-acceptance-and-additional-reports)
record subsequent evidence separately. These functional observations cannot
establish latency, physical stop-to-silence, intelligibility or memory growth.
