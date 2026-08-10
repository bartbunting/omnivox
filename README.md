# Omnivox

Cross-platform Emacspeak speech server written in Rust. A drop-in replacement for SwiftMac with support for macOS, Linux, and Windows.

## Features

- **Cross-platform TTS**: macOS (AVSpeechSynthesizer), Windows (WinRT SpeechSynthesizer), Linux (espeak-ng), with espeak-ng as universal fallback
- **Audio pipeline**: All audio goes through a configurable effects pipeline (silence trimming, volume control, channel routing)
- **Concurrent streams**: Speech, tones, and audio icons play on independent streams with backlog limits (no blocking between stream types)
- **Tone generation**: Pure-Rust sine wave generator with fade envelopes
- **Capitalization cues**: Requested speech anchors place overlaid capital and all-caps tones, with deterministic degradation for engines lacking exact markers
- **Audio icon playback**: Bounded OGG/WAV loading with decoded LRU caching
- **Timeline mixing**: Serial insertion and sample-aligned overlays render in bounded chunks with cross-chunk tails
- **Playback events**: Engine markers and opaque semantic actions follow mixed-audio frame positions and cancellation
- **Emacspeak presentation protocol**: Command parsing, queue dispatch, voice switching, and state management; deprecated global-language and single-process notification commands return explicit unsupported errors
- **Engine fallback**: Tries platform-native TTS first, falls back to espeak-ng
- **Portable multi-engine voices**: Structured descriptors and late-bound logical voices route queued spans to engine/voice pairs with deterministic degradation, persistent engine health, and bounded same-chunk runtime retry
- **Lossless text routing**: Per-engine repertoire metadata keeps Unicode text and source anchors intact by selecting a capable fallback before synthesis
- **Classic voice dimensions**: Helper protocol v3 maps pitch range, stress, and richness to native Eloquence and DECtalk controls
- **Policy-preserving presentations**: Capability-gated timeline V2 frames retain ordered, urgent, and keyed-replaceable delivery semantics behind explicit stop barriers
- **Tracked playback**: Capability-gated dispatch reports completed, cancelled, or failed only after its queued audio reaches a terminal state
- **Failure diagnostics**: Persistent privacy-safe session logs correlate the synthesis worker, routing, and native helpers; optional WER dumps capture native Windows crashes
- **Self-registering Emacs module**: `omnivox-voices.el` hooks into emacspeak via advice -- no need to modify emacspeak files

## Prerequisites

### All Platforms

- [Rust toolchain](https://rustup.rs/) (1.70+)
- C compiler (for espeak-ng build)
- CMake (for espeak-ng build)

### macOS

No additional dependencies. AVSpeechSynthesizer is built in, and espeak-ng is compiled from source automatically.

**Recommended Emacs build**: [emacs-plus](https://github.com/d12frosted/homebrew-emacs-plus) via Homebrew provides a well-maintained macOS-native Emacs with accessibility support:

```bash
brew tap d12frosted/emacs-plus
brew install emacs-plus --with-native-comp
```

### Linux

Install espeak-ng development headers (used at build time):

```bash
# Debian/Ubuntu
sudo apt install espeak-ng-dev cmake

# Fedora
sudo dnf install espeak-ng-devel cmake

# Arch
sudo pacman -S espeak-ng cmake
```

### Windows

Install CMake and a C compiler (MSVC or MinGW). espeak-ng is compiled from source.

## Building

```bash
# Build release binary
make build

# Build debug binary
make dev

# Run tests (326 tests, including one documentation test)
make test

# Run clippy lints
make lint

# Format code
make fmt
```

## Installation

```bash
make install
```

This installs the `omnivox` binary to `~/.cargo/bin/`.

## Emacspeak Integration

### Setup (macOS / Linux)

1. Build and install omnivox:

   ```bash
   cd /path/to/omnivox
   make build
   make install
   ```

2. Ensure `~/.cargo/bin` is in your PATH.

3. **Register omnivox with Emacspeak** (required for audio icon support):

   ```bash
   # Create symlink in emacspeak servers directory
   ln -sf ~/.cargo/bin/omnivox ~/.emacspeak/servers/omnivox
   ln -sf ~/.cargo/bin/omnivox ~/.emacspeak/servers/log-omnivox

   # Add omnivox to .servers file (tells Emacspeak it handles audio natively)
   echo "omnivox" >> ~/.emacspeak/servers/.servers
   echo "log-omnivox" >> ~/.emacspeak/servers/.servers
   ```

   **Why this matters**: Without being in `.servers`, Emacspeak uses external `play` commands for audio icons, bypassing omnivox's volume control entirely.

4. Add to your Emacs init.el (**before** emacspeak loads):

   ```elisp
   ;; Load omnivox voice module (self-registering, no emacspeak file edits needed)
   (add-to-list 'load-path "/path/to/omnivox/elisp")
   (require 'omnivox-voices)

   ;; Override defaults as needed (M-x customize-group RET omnivox RET for all options)
   (setq omnivox-default-voice-id "en-US:Alex")
   (setq omnivox-default-speech-rate 60)  ;; 0-100, 50 = normal (use dtk-set-rate to adjust at runtime)
   (setq omnivox-default-voice-volume 1.0)
   (setq omnivox-default-tone-volume 0.1)
   (setq omnivox-default-sound-volume 0.5)

   ;; macOS: prevent accessibility permission prompts that interfere with TTS
   (when (eq system-type 'darwin)
     (setq mac-ignore-accessibility t))

   ;; Then load emacspeak with omnivox as the TTS server
   (setq dtk-program "omnivox")
   (require 'emacspeak-setup)
   ```

   **Note on `mac-ignore-accessibility`**: On macOS, Emacs may request accessibility permissions which can interfere with omnivox's TTS output. Setting `mac-ignore-accessibility` to `t` prevents this. This is specific to emacs-plus and NS-port builds of Emacs.

5. Start Emacspeak as usual. Omnivox will be used as the speech server.

### Setup (Windows)

Windows requires extra steps because Emacspeak looks for speech servers in its `servers/` directory and Emacs may resolve `~` to `%APPDATA%` rather than `%USERPROFILE%`.

1. **Set HOME environment variable** so Emacs finds your config:

   ```powershell
   setx HOME C:\Users\YourUsername
   ```

2. **Build and install omnivox** (requires LIBCLANG_PATH for espeak-ng build):

   ```bash
   export LIBCLANG_PATH="C:\\LLVM\\bin"
   cd /path/to/omnivox
   cargo build --release
   cargo install --path omnivox-cli
   ```

3. **Copy omnivox into Emacspeak's servers directory** (symlinks require admin on Windows):

   ```bash
   cp ~/.cargo/bin/omnivox.exe ~/.emacspeak/servers/omnivox.exe
   ```

   You must re-copy after rebuilding omnivox.

4. **Generate emacspeak-loaddefs.el** if it doesn't exist (required on fresh clones):

   ```bash
   cd ~/.emacspeak/lisp
   emacs --batch -l ./emacspeak-preamble.el -l ./emacspeak-autoload.el \
     -f emacspeak-auto-generate-autoloads
   ```

5. **Configure Emacs** (same as macOS/Linux, see step 4 above).

6. Start Emacspeak. The default speech rate on Windows may be fast; adjust via `omnivox-set-rate` or customize `omnivox-default-speech-rate`.

### Testing

After configuration, start Emacs with Emacspeak. You should hear "Omnivox version 1 dot 0 dot 0" on startup. If not, check:

```bash
# Verify the binary runs
omnivox <<< "version"

# List available voices
omnivox --list-voices

# Run diagnostic self-test
omnivox --check
```

### Troubleshooting

- **No speech output**: Ensure your audio device is working. Try `omnivox --check` to verify TTS engine initialization.
- **macOS: Speech interrupted or accessibility prompts**: Add `(setq mac-ignore-accessibility t)` to your init.el before emacspeak loads. This prevents macOS accessibility permission dialogs from interfering with TTS.
- **espeak-ng errors on Linux**: Install `espeak-ng` and `espeak-ng-data` packages.
- **Slow startup**: First run compiles espeak-ng data; subsequent starts are faster.
- **Wrong voice**: Use `omnivox-select-voice` in Emacs to interactively pick a voice.
- **Windows: "Cannot open load file: emacspeak-loaddefs"**: Run the `emacs --batch` command in step 4 of the Windows setup to generate the loaddefs file.
- **Windows: Emacs not loading init.el**: Check `M-: (expand-file-name "~")` in Emacs. If it points to `%APPDATA%` instead of your home directory, set the `HOME` environment variable (step 1 of Windows setup).
- **Windows: Speech too fast**: Set a lower rate via `omnivox-set-rate` or customize `omnivox-default-speech-rate`.

### Known Issues

- **Text containing `;;`**: Parser regression tests confirm that Omnivox preserves
  semicolons and the text following them. If speech is still audibly truncated,
  capture it with `--dump-wav` and investigate preprocessing or the selected TTS
  backend rather than assuming the command parser removed the text.

## Roadmap

See [NEXT_STEPS.md](NEXT_STEPS.md) for the multi-engine voice architecture,
fallback contract, Emacsvox protocol work, new engine plans, and consolidated
project backlog.

The versioned Base64-JSON discovery and configuration transport is specified in
[CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md). Legacy speech commands remain
unchanged.

Structured mixed-engine Aural presentation, including inserted and overlaid
audio, semantic events, persistent effects, and marker-v2 degradation records,
is specified in
[PRESENTATION-TIMELINE-PROTOCOL.md](PRESENTATION-TIMELINE-PROTOCOL.md).

## Configuration

See [ENV-VARS.md](ENV-VARS.md) for full CLI flags, environment variables, and Emacs customization reference.

### CLI Flags

```bash
omnivox --voice "en-US:Alex" --rate 0.6 --pitch 1.0
omnivox --voice-volume 1.0 --tone-volume 0.1 --sound-volume 0.5
omnivox --engine espeak        # Force espeak-ng
omnivox --audio-target left    # Left channel only
omnivox --list-voices          # List available voices
omnivox --check                # Diagnostic self-test
```

### Emacs Customization

All settings are in the `omnivox` customization group:

```
M-x customize-group RET omnivox RET
```

Interactive commands:

| Command | Description |
|---------|-------------|
| `dtk-set-rate` | Set speech rate (standard Emacspeak command) |
| `omnivox-select-voice` | Choose voice with completion from server's list |
| `omnivox-set-pitch` | Set pitch multiplier (0.5-2.0) |
| `omnivox-set-voice-volume` | Set voice volume (0.0-1.0) |
| `omnivox-set-tone-volume` | Set tone volume (0.0-1.0) |
| `omnivox-set-sound-volume` | Set sound/icon volume (0.0-1.0) |
| `omnivox-list-voices` | Display all available voices in a buffer |
| `omnivox-refresh-voices` | Re-query voices from the server |
| `omnivox-status` | Show current settings |

### Environment Variables

Relevant environment variables are:

- **OMNIVOX_ENGINE**: Set to `espeak` to force espeak-ng on platforms with native TTS
- **OMNIVOX_AUDIO_TARGET**: Set to `left`, `right`, or `both` for channel routing (used by Emacspeak for dual-server notification mode)
- **OMNIVOX_ELOQUENCE_HELPER**: Optional Windows path to `OmnivoxEloquenceHelper32.exe`; otherwise Omnivox looks beside its own executable
- **OMNIVOX_ECI_DLL**: Optional path passed through to the Eloquence helper for a user-supplied 32-bit `ECI.DLL`
- **OMNIVOX_DECTALK_HELPER**: Optional Windows path to `OmnivoxDectalkHelper32.exe`; otherwise Omnivox looks beside its own executable
- **OMNIVOX_DECTALK_DLL**: Optional path to `DECtalk.dll`; `dtalk_us.dic` must be beside it

When present, the helpers add engine `eloquence` with native voice IDs `v1`
through `v8`, and engine `dectalk` with its named voices such as `paul` and
`betty`, to structured inventory. WinRT remains the preferred legacy engine;
logical voices may select either helper per span and retain their ordered
fallbacks when a helper, runtime, or requested voice is unavailable.
Proprietary speech runtimes are not distributed with Omnivox.

All other settings use CLI flags (terminal) or protocol commands via Emacs defcustoms.

### Language and notification ownership

Emacsvox logical voices own language selection.  Give a logical voice a BCP 47
language and ordered physical selectors; Omnivox resolves that language for
each queued span.  The legacy global commands `set_lang`, `set_next_lang`,
`set_previous_lang`, and `set_preferred_lang` remain parseable for one
migration cycle but return a machine-readable `unsupported_operation` error.

Notification separation is process-based.  With one process, notifications
share its queue owner and channel route.  To isolate them, start a second
Omnivox process with `OMNIVOX_AUDIO_TARGET=left`, `right`, or `both`.
`tts_set_notification_channel` is likewise retained only as an explicitly
unsupported migration command.

## Architecture

The project is organized as a Cargo workspace:

- **omnivox-core** - Command parsing, queue/state types, and pure presentation timeline projection
- **omnivox-tts** - TTS engine trait and backends (macOS AVSpeechSynthesizer, Windows WinRT, espeak-ng)
- **omnivox-audio** - The single canonical stereo 44.1 kHz `AudioBuffer`, effects pipeline, tone generator, file loader, and playback
- **omnivox-cli** - Main binary wiring everything together
- **elisp/** - Emacs voice module (`omnivox-voices.el`)

### Audio Pipeline

All audio flows through the pipeline:

```
Source (TTS / Tone / File) -> AudioBuffer -> Pipeline -> AudioOutput
                                               |
                                    SilenceTrimmer (speech only)
                                    VolumeAdjust
                                    ChannelRouter
```

## Emacspeak Protocol

Omnivox implements the standard Emacspeak speech server protocol:

| Command | Description |
|---------|-------------|
| `q {text}` | Queue speech |
| `c {codes}` | Queue voice/pitch codes |
| `d` | Dispatch queued items |
| `s` | Stop all speech |
| `l char` | Speak letter immediately |
| `t freq dur` | Queue an independent tone |
| `emacsvox_tone 1 MODE freq dur` | Queue a capability-negotiated `insert` or `overlay` presentation tone |
| `a path` | Queue an icon overlay at the current presentation boundary |
| `p path` | Play sound immediately |
| `sh dur` | Queue silence |
| `tts_say {text}` | Speak immediately |
| `tts_set_speech_rate N` | Set speech rate |
| `tts_set_voice name` | Set voice |
| `tts_set_pitch_multiplier N` | Set pitch multiplier |
| `tts_set_voice_volume N` | Set voice volume |
| `tts_set_tone_volume N` | Set tone volume |
| `tts_set_sound_volume N` | Set sound volume |
| `tts_set_capitalization_presentation MODE` | Select `none`, `spoken`, `tone`, `spoken-tone`, or `custom` for isolated capitals |
| `tts_sync_state punct split_caps legacy_caps rate` | Sync state (`legacy_caps` is retained as an ignored placeholder) |
| `tts_reset` | Reset all state |
| `tts_exit` | Shut down |
| `version` | Speak version |

Presentation tone gain is applied once in every delivery path. This corrects
older legacy behavior that effectively squared `tts_set_tone_volume`; users
who raised tone volume to compensate may want to restore their intended value.

## License

MIT
