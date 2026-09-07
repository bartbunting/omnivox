# Real SSH workstation acceptance, 2026-09-07

Feature 5 now has repeatable acceptance using Emacs on a separate Linux host
and speech on this workstation. The existing remote client recovered both
lanes after an actual SSH tunnel interruption. No production protocol,
reconnection, installation, or runtime changes were required for these cases.
The feature remains preview.

## Setup and ownership

- Remote host: Ubuntu 26.04.1 LTS, x86_64, GNU Emacs 31.1. Its existing
  Emacsvox checkout was left untouched.
- Client: a private, source-only snapshot of committed Emacsvox
  `05828daa7f6d9c19dc2c0c38e31a2fc04237f48b`, loaded with `-Q --batch`
  and a temporary home. Local executable resolution was replaced with a
  failing test stub to catch accidental synthesis launches on the remote host.
- Windows service: existing development runtime `98084bb159c06059`, which
  reports version 1.7.1 and includes the remote preview. Executable SHA-256:
  `d896ad6b9b811dba5d39c69a17bc7ff38cb4a6fd084b63bb6c674c18e99c9647`.
  This is development-runtime evidence, not acceptance of a newly built 1.8
  Windows release artifact.
- Linux service: staged 1.8.0 development executable SHA-256
  `332ab8dcde66e3640ad54deb9711e335576a7af1ba02460aefd3a210c27522c3`.
- Windows OpenSSH owned the Windows-service reverse forward; Linux OpenSSH
  owned the Linux-service forward. Both used established host trust and keys.
  These runs used real SSH, without the earlier test-only .NET relay.
- Every run created a separate service, ephemeral loopback ports, a private
  token, and a temporary remote snapshot. The check verified the remote
  listener bound only to `127.0.0.1`. All three successful runs removed their
  remote snapshots and shut down their owned processes.

The fixture SHA-256 for the runs below is
`2d265bbcd2625b0db156d6c7d85b3606315e2947c995fe5819c3890d57937ce2`.
See [the repeatable commands](../REMOTE.md#repeatable-real-ssh-check).

## Results

| Workstation service | Output | Interrupted requests failed | Restore tunnel through completion of both fresh announcements | Result |
| --- | --- | --- | --- | --- |
| Windows DECtalk | Null | 0.124 s | 2.572 s | Passed |
| Windows DECtalk | Device | 0.080 s | 4.152 s | Passed |
| Linux eSpeak | Null | 0.067 s | 1.715 s | Passed |

The recovery interval includes SSH setup and two sequential utterances. These
are single-run acceptance timings, not command-to-sound latency measurements
or a comparison of Windows and Linux acoustic responsiveness.

Each run verified:

1. Both lanes obtained workstation inventory, registered voices, and applied
   the requested routing policy. Playback markers identified the exact
   requested engine, and both initial utterances completed.
2. A 22-second idle period preserved the same two connections, exceeding the
   service's 20-second lease and exercising the client's normal heartbeat.
3. The harness queued pending work on both lanes and terminated its own SSH
   forwarding process. Both interrupted dispatches failed exactly once.
4. After restoring the same port, the running Emacs automatically obtained
   replacement connections and fresh inventory, registration, and routing.
   Both new utterances completed within the fixture's deadline; interrupted
   dispatches did not later receive another terminal callback.

The Windows device run used the real device backend and completed four marked
announcements. Listening confirmation and physical audio measurement have not
been recorded; completion follows mixer consumption and cannot prove what a
person heard.

Before these runs, seven existing remote-service tests passed on each of Linux
and Windows, including live Emacs, exact engine checks, and heartbeat expiry.
Local supervisor regression tests also verify that early exits retain useful
diagnostics, loss of the client connection retires remote Emacs, and normal
completion does not hang on an open SSH input stream.

## Harness findings and remaining work

Initial harness attempts exposed test assumptions, which were corrected before
the successful runs: the notification lane does not set the foreground-only
initial-routing hook flag; batch stdout buffers synchronization messages;
null output consumes silence without a wall-clock delay; and a Python daemon
thread must not hold buffered stdin's lock during interpreter shutdown.

Raw, token-redacted reports are retained locally under the ignored
`target/remote-workstation-20260907/` directory. Failed harness attempts are
kept alongside the passing evidence rather than presented as product failures.

Remaining acceptance includes native macOS, longer outages and workstation
sleep/resume, repeated recovery during interactive editing, installed-client
and matched-release payload checks, and listening confirmation. The current
test models prompt SSH loss with a broken connection; it does not establish
recovery timing for an undetected network blackhole. Automatic production
tunnel management remains deferred under ADR 0008.
