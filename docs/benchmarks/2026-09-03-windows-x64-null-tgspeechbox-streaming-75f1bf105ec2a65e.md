# Windows x64 TGSpeechBox Streaming Comparison, 2026-09-03

This records the isolated TGSpeechBox null-output rerun after enabling bounded
progressive synthesis from the native DSP pull through the main server's audio
source. It compares against the immediately preceding state-reuse build.

## Scope and provenance

- Target: `x86_64-pc-windows-gnu`, run through Windows interoperability from
  WSL2 on `Linux-6.18.33.2-microsoft-standard-WSL2-x86_64-with-glibc2.43`.
- Before: build `47d9d79fec39751d`, Omnivox commit
  `0cf3cb590bb1a23b934eb8ff6927b6e395fc3b62`.
- After: build `75f1bf105ec2a65e`, Omnivox commit
  `dda2bf780b6f4e0d4fa58e65604ec0fc7e34434b`.
- Emacsvox deployment commit for both:
  `3c47d0a2afa46ad4c9a32a5215bcddc4e7bdb019`.
- Server version for both: `1.6.4`.
- Engine and exact voice: TGSpeechBox `en-us/adam` at its unchanged native
  44.1 kHz setting.
- Output: explicit null backend; no audio was played.

Both payloads use the supported development path and label themselves
`local-dirty-worktree`. Their recorded Omnivox and Emacsvox tracked-worktree
hashes are the SHA-256 empty-diff hash, so each staged source matched its named
commit.

## Method

The [streaming plan](plans/2026-09-03-windows-x64-null-tgspeechbox-streaming-75f1bf105ec2a65e.json)
matches the frozen
[state-reuse plan](plans/2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d.json):
20 recorded iterations for character, word, line, dense-action, multipart, and
five-dispatch replacement workloads in both cold and warm modes, with two
unrecorded warmups before each warm case. Each build contributes 240 recorded
winner samples.

Every after sample completed through the expected engine and exact physical
voice. All 40 replacement winners observed the other four dispatches
cancelled. Measurements use `time.perf_counter_ns`; source is client receipt
of the first null-source marker, not physical audio onset. Terminal timing
excludes waveform playback duration.

The ordinary character, word, line, multipart, and replacement workloads are
markerless and anchorless, so they use the progressive path. The dense-action
workload carries timed presentation anchors and deliberately remains on the
buffered path until incremental marker remapping is available.

## Results

Warm dispatch-to-source p50, in milliseconds:

| Workload | Buffered | Streaming | Change |
| --- | ---: | ---: | ---: |
| Character | 8.282 | 5.558 | 32.9% faster |
| Word | 47.482 | 9.725 | 79.5% faster |
| Line | 232.447 | 13.698 | 94.1% faster |
| Dense | 389.420 | 416.812 | 7.0% slower |
| Multipart | 267.394 | 7.834 | 97.1% faster |
| Replacement | 508.264 | 39.713 | 92.2% faster |

Warm streaming-path p95/p99 dispatch-to-source results were 6.476/6.949 ms
for character, 11.166/11.256 ms for word, 17.683/17.723 ms for line,
10.902/13.573 ms for multipart, and 46.553/51.172 ms for replacement. The
buffered dense-action p95/p99 values were 492.185/496.444 ms.

Warm dispatch-to-terminal p50 changed from 8.318 to 8.003 ms for character,
47.685 to 45.785 ms for word, 233.523 to 233.492 ms for line, 562.681 to
596.575 ms for dense action, 268.614 to 278.576 ms for multipart, and 562.272
to 567.196 ms for replacement. This metric still waits for synthesis and
complete null-source consumption, so streaming should primarily move source
onset rather than terminal completion.

Cold process startup dominates small requests. Even so, progressive source
delivery reduced cold dispatch-to-source p50 from 509.413 to 286.686 ms for a
line, 543.565 to 316.602 ms for multipart speech, and 747.207 to 275.419 ms for
replacement speech. Character and word cold source medians varied in the
opposite direction by about 30 ms, within the larger per-sample helper startup
cost. The dense-action path remained buffered.

Replacement cancellation p95 improved from 0.751 to 0.523 ms warm and changed
from 0.504 to 0.529 ms cold. Every value remained below one millisecond.

## Interpretation and limitations

The long markerless workloads now expose PCM to playback after the first
bounded native pull instead of after the complete utterance. The 94% to 97%
warm source-latency reductions for line and multipart speech validate that the
progressive path reaches the measured mixer-source boundary. Nearly unchanged
terminal medians confirm that the gain comes from overlapping synthesis with
consumption, not from shortening the generated waveform.

- This was a later single-engine run rather than an interleaved paired trial;
  host drift can affect the comparison, especially the intentionally buffered
  dense workload.
- Twenty samples per group characterize this development machine, not every
  Windows system.
- Null output excludes waveform duration, device buffering, underruns, and
  physical acoustic onset.
- The comparison holds native sample rate and exact voice constant. It does
  not evaluate the separate experimental 22.05 kHz option, which remains
  buffered to preserve its whole-utterance sinc conversion.
- TGSpeechBox remains experimental and exposes no speech markers.

## Raw evidence

- [Streaming suite index](data/2026-09-03-windows-x64-null-tgspeechbox-streaming-75f1bf105ec2a65e/suite.json)
- [Streaming TGSpeechBox JSON](data/2026-09-03-windows-x64-null-tgspeechbox-streaming-75f1bf105ec2a65e/repeat-001-order-001-tgspeechbox-adam.json)
- [Streaming SHA-256 manifest](data/2026-09-03-windows-x64-null-tgspeechbox-streaming-75f1bf105ec2a65e/SHA256SUMS)
- [Buffered TGSpeechBox JSON](data/2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d/repeat-001-order-001-tgspeechbox-adam.json)
