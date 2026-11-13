# Multi-engine Windows evidence

See the [report](../../2026-11-10-windows-multi-engine.md) for interpretation and
limits. These are isolated Windows x64/WSL measurements on 2026-11-10, with null
output. They do not establish acoustic timing or listening acceptance.

- `runtime-path.json` identifies final runtime `ff90ae379e43e184`; `provenance/`
  preserves exact staged build records and payload hashes. `source-path.patch`
  is its exact dirty source against `5a892fa`. `source-equivalence.json` records
  reconstruction and comparison with committed executable source at `dce3f3b`.
- `runtime-1.json` and `source-1.patch` are the first multi-helper candidate.
  `runtime-2.json`/`source-2.patch` preserve the rejected process-flag experiment.
  `runtime-full.json`/`source-full.patch` identify full helper staging before the
  .NET path correction; `runtime-full2.json`/`source-full2.patch` identify the
  committed helper-build base reused by the final runtime.
- `implementation.patch` isolates final Rust changes against `5a892fa`.
  `client-final.patch` is Emacsvox `d44a2c9a4` against `b83c43aad`; `client.patch`
  retains the earlier worktree input used in staging. Baseline Lisp source and
  the tested byte-code hashes are in `baseline-client/`. Local `.elc` files are
  deliberately not distributed; compile with the recorded Emacs version.
- `raw-windows.tar.gz` retains complete private run directories: source JSONL,
  stderr, configurations, damaged management fixtures, source timing, startup
  spans and all failed attempts. It also includes the eight final private-profile
  roots and the console-free helper pilot sources. Executables/DLLs are omitted;
  their hashes and provenance are retained separately.
- `checks.tar.gz` retains build/check logs, native unit outputs and result codes,
  failed fixture checks, and full Emacs candidate/baseline logs. The attempted
  narrow baseline wrapper was superseded by loading ERT and therefore ran the
  full suite; `client-baseline-comparison.json` records the actual full results.
- `summary.json` contains source/interaction/startup/availability summaries;
  `startup-native-summary.json` and `winrt-attribution.json` retain startup and
  per-request attribution. `interaction-followup.json` records the longer
  alternating follow-up; `flite-attribution.json` correlates its replacement
  source and worker queue times. Descriptive p95 uses nearest rank, not a tail guarantee.
- `listening/start.py` is the final private launcher. Its pinned runtime must
  remain staged, and the explicit experimental Emacsvox checkout must have current
  byte-code. It creates a fresh isolated profile; `--batch-check` suppresses device
  output. The launcher does not install or update a personal profile.

Source patches, baseline Lisp and a few historical fixtures use a `.gz` suffix
to preserve their original whitespace byte for byte. Names above refer to the
decompressed contents. Decompress into a separate temporary directory when
reconstructing a baseline; do not rewrite the retained evidence.

## Reproduction boundaries

Fixtures locate Omnivox by walking to `Cargo.toml` and reuse the retained
`resident-reload` and `emergency-speech` Python helpers. Keep this pack in the
repository. Paths in the runtime manifests refer to this machine's private
Windows Temp staging and external native runtime locations; provision a matching
private payload before rerunning elsewhere. Each fixture requires a new output
root. Preserve previous roots and results.

The final character probe used `REPEATS=4 SAMPLES=40`, with
`python3 fixtures/engines.py runtime-path.json NEW_WINDOWS_TEMP_ROOT`.
The native matrix used `python3 fixtures/native.py runtime-path.json NEW_ROOT`.
Both read the runtime and RHVoice locations from that manifest.

`interaction.py` and `interaction-followup.py` take executable and output-root
arguments. Export `OMNIVOX_TEST_WINDOWS_DATA`, `OMNIVOX_TEST_RHVOICE_LIBRARY` and
`OMNIVOX_TEST_RHVOICE_DATA` from the manifest. The initial run has 20 Stop and 20
replacement trials per engine/path; the follow-up has four alternating blocks of
40, covering replacement for Flite/DECtalk/RuTTS and Stop for Eloquence.

`startup.py` and `restart-availability.py` take the same executable/root and
`--profile`, with the three environment values above and
`OMNIVOX_TEST_BYTECODE=1`. Set `OMNIVOX_TEST_STARTUP_BASELINE` to the explicitly
compiled baseline `tts-speak.elc` only for baseline runs. Startup ran four fresh
profiles in baseline/candidate/candidate/baseline order, four restarts each.
Continuous availability ran four restarts per client separately. Its initial
failed fixture used a single `accept-process-output` as a duration wait; the
corrected fixture waits to a deadline and preserves intermediate samples before
assertions. Timer overhead and 25 ms sampling are part of these observations.

The final profile loop ran `python3 listening/start.py --engine ENGINE
--batch-check` for Flite, Eloquence, RHVoice, DECtalk, RuTTS, TGSpeechBox, WinRT and
eSpeak. Full-profile setup, current compiled paths, exact selected source engine,
reload, restart, retained recovery, emergency eSpeak and return to saved settings
are checked in each fresh process. No interactive listening run was performed.

## Build and verification

`fixtures/check-native.sh` records the locked Windows CLI/TTS Clippy and test
compilation wrapper. `fixtures/unit.py` runs explicit private native test binaries,
including ignored runtime cases; the final `unit-path/result.json` in the checks
archive records the binary hash, command and result. The workspace uses Rust
1.97.1 from `rust-toolchain.toml`; the pinned container records its compiler and
C#/MinGW inputs in provenance.

The Linux checks were `cargo test --locked --workspace` and
`cargo clippy --locked --workspace --all-targets --features omnivox-cli/piper --
-D warnings`, using the selected toolchain and the existing explicit Piper inputs.
Helper checks were `make windows-helpers-test`. Runnable Windows payloads used
paired `windows-omnivox-dev` and then `windows-omnivox-main-dev`, with the
private release wrapper and destination recorded in staging logs. No direct
Cargo output was presented as a distributable payload.

Emacs checks used `/home/bart/opt/emacs-31/bin/emacs` (31.0.90), focused recovery
ERT, byte-code build/preflight, full tests and `docs-release-check`. A first
byte-code attempt with the system Emacs failed on compiler-version compatibility;
the explicit intended compiler passed. Full-suite failures are the same seventeen
on unchanged baseline and candidate; they are not claimed as passes.

Archives preserve their member listing and hashes in `archive-inventory.json`.
`SHA256SUMS` covers all files in this pack except itself. Historical evidence packs
and the 1.18.0 release baseline remain unchanged.
