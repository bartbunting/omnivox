# Windows x64 TGSpeechBox State-reuse Comparison, 2026-09-03

This records the isolated TGSpeechBox null-output rerun after removing
redundant per-utterance language, profile, and eSpeak voice setup. It precedes
the helper-protocol streaming work, so both compared builds still return a
fully buffered synthesis result.

## Scope and provenance

- Target: `x86_64-pc-windows-gnu`, run through Windows interoperability from
  WSL2 on `Linux-6.18.33.2-microsoft-standard-WSL2-x86_64-with-glibc2.43`.
- Before: build `f7204ac69b6010f1`, Omnivox commit
  `2973d5efc86d40520f146bd635209e40b4154c9e`.
- After: build `47d9d79fec39751d`, Omnivox commit
  `0cf3cb590bb1a23b934eb8ff6927b6e395fc3b62`.
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

The [after plan](plans/2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d.json)
matches the TGSpeechBox entry in the frozen
[pre-optimization plan](plans/2026-09-03-windows-x64-null-f7204ac69b6010f1.json):
20 recorded iterations for character, word, line, dense-action, multipart,
and five-dispatch replacement workloads in both cold and warm modes, with two
unrecorded warmups before each warm case. Each build contributes 240 recorded
winner samples.

Every after sample completed through the expected engine and exact physical
voice. All 40 replacement winners observed the other four dispatches
cancelled. Measurements use `time.perf_counter_ns`; source is client receipt
of the first null-source marker, not physical audio onset. Terminal timing
excludes waveform playback duration.

The implementation comparison also generated one fixed raw WAV and canonical
pipeline WAV from each helper. The before and after SHA-256 values matched
exactly for both files, confirming that state reuse did not change that speech
output. The temporary WAVs are not part of this evidence pack.

## Results

Warm dispatch-to-source p50, in milliseconds:

| Workload | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Character | 198.902 | 8.282 | 95.8% |
| Word | 223.272 | 47.482 | 78.7% |
| Line | 477.919 | 232.447 | 51.4% |
| Dense | 601.004 | 389.420 | 35.2% |
| Multipart | 535.652 | 267.394 | 50.1% |
| Replacement | 795.458 | 508.264 | 36.1% |

Warm dispatch-to-terminal p50, in milliseconds:

| Workload | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Character | 198.992 | 8.318 | 95.8% |
| Word | 223.495 | 47.685 | 78.7% |
| Line | 478.779 | 233.523 | 51.2% |
| Dense | 1060.766 | 562.681 | 47.0% |
| Multipart | 536.551 | 268.614 | 49.9% |
| Replacement | 1038.269 | 562.272 | 45.8% |

Cold medians, in milliseconds:

| Workload | Before start to source | After start to source | Before dispatch to source | After dispatch to source |
| --- | ---: | ---: | ---: | ---: |
| Character | 843.384 | 884.526 | 262.541 | 271.413 |
| Word | 923.530 | 928.822 | 331.355 | 303.172 |
| Line | 1113.501 | 1085.421 | 518.022 | 509.413 |
| Dense | 1191.138 | 1209.686 | 637.548 | 709.368 |
| Multipart | 1099.449 | 1172.679 | 545.564 | 543.565 |
| Replacement | 1252.377 | 1299.121 | 741.293 | 747.207 |

Cold behavior has no consistent directional change, as expected: every cold
sample creates a new helper and cannot reuse an earlier utterance's
configuration. The after cold multipart group contains one retained
7,515.064 ms dispatch-to-source p99 outlier; all samples still completed.

Replacement cancellation p95 changed from 0.476 to 0.504 ms cold and from
0.569 to 0.751 ms warm. All values remain below one millisecond, and this run
does not establish a cancellation regression.

## Interpretation and limitations

The large warm reductions confirm that repeated YAML pack loading and voice
selection were a major TGSpeechBox setup cost, especially for navigation-sized
text. Longer input remains expensive because this protocol stage still waits
for fully materialized PCM before emitting its first audio chunk. Streaming is
therefore the next distinct optimization target.

- This was a later single-engine run rather than an interleaved paired trial;
  host drift can affect the comparison.
- Twenty samples per group characterize this development machine, not every
  Windows system.
- Null output excludes waveform duration, device buffering, underruns, and
  physical acoustic onset.
- The comparison holds native sample rate and exact voice constant. It does
  not evaluate the separate experimental 22.05 kHz option.
- TGSpeechBox remains experimental and exposes no speech markers.

## Raw evidence

- [After suite index](data/2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d/suite.json)
- [After TGSpeechBox JSON](data/2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d/repeat-001-order-001-tgspeechbox-adam.json)
- [After SHA-256 manifest](data/2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d/SHA256SUMS)
- [Before TGSpeechBox JSON](data/2026-09-03-windows-x64-null-f7204ac69b6010f1/repeat-001-order-008-tgspeechbox-adam.json)
