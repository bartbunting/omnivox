# Omnivox Configuration

The command line configures a standalone server process. Runtime protocol
commands configure a process already launched by Emacs. Emacsvox and upstream
Emacspeak use different Lisp adapters; their variable names are documented
separately below.

## Command-line options

`omnivox --help` is authoritative. The current options are:

| Option | Meaning |
|---|---|
| `--help`, `-h` | Show help. |
| `--version`, `-V` | Print the workspace version. |
| `--check` | Run the diagnostic self-test. |
| `--list-voices` | Print voices for the selected startup engine. |
| `--list-voices-alist` | Print the same list as Emacs-readable data. |
| `--engine NAME` | Select `native`, `espeak`, or opt-in `piper`. |
| `--voice ID` | Set the startup physical voice. |
| `--rate FLOAT` | Set normalized startup rate from 0.0 through 1.0. |
| `--pitch FLOAT` | Set pitch multiplier from 0.5 through 2.0. |
| `--voice-volume FLOAT` | Set speech gain from 0.0 through 1.0. |
| `--tone-volume FLOAT` | Set tone gain from 0.0 through 1.0. |
| `--sound-volume FLOAT` | Set sound/icon gain from 0.0 through 1.0. |
| `--audio-target TARGET` | Route to `left`, `right`, or `both`. |
| `--piper-model PATH` | Supply a Piper `.onnx` model. |
| `--dump-wav VOICE OUTPUT [TEXT]` | Synthesize a diagnostic WAV. |
| `--play-wav FILE` | Play a WAV through the Omnivox audio path. |

Without an action option, Omnivox starts the stdin protocol server. Protocol
rate commands use Emacspeak's integer scale; Omnivox normalizes values greater
than 1 by dividing by 100. Higher values produce a faster requested rate.

## Server environment

### Engine selection

`OMNIVOX_ENGINE`

- `native` or empty selects the platform default (`macos`, `winrt`, or eSpeak
  where no native engine exists).
- `espeak` makes eSpeak NG preferred without removing other registered engines.
- `piper` selects the optional helper-backed Piper engine and requires a
  Piper-enabled build plus a model.
- Equivalent startup option: `--engine`.

`OMNIVOX_PIPER_MODEL`

- Path to a Piper `.onnx` model.
- Overridden by `--piper-model`.

`OMNIVOX_PIPER_HELPER`

- Optional path to `omnivox-piper-helper`.
- Otherwise the server looks beside its own executable.

`OMNIVOX_PIPER_ESPEAK_DATA`

- Optional Piper-specific path containing `espeak-ng-data/`.
- Used only by the in-process Piper implementation; ordinary packaged use
  normally relies on the helper and its own discovery.

### Audio routing

`OMNIVOX_AUDIO_TARGET`

- `left`, `right`, `both`, or empty; empty means both channels.
- Applies to every output stream owned by that process.
- Equivalent startup option: `--audio-target`.
- Notification isolation uses a second process with its own value; Omnivox has
  no hidden notification stream inside one process.

### Diagnostics

`OMNIVOX_LOG_SYNTHESIS_TEXT`

- Values `1`, `true`, `yes`, or `on`, ignoring case and surrounding whitespace,
  opt in to full synthesis-text logging.
- Disabled by default. Text may contain passwords, messages, documents, and
  other private content.

`OMNIVOX_LOG_DIRECTORY` (Emacsvox WSL launcher only)

- Linux directory for one private stderr log per launch.
- Defaults to `$XDG_STATE_HOME/emacsvox/omnivox`, or
  `~/.local/state/emacsvox/omnivox` when `XDG_STATE_HOME` is unset.
- The Rust process does not read this variable; the launcher redirects stderr.

See [docs/DIAGNOSTICS.md](docs/DIAGNOSTICS.md) for collection and privacy
guidance.

### Optional Windows helpers

`OMNIVOX_ELOQUENCE_HELPER`

- Optional path to `OmnivoxEloquenceHelper32.exe`.
- Otherwise Omnivox looks beside its executable.

`OMNIVOX_ECI_DLL`

- Optional path passed to the Eloquence helper for a user-supplied 32-bit
  `ECI.DLL`.

`OMNIVOX_DECTALK_HELPER`

- Optional path to `OmnivoxDectalkHelper32.exe`.
- Otherwise Omnivox looks beside its executable.

`OMNIVOX_DECTALK_DLL`

- Optional path to user-supplied `DECtalk.dll`.
- `dtalk_us.dic` must be in the same directory.

A missing helper or runtime removes only that engine from usable inventory;
normal fallback remains available. Proprietary runtimes are not distributed
by Omnivox or Emacsvox.

`ESPEAK_NG_DATA` is also forwarded by the Emacsvox WSL launcher when a staged
Windows runtime supplies its content-addressed eSpeak data directory. Normal
source builds use the backend's standard data discovery.

## Emacsvox adapter

Emacsvox ships its own adapter in `lisp/omnivox-voices.el`. Common settings
include:

| Variable | Default | Meaning |
|---|---:|---|
| `omnivox-default-speech-rate` | `60` | Initial integer rate on the 0--100 scale. |
| `omnivox-default-voice-id` | `""` | Empty means the preferred engine default. |
| `omnivox-engine-priority-ids` | `nil` | Explicit preferred engine order. |
| `omnivox-fallback-engine-ids` | `("espeak")` | Global fallback engine order. |
| `omnivox-disabled-engine-ids` | `nil` | Engines disabled by runtime policy. |
| `omnivox-logical-voice-preferences` | `nil` | Advanced portable/exact selector input. |

Prefer `M-x emacsvox-aural-voice-workbench` for route and voice configuration;
it keeps portable palette data separate from machine-local exact identifiers.
Select Omnivox with `M-x omnivox`.

## Upstream Emacspeak adapter

This repository's `elisp/omnivox-voices.el` is a separate compatibility module
for upstream Emacspeak. Its common customizations are:

| Variable | Default | Meaning |
|---|---:|---|
| `omnivox-speech-rate` | `60` | Initial integer rate on the 0--100 scale. |
| `omnivox-voice-id` | `""` | Empty means the engine default. |
| `omnivox-pitch` | `1.0` | Pitch multiplier. |
| `omnivox-voice-volume` | `1.0` | Speech gain. |
| `omnivox-tone-volume` | `0.1` | Tone gain. |
| `omnivox-sound-volume` | `0.5` | Sound/icon gain. |
| `omnivox-notification-channel` | `"left"` | Target for the separate notification process. |

Example:

```elisp
(add-to-list 'load-path "/path/to/omnivox/elisp")
(require 'omnivox-voices)
(setq omnivox-speech-rate 60
      omnivox-voice-id ""
      dtk-program "omnivox")
(require 'emacspeak-setup)
```

Use `omnivox-set-rate`, `omnivox-select-voice`, and the volume/pitch commands
for live changes. The `dtk-*` names in this example belong to upstream
Emacspeak and are intentionally not Emacsvox configuration names.

## Ownership details

CLI parsing lives in `omnivox-cli/src/cli.rs`. Process-wide channel selection
is applied during engine/audio initialization. Runtime state commands are
handled by `omnivox-cli/src/server.rs`. Logical voices and routing policy are
snapshotted at dispatch so later configuration changes affect later work only.

The deprecated `tts_set_notification_channel` command returns an explicit
unsupported-operation response; start a separately targeted process instead.
Legacy global language commands are likewise unsupported because language is a
property of each logical voice rather than process-global mutable state.
