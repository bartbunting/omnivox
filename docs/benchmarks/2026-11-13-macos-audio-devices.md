# macOS device-selection implementation checks

## Result and scope

The CoreAudio implementation reuses the existing output owner for discovery,
exact-ID Apply, default following and disconnected-device recovery. Main and
notification workers remain independent. Both modes open enumerated devices by
UID, avoiding CPAL's automatically rerouting default AudioUnit. Host preferences
use the Mac host UUID. The only dependency change is a macOS-only direct edge to
the already locked `objc2-core-audio` 0.3.2 bindings.

Local cross-target checks pass for Apple Silicon and Intel. **No native Mac
execution or listening was performed.** The [Mac test guide](../guides/macos-audio-devices.md)
provides native build, ignored hardware smoke-test and physical listening steps.
Nothing was deployed to the existing Windows candidate or published as a release.

## Verification

| Check | Result |
| --- | --- |
| `cargo +1.97.1 test --locked --workspace` on Linux/WSL | 1,232 passed, 0 failed, 2 ignored across 38 test binaries/doc-test groups |
| Locked workspace Clippy, all targets, warnings denied | Pass |
| Locked audio-crate Clippy, all targets, Apple Silicon | Pass (cross-target only) |
| Locked audio-crate Clippy, all targets, Intel Mac | Pass (cross-target only) |
| Locked audio-crate Clippy, all targets, Windows GNU x64 | Pass (cross-target only) |
| Formatting, whitespace, documentation and paired links | Pass |

The new portable callback tests exercise default/device-list event routing,
expired-cookie rejection, closed/dropped owners and bounded event overflow.
Existing shared owner tests cover failed/cancelled Apply, default changes,
explicit-device loss without fallback, reconnect without replay, shutdown and
queue retirement. Mac-only error classification and backend rejection tests
are compile-checked; their execution and the explicitly ignored real-output
smoke test remain pending on Mac.

The initial fresh-worktree checks lacked prepared native sources. After the
locked preparation targets, Piper's eSpeak data compiler encountered its known
fixed-size path limit in the long worktree path. The completed workspace checks
used the shorter scratch target recorded in provenance, linking the same
verified source inputs and limiting CMake parallelism to four. No Piper or
native-source code was changed. Retained logs include the missing Piper inputs
and long-path failure; the earlier missing Flite-input diagnostic was not saved.

## Provenance and limits

The Omnivox base is `3ed4e8d7561fe6ae7943fd8159fab18e602fd971`.
The [implementation patch](data/2026-11-13-macos-audio-devices/implementation.patch.gz),
[commands and toolchain](data/2026-11-13-macos-audio-devices/provenance.json), and
[raw check logs](data/2026-11-13-macos-audio-devices/checks.tar.gz) identify the
checked source and environment. The paired client starts from Emacsvox
`85214244b` and changes platform wording/documentation only.

These checks establish shared logic and target compilation. They do not establish
CoreAudio listener delivery, native linking, audible routing, Bluetooth profiles,
sleep/wake recovery, acoustic latency or native process-failure recovery. Native
Mac CI and a tester with two physical outputs are the next acceptance gates.
Linux physical-device selection and the macOS independent cleanup guard remain
separate work.
