# Windows output recovery and onset fix, 2026-09-28

This development change completes the Windows follow-default implementation
under [ADR 0010](../adr/0010-windows-default-output-recovery.md) and fixes the
[reproduced idle-onset stretch](2026-09-28-idle-onset.md). It is not part of the
published 1.13.0 release.

Windows output owns endpoint notifications and stream replacement on a separate
thread. Replacement cancels old queues, overlays and synthesis generations,
including progressive sources waiting for PCM, without restarting engines or
replaying speech. Missing output rejects new audio; bounded retries and fresh
events/requests can recover it. Native work stays outside admission/stop locks.

Both regular Rodio sinks and replaceable Windows queues now keep their output
metadata at Omnivox's canonical 44.1 kHz stereo format through idle periods and
source boundaries. This prevents the first 256 stereo frames from being expanded
into 512 frames. The device mixer still converts the canonical stream to the
selected endpoint's channel count and rate. No helper protocol, playback reserve
or silence-trim setting changes.

## Verification

Checks used the pinned Rust 1.97.1 toolchain and locked dependencies on Linux
x64 under WSL2 and native Windows x64. The
[provenance](data/2026-09-28-device-recovery-and-onset-fix/provenance.json)
records the base commit, development source hashes, commands and platform details.

| Check | Result |
| --- | --- |
| Old queue metadata control | Three onset regression tests fail in an isolated source copy. |
| Linux audio suite | 158 unit tests and 31 integration tests pass. |
| Locked workspace tests | All suites pass after staging matching generated eSpeak data. |
| Native Windows audio suite | 141 tests pass; the real-output test is ignored in the ordinary run. |
| Native Windows real output | The ignored test passes when explicitly run: opens an actual stream, cancels old speech on replacement, completes fresh speech and rejects audio after shutdown. |
| Clippy | Workspace/all targets and Windows audio/CLI/all targets pass with warnings denied. |
| Build and documentation | `make dev`, formatting, documentation links, paired Emacsvox links and Emacsvox documentation review pass. |
| Windows development staging | Full `make windows-omnivox-dev` passes, including payload verification and live engine checks. |
| Packaged Windows output | Two concurrent processes, selecting left and right output, play and drain a silent WAV through `--play-wav` successfully. |

Onset regressions compare every interleaved sample, using distinguishable left
and right channels. They cover buffered/tracked speech, tones and sounds, cold
start, repeated idle periods, adjacent sources of 1/255/256/257/2,048 frames,
progressive window boundaries, normal and letter reserves, cue order,
completion, cancellation fades and fresh speech after cancellation. Conversion
checks compare the onset with a directly converted source at mono 44.1 kHz,
mono/stereo 48 kHz and six-channel 44.1 kHz. Replacement tests repeat buffered
and progressive playback across three simulated endpoint selections.

Recovery tests cover active/queued audio, deferred overlays, blocked progressive
consumers, unprimed producers, stale connection attempts, stop/shutdown during a
blocked open, duplicate/unrelated events, absent endpoints, bounded retries,
notification overflow and native callback role/property filtering.

The native real-output test uses only zero-valued samples. It injects an endpoint
event to replace the actual connection on the same Windows default endpoint;
it does not change the user's selected device. This establishes native lifecycle
behavior, independently of simulated mixer tests and physical listening.

The full Emacsvox development target staged build `ad5be9cca3f9e40b` and selected
it for subsequent speech-process starts. Dependency and shared audio changes
require this full target. The
[runtime provenance](data/2026-09-28-device-recovery-and-onset-fix/runtime-PROVENANCE)
and [staging log](data/2026-09-28-device-recovery-and-onset-fix/windows-development-stage.log.gz)
record the built payload. The packaged output check used the Windows-local
executable with that provenance's SHA-256 and isolated temporary configurations;
its [results and input](data/2026-09-28-device-recovery-and-onset-fix/packaged-output/)
are retained. Existing user speech processes were not restarted.

The first workspace run failed 13 eSpeak tests because the local staged data did
not match the newly built library. `make dev` staged the generated data through
the supported wrapper; the unchanged tests then passed. Both the
[initial failure](data/2026-09-28-device-recovery-and-onset-fix/workspace-tests.log.gz)
and [passing rerun](data/2026-09-28-device-recovery-and-onset-fix/workspace-tests-staged-data.log.gz)
are retained. No source or dependency change was needed for that setup issue.

## Reproduce the implementation checks

```sh
make dev
cargo +1.97.1 test --locked --workspace
cargo +1.97.1 clippy --locked --workspace --all-targets -- -D warnings
cargo +1.97.1 test --locked -p omnivox-audio --lib idle_tests
cargo +1.97.1 test --locked -p omnivox-audio --lib device::tests
cargo +1.97.1 test --locked -p omnivox-audio --target x86_64-pc-windows-gnu --lib --no-run
```

Run the resulting Windows test executable through WSL interop, first with
`--test-threads=1`, then with
`native_output_reopens_and_retires_old_audio --ignored --nocapture --test-threads=1`.
The second command requires a working Windows default output.

From the sibling Emacsvox checkout, run `make windows-omnivox-dev` to rebuild,
stage and verify the complete development payload. The WSL-only
[packaged output check](data/2026-09-28-device-recovery-and-onset-fix/check_packaged_output.py)
accepts `--program` with the Windows-local executable's WSL path and `--output`
with a new evidence directory. It uses only zero-valued PCM.

## Limits

No physical headphone plug/unplug, actual default-endpoint change, listening
assessment or WASAPI process-loopback recording was performed. Mixer sample
equality and native stream replacement support different claims; neither is a
measurement of acoustic onset or stop-to-silence. The initial frame loss and
lead-in collapse from the original email remain unconfirmed.

The [evidence directory](data/2026-09-28-device-recovery-and-onset-fix/) retains
compressed raw logs and source provenance. The original reproduction evidence
is retained separately.
