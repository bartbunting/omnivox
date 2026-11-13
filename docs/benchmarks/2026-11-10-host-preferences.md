# Complete host preference reload qualification

The wider host reload passed development screening on 2026-11-10. It extends
[resident punctuation reload](2026-11-10-resident-reload.md) through the same
bounded tasks and installation boundary under
[ADR 0017](../adr/0017-independent-host-preference-reload.md), without changing
speech transport, native ownership, output or managed Apply.

## Delivered behaviour

`host_preferences_reload_v1` enables `reload_host_preferences_v1`. A single
worker operation installs punctuation, capital pitch, chunking and saved Reset
defaults coherently. Existing requests retain their old complete state. Current
voice, rate, pitch, volumes and other client choices remain; Reset uses the new
saved baseline. Engine, routing and output changes reject the whole operation.

Punctuation-only reload remains available. It compares other preferences against
the currently installed policy, so it works after a full host reload and rejects
mixed edits truthfully. Scope is part of retry identity; a punctuation retry
cannot be mistaken for a full host operation. Status reports complete live and
startup policy hashes separately. See the
[protocol](../protocols/control.md#independent-local-punctuation-reload).

Emacsvox adds `omnivox-reload-preferences` and negotiates the wider capability
independently per worker. Older workers report unsupported; the client never
silently substitutes a partial operation. The punctuation editor remains scoped
to its saved file. Current settings and saved defaults are explained separately.

## Matched measurements

Four rotated blocks compared v1.18.0, the retained punctuation payload, the wider
payload idle, and the wider payload during reload. Each cell contains 1,024
requests; all 8,192 completed. Same eSpeak engine, two ordinary owners, Python
client, initial settings and null output. Builds were stopped for this matrix;
background workstation load is recorded per case. Values are client submission
to observed `utterance_started`, not acoustic onset or first PCM.

| Workload | v1.18.0 median / p95 ms | Punctuation payload idle | Wider payload idle | Wider payload with reload |
| --- | --- | --- | --- | --- |
| Ordered short speech | 1.571 / 2.289 | 1.586 / 2.224 | 1.576 / 2.319 | 2.168 / 4.551 |
| Isolated characters | 1.482 / 2.106 | 1.516 / 2.227 | 1.520 / 2.172 | 2.106 / 4.521 |

Reload cells mix foreground requests submitted concurrently with both reloads
and secondary requests afterward. The subset submitted inside its own worker's
observed management interval contains 512 requests per workload: ordered
median/p95 3.357/5.137 ms; characters 3.209/5.040 ms. Per-worker reload completion
median/p95 was 8.877/11.239 ms and 8.658/12.040 ms respectively. All proposed
[screening thresholds](../plans/lightweight-preference-reload.md) passed.

Rare approximately 152 ms observations remain in every build, including the
release. They were not trimmed and their cause is unresolved. Raw timestamps,
block medians, maxima and nearest-rank empirical p95 are retained. These sample
statistics do not establish a population tail bound or physical output latency.
No admission hold or engine replacement is introduced by reload.

## Native execution and failure checks

Linux Flite and Windows Flite each completed 100 two-worker policy changes and
200 speech requests. Linux Piper completed 40 changes and 80 requests using the
retained real helper and Lessac medium model. Fresh source and compiled Linux
Emacs each completed 25 full-policy reloads; fresh compiled Emacs to native Windows
completed 25, approximately 29 ms observed including its 25 ms poll interval.
Owner identities stayed unchanged. These native runs establish execution with
representative engines, not a matched release performance comparison; some Linux
functional checks ran while builds were active.

Checks cover malformed configuration, native-scope edits, host-only hash changes,
wrong-scope duplicate retry, punctuation reload after host installation and a
missing notification owner. Rejected changes preserve working speech. Unit tests
also verify coherent old/new request state, transient client choices, Reset,
whole-policy rejection when the selected profile disappears, and the existing
bounded preparation/reply/teardown cases. Remote connections omit both reload
capabilities and remain usable after a denied management request.

Locked workspace tests passed: 1,207 tests, two ignored across 38 suites. Relevant
CLI/TTS Clippy with Piper and formatting passed. Emacs source ERT passed 18 tests
with four conditional skips; unchanged graphical cases had passed in the preceding
slice. Full bytecode compilation was followed by an incremental rebuild after a
Help-text edit made its first final preflight stale; the final preflight passed.
Headers, generated documentation and paired documentation gates passed. Native
Windows full development staging rebuilt helpers and exercised them in an isolated
temporary runtime. Installed speech was not changed.

## Provenance and limits

The [evidence pack](data/2026-11-10-host-preferences/README.md) retains source
patches relative to Omnivox `45feca2` and Emacsvox `8a7b2e852`, raw evidence,
fixtures, logs, payload identities and checksums. Linux executable SHA-256:
`6a269638938cfd487f3dba1677c0fec073194f2c4bf0a0face1ba912fb0e4d8d`.
Windows executable SHA-256:
`badd0fe24b714684fe3ac578b1e7695bf1c730a7b38d663b700db5d10db8847a`, runtime
`cdbbcb7fc4882572`. Both earlier evidence packs and payloads remain unchanged.

Native macOS, acoustic onset and Stop-to-silence remain unqualified here. Piper
execution is not new companion-distribution qualification. Reload does not repair
native ownership, a damaged retained startup, unwritable owner storage or lost
cleanup evidence. Ordinary retained recovery and the separately proposed emergency
boundary are the next slices; this report does not declare those problems solved.
