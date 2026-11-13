# Windows audio-device selection qualification

The [ADR 0020](../adr/0020-audio-output-device-selection.md) implementation
supports independent physical outputs for main and notification speech, live
Apply without rebuilding engines, separate Save, and retained worker recovery.
The full Windows development payload and paired compiled Emacsvox clients pass
the functional checks below. Physical unplugging, listening and acoustic timing
remain separate acceptance work in the [delivery plan](../plans/audio-device-selection.md).
This is development qualification, not a release announcement.

## Candidate and method

Omnivox starts at dependency-migration commit `601895daee414ecd32a26d2733759deb7c2e565b`;
Emacsvox starts at `13bb6b7764d8954c15fcd21485909ee198490717`.
[Provenance](data/2026-11-13-audio-device-selection/provenance.json) and compressed
source patches identify the exact tested changes. The
[runtime provenance](data/2026-11-13-audio-device-selection/runtime-PROVENANCE)
identifies complete Windows development build `9481974fc10b5ace`, including
Rust 1.97.1, Rodio 0.23.0, CPAL 0.18.2, container/compiler inputs, helpers and
eSpeak data. [Runtime hashes](data/2026-11-13-audio-device-selection/runtime-hashes.json)
record staged executable and library identities. Native Windows Emacs is 31.1;
WSL Emacs is 31.0.90.

The initial full staging attempt reached the Flite provenance step, where Docker
could not resolve the linked worktree's external Git metadata. A regular isolated
clone received the exact source patch; the unchanged full
`make windows-omnivox-dev` target then succeeded. Both logs are retained. No
main-only staging guard was bypassed. Piper was omitted by the standard
development target; its models and runtime were not changed or qualified here.

The runtime was staged before the last Lisp-only recovery corrections. Its
Emacsvox worktree hash describes that staging input. Separate client patches,
native source comparisons and final byte-code checks identify the later client.

The isolated harness starts two guarded speech lanes and checks actual engine
IDs plus source-completion callbacks. It opens devices, queries state, applies
choices, confirms owner/helper identities, retires owned workers and reloads
saved choices from an isolated Customize file. Operation waits use Emacs
`float-time` deadlines; they are functional bounds, not audible latency samples.
Native process snapshots confirm that live Apply retains the speech worker and
helper PIDs. WSL checks use the ordinary launcher and its environment forwarding.

## Results

| Check | Observed result |
| --- | --- |
| Locked Rust workspace | 1,227 top-level cases passed, zero failed, two ignored; one isolated child test also passed, giving 1,228 logged successes. |
| Locked Clippy | Workspace/all-target Linux and affected CLI/TTS/audio all-target Windows GNU checks passed with warnings denied. |
| Native Windows audio binaries | 160 unit and 31 integration tests passed; both normally ignored real-device checks passed when invoked explicitly. Executable hashes are in the native audio results. |
| Complete Windows staging | Main executable, helpers, data and notices staged; companion checks and native Flite/RuTTS/TGSpeechBox synthesis passed. |
| Final source client regressions | 39 device/recovery tests passed, including late replies after explicit recovery, retired owners with live pipes, and emergency recovery with unresolved ordinary Apply. |
| Final native compiled client | All 15 device tests passed; native byte-code preflight passed for 379 modules. |
| Graphical compiled menus | 288 passed, zero failed or skipped. The later recovery-only correction also passed fresh native compiled regression checks. |
| Full Emacs suite | 4,496 passed, three failed, 249 skipped. All three failures reproduced on unchanged master; details below. |
| Documentation and formatting | Byte-code, headers, version, generated manual/reference review, Rust formatting, paired links and whitespace checks passed. |

Five isolated native client runs passed: two each with Eloquence and DECtalk,
and one with WinRT. The final Eloquence/DECtalk runs include two distinct physical
endpoint connections: main stays on the current endpoint while notifications
open the other enumerated endpoint, then return to default. No speech is submitted
to the alternate endpoint during that connection check. Two WSL-to-Windows runs,
one each with Eloquence and DECtalk, passed discovery, Apply, recovery and Save.
Those WSL runs precede the final client recovery corrections, which are covered
by source and native compiled regression tests.

Across these runs:

- A missing candidate is rejected while the previous route remains usable.
- Main Apply leaves the notification revision and selection unchanged. Owners,
  native children and immutable engine activation remain in place.
- Speech completes with the requested engine before and after Apply. Tone and
  icon commands are exercised, without an independent acoustic assertion.
- Owned main recovery establishes the confirmed explicit device before fresh
  speech; the healthy notification worker remains unchanged.
- Saved host choices reload independently: explicit main output and default
  notification output. Browsing and Apply do not implicitly Save.
- Simulated backend tests cover failed preparation, cancellation, stale revisions,
  full notification queues and explicit-device loss/reconnect without fallback
  or replay. Real-device component tests force reconnection; they do not unplug
  hardware or change the Windows system default.

The ordinary personal Windows configuration also passed fresh graphical checks
with both Eloquence and DECtalk, including early startup, both guarded lanes,
device capability negotiation, and existing preference reload. A separate
**Emacsvox Audio Devices** desktop shortcut launches this candidate with Eloquence
by default. Its [handoff record](data/2026-11-13-audio-device-selection/handoff.json)
records the target and verifies that the normal Windows shortcut was unchanged.
The candidate uses the existing personal configuration; the check did not save
device preferences into it.

## Failures and retained evidence

The full Emacs suite's failures are the stale generated aural-reference audit,
the stderr-rotation test's 1.5-second deadline, and an obsolete native-Windows
ARM64 documentation expectation. All three fail on clean master
`13bb6b7764d8954c15fcd21485909ee198490717` with the same selected Emacs. They were
not changed as part of device selection. Batch graphical skips are not counted
as passes; the separate graphical suite establishes its own result.

Three early native harness attempts are retained: a missing special-variable
declaration stopped startup, an unset `user-init-file` prevented isolated
Customize saving, and a list/vector mismatch prevented final JSON serialization
after the functional steps. Corrected reruns passed. These harness failures are
not removed or presented as product failures.

[Build/check logs](data/2026-11-13-audio-device-selection/build-and-check-logs.tar.gz),
[native/WSL logs](data/2026-11-13-audio-device-selection/native-and-wsl-logs.tar.gz),
[harnesses](data/2026-11-13-audio-device-selection/harnesses.tar.gz), and
[native audio results](data/2026-11-13-audio-device-selection/native-audio-results.json)
retain raw outcomes and reproduction inputs. Runtime ownership snapshots are
not archived wholesale because they can contain inherited environment data;
the harness's observed process IDs and cleanup results are retained.
Verify the pack with its [checksums](data/2026-11-13-audio-device-selection/SHA256SUMS).

## Remaining acceptance

Physical unplug/reconnect during active and idle speech, audible separation of
the two outputs, Bluetooth profile changes, and acoustic onset/stop-to-silence
remain unmeasured. Source completion and a connected software endpoint do not
establish audibility. No performance improvement is claimed. Linux/macOS device
selection, native PulseAudio selection, remote management and binaural rendering
are outside this implementation.
