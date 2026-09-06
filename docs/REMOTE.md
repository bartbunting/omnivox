# Remote workstation speech (preview)

This preview feature lets remote Emacs use speech engines and speakers on
your workstation. Published Omnivox 1.7.1 binaries do not have `--serve`.
Use a staged development payload (`make dev` on Linux/macOS, or Emacsvox's
`make windows-omnivox-dev` for Windows) and the matching Emacsvox remote client.

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
A separate SSH host and native macOS have not yet been exercised.

The [wire contract](protocols/REMOTE-PROTOCOL.md) specifies framing, limits,
authentication, resource rules, and lifecycle behavior.
