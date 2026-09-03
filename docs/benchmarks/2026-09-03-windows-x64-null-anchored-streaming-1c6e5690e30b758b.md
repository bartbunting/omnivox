# Windows x64 Anchored Streaming Follow-up, 2026-09-03

This records the Eloquence and DECtalk null-output rerun after Omnivox moved
presentation anchors and timeline rendering onto the progressive path and
replaced per-helper linear upsampling with its continuous sinc converter.

## Scope and provenance

- Target: `x86_64-pc-windows-gnu`, run through Windows interoperability from
  WSL2 on `Linux-6.18.33.2-microsoft-standard-WSL2-x86_64-with-glibc2.43`.
- Build: `1c6e5690e30b758b`, Omnivox commit
  `6b6cd64811ab79732ade7566553be82df1a3caea`.
- Emacsvox deployment commit:
  `3c47d0a2afa46ad4c9a32a5215bcddc4e7bdb019`.
- Server version: `1.6.4`.
- Exact engines and voices: Eloquence `v1` and DECtalk `paul`.
- Output: explicit null backend; no audio was played.

The payload used the supported development staging path. Its recorded Omnivox
and Emacsvox tracked-worktree hashes are the SHA-256 empty-diff hash, so the
staged source matches the named commits.

## Method

Each exact voice ran 20 recorded warm iterations of the character, word, line,
dense-action, multipart, and five-dispatch replacement workloads, with two
unrecorded warmups before each workload. All 240 winner samples completed
through the required engine and physical voice. All replacement winners saw
the other four dispatches cancelled.

The dense workload is the anchored control used by the earlier
[streaming comparison](2026-09-03-windows-x64-null-eloquence-dectalk-streaming-09b89b3ff537d12b.md).
It now streams: exact Eloquence anchors and DECtalk word-boundary anchors drive
incremental semantic actions on the same bounded playback clock. DECtalk
aliases requested anchors to its existing native word indexes, so it does not
add another `index` command to the synthesized text.

Measurements use `time.perf_counter_ns`. Source is client receipt of the first
`utterance_started` event from the null source, not physical audio onset.
Terminal timing excludes waveform playback duration.

## Results

Warm dispatch-to-source latency, in milliseconds:

| Workload | Eloquence p50 | Eloquence p95 | DECtalk p50 | DECtalk p95 |
| --- | ---: | ---: | ---: | ---: |
| Character | 26.213 | 32.736 | 45.605 | 49.975 |
| Word | 28.179 | 37.243 | 45.676 | 47.543 |
| Line | 24.493 | 27.875 | 46.499 | 48.061 |
| Dense | 27.746 | 37.038 | 41.513 | 47.913 |
| Multipart | 28.425 | 33.406 | 46.464 | 47.941 |
| Replacement | 60.431 | 71.605 | 70.992 | 78.739 |

Relative to the preceding linear-streaming build, dense anchored source p50
fell from 50.386 to 27.746 ms for Eloquence (44.9%) and from 71.717 to 41.513
ms for DECtalk (42.1%). Dense action handling no longer adds a whole-result
wait: its median is within ordinary-workload variation for both engines.

The quality-preserving sinc conversion does more work before the first
canonical window than the temporary linear converter. Consequently ordinary
line source p50 is higher than that intermediate build's 4.081 ms Eloquence
and 31.273 ms DECtalk results. Compared with the original fully buffered sinc
baseline, line source p50 remains 22.2% lower for Eloquence and 43.0% lower for
DECtalk. The short Eloquence character and word cases no longer beat that
original buffered baseline; reducing continuous-sinc startup cost is a future
optimization that must not reintroduce the reported quality regression.

Ten additional stress iterations per engine passed interleaved replacement
domains and three hard stops, including contiguous marker and semantic-event
history and exactly one terminal result per dispatch. Separate 25-iteration
DECtalk and 10-iteration Eloquence helper runs passed progressive requested
anchors and in-flight cancellation against the real runtimes.

## Limitations

- This warm-only follow-up isolates steady-state rendering; it does not
  characterize cold process startup.
- The two engines ran sequentially rather than as an interleaved paired suite,
  so host drift can affect comparisons.
- Null output cannot assess audible quality, real-time underruns, device
  latency, or physical acoustic onset. An audible Eloquence comparison remains
  required for the originally reported timbre/static symptom.
- Eloquence remains an external licensed installation. DECtalk used the pinned
  runtime recorded by the staged provenance.

## Raw evidence

- [Eloquence JSON](data/2026-09-03-windows-x64-null-anchored-streaming-1c6e5690e30b758b/eloquence-v1.json)
- [DECtalk JSON](data/2026-09-03-windows-x64-null-anchored-streaming-1c6e5690e30b758b/dectalk-paul.json)
- [SHA-256 manifest](data/2026-09-03-windows-x64-null-anchored-streaming-1c6e5690e30b758b/SHA256SUMS)
