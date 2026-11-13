# Windows audio-device and channel listening acceptance

Accepted on 2026-11-13 for the tested Windows development candidate. Bart
reported that listening worked well while switching repeatedly between Left
and Right and selecting different physical devices for main and notifications.
This is a listening observation, not an acoustic timing measurement or release.

## Candidate and scope

The Emacsvox Audio Devices shortcut points to `AudioDeviceTests/transient-20261113`,
using Emacsvox client `a89c70565` and Windows runtime `287fd0119a8f77d0` from
Omnivox source `89a0b98`. Native Windows Emacs is 31.1. The
[client report](https://github.com/bartbunting/emacsvox/blob/a89c70565/docs/audio-device-menu-check-2026-11-13.org)
retains its automated checks, Transient menu implementation and notification
failure diagnosis. The engine was not specified in the listening report.
Automated Eloquence and DECtalk coverage is distinct from this observation.

The runtime's [routing qualification](2026-11-13-live-notification-routing.md)
retains its full staging, source and payload identities. The earlier
[device qualification](2026-11-13-audio-device-selection.md) describes the
initial implementation and its own candidate; its historical results are
unchanged. The final client also fixes command-error feedback that created
abandoned workers after notification failure. Its original native startup
timeout remains unexplained and was not reproduced in fresh tests.

## Remaining coverage

The reported Windows main/notification device switching and Left/Right listening
check is complete. Physical unplug/reconnect during speech or idle, Bluetooth
profile changes, additional hardware and platforms, and acoustic onset or
stop-to-silence measurements remain separate work. Save and subsequent startup
have automated coverage; this report adds no listening claim for that sequence.
