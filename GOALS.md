# Omnivox Goals

## Project Vision

Cross-platform Emacspeak speech server written in Rust. Drop-in replacement for SwiftMac with support for Windows, Linux, and macOS.

## Core Requirements

### 1. Cross-Platform Voice Synthesis

**Primary approach:** Native OS libraries per platform
- **macOS:** AVSpeechSynthesizer (via Objective-C bindings)
- **Windows:** SAPI or Windows.Media.SpeechSynthesis
- **Linux:** Speech Dispatcher

**Fallback:** eSpeak-ng as universal fallback engine when native TTS unavailable

### 2. Mandatory Audio Processing Pipeline

**Architecture:** All audio MUST flow through effects pipeline before playback
- TTS engine → PCM buffer → effects → audio device
- No direct playback path
- Support WAV/PCM intermediate format for processing

**Critical effects (Phase 1):**
- Silence trimming (leading/trailing) - CRITICAL for voice switching
- Channel panning (left/right/both stereo positioning)
- Volume control (separate per audio type: voice/tone/sound/notification)

**Advanced effects (Phase 2):**
- Echo/reverb
- Pitch shifting (at audio level, not just TTS level)
- Speed/tempo changes
- Equalization/filtering

**Design principle:** Streaming effects optimized for speed over perfect audio quality

### 3. Performance - Latency is God

**Target latency:** ~50-100ms from command to audio output (match SwiftMac)

**Optimization strategies:**
- Buffer-based synthesis (no file I/O in critical path)
- Small chunk processing (15-word chunks like SwiftMac)
- Aggressive silence trimming
- Async throughout (Tokio runtime)
- Simple effects inline during buffer processing
- Complex effects pre-computed or approximated cheaply

### 4. Voice Switching - Intra-Sentence Support

**Capability:** Change voices mid-utterance for "audio syntax highlighting"

**Implementation:** State-change commands queued alongside speech
```
c [{voice en-US:Samantha}]
q Hello
c [{voice en-US:Alex}]
q World
d
```

Voice switches happen between queued chunks, processed sequentially on dispatch.

### 5. Complete Emacspeak Protocol Support

**Phase 1 - Core + SwiftMac Features:**

*Core Commands:*
- `q {text}` - queue speech
- `c {codes}` - queue inline codes (voice changes, pitch, etc.)
- `l {letter}` - speak letter immediately (pitch raise for capitals)
- `d` - dispatch queue
- `s` - stop speaking immediately
- `a {path}` - queue audio icon/sound file
- `p {path}` - play audio icon immediately
- `t {freq} {duration}` - queue tone
- `sh {duration}` - queue silence

*State Management:*
- `tts_say {text}` - speak immediately (bypass queue)
- `tts_set_punctuations {mode}` - none/some/all
- `tts_set_speech_rate {rate}` - speech rate
- `tts_set_character_scale {factor}` - character rate multiplier
- `tts_split_caps {0|1}` - space before capitals
- `tts_allcaps_beep {0|1}` - beep vs pitch raise on capitals
- `tts_sync_state {punct splitcaps capsbeep rate}` - atomic state update
- `tts_reset` - reset to defaults
- `version` - speak server version

*SwiftMac Extensions:*
- `tts_set_voice {lang:voice}` - change voice
- `tts_set_pitch_multiplier {0.5-2.0}` - pitch adjustment
- `tts_set_sound_volume {0-1}` - audio icon volume
- `tts_set_tone_volume {0-1}` - beep volume
- `tts_set_voice_volume {0-1}` - speech volume
- Multi-device routing commands
- Runtime channel switching

**Phase 2 - Advanced Features:**
- Sox-style effects: `[{phaser ...}]`, `[{reverb ...}]`, `[{echo ...}]`, `[{chorus ...}]`, `[{tremolo ...}]`
- Language switching with tables (`set_lang`, `set_next_lang`, `set_previous_lang`)
- Language aliases and per-language voice memory

### 6. Network Mode Support

Support TCP listener mode like SwiftMac's `-p` flag for remote operation.

### 7. Multi-Device Audio Routing

Support routing different audio types to different devices/channels (SwiftMac feature):
- Speech → Device A, Channel both
- Notifications → Device B, Channel left
- Tones → Device A, Channel both
- Sounds → Device B, Channel right

## Implementation Language: Rust

**Rationale:**
- Memory safety (solving SwiftMac's crashing problems)
- Excellent concurrency (Tokio async runtime)
- Good cross-platform audio ecosystem (cpal, rodio)
- Strong FFI for native TTS bindings
- Modern tooling (Cargo for cross-compilation)

**Key Dependencies (anticipated):**
- `cpal` - Cross-platform audio output
- `tokio` - Async runtime
- Platform-specific TTS bindings (FFI)
- `rubato` - Audio resampling
- Custom DSP for effects (or lightweight crates)

## Project Structure

**Location:** `/Users/rmelton/projects/robertmeta/omnivox/`

**Relationship to SwiftMac:**
- Standalone replacement
- Parallel during development
- Eventually deprecates SwiftMac when feature-complete and stable

## Success Criteria

1. **Feature Parity:** 100% SwiftMac feature compatibility
2. **Cross-Platform:** Builds and runs on macOS, Linux, Windows
3. **Performance:** Latency ≤ SwiftMac (~50-100ms)
4. **Stability:** No crashes (Rust safety guarantees)
5. **Emacspeak Compatible:** Drop-in replacement for existing Emacspeak servers

## Development Phases

**Phase 1: Core Foundation**
- Emacspeak protocol parser
- Command queue system
- Single-platform TTS (start with macOS or Linux)
- Basic audio pipeline (PCM processing)
- Core effects (trim, pan, volume)

**Phase 2: Cross-Platform TTS**
- macOS: AVSpeechSynthesizer bindings
- Linux: Speech Dispatcher integration
- Windows: SAPI bindings
- eSpeak-ng fallback

**Phase 3: Advanced Features**
- Multi-device routing
- Network mode
- Sox-style effects
- Language switching

**Phase 4: Polish & Performance**
- Optimization
- Comprehensive testing
- Documentation
- Emacspeak integration testing

## Non-Goals

- GUI or visual interface
- Audio recording/capture
- Speech recognition
- Non-Emacspeak protocol support
- Perfect audio quality (favor speed over perfection)
