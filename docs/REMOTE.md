# Remote workstation speech (preview)

This preview feature, introduced in Omnivox 1.8.0, lets remote Emacs use
speech engines and speakers on your workstation. Use an Omnivox 1.8.0 or newer
release payload and the matching Emacsvox remote client. For development, use
`make dev` on Linux/macOS or Emacsvox's `make windows-omnivox-dev` for Windows.

1. Create a token with `python3 tools/create_remote_token.py PATH`. The tool
   uses 32 cryptographically random bytes, creates a private file, and refuses
   to replace a file. Keep a private copy on the remote Emacs host using
   verified SSH/SCP. Unix files require mode 600; on Windows place the token
   under your private user profile and retain its user-only access controls.
2. On the workstation, start
   `omnivox --serve --token-file PATH --sound-root /path/to/emacsvox/sounds`.
   Keep runtime data and notices beside the executable. If using Emacsvox's
   WSL launcher to run Windows Omnivox, pass Windows paths for both options
   using `wslpath -w`. Without a sound root, file-based icons are disabled.
3. From the workstation open the SSH reverse forward:

   ```sh
   ssh -o ExitOnForwardFailure=yes -o ServerAliveInterval=10 \
       -o ServerAliveCountMax=2 \
       -R 127.0.0.1:6417:127.0.0.1:6417 user@emacs-host
   ```

   For Windows Omnivox, run this SSH connection using Windows OpenSSH
   (`ssh.exe` from WSL, or `ssh` in PowerShell). WSL under NAT has a separate
   loopback, so a WSL-native SSH forward would target the wrong host. Windows
   OpenSSH uses your Windows SSH keys and host-trust configuration. See
   [Microsoft's WSL networking guide](https://learn.microsoft.com/en-us/windows/wsl/networking).

4. Before loading Emacsvox on that host, configure:

   ```elisp
   (setq tts-program "omnivox"
         omnivox-remote-host "127.0.0.1"
         omnivox-remote-port 6417
         omnivox-remote-token-file "~/.config/omnivox/remote-token")
   ```

Use `M-x omnivox-remote-connect`, `omnivox-remote-status`, and
`omnivox-remote-disconnect` to manage the connection. Disconnect suspends
automatic retries. Unexpected connection loss fails unfinished speech and
reconnects with fresh inventory and routing; speech is never replayed.

Both endpoints must stay on loopback. Do not enable SSH `GatewayPorts`.
SSH authenticates and encrypts the tunnel; the token protects its forwarded
endpoint from other users on the SSH host. A second Emacs session is refused
until the first releases both lanes. A restarted lane may briefly report busy
while its former worker retires. Change the left-hand forwarded port and the
Emacs option together if the remote port is occupied.

Install engines on the workstation. `OMNIVOX_ENGINE` selects its preferred
engine as in ordinary server mode. `OMNIVOX_REMOTE_NOTIFICATION_TARGET` may
select a workstation-local notification target (`left`, `right`, or `both`);
clients cannot change the service environment or helper paths. Bundled sounds
use stable `omnivox-icon:packs/...` IDs and require matching local packs.
Custom uploads and external personal sound paths are deferred.

Type `quit` in the service terminal, or interrupt it, to stop its workers.
Rotate a token by stopping the service and replacing both private copies.
The broker does not manage SSH, login startup, or a system-wide service.

For silent acceptance use `--audio-output null`. Run `make remote-test` after
`make dev`; set `OMNIVOX_REMOTE_TEST_SLOW=1` for heartbeat-expiry coverage.
For live Emacsvox acceptance also set `OMNIVOX_REMOTE_TEST_EMACS` to the pinned
Emacs executable and `OMNIVOX_REMOTE_TEST_EMACSVOX` to that checkout. Tests use
temporary tokens, ephemeral listeners, and isolated workers. Native device,
real SSH-host, and macOS acceptance remain separate from null-output tests.

For WSL-to-Windows acceptance, set `OMNIVOX_REMOTE_TEST_WINDOWS=1` and
`OMNIVOX_REMOTE_TEST_PROGRAM` to the Emacsvox launcher. The harness uses the
installed Windows .NET Framework C# compiler for a temporary test-only stdio
relay across WSL NAT; production setup uses Windows OpenSSH. The test relay,
tokens, listeners, and workers are removed at shutdown. Set `OMNIVOX_ENGINE`
and `OMNIVOX_REMOTE_TEST_EXPECT_ENGINE` to `dectalk` to verify the realized
engine, and opt into `OMNIVOX_REMOTE_TEST_AUDIO_OUTPUT=device` for an audible
device check. The normal test default is null output.

Development acceptance on 2026-09-05 passed Linux service tests, Linux Emacs
against Windows DECtalk (both lanes, Unicode, icons, markers, cancellation,
heartbeat expiry, and fresh reconnection), and one real-device DECtalk
completion check. The Windows development runtime is `98084bb159c06059`.
On 2026-09-07, Emacs 31.1 on a separate Linux SSH host passed the real-forward
check below against Windows DECtalk (null and device output) and Linux eSpeak
(null output). Both channels recovered automatically after the owned SSH
tunnel was interrupted during pending speech. See the
[acceptance report](experiments/2026-09-07-remote-ssh.md) for exact development
payloads, timings, and remaining limits. Native macOS is still untested.

## Repeatable real SSH check

From Linux or WSL, use an existing trusted SSH alias with working key-based
authentication. The remote Linux host needs Python 3, `tar`, `ss`, and an
installed Emacs version supported by the Emacsvox checkout. No package
installation is performed. The check snapshots **committed Emacsvox HEAD**;
uncommitted edits and the remote host's installed Emacsvox are not exercised.

```sh
python3 tools/check_remote_ssh.py \
  --host emacs-host \
  --program "$PWD/target/release/omnivox" \
  --emacsvox ../emacsvox \
  --remote-emacs /absolute/path/to/emacs \
  --engine espeak \
  --report-dir target/remote-ssh-linux
```

Build the staged payload with `make build` first if necessary. For a Windows
workstation service launched from WSL, replace the program and engine options
and add Windows OpenSSH:

```sh
python3 tools/check_remote_ssh.py \
  --host emacs-host \
  --ssh /mnt/c/Windows/System32/OpenSSH/ssh.exe \
  --windows --program ../emacsvox/servers/omnivox \
  --emacsvox ../emacsvox \
  --remote-emacs /absolute/path/to/emacs \
  --engine dectalk \
  --audio-output device \
  --report-dir target/remote-ssh-windows-device
```

The default output is silent (`null`). Explicit `device` output speaks four
short announcements. Confirm hearing both the initial and recovered foreground
and notification announcements; completion events alone cannot establish
audibility. Each report directory must be new.

The check creates a private temporary token and remote source snapshot, starts
its own loopback service and SSH reverse forward, and checks the forward's
actual bind address. It verifies both lanes' inventory, voice registration,
routing, realized engine, marked completion, and a 22-second idle heartbeat.
It then interrupts **only its own tunnel**, checks that both pending requests
fail once, restores the same forwarded port, and waits for automatic client
reconnection and fresh speech. It does not restart Emacs to obtain recovery.

Owned processes, tokens, and the remote snapshot are removed on completion.
The remote test supervisor also retires its Emacs on client-connection EOF or
a 180-second deadline. Cleanup failures make the check fail. Existing services,
SSH tunnels, installed checkouts, and configuration files remain in place.
Test reports redact the temporary token; logs still contain local paths and
machine details. Local supervisor failure tests require no SSH host:
`make remote-ssh-harness-test`.

## Setup and recovery troubleshooting

| Symptom | Check or action |
| --- | --- |
| SSH fails before the check starts | Establish host trust and key authentication using the selected SSH client. The check uses batch mode and strict host checking, so it cannot answer login or host-key prompts. |
| Forwarding is reported ready but Emacs cannot connect | Confirm the workstation service is still listening and the SSH client runs on the same side of WSL's loopback boundary. `ExitOnForwardFailure` checks listener creation; it does not prove the destination service is reachable. See [OpenSSH's option documentation](https://man.openbsd.org/ssh_config#ExitOnForwardFailure). |
| Authentication or token-file error | Use matching private token copies and the intended port. Unix token files must have no group or other permissions. Never paste the token into commands or diagnostics. |
| `busy` after a dropped connection | One session owns the two lanes. Allow the old workers to retire; check for another connected Emacs before starting a new session. |
| Speech stops after losing SSH | Restore the reverse forward on the same port. Emacs retries automatically, with backoff up to 30 seconds, and fails interrupted speech instead of replaying it. After an explicit `omnivox-remote-disconnect`, use `omnivox-remote-connect` to resume retries. |
| Speech completes but nothing is heard | Check that the service uses `device` output, the intended workstation output is audible, and foreground/notification channel routing matches the speakers or headphones. A null-output check is intentionally silent. |

The [wire contract](protocols/REMOTE-PROTOCOL.md) specifies framing, limits,
authentication, resource rules, and lifecycle behavior.
