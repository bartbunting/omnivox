# ADR 0004: Workstation speech service and worker ownership

- Status: Accepted
- Consolidated: 2026-09-27 from the accepted remote-workstation decision.
- Related: [Engine isolation](0001-engine-isolation-and-distribution.md),
  [managed activation](0007-managed-voice-lifecycle.md).

## Context

Emacs may run on an SSH host while speech libraries, audio devices and the user's
two speech lanes remain on a workstation. Network access must preserve existing
worker isolation without becoming an executable or resource-management channel.

## Decision

Provide an opt-in loopback TCP service in the Omnivox executable. Authenticate a
versioned handshake before starting an ordinary stdio worker for each lane.
Forward existing speech, control, timeline, marker and completion records under
bounded framing. Synthesis and PCM stay on the workstation.

Admit one Emacs session with at most one foreground and one notification
connection. Replacing a lane requires closing its prior connection. Disconnect,
heartbeat expiry, malformed input or blocked forwarding retires that lane and
its owned worker tree. Disconnected speech is never drained or replayed. A new
connection creates a fresh worker and repeats capability, inventory, policy and
logical-voice initialization. Each lane retains independent cancellation.

The listener accepts loopback addresses only. Remote use requires workstation-
initiated SSH reverse forwarding for network encryption and host authentication.
A separate token protects the forwarded endpoint from other users on the SSH
host. Load that token from a file, never an argument or diagnostic. Bound
connections, input records, forwarding and cleanup.

Remote resources use `omnivox-icon:` identifiers inside an explicit workstation
sound directory. Reject other paths, traversal and links escaping that directory.
Remote clients cannot select helper executables, install libraries or upload
resources. Local stdio speech retains its ordinary file-resource behavior.

The native owner establishes the job/process-group boundary before a worker can
initialize helpers. Retirement retains ownership until tree and reader cleanup
is confirmed; a live control connection alone does not prove working speech.
Local managed activation uses the distinct local ownership contract in
[ADR 0007](0007-managed-voice-lifecycle.md), without exposing its management
operations on this network service.

## Consequences and alternatives

Clients discover the workstation's actual inventory and need no local engines.
Network latency remains part of speech responsiveness. Tests must exercise
authentication, lane replacement, blocked connections, worker retirement,
reconnection and no replay with real Windows ownership as well as unit fixtures.
SSH setup and platform/acoustic acceptance remain explicit operational work.

Native TLS, general network listeners, multi-user sharing, resource uploads and
automatic tunnel management are deferred. Direct tailnet or iOS receiver access
would require revisiting this boundary rather than merely changing a bind address.
The [remote protocol](../protocols/remote.md) specifies exact framing and the
[operations guide](../guides/remote-speech.md) describes setup and acceptance status.
