# Windows engine snapshot capture and historical retention

Functional development evidence, 2026-09-28. This resolves the native startup
failure recorded in the [previous acceptance report](2026-09-27-engine-framework-acceptance.md).
That report and its original artifacts remain unchanged. These are native process
and PCM checks, not acoustic measurements or release qualification.

## Cause and fixes

Windows `std::env::vars_os()` exposes hidden drive-directory entries such as
`=C:`. The initial implementation incorrectly assumed it excluded them. Strict
snapshot validation rejected the captured entry, so every engine startup failed
before inventory. The existing 33 Windows configuration tests passed because
their environments were constructed explicitly. A new test capturing the actual
native environment reproduced the startup failure.

Commit `3faea73` filters inherited Windows drive-directory bookkeeping at capture.
Both engine and local-owner capture use the shared path. Ordinary native values,
empty values and unpaired UTF-16 surrogates survive. Complete native-pair records
still reject equals signs, NULs, empty names and duplicate launch keys.

Historical owner records used UTF-8 maps and also captured the hidden entries.
Their rejection could block inspection of retained package references. Commit
`8129f58` applies the same omission only to the historical map representation;
new native-pair records retain strict validation. A native regression checks that
both historical and current records pin package files until a matching retirement
receipt, after which reviewed removal can complete. Records without an engine
snapshot still require a fresh activation to start a current worker.
The retention test uses synthetic package/session metadata and an explicit test
receipt; actual process-tree retirement is covered separately by the owner fixture.

The Windows process fixture also needed native path comparisons: a canonical root
can include the verbatim prefix, and relative source paths use backslashes.
Commit `077e42d` corrects those assertions without changing public status data.

## Verification

The retained native tests run actual Windows GNU executables through WSL interop,
using Rust 1.97.1. The configuration-only test build disables the eSpeak feature;
full payload and process checks use the supported build wrappers and staged data.

| Check | Observed result |
| --- | --- |
| Captured native environment regression, before fix | Failed with the reproduced launch-snapshot error. |
| Historical map regression, before compatibility fix | Failed with invalid snapshot environment name. |
| Final native Windows configuration suite | 37 passed. |
| Native package-retention regression | Passed for current and historical representations. |
| Final locked workspace tests and all-target Clippy | Passed; Windows GNU CLI/Piper cross-target Clippy also passed. |
| Linux development payload and complete framework process fixture | Passed for the capture fix. |
| Fresh compiled Emacs configuration tests against Linux | 9 passed, including paired startup, frozen inputs and independent recovery. |
| First full Windows development staging, runtime `60183e3de0d6afe1` | Passed payload and live verification after the capture fix. |
| Windows external-helper process fixture against that runtime | Passed diagnostics, canonical WAV, preview, tracked speech, independent acknowledgements, frozen inputs, fresh activation, blocked startup transmission and retirement. |
| Final full Windows development staging, runtime `454e40235fb929ce` | Passed payload/live verification with both fixes; retained as the launcher default. |
| Complete Windows framework fixture against final runtime | Passed. |
| Fresh compiled Emacs configuration tests against final Windows runtime | 9 passed after adapting the fixture's native paths and explicit staged eSpeak data. |
| Final Windows remote broker and compiled Emacs acceptance | All 8 passed, including authentication, independent lanes, frozen snapshots, total disconnect/recovery, Unicode, markers, stop, heartbeat expiry and exact eSpeak speech. |

The first Windows fixture run stopped at its path-spelling assertion after
successful voice listing and WAV synthesis; the corrected fixture completed.
The retained removal regression's initial failure was in the added historical
case; its final run verifies a specific unretired-session blocker rather than
accepting a generic metadata error.

The first local Emacs attempt used the Unix-only fixture with Windows storage;
the second supplied native configuration paths but omitted the external staged
eSpeak data. Both failures remain recorded. The revised test places host files
on Windows, keeps launcher log pipes on WSL, and accepts the exact staged data
path. Its final Windows and Linux runs each pass all nine tests. Production Lisp
was unchanged, and both runs checked that readiness/acknowledgement functions
loaded from current `.elc` files.

## Retained evidence

[Provenance](data/2026-09-28-windows-engine-snapshot/provenance.json) records the
source commits, source/binary digests, toolchain and commands. Each Windows
runtime retains its build provenance; [checksums](data/2026-09-28-windows-engine-snapshot/SHA256SUMS)
cover the raw artifacts. The initial failed runs and intermediate passing runs
remain alongside the final results.

- [Native environment failure](data/2026-09-28-windows-engine-snapshot/captured-environment-before.log.gz),
  [historical map failure](data/2026-09-28-windows-engine-snapshot/legacy-environment-before.log.gz),
  [final native configuration tests](data/2026-09-28-windows-engine-snapshot/windows-configuration-final.log.gz)
  and [retention regression](data/2026-09-28-windows-engine-snapshot/legacy-retention-after.log.gz).
- [Workspace tests](data/2026-09-28-windows-engine-snapshot/workspace-tests-final.log.gz),
  [workspace Clippy](data/2026-09-28-windows-engine-snapshot/workspace-clippy-final.log.gz)
  and [Windows cross-target Clippy](data/2026-09-28-windows-engine-snapshot/windows-cross-clippy-final.log.gz).
- [Final Windows staging](data/2026-09-28-windows-engine-snapshot/windows-development-staging-final.log.gz)
  and [framework process fixture](data/2026-09-28-windows-engine-snapshot/windows-framework-process-final.log.gz).
- [Windows compiled Emacs](data/2026-09-28-windows-engine-snapshot/windows-compiled-emacs-third.log.gz),
  [Linux compiled Emacs](data/2026-09-28-windows-engine-snapshot/linux-compiled-emacs-final.log.gz)
  and [Windows remote acceptance](data/2026-09-28-windows-engine-snapshot/windows-remote-compiled-emacs.log.gz).

## Qualification scope

Windows live staging checks eSpeak inventory/cache, Flite and RuTTS voices/PCM,
and TGSpeechBox voices/PCM at 44,100 and 22,050 Hz. Piper is excluded by the full
development target used here. This does not renew every proprietary/native
adapter's independent qualification, establish macOS behavior, or prove audible
output. No existing Emacs session was restarted.

The framework's full acceptance-matrix audit and final configuration-reference,
roadmap and changelog reconciliation remain outstanding.
