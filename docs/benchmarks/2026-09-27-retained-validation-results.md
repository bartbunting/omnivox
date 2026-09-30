# Retained native-validation and routing test results

This report preserves test observations previously embedded in the voice-library
and per-fallback-tuning ADRs. It was extracted on 2026-09-27; these tests were
not rerun during documentation consolidation. It is functional evidence, not a
latency or memory baseline.

The source is the pre-consolidation Git revision `cdd6176`. Original record
numbers below refer only to that revision. The original accounts can be read
with `git show cdd6176:docs/adr/0012-voice-library-and-model-lifecycle.md` and
`git show cdd6176:docs/adr/0011-per-fallback-voice-tuning.md`. Those accounts did
not retain a raw local result file for every check. Missing commands, runtime
versions, sample counts and hashes have not been reconstructed or invented.

## Native loading, cancellation and eligibility

| Observed check | Retained outcome and scope | Limits |
| --- | --- | --- |
| Linux Piper malformed-model probe, 2026-09-15 | A file containing `not an ONNX model` with a valid Kristin configuration caused SIGABRT, return code -6, from an uncaught `Ort::Exception` during protobuf parsing. No playback occurred. | Historical failure before exception containment; temporary input was removed. |
| Piper construction/cleanup containment | Linux native checks exercised repeated malformed-config, invalid-model and valid-model loads, refusal of overlapping construction, inference failure and subsequent valid synthesis in one process. | No Windows result or measured memory-release result in this account. |
| Cancellation before synthesis-worker admission | A blocked-worker regression verified that the adapter saw the permanent request cancellation token. | Deterministic contract coverage; no acoustic stop measurement. |
| Managed Piper selection | Owned deterministic ONNX fixtures produced PCM and exercised speakers, lifecycle, model-specific failures, cancellation, disabled speakers, empty selection and helper versions 1–5 on Linux. | Fixtures contained no trained weights; no speech quality or Windows claim. |
| Managed Flite selection | Linux and native Windows x64 GNU checks covered compiled-in and temporary exported SLT data, buffered/streamed synthesis, physical-ID mismatch, changed size, partial failure/recovery and empty managed selection. Helper versions 1–5 were exercised. Temporary files were removed after engine disposal to check handle release. | No MSVC, two-worker activation or measured memory-recovery claim. |
| Shared administrative eligibility | Tests covered exact/default/property selection, fallback, direct and typed synthesis, saved references, native defaults, global exclusions under overrides, late discovery, empty providers and health separation. | Registry-level behavior; not evidence that unconfigured native models were never loaded. |
| Asset/generation verification | Checks covered same-size mutations in Piper/Flite, interrupted bounded reads, generation-byte identity and provider overrides. Piper reread hashes on model load; a repaired model stayed unavailable under an old generation. | Native reopening by path does not prove immutability against concurrent file replacement. |
| Managed helper protocol on Windows | Linux and Windows GNU helper versions 1–5 passed with generation files in the Windows native temporary directory. | Generation files reached through the WSL share intermittently timed out before greeting, even with no assets to hash. |
| Helper retirement | Linux and Windows GNU real-process tests covered blocked stdin and fault-injected blocked replacement/retry; Linux also covered an inherited stdout pipe. Retirement checked direct-child exit and reader completion within five seconds. | These adapter tests alone did not establish descendant cleanup. |
| Main-server managed startup | Owned Linux probes covered Flite/Piper previews, legacy status, exclusions, default reselection, policy generations, input precedence, helper failures, generation changes and model overrides. Windows GNU startup components passed with native temporary files and matching GCC DLLs. | The first raw Windows Cargo launch lacked DLL setup. The complete staged Windows server probe was blocked by missing generated eSpeak data; no substitute data was used. |

## Supervised native validation and retained reports

| Observed check | Retained outcome and provenance | Limits |
| --- | --- | --- |
| Linux disposable validation | Probes covered Piper speakers, compiled-in/external Flite, failed hashes/native models, deadlines, cancellation, supervisor death, descendant reaping, inherited limits and refusal to continue after unconfirmed cleanup. | No physical audio output or completed installation transaction. |
| Windows x64 GNU supervision | Component checks covered private-job termination, descendant pipes, last-handle closure and an over-budget native allocation. | Component evidence, not full Windows/MSVC server acceptance. |
| Intel and Apple Silicon native validation | [Run 35035356056](https://github.com/bartbunting/omnivox/actions/runs/35035356056), commit `9107e6f578094c6dfba90a75d4a62a6a390c2179`: each target passed six supervision tests five times, native staging, relevant Clippy and the complete silent validation/fault probe. A Darwin all-zombie-group `EPERM` finding led to a regression requiring confirmed group absence. | No acoustic, durable recovery or paired-worker activation claim. |
| Linux/Windows saved reports | Linux native report checks and Windows GNU component/filesystem checks passed. Reports bound generation and load identities to observed staged inputs before and after native validation. | Reports were observations, not reusable validation caches or loaded-module attestations. Raw local logs were not linked by the source ADR. |
| Intel and Apple Silicon saved reports | [Run 35042369039](https://github.com/bartbunting/omnivox/actions/runs/35042369039), commit `7ff386693701d8a7a50cb10be615f455062516ca`: actual Piper/Flite loads, report creation/comparison, changed-input rejection and no report after cancellation or supervisor death passed. | No full Windows/MSVC, power-loss recovery or activation-transaction claim. Remote CI artifacts may have independent retention limits. |

## Layered voice-tuning checks

The original tuning record reported nine focused composition tests, 655 passing
workspace tests and passing workspace Clippy for the initial typed-value slice.
It then reported seven additional registration/atomicity/codec/bounds tests,
662 passing workspace tests and passing workspace Clippy. The client storage
checkpoint was `3abe311ca`; the record did not identify a corresponding server
commit or raw log for each test count. Those counts describe historical runs,
not today's suite or proof of completed routed playback at that point.

Later execution, preview, helper-version-6 and paired-client results have their
own dated reports in the [benchmark index](README.md). Those reports and their
raw files remain the source for their measurements; this extraction does not
replace them.

## Regression use

Use these observations to identify behaviors that a change must preserve and
faults that should remain covered. Run the current corresponding tests for a
new candidate. Use retained raw timing/resource packs for quantitative
comparisons; a historical pass count cannot establish latency, memory stability,
intelligibility or audible acceptance.
