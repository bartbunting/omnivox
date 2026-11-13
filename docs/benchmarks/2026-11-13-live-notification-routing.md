# Live notification routing qualification — 2026-11-13

Functional development checks for
[local channel selection](../protocols/control.md#local-output-channel-selection)
and the paired Emacsvox notification menu. This is not a release or an acoustic
measurement. The earlier [physical-device report](2026-11-13-audio-device-selection.md)
remains unchanged.

## Builds and method

Omnivox starts at `66985e1`; Emacsvox starts at `c3ca91971`. Both use
`feat/audio-device-selection` plus the retained source patches. Rust is 1.97.1.
Full `make windows-omnivox-dev` produced runtime `287fd0119a8f77d0`, including
helper rebuilds and native staging checks. The regular build clone starts at
`601895d` with both feature increments applied; 291 tracked Rust/TOML/lock inputs
match the primary checkout. Its exact patch and runtime provenance are retained.

Native Emacs 31.1 compiled and checked all 380 modules in a fresh local candidate.
WSL Emacs 31.0.90 used a full byte-code rebuild and the ordinary Windows launcher.
Tests use isolated configuration and voice roots, source-completion events and
typed status queries. Native tests additionally compare main and notification
worker/helper process IDs. WSL results do not inspect native child PIDs.

## Results

| Check | Observed result |
| --- | --- |
| Locked Rust workspace | 1,229 top-level passes plus one nested isolated child; two ignored, zero failures. |
| Formatting and locked Clippy | Formatting, Linux workspace/all-target Clippy and affected CLI/TTS/audio Windows GNU all-target Clippy pass with warnings denied. |
| Focused source ERT | 48 passes covering channels, physical devices and recovery. |
| Native compiled ERT | 24 channel/device tests pass. |
| Compiled graphical menus | 289 pass, none skipped, after correcting menu action wiring. |
| Full ERT | Before the final menu wiring correction: 4,507 pass, three known baseline failures, 250 skipped. The three failures are the stale aural reference, launcher stderr timing deadline and obsolete Windows-guide ARM64 expectation, already reproduced on clean master in the physical-device report. Final menu wiring is covered by the compiled graphical and personal-profile checks. |
| Native Eloquence and DECtalk | Both pass live Left/Right/Both/Automatic, unchanged owners/helpers, subsequent speech completion, Reset, notification recovery, Main-output handoff, notifier recreation, separate Save and saved startup. |
| WSL to Windows | Eloquence and DECtalk pass the same lifecycle/status checks using compiled client modules. |

All three channel fields are confirmed before success. Tone and earcon commands
are exercised on each channel; these observations do not prove what was heard.
Previously admitted requests retain their captured route, verified by the Rust
state test. Source tests cover lost replies, uncertain state blocking Save and
assumed recovery, stale worker replies, cleanup failure and preparation failure.
Raw server Reset retains its startup semantics; the client reapplies its override.

The normal native personal-profile checks pass with Eloquence and DECtalk,
exercising both speech lanes, Left and
Right through the actual graphical menu, retained worker identities and preference
reload. Their individual JSON results remain in the evidence pack. The handoff
receipt identifies the shortcut target and separately verifies the ordinary
Windows shortcut.

## Corrections and limits

Two initial native runs stopped at a test comparing complete private startup
references across different owners. A new notifier correctly has a different
private path/hash while acknowledging the same activation ID. Main worker and
helper IDs and the physical device were preserved. The final harness compares
the acknowledged activation identity; the failed results are retained.

The first graphical run caught the live operation attached to the toggle helper
instead of the route-choice helper. The menu now applies before committing the
option or announcing success. Its passing rerun uses fresh compiled code.
The native preflight also rejected stale compiled parents after this menu edit;
recompiling those parents passed the preflight. Neither failure was bypassed.

There is no new acoustic left/right acceptance, performance claim, physical
disconnect experiment or non-Windows output-backend qualification here. Remote,
emergency and older workers keep their previous controls. Other live sessions and
saved personal preferences are preserved. No push or release publication is part
of these checks.

## Evidence

The [provenance](data/2026-11-13-live-notification-routing/provenance.json) identifies
inputs and commands. [Build and test logs](data/2026-11-13-live-notification-routing/check-logs.tar.gz),
[native and WSL results](data/2026-11-13-live-notification-routing/native-results.tar.gz),
[reproduction harnesses](data/2026-11-13-live-notification-routing/harnesses.tar.gz),
[runtime provenance](data/2026-11-13-live-notification-routing/runtime-PROVENANCE)
and [checksums](data/2026-11-13-live-notification-routing/SHA256SUMS) retain the
observations, including failed attempts. The source patches capture the final
implementation; runtime provenance separately records its build-time inputs.
