# Resident owned-worker punctuation reload

The punctuation delivery passed its development screening on 2026-11-10 in
`experimental/lightweight-reload`, paired with Emacsvox
`feat/omnivox-lightweight-reload`. Continue independent reload under
[ADR 0017](../adr/0017-independent-host-preference-reload.md). This implements the
completed [private investigation](2026-11-10-lightweight-reload.md) without its
candidate subprocess or socket. No installed runtime or live Emacs was changed.

## Implemented boundary

Local owned workers negotiate `host_preferences_v1` over existing stdio. One
bounded preparation task captures selected files using retained native context
and pinned assets; a bounded reply task keeps these management writes off
admission. Policy and profile catalogue hashes are prepared together. Installation
swaps complete immutable policy without engine reconstruction or playback drain.
Status distinguishes consumed startup from actual live policy. Requests carry no
paths and remote/unowned workers cannot invoke local reload. Existing unrelated
synchronous control handlers have not all been moved off admission.

Emacsvox provides `omnivox-reload-punctuation`, `omnivox-preferences-status` and
`R` in the punctuation editor. Each worker has an independent result. Editor
reload compares the worker's selected file to the saved editor target. Old workers
retain explicit restart and matching-catalogue behaviour. Supporting workers can
use their own named profile even when notification policy differs.

## Matched speech measurements

Eight blocks rotated release, maintenance, resident idle and resident reload.
Each cell contains 1,024 requests; all 12,288 completed. No builds were launched
during the matrix. This was a shared workstation; recorded load averages describe
background activity, not a dedicated idle machine. The same Python client, two
ordinary owners, eSpeak, short text and null output were used. Measurements run
from client submission to receipt of `utterance_started`, a source event. They
are neither protocol acknowledgements nor first PCM or acoustic onset.

| Workload | v1.18.0 median / p95 ms | Maintenance median / p95 ms | Resident idle median / p95 ms | Resident reload median / p95 ms |
| --- | --- | --- | --- | --- |
| Ordered short speech | 1.581 / 2.287 | 1.590 / 2.229 | 1.594 / 2.261 | 1.919 / 4.061 |
| Isolated characters | 1.517 / 2.108 | 1.516 / 2.209 | 1.475 / 2.244 | 2.099 / 4.697 |
| Sequential replacement | 21.954 / 22.650 | 21.937 / 22.693 | 21.904 / 22.556 | 21.446 / 22.352 |

The reload cells include one foreground request submitted concurrently with two
reloads and one secondary request after they finish. The stricter subset whose
submission falls inside its own worker's client-observed management interval has
512 requests per workload: ordered median/p95 3.052/4.459 ms, character
3.120/5.597 ms, replacement 4.007/6.168 ms. This establishes overlap with observed
reload, not exact internal preparation execution. Control traffic ends an existing
replacement coalescing read early, explaining the lower replacement subset; do not
interpret it as a synthesis speedup. Preference wake alone preserves that read.

Rare 150–173 ms observations occur in release, maintenance and resident runs.
For characters, counts above 100 ms were respectively 1, 5, 6 and 3; maximum
resident character response was 152.751 ms idle and 152.453 ms during reload.
These are retained, not trimmed. Their cause is not established. The summary
contains per-block medians, maxima and all raw samples; empirical p95 is a
nearest-rank statistic, not a population tail guarantee.

Per-worker reload completion, including status observation, had median/p95
7.503/9.597 ms during ordered speech and 7.671/13.943 ms during characters.
There is no deliberate speech blackout: engines and output remain running.
Sampled speech continued during preparation; this does not measure a universal
zero-unavailability bound. The proposed screening thresholds in the
[plan](../plans/lightweight-preference-reload.md) passed.

## Native engines and clients

| Check | Observed result |
| --- | --- |
| Linux Flite, 80 requests during 40 two-worker reloads | Source median/p95 4.425/7.379 ms; per-worker reload median/p95 7.321/13.920 ms, including one fault-test reload |
| Linux Piper, same workload and counts | Source 36.213/48.650 ms; reload 6.398/9.508 ms; real retained helper and Lessac medium model |
| Native Windows Flite through direct WSL stdio, 120 requests | Source 4.432/7.005 ms; reload 23.535/27.431 ms, including one fault-test reload |
| Fresh source and compiled Emacs | 25 public two-worker reloads each, unchanged owners and character submissions |
| Fresh compiled Emacs to native Windows | 25 reloads, approximately 29 ms observed completion including the client's 25 ms polling interval; editor file-identity check enabled |
| Maximum resolved policy | Twelve Flite reloads with 32 profiles of 512 entries each; read-only configuration directory; unchanged incarnation and correct cached catalogues |
| Rapid navigation and notifications | 200 characters, repeated Stop, 25 reloads, 15 simultaneous notifications; one terminal result per request, cancellation observed and fresh speech afterward |
| Resource soak | 500 Flite reloads: worker PID, five file descriptors, twelve threads and eight management files unchanged; RSS 16,296 to 16,992 KiB |

The native engine runs establish functioning reload with representative costlier
engines; they are not matched release comparisons. Their earlier Linux executable
identity differs from the final matrix build and is recorded per result. The final
Linux payload is `bcb94e5eb895ef7eb74468daa393817febe9fa866db501898d84f3d8bb780190`.
Windows executable SHA-256 is
`fee94f41f4f6e5684830d44dd25d460bd21535c7b932195ef72041f2d9ff656b`, runtime
`b7c1018ba58a7b73`. Full development staging rebuilt and exercised the native
helpers in a temporary runtime. Main-only staging rejected shared protocol changes;
that guard was retained.

## Failure and recovery evidence

| Failure or condition | Speech and recovery result |
| --- | --- |
| Malformed files or mixed punctuation/native-scope edit | Reload rejected; existing speech and policy retained |
| Lost reply, duplicate operation, stale revision or A-to-B-to-A edit | Status exposes actual state; no stale replay or rollback |
| Secondary legacy batch keeps reload pending | Foreground installs and speaks; secondary installs after Stop |
| Secondary absent | Foreground reload and speech remain usable |
| Selected profile removed | Reload rejected, working selected policy retained |
| Preparation or management reply blocked | Bounded unit tests show admission/teardown remain responsive; these tasks own no native engine |
| Configuration directory read-only | Reload succeeds without a preference history write |
| Managed active pointer damaged | Existing worker still reloads and speaks; fresh capture rejects the pointer |
| Retained startup after confirmed ordinary retirement | Speech restored despite damaged active pointer; 65.006 ms first usable source response in one observation; old punctuation restored and then reloadable |
| Preferred Flite helper missing | Genuine eSpeak fallback speaks and reloads; 129.600 ms first usable source response in one observation |
| Windows canonical configuration path | Initial reload rejected while speech continued; fixed with existing checked canonical-to-native conversion, then native and compiled-client reruns passed |

The resource soak observes stable counts; it does not prove a universal memory
bound. Unit tests separately exercise bounded queues and retained operation state.
No ownership record was removed or missing cleanup evidence fabricated.

## Verification and retained evidence

Locked workspace tests passed: 1,205 passed, two ignored across 38 suites. Relevant
CLI/TTS Clippy including Piper, formatting, remote capability-denial integration,
documentation and paired links passed. Emacs source ERT passed 16 tests with four
conditional skips; the two graphical cases subsequently passed under private Xvfb.
The remaining two native opt-in ERT cases were not run by that suite. Fresh native
source/compiled fixtures above are separate checks. Bytecode rebuild/incremental
checks, headers and generated-document release checks passed.

The [evidence pack](data/2026-11-10-resident-reload/README.md) retains scripts,
source patches, payload manifests, summary, raw timestamps, configuration inputs,
failed runs and gate logs. The earlier investigation pack is unchanged. Failed
fixtures include missing startup framing, premature capability inspection, an
invalid character frame, and Unix fixture paths passed to native Windows. Fixes
preserved server validation. The real canonical Windows reload defect is
separately identified above.

## Limits and next slice

Null output and source events cannot establish acoustic onset or Stop-to-silence.
Piper uses the retained prior companion/model fixture; it does not qualify a newly
distributed companion. Native macOS and listening checks remain unavailable.
No cross-platform release readiness is claimed.

Extend the same bounded installation path to capitalization, chunking and saved
Reset defaults, preserving current client choices and request coherence. Engine,
model, output and managed activation stay outside ordinary reload. Separately
qualify recovery: this slice does not establish emergency speech independent of
damaged history, simultaneous owner/guard failure recovery, or reboot recovery.
Those require actual ownership evidence and safe asset exclusion.
