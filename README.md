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

## Prerequisites

### All Platforms

- [Rust toolchain](https://rustup.rs/) (1.70+)
- C compiler (for espeak-ng build)
- CMake (for espeak-ng build)

### macOS

No additional dependencies. AVSpeechSynthesizer is built in, and espeak-ng is compiled from source automatically.

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
# Build release binaries
make build

# Build debug binaries
make dev

# Run tests
make test

# Run clippy lints
make lint

# Format code
make fmt
```

The release build produces two binaries:

- `omnivox` - The speech server
- `list-voices` - Utility to list available TTS voices

## Installation

```bash
make install
```

This installs the `omnivox` and `list-voices` binaries to `~/.cargo/bin/`.

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

4. Add to your Emacs configuration:

   ```elisp
   ;; Set environment variables BEFORE loading emacspeak
   (when (eq system-type 'darwin)
     (setenv "OMNIVOX_VOICE_VOLUME" "1.0")
     (setenv "OMNIVOX_TONE_VOLUME" "0.1")
     (setenv "OMNIVOX_SOUND_VOLUME" "0.1"))

   ;; Configure omnivox as TTS server
   (setq dtk-program "omnivox")
   ```

5. Start Emacspeak as usual. Omnivox will be used as the speech server with proper volume control.

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

5. **Configure Emacs** with volume environment variables:

   ```elisp
   (setq dtk-program "omnivox")
   (setenv "OMNIVOX_VOICE_VOLUME" "1.0")
   (setenv "OMNIVOX_TONE_VOLUME" "0.1")
   (setenv "OMNIVOX_SOUND_VOLUME" "0.1")
   ```

6. Start Emacspeak. The default speech rate on Windows may be fast; use `tts_set_speech_rate` or `tts_sync_state` to adjust.

### Alternative: Place Binary in Emacspeak Servers Directory

On macOS/Linux, instead of relying on PATH, you can symlink or copy the binary into Emacspeak's servers directory:

```bash
ln -s ~/.cargo/bin/omnivox /path/to/emacspeak/servers/omnivox
```

Then set `dtk-program` to `"omnivox"` as above.

### Testing

After configuration, start Emacs with Emacspeak. You should hear "Omnivox version 0 dot 1 dot 0" on startup. If not, check:

```bash
# Verify the binary runs
omnivox <<< "version"

# List available voices
list-voices
```

### Troubleshooting

- **No speech output**: Ensure your audio device is working. Try `list-voices` to verify TTS engine initialization.
- **espeak-ng errors on Linux**: Install `espeak-ng` and `espeak-ng-data` packages.
- **Slow startup**: First run compiles espeak-ng data; subsequent starts are faster.
- **Wrong voice**: Use `tts_set_voice` command or configure in Emacs with `dtk-default-voice`.
- **Windows: "Cannot open load file: emacspeak-loaddefs"**: Run the `emacs --batch` command in step 4 of the Windows setup to generate the loaddefs file.
- **Windows: Emacs not loading init.el**: Check `M-: (expand-file-name "~")` in Emacs. If it points to `%APPDATA%` instead of your home directory, set the `HOME` environment variable (step 1 of Windows setup).
- **Windows: Speech too fast**: The WinRT speech rate default may be fast. Set a lower rate via `tts_set_speech_rate` in Emacspeak or configure with `dtk-speech-rate`.

### Known Issues

- **Text after `;;` may be skipped**: When text contains `;;` (e.g. Lisp comments), omnivox may drop text after the semicolons until the next quote character. This is an omnivox parsing bug, not an Emacspeak issue (other speech servers handle this correctly). Investigation and fix pending.

## Voice Configuration

List available voices:

```bash
list-voices
```

Output groups voices by language and shows quality level (Compact, Enhanced, Premium).

Set the default voice in Emacs:

```elisp
;; macOS example
(setq dtk-default-voice "en-US:Samantha")

;; espeak-ng example
(setq dtk-default-voice "en")
```

## Environment Variables

Omnivox recognizes environment variables for engine selection and audio routing. See [ENV-VARS.md](ENV-VARS.md) for complete documentation.

### Quick Reference

- **OMNIVOX_ENGINE**: Set to `espeak` to force espeak-ng on platforms with native TTS
- **OMNIVOX_AUDIO_TARGET**: Set to `left`, `right`, or `both` for channel routing (used by Emacspeak for dual-server notification mode)

Example:

```bash
# Force espeak-ng engine
OMNIVOX_ENGINE=espeak omnivox

# Test left-channel routing
echo "tts_say {Left ear only}" | OMNIVOX_AUDIO_TARGET=left omnivox
```

## Architecture

The project is organized as a Cargo workspace:

- **omnivox-core** - Command parsing, queue management, state types
- **omnivox-tts** - TTS engine trait and backends (macOS AVSpeechSynthesizer, Windows WinRT, espeak-ng)
- **omnivox-audio** - Audio buffer, effects pipeline, tone generator, file loader, playback
- **omnivox-cli** - Main binary wiring everything together

### Audio Pipeline

All audio flows through the pipeline:

```
Source (TTS / Tone / File) -> AudioBuffer -> Pipeline -> AudioOutput
                                               |
                                    SilenceTrimmer (speech only)
                                    VolumeAdjust
                                    ChannelRouter
```

## Neural TTS (Piper)

Piper neural TTS was evaluated as an optional backend for higher-quality offline voices. The available Rust crates (`piper-rs`, `piper-tts-rust`) have unstable dependency chains that prevent reliable compilation. This integration should be revisited when the Rust Piper ecosystem matures.

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
| `tts_sync_state punct split_caps allcaps_beep rate` | Sync state |
| `tts_reset` | Reset all state |
| `tts_exit` | Shut down |
| `version` | Speak version |

## License

MIT
