# Omnivox

Cross-platform Emacspeak speech server written in Rust. A drop-in replacement for SwiftMac with support for macOS, Linux, and Windows.

## Features

- **Cross-platform TTS**: macOS (AVSpeechSynthesizer), Windows (WinRT SpeechSynthesizer), Linux (espeak-ng), with espeak-ng as universal fallback
- **Audio pipeline**: All audio goes through a configurable effects pipeline (silence trimming, volume control, channel routing)
- **Concurrent streams**: Speech, tones, and audio icons play on independent streams with backlog limits (no blocking between stream types)
- **Tone generation**: Pure-Rust sine wave generator with fade envelopes
- **Audio icon playback**: OGG Vorbis and WAV file loading with caching
- **Full Emacspeak protocol**: Command parsing, queue dispatch, voice switching, state management
- **Engine fallback**: Tries platform-native TTS first, falls back to espeak-ng
- **Portable voice foundation**: Structured engine/voice descriptors and a tested late-binding resolver support exact choices, property selectors, and deterministic degradation; runtime routing is the next phase
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

# Run tests (192 tests)
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

Only two environment variables are recognized (for Emacspeak integration):

- **OMNIVOX_ENGINE**: Set to `espeak` to force espeak-ng on platforms with native TTS
- **OMNIVOX_AUDIO_TARGET**: Set to `left`, `right`, or `both` for channel routing (used by Emacspeak for dual-server notification mode)

All other settings use CLI flags (terminal) or protocol commands via Emacs defcustoms.

## Architecture

The project is organized as a Cargo workspace:

- **omnivox-core** - Command parsing, queue management, state types
- **omnivox-tts** - TTS engine trait and backends (macOS AVSpeechSynthesizer, Windows WinRT, espeak-ng)
- **omnivox-audio** - Audio buffer, effects pipeline, tone generator, file loader, playback
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
| `t freq dur` | Queue tone |
| `a path` | Queue audio icon |
| `p path` | Play sound immediately |
| `sh dur` | Queue silence |
| `tts_say {text}` | Speak immediately |
| `tts_set_speech_rate N` | Set speech rate |
| `tts_set_voice name` | Set voice |
| `tts_set_pitch_multiplier N` | Set pitch multiplier |
| `tts_set_voice_volume N` | Set voice volume |
| `tts_set_tone_volume N` | Set tone volume |
| `tts_set_sound_volume N` | Set sound volume |
| `tts_sync_state punct split_caps allcaps_beep rate` | Sync state |
| `tts_reset` | Reset all state |
| `tts_exit` | Shut down |
| `version` | Speak version |

## License

MIT
