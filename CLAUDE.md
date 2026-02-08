# CLAUDE.md - Omnivox Project

## Project Overview

Omnivox is a cross-platform Emacspeak speech server written in Rust. It is a drop-in replacement for SwiftMac that works on macOS, Linux, and Windows.

## Architecture

4-crate Cargo workspace:

- **omnivox-core** - Command parsing (27 Emacspeak protocol commands), queue management, state types. Pure Rust, no platform dependencies.
- **omnivox-tts** - TTS engine trait + backends. `TtsEngine::synthesize()` returns an `AudioBuffer`. Backends: macOS AVSpeechSynthesizer (via ObjC bridge), espeak-ng (via espeak-rs-sys). espeak-ng is always compiled in as cross-platform fallback.
- **omnivox-audio** - Audio buffer (stereo f32 @ 44100Hz canonical format), effects pipeline (`AudioEffect` trait), tone generator, OGG/WAV file loader with LRU cache, rodio-based output.
- **omnivox-cli** - Main binary wiring everything together. Reads Emacspeak protocol from stdin, dispatches to TTS/tone/audio-icon sources, runs through pipeline, plays via rodio.

### Key Design Decisions

- **Stereo f32 @ 44100Hz** is the universal internal buffer format. All sources convert to this before pipeline processing.
- **Everything statically linked** - no external process calls (no sox, no CLI tools).
- **espeak-ng always compiled in** (not feature-gated) as guaranteed cross-platform fallback.
- **ObjC bridge** for macOS: `macos_bridge.m` compiled via `cc` crate in build.rs. This was necessary because Rust `block` crate produces blocks incompatible with AVSpeechSynthesizer's `writeUtterance:toBufferCallback:`.
- **Persistent singleton AVSpeechSynthesizer** in the ObjC bridge for stop() support.
- **AudioEffect trait** with `Vec<Box<dyn AudioEffect>>` pipeline for extensible processing.
- **rodio** for cross-platform audio output and OGG/WAV decoding.
- **OMNIVOX_ENGINE=espeak** environment variable to force espeak-ng on macOS.

### Audio Pipeline Flow

```
Source (TTS / Tone / File) -> AudioBuffer (stereo f32 @ 44100Hz) -> Effects Pipeline -> AudioStreams
                                                                         |                    |
                                                              SilenceTrimmer (speech)   Speech Sink (max 10)
                                                              VolumeAdjust              Tone Sink (max 3)
                                                              ChannelRouter             Sound Sink (max 5)
```

Three concurrent audio streams (rodio Sinks on same OutputStreamHandle, automatically mixed):
- **Speech**: TTS output + silence gaps. Serialized within stream.
- **Tones**: Beeps. Serialized within stream, concurrent with speech/sounds.
- **Sounds**: Audio icons. Serialized within stream, concurrent with speech/tones.

Each stream has a max backlog depth. On overflow, old items are dropped to keep audio current.

## Build Commands

```bash
make build     # Release build
make dev       # Debug build
make test      # Run all tests (161 tests)
make lint      # Clippy
make fmt       # Format
make install   # Install to ~/.cargo/bin
make clean     # Clean build artifacts
```

## Testing

161 tests total, all passing:

- omnivox-audio: 60 unit + 31 integration = 91
- omnivox-core: 34 unit + 1 doc = 35
- omnivox-tts: 22 unit
- omnivox-cli: 13 unit

Run: `cargo test`

## Manual Testing

```bash
# Basic speech
echo "tts_say {Hello world}" | ./target/release/omnivox

# espeak-ng engine
echo "tts_say {Hello world}" | OMNIVOX_ENGINE=espeak ./target/release/omnivox

# List voices
./target/release/list-voices

# Full feature test script
./test-all-features.sh
```

## Platform Status

| Platform | Native TTS | espeak-ng | Status |
|----------|-----------|-----------|--------|
| macOS | AVSpeechSynthesizer (ObjC bridge) | Compiled in | Working |
| Linux | Not yet (Speech Dispatcher planned) | Compiled in | espeak-ng works |
| Windows | Not yet (SAPI planned) | Needs testing | Not started |

## Current State (as of 2026-02-07)

### Working

- Full Emacspeak protocol (27 commands)
- macOS native TTS via AVSpeechSynthesizer buffer capture (ObjC bridge)
- espeak-ng TTS (always compiled in, cross-platform)
- Audio pipeline: silence trimming, volume adjust, channel routing
- Tone generation (pure Rust sine wave with fade envelopes)
- Audio icon playback (OGG/WAV via rodio, with LRU cache)
- Punctuation replacement (none/some/all levels)
- Split caps, letter speaking with pitch raise
- Voice switching (intra-sentence)
- Stop/reset with persistent synthesizer

### Not Yet Implemented

- **Windows SAPI backend** - Next priority. Add `omnivox-tts/src/windows.rs` implementing `TtsEngine` trait using Windows SAPI or `windows-rs` bindings.
- **Linux Speech Dispatcher backend** - For native Linux voices (espeak-ng works as fallback).
- **Network mode** (-p flag for TCP listener)
- **Multi-device audio routing**
- **Sox-style effects** (reverb, echo, chorus)
- **Language switching tables**

## Key Files for Windows Implementation

- `omnivox-tts/src/lib.rs` - TtsEngine trait definition (line 196). New backends must implement `synthesize() -> Result<AudioBuffer, TtsError>`.
- `omnivox-tts/src/espeak.rs` - Reference implementation of TtsEngine. Shows the pattern for buffer synthesis, voice listing, parameter mapping.
- `omnivox-tts/src/macos.rs` - macOS implementation (uses FFI to ObjC bridge). Shows the pattern for platform-specific code with `#[cfg(target_os)]` guards.
- `omnivox-tts/src/macos_bridge.m` - ObjC bridge. Not needed for Windows but shows the buffer capture pattern.
- `omnivox-cli/src/main.rs` - Main binary. The `create_engine()` function (around line 30) needs a Windows branch added.
- `omnivox-tts/Cargo.toml` - Add Windows-specific dependencies here.
- `omnivox-tts/build.rs` - Add Windows-specific build steps here if needed.

## Windows SAPI Implementation Guide

1. Create `omnivox-tts/src/windows.rs` with `WindowsTtsEngine` implementing `TtsEngine`.
2. Use `windows-rs` crate for SAPI bindings, or use `ISpVoice` COM interface.
3. Key SAPI functions needed:
   - `ISpVoice::Speak` with `SPF_IS_NOT_XML` flag for synthesis
   - To get buffer output: use `ISpStream` with memory stream, or `SetOutput` to redirect to memory
   - `ISpVoice::GetVoices` for voice enumeration
   - `ISpVoice::SetRate` / `ISpVoice::SetVolume` for parameters
4. Add `pub mod windows;` to `omnivox-tts/src/lib.rs` behind `#[cfg(target_os = "windows")]`.
5. Add Windows branch in `omnivox-cli/src/main.rs` `create_engine()` function.
6. Pattern: Synthesize to buffer -> convert to f32 stereo 44100Hz -> return AudioBuffer.

## Dependencies

Key workspace dependencies (Cargo.toml):

- tokio, thiserror, anyhow, tracing, tracing-subscriber, regex, once_cell
- omnivox-tts: espeak-rs-sys (with compile-espeak-intonations), cc (build)
- omnivox-audio: rodio (vorbis + wav features)
- omnivox-cli: all workspace crates + tokio

## Dual AudioBuffer Types

There are two `AudioBuffer` types (technical debt):

- `omnivox_tts::AudioBuffer` - in omnivox-tts/src/lib.rs (TTS output format)
- `omnivox_audio::AudioBuffer` - in omnivox-audio/src/buffer.rs (pipeline format)

Both are stereo f32 @ 44100Hz but are separate types. The CLI converts between them via `tts_buffer_to_audio_buffer()` in main.rs. A future cleanup could unify these.

## Emacspeak Integration

```elisp
(setq dtk-program "omnivox")
(setq emacspeak-speech-server "omnivox")
```

Ensure `~/.cargo/bin` is in PATH, or symlink into emacspeak/servers/.
