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

### Setup

1. Build and install omnivox:

   ```bash
   cd /path/to/omnivox
   make build
   make install
   ```

2. Ensure `~/.cargo/bin` is in your PATH.

3. Add to your Emacs configuration:

   ```elisp
   (setq dtk-program "omnivox")
   (setq emacspeak-speech-server "omnivox")
   ```

4. Start Emacspeak as usual. Omnivox will be used as the speech server.

### Alternative: Place Binary in Emacspeak Servers Directory

Instead of relying on PATH, you can symlink or copy the binary into Emacspeak's servers directory:

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
