# ADR 0020: Audio output device selection

- Status: Accepted for implementation on 2026-11-13. Implementation, native
  qualification and release remain separate.
- Date: 2026-11-13.
- Extends: [0009](0009-local-speech-preferences.md#preserve-per-lane-output-choices)
  and [0010](0010-windows-default-output-recovery.md).
- Related: [Playback and cancellation](0003-progressive-audio-and-markers.md),
  [independent workers](0004-workstation-service-and-worker-ownership.md),
  [PulseAudio](0005-native-pulseaudio-output.md),
  [independent preference reload](0017-independent-host-preference-reload.md),
  [emergency speech](0018-minimal-emergency-speech.md) and
  [native creation ownership](0019-windows-native-creation-ownership.md).

## Context

Channel selection and stereo positioning do not select a physical output.
Following the Windows default also cannot keep speech on a particular headset
while another application changes the default. Main speech and notifications
already have independent workers, so each can use a different output without
changing engine isolation.

ADR 0009 defines saved backend, channel and latency preferences. ADR 0010 defines
Windows follow-default recovery. Both leave named-device selection outside their
scope. Neither defines an explicit device's disappearance, live selection or
retention through worker recovery. These choices affect where speech can emerge
and need a durable policy.

## Decision

### Identify outputs on the speech host

Offer two modes per lane: follow the system default, or use one explicit output
identified by its backend and opaque device ID. Obtain IDs from the backend;
never derive identity from a display name, list position or enumeration order.
Friendly names are labels and may be duplicated or change. If a backend cannot
supply a usable identity, report the limitation instead of inventing one.

Keep saved choices scoped to the speech host and backend. A native Windows
endpoint is not a Linux/WSLg endpoint, even when both ultimately reach the same
headphones. Preserve a missing saved choice as unavailable so that the user can
recognize and replace it. Do not remap it by name to another device.

Main and notification choices are independent. A lane's speech, tones and sounds
share its physical destination; channel selection, spatial balance and gains
remain separate settings. This does not introduce extra speech workers or merge
their cancellation domains.

### Apply deliberately without restarting speech engines

Browsing a list only changes the highlighted candidate. Apply explicitly changes
the selected lane's output. Save explicitly stores its choice for future startup;
applying does not implicitly save. Show the saved choice, actual applied choice
and output availability separately. Independent lane outcomes are not an atomic
two-lane transaction.

Use the existing worker connection with explicit capability negotiation. The
worker's output owner opens and replaces the connection outside speech admission
locks, retaining engine/helper processes and current speech settings. Keep
operations and replies bounded and correlated to the worker incarnation and
output revision. Recheck the requested identity and revision before publication.
Unsupported peers retain their existing startup and restart behavior.

Preflight a candidate while the working route remains available. A rejected or
cancelled preparation leaves that route in place. Publication retires all three
old output queues and their request generation, cancels interrupted synthesis
and admits fresh speech on the new connection. Never replay an interrupted
utterance or an unreached cue. If the old device independently disappears,
continued availability cannot be promised.

A lost reply leaves the operation outcome unknown until queried; it does not
justify rollback, resubmission under a new identity or a success announcement.
Bound retained results and reject stale requests. Apply neither rereads arbitrary
configuration files nor changes the backend, buffer policy, engines or managed
assets. It is separate from host-preference Reload.

### Preserve the chosen destination during failure and recovery

Follow-default mode follows the host's supported default-output policy. Explicit
mode ignores unrelated default changes and retries only the chosen identity.
When that device disappears, cancel its queued speech and report unavailable;
do not silently send speech to speakers or another headset. A new explicit Apply
can choose follow-default or another device.

Reuse bounded retry, owner-thread cleanup, generation checks and no-replay rules
from ADR 0010. Requests during an outage are discarded. Initial failure to open
the requested output remains a startup error. Output failure does not mark a
healthy speech engine as failed.

Retain each confirmed applied choice with the owning client's lane launch state
for automatic worker replacement. Preserve immutable engine startup records;
device selection is a separate per-lane output override. Recovery must establish
the intended route before admitting speech. An unresolved Apply must be
reconciled or reported explicitly rather than recovering onto an assumed route.
Fresh startup uses the saved choice; unsaved changes need not survive client
exit. Emergency speech remains an explicit operation with its own output choice
and must not be defeated by an unavailable saved device.

### Qualify each backend and preserve existing boundaries

Use Rodio 0.23 with CPAL 0.18.2 for device playback and identity. Retain the
canonical stereo PCM boundary, existing WAV/Vorbis decoding, audio bounds and
independent helper executables. Do not enable new optional CPAL backends or audio
effects as part of the upgrade.

First qualify the existing Windows device backend, including native Emacs and
WSL launching a Windows worker. Portable identity APIs alone do not qualify
Linux/macOS hotplug handling or expose Windows endpoints to a Linux WSLg worker.
Keep existing PulseAudio, null and other-platform behavior until their selection
and recovery paths are separately implemented and tested under this policy.
Present unsupported selection honestly. Detailed delivery and acceptance remain
in the [plan](../plans/audio-device-selection.md).

Initial discovery and Apply are local to the speech host. Do not extend remote
management authority or add helper/configuration upload. Keep existing native
ownership, asset retention and verified cleanup requirements. Dependency and
public protocol changes require full Windows development staging before product
qualification. Acceptance does not establish a released capability or acoustic
performance.

## Consequences and alternatives

Users can keep speech on a selected headset and route notifications independently.
Explicit-device outages can leave speech unavailable; an accessible way to choose
follow-default and the existing explicit emergency path are therefore essential.
Silent default fallback improves availability but can disclose speech on an
unintended output, so it is not the proposed policy.

Restart-only selection is simpler but rebuilds engines for an output change.
Output-only Apply preserves working engines at the cost of revision, publication
and recovery handling. Selecting by friendly name avoids opaque saved values but
cannot distinguish duplicate or renamed devices. A shared output process would
change failure and ownership boundaries without being required for this feature.
