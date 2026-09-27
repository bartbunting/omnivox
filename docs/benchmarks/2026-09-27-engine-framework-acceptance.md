# Engine framework process acceptance and Windows startup failure

Functional development evidence, 2026-09-27. This extends the
[deadline ownership report](2026-09-27-engine-startup-deadline.md); previous
reports and their measurements remain unchanged. It is not release qualification
or an acoustic measurement.

## Verified coverage

The Linux/WSL process fixture passes with the staged development executable and
null audio output. It checks an independently registered helper through exact
voice listing, missing-engine rejection, canonical float WAV output, explicit
preview and ordinary tracked speech. The helper receives literal arguments,
including an empty argument and shell metacharacters, from a path with spaces.

The same run verifies shared preparation, independent worker acknowledgements
and audio targets, retained launch inputs after configuration/manifest/environment
changes, independent retirement, fresh activation and bounded transmission to a
worker that never opens its startup pipe. The fake helper emits a short sine
tone so normal silence trimming leaves meaningful PCM.

A Rust regression initializes the same adapter through external discovery and
the shipped helper initializer using a test-only compiled definition. Descriptor,
engine/voice identity and PCM samples match after the external manifest is removed.
This does not qualify promotion of a real distribution or native speech runtime.

| Check | Result | Evidence |
| --- | --- | --- |
| Linux complete process fixture | Passed | [Log](data/2026-09-27-engine-framework-acceptance/linux-framework-process-8.log) |
| Adapter distribution independence | Passed | [Log](data/2026-09-27-engine-framework-acceptance/distribution-test.log.gz) |
| Locked workspace tests | Passed | [Log](data/2026-09-27-engine-framework-acceptance/workspace-tests.log.gz) |
| Locked workspace all-target Clippy | Passed, warnings denied | [Log](data/2026-09-27-engine-framework-acceptance/workspace-clippy.log) |
| Emacs selection and bytecode preflight | Passed | [Log](data/2026-09-27-engine-framework-acceptance/emacs-preflight.log) |
| Paired release-documentation gate | Passed | [Log](data/2026-09-27-engine-framework-acceptance/emacs-documentation.log) |

Formatting and Python compilation checks also passed. Earlier process-fixture
trials are retained in the data directory: trials 1–3 corrected WAV validation
and fake audio, 4–6 corrected the preview payload, and 7 added marker handling
before the tracked terminal. They are harness development failures.

## Native Windows blocker

Full `make windows-omnivox-dev` rebuilt and staged runtime `308ff70ae39887f0`.
Payload verification passed, but live verification failed; the overall command
returned 2. The [build log](data/2026-09-27-engine-framework-acceptance/windows-development-staging.log.gz)
and [runtime provenance](data/2026-09-27-engine-framework-acceptance/windows-PROVENANCE)
retain the exact source/toolchain identity. Piper was not included.

The new Windows fixture compiled its test-only C# helper with the existing .NET
Framework compiler, then failed at the first exact diagnostic. Direct eSpeak,
Flite and RuTTS diagnostics also returned 1 before inventory with:

```text
Error: launch snapshot: invalid or incomplete launch snapshot
```

The [fixture failure](data/2026-09-27-engine-framework-acceptance/windows-framework-process.log)
and [direct diagnostics](data/2026-09-27-engine-framework-acceptance/windows-direct-diagnostics.json)
establish a native startup failure independent of the external adapter. The
underlying rejected field or invariant is not yet identified. Cross-target
compilation and passing Linux snapshot tests did not establish native round-trip
correctness. Windows process acceptance, maintained adapter live qualification
and compiled-client acceptance against this runtime remain blocked.

The launcher default was restored to the previous available runtime
`2faa4a181c27b40c` after its eSpeak diagnostic returned 141 voices with the staged
data path. The failed runtime remains available by its versioned path; no running
Emacs or speech process was restarted. The
[restoration record](data/2026-09-27-engine-framework-acceptance/windows-default-restoration.json)
does not claim a full rerun of the previous runtime's acceptance suite.

## Provenance and remaining work

The [source/binary provenance](data/2026-09-27-engine-framework-acceptance/provenance.json)
records test commit `ac38083`, binary digests, pinned Rust and commands.
[Checksums](data/2026-09-27-engine-framework-acceptance/SHA256SUMS) cover retained
artifacts. The process tests used synthetic PCM; no sound-device or timing
performance claim follows from them. No macOS native qualification was attempted.

Investigate native Windows launch-snapshot validation, add a native regression,
then repeat full development staging and process/client qualification. The
[framework acceptance matrix](2026-09-28-engine-framework-audit.md#acceptance-checklist)
and final documentation reconciliation remain incomplete.
