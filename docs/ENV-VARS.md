# Omnivox Configuration

The command line configures a standalone server process. Runtime protocol
commands configure a process already launched by Emacs. Emacsvox and upstream
Emacspeak use different Lisp adapters; their variable names are documented
separately below.

## Command-line options

`omnivox --help` is the authoritative syntax summary. The supported value
ranges below are the values callers should supply; the parser rejects malformed
numbers but does not currently reject every out-of-range value before it reaches
the selected backend.

| Option | Meaning |
|---|---|
| `--help`, `-h` | Show help. |
| `--version`, `-V` | Print the workspace version. |
| `--check` | Run the diagnostic self-test; inspect each printed status and confirm that its tone and speech are audible. |
| `--list-voices` | Print voices for the selected startup engine. |
| `--list-voices-alist` | Print the same list as Emacs-readable data. |
| `--engine NAME` | Select `native`, `espeak`, `piper`, `rhvoice`, `flite`, or `rutts`. |
| `--voice ID` | Set the startup physical voice; copy an exact ID from `--list-voices`. |
| `--rate FLOAT` | Set normalized startup rate from 0.0 through 2.0; 0.5 is normal. |
| `--pitch FLOAT` | Set pitch multiplier from 0.5 through 2.0. |
| `--voice-volume FLOAT` | Set speech gain from 0.0 through 1.0. |
| `--tone-volume FLOAT` | Set tone gain from 0.0 through 1.0. |
| `--sound-volume FLOAT` | Set sound/icon gain from 0.0 through 1.0. |
| `--audio-target TARGET` | Route to `left`, `right`, or `both`. |
| `--piper-model PATH` | Supply a Piper `.onnx` model for the server or voice-list actions. |
| `--dump-wav VOICE OUTPUT [TEXT]` | Synthesize a canonical diagnostic WAV and a raw intermediate WAV. |
| `--play-wav FILE` | Play a WAV through the Omnivox audio path. |

Without an action option, Omnivox starts the stdin protocol server. Protocol
rate commands conventionally use Emacspeak's integer scale. Omnivox divides
values greater than 1 by 100 and then clamps the normalized value to `0.0..2.0`
(`50` becomes `0.5`, `150` becomes `1.5`, and `300` becomes `2.0`). Higher
values request faster speech; individual engines may impose a lower maximum.
An invalid `--audio-target` is logged and leaves the default `both` routing in
place. Unsupported `--engine` names currently select the platform default; use
only the documented names rather than relying on that fallback.

`--check` exits nonzero when it cannot create an engine. Synthesis and audio
device failures discovered later in the check are printed as `FAILED` but do
not currently change its exit status, so successful process exit alone is not
a complete pass. `--dump-wav` writes `OUTPUT` after canonical conversion and a
second path formed by replacing `.wav` with `_raw.wav`; use an output filename
ending in `.wav` to keep those files distinct. The `--piper-model` option is not
currently forwarded to `--check` or `--dump-wav`; set `OMNIVOX_PIPER_MODEL` for
those two diagnostic actions.

## Server environment

### Engine selection

`OMNIVOX_ENGINE`

- `native` or empty selects the platform default (`macos`, `winrt`, or eSpeak
  where no native engine exists).
- `espeak` selects eSpeak NG as the startup engine.
- `piper` selects the optional helper-backed Piper engine and requires a
  Piper-enabled build plus a model.
- `rhvoice` selects the helper-backed, user-installed RHVoice runtime.
- `flite` selects the source-built Flite companion and its compiled-in SLT
  voice.
- `rutts` selects the source-built RuTTS companion and its built-in Russian
  voices.
- Equivalent startup option: `--engine`.

In server mode, the selected startup engine controls the initial preference;
it does not remove other available engines from inventory. Windows registers
WinRT and eSpeak plus adjacent or explicitly configured Eloquence and DECtalk
helpers. macOS registers AVSpeechSynthesizer and eSpeak, while Linux registers
eSpeak. Staged or explicitly configured RHVoice, Flite, and RuTTS companions
register on every desktop platform. A build with Piper support also registers
Piper when `OMNIVOX_PIPER_MODEL` or `--piper-model` supplies a model. Single-action
diagnostics such as `--list-voices` continue to create only the selected
engine. Eloquence and DECtalk remain runtime-routing inventory IDs rather than
accepted startup values.

### RHVoice helper and runtime

`OMNIVOX_RHVOICE_HELPER`

- Optional path to `omnivox-rhvoice-helper`.
- Otherwise Omnivox first checks the `rhvoice/` directory beside itself, then
  accepts the legacy layout with the helper directly beside it.

`OMNIVOX_RHVOICE_LIBRARY`

- Absolute path to the user-installed RHVoice C API library. It overrides
  restricted platform discovery and is required on Windows.

`OMNIVOX_RHVOICE_DATA`

- Optional absolute RHVoice data directory containing installed languages and
  voices.

`OMNIVOX_RHVOICE_CONFIG`

- Optional absolute RHVoice configuration directory.

`OMNIVOX_RHVOICE_RESOURCES`

- Optional platform-separated list of absolute additional language/voice
  resource directories.

See [RHVOICE.md](RHVOICE.md) for compatible versions, installation paths,
platform status, and verification.

`OMNIVOX_FLITE_HELPER`

- Optional path to `omnivox-flite-helper`.
- Otherwise Omnivox checks `flite/` beside itself and then the legacy adjacent
  location.

`OMNIVOX_FLITE_VOICES`

- Optional platform-separated list of absolute `.flitevox` file paths (`:` on
  Linux/macOS, `;` on Windows).
- Only English Clustergen voices compatible with Flite v2.2 can load in the
  SLT-only companion. Invalid entries degrade the engine but do not remove the
  built-in `cmu_us_slt` voice.

See [FLITE.md](FLITE.md) for installation, build, voice-file, verification,
and licensing details.

`OMNIVOX_RUTTS_HELPER`

- Optional path to `omnivox-rutts-helper`.
- Otherwise Omnivox checks `rutts/` beside itself and then the legacy adjacent
  location.

The source-build wrapper additionally accepts `OMNIVOX_RUTTS_INPUTS_DIR` as a
verified-cache override. Advanced direct Cargo builds use
`OMNIVOX_RUTTS_SOURCE_DIR` to name an already verified RuTTS v6.3.3 source
tree; this is a build input, not a server runtime setting.

See [RUTTS.md](RUTTS.md) for installation, source build, text repertoire,
pronunciation, verification, and licensing details.

`OMNIVOX_PIPER_MODEL`

- Path to a Piper `.onnx` model. A matching configuration must be adjacent as
  either `<model>.onnx.json` or `<model>.json`.
- Overridden by `--piper-model` for the server and voice-list actions. The
  current `--check` and `--dump-wav` actions use this environment variable
  instead of the option.

`OMNIVOX_PIPER_HELPER`

- Optional path to `omnivox-piper-helper`.
- Otherwise the server first looks in a `piper/` companion directory beside
  its own executable, then accepts the legacy layout with the helper directly
  beside it.

`OMNIVOX_PIPER_ESPEAK_DATA`

- Optional Piper-specific path to `espeak-ng-data/` or its parent directory.
- Read by the Piper engine inside `omnivox-piper-helper`; the main server
  inherits this environment into the child. The helper otherwise prefers the
  companion data beside itself before checking `ESPEAK_NG_DATA`, its build-time
  path, and compatible system data.

`ESPEAK_NG_DATA`

- Parent directory containing `espeak-ng-data/phontab`, not the
  `espeak-ng-data` directory itself.
- The eSpeak TTS backend checks this value first, then an `espeak-ng-data`
  directory beside the executable, its staged Cargo-profile path, and common
  system data locations. The Piper helper also accepts this parent-directory
  convention after checking `OMNIVOX_PIPER_ESPEAK_DATA`.
- Supported local builds and generic GitHub release archives package matching
  data beside the executable, so this variable is normally unnecessary for
  those layouts. Keep the packaged directory adjacent when relocating the
  binary.
- The Emacsvox WSL launcher forwards this value for its content-addressed
  staged Windows runtime.

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

The following variables belong to the Emacsvox launcher rather than the Rust
process:

`OMNIVOX_PROGRAM`

- Absolute path to the Omnivox executable the launcher should run.
- Takes precedence over the content-addressed Emacsvox runtime and `PATH`.

`OMNIVOX_LOG_DIRECTORY`

- Linux directory for private stderr logs.
- Defaults to `$XDG_STATE_HOME/emacsvox/omnivox`, or
  `~/.local/state/emacsvox/omnivox` when `XDG_STATE_HOME` is unset.
- A session normally uses numbered `omnivox-...-partNNNNNN.log` files. The
  launcher falls back to one unnumbered log if its rotation helper is missing.

`OMNIVOX_LOG_MAX_FILE_BYTES`

- Approximate per-part rotation threshold; defaults to 16 MiB (`16777216`).
- Rotation occurs between complete log lines, so one oversized line may exceed
  the threshold.

`OMNIVOX_LOG_RETAINED_FILES`

- File-count target used when pruning retained log parts; defaults to 16.

`OMNIVOX_LOG_RETAINED_BYTES`

- Aggregate-byte target used when pruning retained log parts; defaults to
  256 MiB (`268435456`).
- The active target of each live session is protected from pruning, so live
  files can temporarily exceed the retention limits.

Nonpositive or nonnumeric log limits revert to their defaults. The launcher
creates its log directory with mode `0700` where possible and log parts with
mode `0600`. The Rust process does not read these launcher-only variables.

See [DIAGNOSTICS.md](DIAGNOSTICS.md) for collection and privacy
guidance.

### Optional Windows helpers

`OMNIVOX_ELOQUENCE_HELPER`

- Optional path to `OmnivoxEloquenceHelper32.exe`.
- Otherwise Omnivox looks beside its executable.

`OMNIVOX_ECI_DLL`

- Optional absolute path inherited and read by the Eloquence helper for a
  complete licensed 32-bit ECI 6.1 installation's `ECI.DLL`.
- Defaults to
  `C:\Program Files (x86)\Freedom Scientific\Shared\Eloquence\6.1\ECI.DLL`.
- Keep the DLL with its matching installed ECI configuration, dictionary, and
  voice data.

`OMNIVOX_DECTALK_HELPER`

- Optional path to `OmnivoxDectalkHelper32.exe`.
- Otherwise Omnivox looks beside its executable.

`OMNIVOX_DECTALK_DLL`

- Optional absolute path inherited and read by the DECtalk helper for a
  user-supplied 32-bit `DECtalk.dll`.
- A matching `dtalk_us.dic` from the same build must be in the same directory.
- Without an override, the helper checks beside itself and in the sibling
  `runtime` directory.

A missing helper or runtime removes only that engine from usable inventory;
normal fallback remains available. Proprietary runtimes are not distributed
by Omnivox or Emacsvox. See the
[Windows helper guide](../windows-helpers/README.md#runtime-requirements-and-installation)
for acquisition, installation, architecture, dependency, and verification
details.

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
