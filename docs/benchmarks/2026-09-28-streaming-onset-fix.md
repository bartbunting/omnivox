# Streaming onset fix: source and Windows verification

## Outcome

The progressive converter now retains its first real output samples instead
of discarding Rubato's reported filter latency as though it were leading
padding. Its bounded input windows, continuous sinc filter, tail flushing,
frame-count limit and native marker mapping remain in use.

This corrects the [reproduced onset loss](2026-09-28-streaming-onset-loss.md).
All 50 fixed-build Windows helper captures retained the onset. The matched
baseline reproduced the defect in all 30 affected-condition captures.
This is an unreleased source fix, not deployment or publication evidence.

## Matched Windows comparison

Both Windows x64 GNU executables were built in the same isolated `v1.14.0`
source archive, with Rust 1.97.1, the same compiler and locked dependencies.
Only [the retained patch](data/2026-09-28-streaming-onset-fix/fix.patch.gz) differs.
The main workspace's separate configuration work is excluded from these builds.
Executable hashes and build commands are in
[provenance](data/2026-09-28-streaming-onset-fix/provenance.json).

The native setup and signal fixtures match the preceding investigation:
Windows x64 through WSL interop, real default-device output, a process-tree
loopback recorder at 44.1 kHz stereo, and a controlled protocol-5 PCM helper.
Each condition sends ten `l {e}` requests, one second apart, after startup.
The 120 ms waveform leaves approximately 0.88 seconds of idle between requests;
the lead-in condition adds 20 ms. Recording begins before the test process.
No user speech runtime, launcher setting or output endpoint was replaced.

| Condition | Baseline missing frames | Fixed missing frames | Trials per build |
| --- | ---: | ---: | ---: |
| 22.05 kHz mono, progressive | 250–251 | 0 | 10 |
| 16 kHz mono, progressive | 346–347 | 0 | 10 |
| 22.05 kHz mono, progressive, 20 ms lead-in | 249–250 | 0 | 10 |
| 22.05 kHz mono, buffered control | See preceding investigation | 0 | 10 fixed |
| 44.1 kHz stereo, progressive control | See preceding investigation | 0 | 10 fixed |

These are apparent capture losses relative to the same helper's buffered
reference WAV. The exact pre-fix source discards are 256 and 352 canonical
frames; Windows output/capture resampling produces several frames of
pre-ringing. The same saved analysis is used for both builds, with a
3,000-frame interior correlation window and a `1e-5` first-signal threshold.
Interior correlations exceed 0.99997. All recorded packet flags are zero.
The lead-in survives the fix instead of absorbing approximately 5.8 ms of loss.

The first cross-build staging attempt could not generate eSpeak data on the
Linux host. For these isolated test packages, both mains were therefore placed
beside the exact published 1.14.0 runtime data and notices, with GNU runtime
DLLs from the existing development payload; their hashes are retained too.
The fixture configurations disable built-in engines. These are
development test packages, not newly qualified release distributions.

## Regression and implementation checks

Two new regressions fail with the original converter and pass with the fix:

- Impulses at the first, interior and final native frames remain near their
  mapped marker positions, allowing only sinc phase and integer rounding.
  This covers eight sample rates from 8 kHz through 384 kHz, mono/stereo,
  fragmented delivery and a partial final window. Stereo channel relationships
  are checked too.
- Tiny one-, 17- and 127-frame native utterances retain actual audio at four
  sample rates. Previously a nominally correct output count could contain
  only silence after the mistaken discard.

Existing converter tests cover callback-boundary independence, exact output
counts, filter quality, invalid calls, bounded reuse and reset after completed
or cancelled utterances. Existing helper tests cover native marker conversion
and cancellation/recovery.

Completed checks:

- Formatting baseline and final `make fmt-check` passed; only the changed Rust
  file was formatted with pinned Rust 1.97.1.
- Targeted converter tests: 9 passed, after demonstrating both new failures.
- `cargo +1.97.1 test --locked --workspace`: 1,141 passed, 2 ignored.
- Native Windows audio unit tests: 143 passed, 1 ignored.
- Locked Clippy passed for default members/all targets, Windows audio/all
  targets, and the optional Piper helper with its `piper` feature.
- Piper adapter unit tests with its native feature: 7 passed. The initial
  launch lacked `libpiper.so`; rerunning with the exact built library directory
  on `LD_LIBRARY_PATH` passed. Both logs are retained.
- `make dev` built the current workspace and staged its generated eSpeak data
  and notices. Workspace checks include the pre-existing configuration work.

## Evidence and remaining limits

- [Comparison summary](data/2026-09-28-streaming-onset-fix/comparison.json),
  [baseline measurements](data/2026-09-28-streaming-onset-fix/baseline-results.json)
  and [fixed measurements](data/2026-09-28-streaming-onset-fix/fixed-results.json).
- [Raw captures](data/2026-09-28-streaming-onset-fix/captures.tar.gz): both
  builds' float PCM, reference WAVs, packet timestamps/flags, helper inputs,
  requests, configuration and process logs. Executables are identified by hash.
- [Failing regressions](data/2026-09-28-streaming-onset-fix/before.log),
  [passing converter tests](data/2026-09-28-streaming-onset-fix/after.log.gz),
  [workspace tests](data/2026-09-28-streaming-onset-fix/workspace.log.gz),
  [native Windows tests](data/2026-09-28-streaming-onset-fix/windows-audio-tests.log.gz),
  and [checksums](data/2026-09-28-streaming-onset-fix/SHA256SUMS).
- The [retained capture runner](data/2026-09-28-streaming-onset-loss/reproduce_windows.py)
  reproduces this workload when supplied each test executable and a new output
  directory. The exact executed build/capture scripts are retained with this
  pack; their temporary paths document this run.

Digital process loopback is not microphone or physical acoustic measurement.
The reporter's own helper and hardware still need their confirmation. The
separate intermittent fresh-process `--play-wav` startup effect, buffered
conversion tail truncation and the older tick-followed-by-zeros report are not
fixed or newly qualified by this change. No helper protocol, dependency,
playback reserve or device-switching policy changed.
