# ADR 0008: Remote Workstation Speech Service

- Status: Accepted
- Date: 2026-09-05

## Context

Emacs may run on an SSH host while the user's speech engines and speakers are
on their workstation. Audio, engine libraries, and helper processes should
remain on that workstation. Emacsvox already owns independent foreground and
notification processes; a remote connection must preserve those lanes.

## Decision

Add an opt-in loopback TCP service to the Omnivox executable. It authenticates
a versioned handshake before starting a normal stdio Omnivox worker for each
lane. Workers retain all accepted engine and helper boundaries in ADRs 0001
through 0007. The service forwards the existing speech, control, timeline,
marker, and completion records unchanged after authentication.

The first version admits one Emacs session, with at most one speaker and one
notification connection. Replacing a lane requires closing its previous
connection. Disconnect, heartbeat expiry, malformed framing, or a blocked
connection retires that lane and its owned worker tree. It never drains or
replays disconnected speech. Reconnection creates fresh workers and repeats
Emacsvox capability, inventory, routing, and logical-voice initialization.

The listener accepts only loopback addresses. Remote use requires SSH reverse
forwarding initiated from the workstation; SSH provides encryption and host
authentication. A separately generated secret token also protects the forwarded
endpoint against other users on the SSH host. The token is loaded from a file,
never an argument or a diagnostic. Connections and record buffers are bounded.

Remote resource requests use `omnivox-icon:` identifiers relative to an
explicit workstation sound directory. The shared audio loader rejects other
paths, traversal, and symlinks escaping that directory for remote workers.
Clients cannot select helper programs, install libraries, or upload sounds.
Ordinary stdio workers retain their existing local resource behavior.

The wire details are specified in
[REMOTE-PROTOCOL.md](../protocols/REMOTE-PROTOCOL.md). Native TLS, shared
multi-user playback, resource uploads, and automatic SSH management are deferred.

## Consequences

- Engine discovery describes the workstation; remote Emacs needs no engines.
- Both lanes preserve their own cancellation, queues, and runtime state.
- Speech responsiveness depends on network latency; PCM stays local.
- Setup needs an SSH tunnel, a private token on both machines, and matching
  sound packs. Missing icons fail without exposing arbitrary local resources.
- TCP service lifecycle and real Windows helper retirement need acceptance
  coverage in addition to existing stdio tests.
