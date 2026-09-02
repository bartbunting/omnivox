# Benchmark Evidence

This directory keeps reviewable benchmark evidence produced by
`tools/benchmark_server.py`. Each evidence pack has a dated report and a
matching directory under `data/` containing the unmodified JSON reports and a
`SHA256SUMS` manifest. Cross-engine packs also retain their exact input under
`plans/`.

Recorded baselines:

- [2026-09-03 Windows x64 null-output pre-optimization baseline](2026-09-03-windows-x64-null-f7204ac69b6010f1.md)
  covers all eight configured physical engines with exact representative
  voices, randomized order, and no audible playback.
- [2026-09-03 Windows x64 TGSpeechBox state-reuse comparison](2026-09-03-windows-x64-null-tgspeechbox-47d9d79fec39751d.md)
  isolates the first non-streaming optimization at unchanged voice and sample
  rate, before the helper-protocol streaming work.
- [2026-09-03 Windows x64 TGSpeechBox streaming comparison](2026-09-03-windows-x64-null-tgspeechbox-streaming-75f1bf105ec2a65e.md)
  measures first-source latency after bounded progressive synthesis, while
  retaining dense anchored speech as an explicit buffered control.
- [2026-09-01 Windows x64 development baseline](2026-09-01-windows-x64-c9458361eb57b94a.md)
  covers WinRT, eSpeak NG, RHVoice, Flite, DECtalk, and Eloquence.
- [2026-09-01 Windows x64 RuTTS development acceptance](2026-09-01-windows-x64-rutts-23baa0a64c9cf117.md)
  covers both built-in voices, exact routing, cancellation, resource sampling,
  mixed queues, hard stops, fallback, and repeated helper recovery.

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
