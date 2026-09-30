# Idle playback onset investigation, 2026-09-28

The half-speed onset reported by Ľuboš Pinteš is reproducible in the released
1.12.0 and 1.13.0 queue-to-mixer path. The first 256 stereo input frames become
512 output frames; the remaining samples are exact. This is a deterministic
sample-level reproduction, including execution on native Windows x64, without
opening an audio device. It does not reproduce the reporter's WASAPI process
loopback experiment.

## Results

The same diagnostic fixture runs against isolated, unchanged release source
snapshots, with one test module appended to `omnivox-audio/src/output.rs`.
Dependencies come from each release's lock file, using Rust 1.97.1 and Rodio
0.19.0. Both Linux x64 under WSL2 and native Windows x64 test executables use
the debug test profile. No development source changes or installed runtimes
are used or modified.

| Check | Result per release and platform |
| --- | --- |
| Buffered, tracked, and progressive speech after idle | All 15 trials reproduce the same 256-to-512-frame stretch: five repetitions per path. |
| Cold queue with two adjacent sources | The first source stretches; the second is sample-exact, for all three paths. |
| Experimental constant stereo queue metadata | All 15 trials preserve every sample, including the onset. |
| Tick followed by internal silence | All 42 progressive trim cases preserve the complete input; all six buffered trim cases do too. |
| Tick and silence through progressive playback | All six cases retain the silence, with the same onset stretch. |
| Initial 256-frame loss | Not reproduced in any of these mixer captures. |

The input ramp contains 2,048 stereo frames with unique, exactly representable
interleaved sample values. Each idle trial advances the mixer through 44,100
silent output frames before enqueueing the next source. Captures assert the
complete affected prefix and complete tail, including the first input sample.
At 44.1 kHz, 256 input frames occupy 5.805 ms and the stretched output occupies
11.610 ms. These are sample-count conversions, not measured hardware latency.

Silence tests use a one-frame tick at amplitudes 0.011, 0.02, and 0.5, followed
by zeros up to a 20 ms or 60 ms lead-in boundary, then a nonzero body. They
exercise seven input window sizes: 1, 128, 256, 512, 882, 1,024, and 4,096 frames.
The trim threshold is 0.01, leading padding is zero, and trailing padding is
5 ms. The mixer tests also exercise Omnivox's letter-navigation prebuffer path.

## Located cause

Rodio 0.19.0's `src/queue.rs` constructs its idle filler with
`Zero::new_samples(1, 44100, 512)`: 512 mono samples. Queue metadata continues
to describe this exhausted filler until `next()` advances to the new source.

`UniformSourceIterator::bootstrap` in `src/source/uniform.rs` reads the queue's
frame length and channel count before pulling that next sample. It therefore
treats the next 512 interleaved stereo samples as mono and duplicates each to
both output channels. These are 256 original stereo frames expanded into 512
output frames. Its next format refresh sees stereo, and the remaining samples
are correct. Rodio's queue source also contains an ignored transition test
with a comment about sample rate and channels not updating immediately.

As a diagnostic control, the fixture wraps only the queue output to advertise
constant 44.1 kHz stereo metadata. It changes no sample values. That removes
the corruption for all three source types in every trial. This establishes
the cause of the reproduced stretch; the wrapper is not a production patch
or qualification of device switching, cancellation, or other device formats.

## Unconfirmed observations

The extra initial frame loss and the reported collapse of tick-plus-zero
lead-ins to about 253 frames remain unconfirmed. The buffered and progressive
trimmers preserve internal silence after an above-threshold tick in these
fixtures, including across window boundaries. The reproduction does not
include the reporter's custom helper, native PCM conversion, speech-engine
configuration, actual `l {e}` request processing, or live device capture.
Those differences prevent ruling out either observation.

The useful next evidence would be the exact helper/engine and native PCM
format, the lead-in generator, and matched dumped WAV and loopback samples.
The confirmed queue-format defect can be addressed independently; its repair
still needs live Windows playback verification.

## Reproduction and retained evidence

The [fixture](data/2026-09-28-idle-onset/idle_onset_repro.rs) asserts the observed
defect and diagnostic controls. Passing these tests means the defect was
reproduced; these are not passing regression tests for a repaired implementation.
The [runner](data/2026-09-28-idle-onset/reproduce.py) extracts the selected commit
into a new temporary directory and leaves the working checkout untouched.
From the repository, use a new output directory on every run:

```sh
python3 docs/benchmarks/data/2026-09-28-idle-onset/reproduce.py \
  --revision v1.13.0 --windows --output /tmp/omnivox-idle-onset-new-run
```

Omit `--windows` for the host-only run. The Windows option requires WSL interop
and the pinned toolchain's Windows GNU target and linker. Repeat with
`--revision v1.12.0` and a different output directory for the earlier release.

- 1.13.0: [provenance](data/2026-09-28-idle-onset/v1.13.0/provenance.json),
  [Linux results](data/2026-09-28-idle-onset/v1.13.0/host-tests.log),
  [Windows results](data/2026-09-28-idle-onset/v1.13.0/windows-tests.log).
- 1.12.0: [provenance](data/2026-09-28-idle-onset/v1.12.0/provenance.json),
  [Linux results](data/2026-09-28-idle-onset/v1.12.0/host-tests.log),
  [Windows results](data/2026-09-28-idle-onset/v1.12.0/windows-tests.log).
- [Artifact hashes](data/2026-09-28-idle-onset/SHA256SUMS) cover the harness,
  provenance, compiler reports, and test logs. Provenance includes release
  commits, compiler/OS details, lock-file hashes, executable hashes, and exact
  build/test commands.

Additional checks in the isolated 1.13.0 snapshot passed:
`cargo +1.97.1 test --locked -p omnivox-audio` (147 unit tests, including the
four diagnostic tests, and 31 integration tests) and
`cargo +1.97.1 clippy --locked -p omnivox-audio --all-targets -- -D warnings`.
Their [test log](data/2026-09-28-idle-onset/v1.13.0/audio-suite.log) and
[Clippy log](data/2026-09-28-idle-onset/v1.13.0/audio-clippy.log) are retained.
No full-workspace build was needed for this evidence-only change. Formatting,
documentation links, and whitespace checks also passed. Documentation checks
used a temporary Git index containing the new evidence, preserving the user's
staging area.

The measurement point is the Rodio dynamic mixer's output iterator at 44.1 kHz
stereo. Idle is simulated by consuming silence, and exact equality of `f32`
samples is the oracle. No timing clock, acoustic recording, process loopback,
physical output device, release deployment, or listening assessment is involved.
The production implementation remains unchanged by this investigation.
