# Windows x64 RuTTS Development Acceptance, 2026-09-01

This is post-v1.5.1 development evidence, not a measurement of the published
v1.5.1 release. It covers Emacsvox's source-built Windows GNU payload and does
not stand in for the native MSVC release gates.

## Scope and provenance

- Target: `x86_64-pc-windows-gnu`, executed through Windows interoperability
  from WSL2.
- Runtime build ID: `23baa0a64c9cf117`.
- Omnivox commit: `6711c2675c18a13f6488444641c6233c8e5ad1bc`.
- Emacsvox deployment commit: `084749cac66ab81e26344ce59ae478630df27e99`.
- Both recorded tracked-worktree hashes are the empty-diff SHA-256. The
  development target still labels the payload `local-dirty-worktree` by
  policy.
- Toolchain: Rust 1.97.1 and pinned MinGW GCC 12-win32.
- Companion: checksum-locked RuTTS 6.3.3 with built-in `male` and `female`
  voices; RuLex was not included.
- Server executable SHA-256:
  `497ff87abf0fdd1024ce0f73402e783cfcbcddfc5c519b2a2cb4aa7bff52c589`.
- RuTTS companion-tree SHA-256:
  `96822d7d7a894f7eddd108645e44ce60861ee490b897d8dd8a58a3a0f44fce73`.

The staged repository and Windows-local copies passed their complete checksum,
provenance, external-input, voice-discovery, and live WAV-synthesis gates
before these runs.

## Persistent helper acceptance

`tools/stress_helper.py` kept one native helper alive for 25 syntheses per
voice. Each run varied the advertised settings, required rate, average pitch,
pitch range/intonation, and volume, sent a health ping every five syntheses,
cancelled five long in-flight requests, validated PCM framing, and sampled
native process resources after every synthesis.

| Voice | Syntheses | Cancellation probes | PCM | Working set first/last/max | Private bytes first/last/max | Handles | Threads |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Male | 25 | 5 | 18.1 MiB | 13.55/14.18/14.97 MiB | 1.50/2.92/3.76 MiB | 95/95 | 4/4 |
| Female | 25 | 5 | 18.0 MiB | 13.55/14.04/14.70 MiB | 1.50/2.18/3.27 MiB | 95/95 | 4/4 |

RuTTS advertises no synchronization markers, so the observed zero marker count
is expected. Handle and thread counts were unchanged from the first to last
helper sample. The short run is evidence against immediate leaks, not a release
threshold or a substitute for a multi-hour soak.

## Exact-route server latency

`tools/benchmark_server.py` ran two unrecorded warmups followed by five recorded
samples for each of character, word, line, dense-action, multipart, and rapid
replacement workloads. All 60 winners carried the exact requested physical
voice. Each of the ten replacement samples cancelled four stale dispatches,
for 40 cancellations without fallback contamination.

Warm dispatch-to-first-mixer-source p50, in milliseconds:

| Voice | Character | Word | Line | Dense | Multipart | Replacement | Cancel p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Male | 19.836 | 19.664 | 39.665 | 69.569 | 39.837 | 89.965 | 0.579 |
| Female | 19.790 | 29.539 | 39.781 | 59.655 | 39.393 | 89.557 | 0.502 |

These are protocol and mixer-source timings from one development machine, not
physical acoustic-onset measurements or broad performance claims.

## Queue, stop, and helper-fault acceptance

The normal server stress reports ran ten iterations per voice. Every iteration
interleaved two replacement domains with ordered and urgent work: six
dispatches produced two expected cancellations and four completions, with no
cancelled dispatch reaching a marker. Three hard stops per voice each cancelled
two outstanding dispatches, emitted exactly one terminal record per dispatch,
and recovered to the requested physical voice.

The separate dispatch-fault report completed three cycles. In each cycle the
harness resolved and killed only the current `omnivox-rutts-helper.exe` child
while a long RuTTS request was outstanding, observed its single terminal
outcome, required eSpeak to realize the fallback probe, requested explicit
engine recovery, and then required exact `rutts/male` output. All three cycles
passed.

## Limitations

- This covers the supported Windows GNU development target, not either native
  MSVC release target.
- Native Linux ARM64, macOS Intel/Apple Silicon, and Windows x64/ARM64 release
  acceptance remains owned by the six-runner workflow.
- Five latency samples per workload are acceptance evidence, not a stable
  performance distribution.
- Resource values are Windows CIM snapshots. No automatic memory-growth
  threshold has been approved.
- RuTTS remains markerless and without RuLex.

## Raw evidence

- [Male helper soak](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/male-helper-soak.json)
- [Female helper soak](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/female-helper-soak.json)
- [Male latency](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/male-latency.json)
- [Female latency](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/female-latency.json)
- [Male server stress](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/male-server-stress.json)
- [Female server stress](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/female-server-stress.json)
- [Dispatch-time helper fault and recovery](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/dispatch-fault-recovery.json)
- [SHA-256 manifest](data/2026-09-01-windows-x64-rutts-23baa0a64c9cf117/SHA256SUMS)
