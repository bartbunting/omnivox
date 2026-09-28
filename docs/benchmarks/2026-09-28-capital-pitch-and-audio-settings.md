# Capital pitch and audio configuration acceptance, 2026-09-28

This is functional acceptance for unreleased configuration version 2. Capital
pitch is implemented by `b450961`; audio output settings by `96498be`. The
[configuration guide](../guides/configuration.md) explains use; the exact
[capital-pitch](../reference/engine-configuration.md#capital-letter-pitch) and
[audio](../reference/engine-configuration.md#audio-output-settings) contracts
remain in the reference.

## Linux checks

Both slices passed locked workspace tests, workspace Clippy with all targets
and the CLI Piper feature, `make dev`, formatting and documentation checks.
The final audio workspace run includes both features. The existing qualified
native-routing and long-session stress tests remain ignored by default.

The tests establish:

- Capital pitch follows the engine actually attempting the letter. Twenty-eight
  cases cover global values, overrides, off, Unicode, lowercase and numeric
  characters through every buffered/streaming primary/fallback combination.
  Ordinary session pitch remains unchanged and character speed still applies.
- Configuration rejects invalid values, unknown/duplicate fields and unregistered
  engine overrides. Historical private startup schemas 1–4 retain their original
  serialized forms; new schema-5 records require the complete saved settings.
- Reset restores the startup channel across speech, tone and sound. Backend
  precedence is command line, environment, then saved file.

The complete process harness passed once after each slice. The final run adds
sample checks on processed WAV files: saved left, environment right and a CLI
both-channel override all produce the expected active and silent channels.
Saved null output is used by `--check` and `--play-wav`; the latter starts no
speech helper. The external fixture receives the configured capital pitch and
saved speech settings before and after reset, on both lanes and after recovery.
File/environment edits do not change retained settings. Shared preparation,
independent acknowledgements, blocked startup writes and retirement also pass.
The remote shared-snapshot/total-disconnect recovery test passed.

Two empty-input starts using the real Linux PulseAudio connection passed.
All three streams requested the saved 45 ms, then the environment override of
30 ms. These checks opened output without generating sound. They confirm
selection and the requested values; they do not measure audible latency or
exercise native reconnect with the new setting.

## Windows checks

The full sibling Emacsvox `windows-omnivox-dev` build passed package and native
inventory checks. Its native Windows release-profile executable then passed the
complete process harness, including saved null output, WAV channel precedence,
capital pitch, reset, both lanes, frozen recovery and cleanup.

The build records Omnivox `96498be` and Emacsvox `85cd277`, both with empty
tracked diffs. It uses a separate staging root and does not replace the installed
launcher. Under existing development policy, experimental TGSpeechBox is included
and Piper is omitted. This is development acceptance, not release publication.

## Method and limitations

The process harness uses a Python helper fixture on Linux and a compiled C#
fixture on Windows. It records synthesis requests and observes lifecycle events;
its samples are a generated tone consumed by null output or written to WAV.
Successful runs remove their temporary request traces. The retained logs report
the assertions, and the committed harness supplies their inputs.

These checks do not establish how strong the cue sounds in a particular native
voice, actual channel output at a speaker, or end-to-end latency. No listening,
macOS or long-running native engine stress was performed. The previously
recorded [dense eSpeak marker failure](2026-09-28-host-chunk-configuration.md#pre-existing-dense-marker-failure)
was not retested or changed. The fixed 15-second silence ceiling is unchanged.
This work is not included in published 1.13.0.

## Retained records and development retries

The [evidence pack](data/2026-09-28-capital-pitch-and-audio-settings/) contains
compressed logs, source and executable identities, the PulseAudio reproduction
script and results, Windows staging provenance and checksums. Acceptance logs
are `omnivox-capital-pitch-workspace-verified.log.gz`,
`omnivox-audio-settings-workspace.log.gz`, their build/Clippy/process logs and
the final documentation gates.

Earlier development logs are also retained. The first workspace invocation
omitted the prepared Piper input location. Initial letter tests used incomplete
fallback/effect fixtures and incorrect module references; those fixtures were
corrected. Two audio compile attempts found an old function call and a generic
function lifetime mismatch; both were fixed before the passing workspace run.
Names such as `letter-final` and `workspace-final` reflect intermediate attempts,
not acceptance. No failing run is counted as a pass.

## Reproduction

Use the pinned Rust 1.97.1 toolchain and prepared native inputs. This run reused
`CARGO_TARGET_DIR=/home/bart/ov113.v3MoTB`, disabled incremental compilation and
set `OMNIVOX_PIPER_INPUTS_DIR` to the verified repository Piper input directory.
From the Omnivox root:

```sh
cargo +1.97.1 test --locked --workspace
cargo +1.97.1 clippy --locked --workspace --all-targets --features omnivox-cli/piper -- -D warnings
make dev
python3 tools/verify_engine_configuration.py /path/to/staged/omnivox
OMNIVOX_REMOTE_TEST_PROGRAM=/path/to/staged/omnivox OMNIVOX_ENGINE=espeak \
  python3 tools/test_remote_service.py RemoteServiceTests.test_engine_snapshot_is_shared_and_survives_total_disconnect
python3 docs/benchmarks/data/2026-09-28-capital-pitch-and-audio-settings/pulse-check.py \
  /path/to/staged/omnivox /new/pulse-evidence-directory
make -C ../emacsvox windows-omnivox-dev OMNIVOX_RUNTIME_DIR=/absolute/separate/runtime
python3 tools/verify_engine_configuration.py --windows /mnt/c/path/to/staged/omnivox.exe
```

Use the native Windows runtime named by `windows-runtime.path` only after the
build succeeds. Reruns need new evidence directories; do not overwrite this pack.
