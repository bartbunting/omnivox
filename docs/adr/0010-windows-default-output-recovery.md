# ADR 0010: Follow the Windows default audio output

- Status: Accepted
- Accepted: 2026-09-28 for automatic Windows headphone switching, cancellation
  of interrupted speech and recovery without restarting engines.
- Renumbered: 2026-09-28 from ADR 0013 during the approved consolidation.
- Extends: [Saved audio output](0009-local-speech-preferences.md#preserve-per-lane-output-choices).
- Related: [Progressive playback](0003-progressive-audio-and-markers.md),
  [worker ownership](0004-workstation-service-and-worker-ownership.md),
  [PulseAudio recovery](0005-native-pulseaudio-output.md).

## Context

Windows device output currently opens the default endpoint at startup and keeps
it. The speakers may remain available after headphones become the default, so
waiting for playback failure cannot implement switching. Restarting the whole
worker unnecessarily rebuilds speech engines and inventory.

## Decision

Windows `device` output follows the system's default rendering endpoint for the
console role, matching the existing CPAL selection. Observe native endpoint
notifications and use endpoint identities, never display names. Keep rodio/CPAL
for mixing, sample conversion and device playback. Other platforms and the
explicit null/PulseAudio backends retain their existing behavior.

One owned output thread registers notifications and opens, replaces and drops
the native connection. Notification callbacks only hand off bounded events;
they never reopen output, wait for speech or unregister themselves. Recheck the
default and intervening notifications before publishing a prepared connection.
There is at most one connection attempt per process at a time.

A switch or loss retires all three output streams, pending overlays and the
worker's old request generation. Interrupt active synthesis through the existing
generation checks, preserving engine health and the no-replay rule. Sources and
unreached cues from the old connection cannot appear on its replacement.
Requests arriving while output is unavailable are not retained for later speech.
Publish fresh queues and retire the unavailable generation before admitting new
playback. The two Omnivox workers follow the default independently.

Opening and teardown happen outside admission/stop locks. Clear local queues
without waiting for a failed device to consume them. Output shutdown closes
admission, cancels producers and retains ownership through native cleanup.
Keep initial setup failure as a startup error. After a successful startup, a
temporary missing device leaves the process alive. Bound automatic retry work;
new endpoint events or fresh audio may request another attempt. Do not silently
try an arbitrary non-default device when the selected default cannot open.

The configured backend, channel, gain and PulseAudio preference retain their
existing meanings. Following the Windows default needs no new setting or
protocol field. Reset does not itself reopen output. Named-device selection,
separate physical destinations and automatic switching on other platforms remain
separate work.

## Consequences and alternatives

Speech engines and their inventory survive output changes. An interrupted
utterance is cancelled; seamless continuation would need a separate policy and
honest acoustic position evidence. Reopening on every utterance would add setup
cost and would not handle an idle change or active speech correctly. Polling
device names cannot establish endpoint identity. Restarting the worker remains
an operational workaround rather than the implementation.

Check switching during buffered/progressive speech, stop and shutdown races,
blocked producers, stale connection attempts, missing devices, bounded retries,
both worker lanes and unchanged channel/gain behavior. Native Windows build and
notification checks are distinct from physical headphone/listening acceptance.
Microsoft documents the [notification callback constraints](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immnotificationclient)
and [stream-routing lifecycle](https://learn.microsoft.com/en-us/windows/win32/coreaudio/stream-routing-implementation-considerations).
