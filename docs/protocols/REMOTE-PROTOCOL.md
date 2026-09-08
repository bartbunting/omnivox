# Remote workstation protocol, version 1

This protocol serves the remote-access preview. Its version identifies the
wire contract, not a claim of stable feature support.

The workstation runs `omnivox --serve --listen 127.0.0.1:6417 --token-file PATH`
with optional `--sound-root DIRECTORY` and `--audio-output null`. The default
listen address is `127.0.0.1:6417`. Only loopback is permitted. Without a sound
root, remote file playback is disabled. No engine-selection or environment
arguments are accepted from clients.

## Authentication and session ownership

All records are LF-terminated UTF-8, with optional CR before LF. Within five
seconds of connecting, send one ASCII record (at most 256 bytes including LF):

```text
OMNIVOX-REMOTE 1 TOKEN SESSION speaker
```

`TOKEN` is exactly 64 lowercase hexadecimal characters generated from 32 random
bytes. Its file contains that value and an optional final newline; Unix token
files must have no group or other permissions. `SESSION` is 32 lowercase
hexadecimal characters, unique to this Emacs process/connection lifecycle.
The other lane name is `notification`. Authentication errors do not echo input.

The service replies `OMNIVOX-REMOTE 1 ready\n` after reserving the lane and
starting its worker, or `OMNIVOX-REMOTE 1 error REASON\n` and closes. Reasons
include `authentication`, `busy`, `handshake`, and `worker`. Readiness here means
transport readiness: the client must still negotiate normal Omnivox readiness.
One session owns both lanes. A duplicate lane is busy until its former worker
has been retired; the client may retry. Ownership is released when both lanes
have been retired. At most four connections may be authenticating or active.

## Established transport

After readiness, the existing stdio protocol is forwarded bidirectionally.
The sole reserved transport command is `OMNIVOX-REMOTE ping\n`; the service
answers `OMNIVOX-REMOTE pong\n` without passing it to the worker. The client
sends it every five seconds and disconnects after 20 seconds without a pong.
The service expires a connection after 20 seconds without a complete record.
Partial bytes do not extend that deadline.

Each record is at most 512 KiB, including LF. Unterminated, oversized, invalid
UTF-8, and NUL-containing input is never forwarded. A bounded four-record
handoff prevents an unresponsive worker consuming unlimited memory. Socket
writes time out after two seconds; a handoff that stays full for two seconds
or a transport error retires the lane. Disconnect cancels speech by terminating its owned worker tree;
queued speech and incomplete records are discarded. Reconnection creates new
workers and repeats configuration; it does not restore pending utterances.

The worker accepts the complete `voice_choice_tuning_v1`,
`presentation_timeline_v4`, `playback_marker_events_v3` bundle over this same
transport. Negotiate each lane independently and re-register voices after
reconnection. Version-4 multipart frames use the existing 512 KiB line bound;
assembly and registry validation belong to the worker, before queue admission.

## Audio resources

Both legacy audio commands and structured timelines use identifiers such as
`omnivox-icon:packs/chimes/button.ogg`. The suffix uses `/` separators and
nonempty components containing only ASCII letters, digits, `_`, `-`, and `.`;
`.` and `..` components are forbidden. Identifiers resolve below the configured
sound root after canonicalization. Absolute paths, backslashes, other URI
schemes, and escaping symlinks are rejected. The workstation owns this directory
and its contents. Existing encoded-size, duration, and cache bounds apply.

## SSH forwarding

On the workstation, connect to the Emacs host with:

```sh
ssh -o ExitOnForwardFailure=yes -o ServerAliveInterval=10 \
    -o ServerAliveCountMax=2 -R 127.0.0.1:6417:127.0.0.1:6417 user@emacs-host
```

Configure remote Emacs to connect to `127.0.0.1:6417` and read its private copy
of the token. Both listeners stay on loopback. Use a different remote port if
6417 is occupied. Do not enable SSH `GatewayPorts` for this service. Provision
the token through a trusted channel, for example `scp` over verified SSH.
The local service accepts `quit` on stdin for controlled shutdown; terminal
interrupt also retires its workers. Rotate the token by stopping the service,
replacing both token files, and starting it again.
