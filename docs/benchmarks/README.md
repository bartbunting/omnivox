# Benchmark Evidence

This directory keeps reviewable benchmark evidence produced by
`tools/benchmark_server.py`. Each evidence pack has a dated report and a
matching directory under `data/` containing the unmodified JSON reports and a
`SHA256SUMS` manifest.

Recorded baselines:

- [2026-09-01 Windows x64 development baseline](2026-09-01-windows-x64-c9458361eb57b94a.md)
  covers WinRT, eSpeak NG, RHVoice, Flite, DECtalk, and Eloquence.

## Preservation policy

Treat a committed evidence pack as immutable. Do not replace its raw samples
or revise measurements in place. A rerun gets a new date/build identifier and
a new report, even when it supersedes an earlier result.

Every report must state the platform, build provenance, harness configuration,
sample count, clock, instrumentation point, and important limitations. In
particular, mixer-source consumption must not be described as physical audible
onset. Preserve the raw samples so reviewers can recompute percentiles and
inspect outliers instead of relying only on a summary table.

From a data directory, verify one pack with:

```sh
sha256sum --check SHA256SUMS
```
