# Omnivox Configuration

Omnivox is configured through CLI flags (for terminal use) and Emacs
defcustoms with protocol commands (for Emacspeak use).

## CLI Flags

```
omnivox [OPTIONS]

OPTIONS:
    --help                Show help message
    --version             Show version number
    --check               Run diagnostic self-test
    --list-voices         List available TTS voices
    --list-voices-alist   List voices as Emacs-readable alist
    --engine NAME         TTS engine: native, espeak
    --voice ID            Default voice (e.g. en-US:Alex)
    --rate FLOAT          Speech rate 0.0-1.0 (0.5 = normal)
    --pitch FLOAT         Pitch multiplier 0.5-2.0 (1.0 = normal)
    --voice-volume F      Voice volume 0.0-1.0
    --tone-volume F       Tone volume 0.0-1.0
    --sound-volume F      Sound/icon volume 0.0-1.0
    --audio-target T      Channel routing: left, right, both
```

Without options, starts the Emacspeak protocol server on stdin.

## Environment Variables

Omnivox recognizes these environment variables:

**OMNIVOX_LOG_DIRECTORY** (Emacsvox WSL launcher only)

- Linux directory used for one persistent stderr log per OmniVox launch
- Default: `$XDG_STATE_HOME/emacsvox/omnivox`, falling back to
  `~/.local/state/emacsvox/omnivox`
- Logs contain request metadata and failure details but not synthesized text
- See [`docs/DIAGNOSTICS.md`](docs/DIAGNOSTICS.md)

**OMNIVOX_ENGINE**

- Values: `espeak`, `native`, or empty
- Default: empty (use platform-native TTS)
- Forces espeak-ng engine on platforms with native TTS (macOS/Windows)
- Equivalent to `--engine`

**OMNIVOX_AUDIO_TARGET**

- Values: `left`, `right`, `both`, or empty
- Default: empty (both channels)
- Controls speech, tone, and sound routing for all output owned by that process
- Used by Emacspeak for dual-server notification mode
- Normally set automatically by Emacspeak, not manually
- Equivalent to `--audio-target`

**OMNIVOX_ELOQUENCE_HELPER** (Windows, optional)

- Path to `OmnivoxEloquenceHelper32.exe`
- Overrides automatic discovery beside `omnivox.exe`
- A missing or failed helper is omitted from inventory; WinRT and eSpeak remain
  available according to normal fallback policy

**OMNIVOX_ECI_DLL** (inherited by the Eloquence helper, optional)

- Path to the user-supplied 32-bit `ECI.DLL`
- Overrides the helper's normal Freedom Scientific installation path
- Omnivox and Emacsvox do not distribute the proprietary runtime

**OMNIVOX_DECTALK_HELPER** (Windows, optional)

- Path to `OmnivoxDectalkHelper32.exe`
- Overrides automatic discovery beside `omnivox.exe`
- A missing or failed helper is omitted independently of other engines

**OMNIVOX_DECTALK_DLL** (inherited by the DECtalk helper, optional)

- Path to the user-supplied 32-bit `DECtalk.dll`
- `dtalk_us.dic` must be in the same directory
- Overrides discovery beside the helper and its development runtime directory

## Emacs Customization

All settings are in the `omnivox` customization group:

```
M-x customize-group RET omnivox RET
```

Settings are sent to the running omnivox process via protocol commands.
Changes take effect immediately.

| Defcustom | Default | Description |
|-----------|---------|-------------|
| `omnivox-default-voice-id` | "" | Voice ID (e.g. "en-US:Alex") |
| `omnivox-default-speech-rate` | 0.6 | Speech rate (0.0-1.0) |
| `omnivox-default-pitch` | 1.0 | Pitch multiplier (0.5-2.0) |
| `omnivox-default-voice-volume` | 1.0 | Voice volume (0.0-1.0) |
| `omnivox-default-tone-volume` | 0.1 | Tone/beep volume (0.0-1.0) |
| `omnivox-default-sound-volume` | 0.5 | Audio icon volume (0.0-1.0) |

### Interactive Commands

| Command | Description |
|---------|-------------|
| `omnivox-select-voice` | Choose voice with completion from server's list |
| `omnivox-set-rate` | Set speech rate (0.0-1.0) |
| `omnivox-set-pitch` | Set pitch multiplier (0.5-2.0) |
| `omnivox-set-voice-volume` | Set voice volume (0.0-1.0) |
| `omnivox-set-tone-volume` | Set tone volume (0.0-1.0) |
| `omnivox-set-sound-volume` | Set sound/icon volume (0.0-1.0) |
| `omnivox-list-voices` | Display all available voices in a buffer |
| `omnivox-refresh-voices` | Re-query voices from the server |
| `omnivox-status` | Show current settings |

### Minimal init.el Example

```elisp
;; Load omnivox voice module (before emacspeak)
(add-to-list 'load-path "/path/to/omnivox/elisp")
(require 'omnivox-voices)
;; Override only what you need:
(setq omnivox-default-voice-id "en-US:Alex")
(setq omnivox-default-speech-rate 0.6)

;; Load emacspeak
(setq dtk-program "omnivox")
(require 'emacspeak-setup)
```

## Dual-Server Notification Mode

When `tts-notification-device` is set to `left` (or `right`), Emacspeak
spawns two omnivox processes:

1. **Main process** - Both channels, handles primary speech
2. **Notification process** - Left channel only, handles notifications

This allows notifications to play in one ear while main content continues.
Emacspeak sets `OMNIVOX_AUDIO_TARGET` automatically for the notification
process.

Omnivox has no hidden notification stream inside either process.  The legacy
`tts_set_notification_channel` command is parsed during migration but returns
`unsupported_operation`; change the second process's environment instead.

## Technical Details

CLI flags are parsed in `omnivox-cli/src/main.rs` via `parse_args()` and
applied to the TTS state via `apply_cli_flags()`.  The `ChannelRouter`
audio effect handles channel routing, and volumes are applied in the audio
pipeline.  Emacs sends runtime changes via protocol commands (e.g.
`tts_set_speech_rate`, `tts_set_voice_volume`).
