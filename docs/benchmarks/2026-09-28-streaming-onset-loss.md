# Omnivox 1.14.0 streaming onset loss

## Result

The follow-up onset loss is reproducible with published Windows x64 1.14.0
and a controlled progressive helper. Conversion from 22.05 kHz mono loses
256 canonical frames (5.805 ms); conversion from 16 kHz mono loses 352 frames
(7.982 ms). The loss occurs before the playback device and does not require
an idle transition.

The reporter's helper and recordings were unavailable. These results establish
a matching Omnivox defect, rather than proving that every observation on that
installation has the same cause. No production code was changed.

## Windows measurement

- Release: `v1.14.0`, commit `c3f7fac2a0c7b171cab5f1757b9dc4856f02d255`.
  The downloaded `omnivox-1.14.0-windows-x64.zip` matches the
  [published checksum](data/2026-09-28-1.14-publication/sha256sums.txt).
- Windows: `10.0.26200.0`, x64, launched through WSL. The default output mix
  format is stereo 48 kHz, 32-bit. No endpoint or running speech session was
  changed.
- Capture: WASAPI process-tree loopback, stereo float32 at 44.1 kHz, started
  before the isolated Omnivox child. Only that test tree is included. The
  recorder uses Microsoft's
  [process-loopback API](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/).
- Input: a deterministic 120 ms chirp with a positive offset. Every native
  sample exceeds the `0.01` trim threshold. A protocol-5 helper sends signed
  16-bit PCM in 256-native-frame chunks.
- Requests: ten `l {e}` commands per condition, one second apart, leaving
  approximately 0.88 seconds of silence. Capture and Omnivox remain running.
- Reference: the same helper's `--dump-wav`, supplemented by direct converter
  output from an exact release source archive.

| Helper condition | Utterances | Apparent missing capture frames | Exact source discard |
| --- | ---: | ---: | ---: |
| 22.05 kHz mono, progressive | 10 | 250–251 | 256 |
| 16 kHz mono, progressive | 10 | 346–347 | 352 |
| 22.05 kHz mono, progressive, 20 ms lead-in | 10 | 249–250 from lead-in | 256 |
| 22.05 kHz mono, buffered | 10 | 0 | No progressive discard |
| 44.1 kHz stereo, buffered | 10 | 0 | No rate conversion |
| 44.1 kHz stereo, progressive | 40 | 0 | No rate conversion |

The last row includes requested pauses of 0, 2, 10 and 20 ms after the first
helper chunk, ten utterances each. Actual Windows sleep times are logged;
they are not assumed to equal the requested interval. Playback prebuffering
prevents those pauses from directly determining playback start.

Analysis aligns a 3,000-frame interior reference window and retains the first
nonzero captured frame as the onset boundary. Interior correlation exceeds
0.99997; fitted gain is approximately 0.99974. Output/capture resampling adds
about five to six frames of pre-ringing, explaining why apparent capture loss
is slightly smaller than the exact source discard. There is no half-speed
stretch. Captured packet flags are zero.

The 20 ms lead-in is the constant native value `500 / 32768`, above the trim
threshold. Approximately 14.2 ms survives conversion, and the following chirp
is intact. The lead-in absorbs the same loss without correcting its cause.

## Source isolation

[`ProgressivePcmCanonicalizer::new`](../../omnivox-audio/src/progressive_pcm.rs)
initializes `output_delay_remaining` from Rubato's `output_delay()`.
`collect_output` skips that many frames. With pinned Rubato 0.16.2 and this
configuration, those frames already contain real input waveform.

A Linux probe built from `git archive v1.14.0`, using pinned Rust 1.97.1 and
locked dependencies, compares conversion of the same native PCM without
opening a device:

- At 22.05 kHz, progressive output is sample-exactly equal to buffered output
  starting at frame 256 throughout their overlap.
- At 16 kHz, the corresponding exact shift is 352 frames.
- The lead-in case has the same exact 256-frame shift.
- Windows capture aligns with this already-truncated progressive output; an
  additional 256-frame device loss is unnecessary to explain these trials.

As a control, only the isolated archive was changed to initialize the discard
count to zero. Progressive output then retains the entire buffered reference
prefix, sample-exactly. This diagnoses the discard; it is not a qualified
production fix.

The buffered converter also emits fewer frames than the nominal resampled
duration because it does not flush its tail. Thus `--dump-wav` is an onset
reference here, not a complete duration oracle. A fix needs onset, tail,
tiny-input, reuse, cancellation and marker coverage, beyond total frame-count
assertions.

## Other observations and limits

Fresh `--play-wav` processes showed a separate intermittent startup effect:
four of six unpadded trials retained the onset; the others missed 185 and
626 frames. With a 20 ms nonzero lead-in, two of three fresh-process trials
missed 185 frames. Files beginning with 100 ms or one second of silence
retained the onset in one trial each. These 44.1 kHz files bypass progressive
conversion. That startup loss remains unresolved. Files included a second
of trailing silence to separate onset loss from process-exit truncation.

Exploratory runs with sleeps after every helper chunk delivered PCM slower
than playback because of Windows timer granularity. They produced repeated
source waits and fragmented playback, and are excluded from the table. An
interrupted buffered run, an earlier random-waveform test and an unpadded
short-WAV test are also retained but excluded.

This is digital loopback and source evidence, not microphone or acoustic
measurement. It does not establish behavior on every Windows device, identify
the reporter's actual helper rate, or resolve the older tick-followed-by-zeros
observation.

## Artifacts and reproduction

- [Loopback results](data/2026-09-28-streaming-onset-loss/loopback-results.json),
  [source results](data/2026-09-28-streaming-onset-loss/source-results.json),
  [isolated control](data/2026-09-28-streaming-onset-loss/source-control.json),
  [provenance](data/2026-09-28-streaming-onset-loss/provenance.json), and
  [checksums](data/2026-09-28-streaming-onset-loss/SHA256SUMS).
- [Raw archive](data/2026-09-28-streaming-onset-loss/raw-data.tar.gz): captures,
  packet timestamps/flags, inputs, reference WAVs, requests, configurations,
  process logs, source outputs and exact executed exploratory scripts.
  `recorded/analysis-final.json` supersedes the first exploratory analysis,
  which did not handle negative alignment offsets correctly.
- [Source reproducer](data/2026-09-28-streaming-onset-loss/reproduce_source.py),
  [Windows reproducer](data/2026-09-28-streaming-onset-loss/reproduce_windows.py),
  and [capture analysis](data/2026-09-28-streaming-onset-loss/analyze.py).

Both saved reproducers were rerun successfully. The additional Windows run
captured three 22.05 kHz mono utterances with apparent losses of 250, 251 and
250 frames. Its recordings and the fresh source assertions are preserved in
the separate [reproducer verification archive](data/2026-09-28-streaming-onset-loss/reproducer-verification.tar.gz).
Reanalysis of the unpacked primary archive reproduces the saved JSON exactly.

Source reproduction requires NumPy and the pinned Rust toolchain:

```sh
python3 docs/benchmarks/data/2026-09-28-streaming-onset-loss/reproduce_source.py \
  --repo . --output /tmp/omnivox-onset-source-new
```

Windows reproduction requires WSL interop, MinGW x64 C++, Windows .NET Framework
C#, and an extracted release. Use a new output directory on the Windows
filesystem. It plays a startup tune and low-level test signals; it does not
change endpoints or capture unrelated process audio:

```sh
python3 docs/benchmarks/data/2026-09-28-streaming-onset-loss/reproduce_windows.py \
  --omnivox /mnt/c/path/to/omnivox.exe \
  --output /mnt/c/path/to/new-onset-capture
```

Related: [original mixer reproduction](2026-09-28-idle-onset.md) and
[1.14.0 onset/recovery fix evidence](2026-09-28-device-recovery-and-onset-fix.md).
