# Omnivox Architecture

## Overview

Rust-based cross-platform Emacspeak speech server with mandatory audio processing pipeline.

## High-Level Architecture

```
┌─────────────────┐
│  Emacspeak CLI  │
└────────┬────────┘
         │ stdin/TCP
         ▼
┌─────────────────────────────────────────┐
│        Command Parser & Router          │
│  (Parse Emacspeak protocol commands)    │
└────────┬────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────┐
│         Command Queue System            │
│   • Speech queue                        │
│   • Code queue (voice changes, etc)     │
│   • Tone queue                          │
│   • Audio icon queue                    │
└────────┬────────────────────────────────┘
         │ on dispatch 'd'
         ▼
┌─────────────────────────────────────────┐
│      TTS Engine (Platform-Specific)     │
│   macOS: AVSpeechSynthesizer           │
│   Linux: Speech Dispatcher             │
│   Windows: SAPI                         │
│   Fallback: eSpeak-ng                  │
└────────┬────────────────────────────────┘
         │ PCM audio buffers
         ▼
┌─────────────────────────────────────────┐
│      Audio Processing Pipeline          │
│   1. Silence Trimming (critical)        │
│   2. Channel Panning (left/right/both)  │
│   3. Volume Control (per type)          │
│   4. Effects (echo/reverb - Phase 2)    │
└────────┬────────────────────────────────┘
         │ processed PCM
         ▼
┌─────────────────────────────────────────┐
│        Audio Output (cpal)              │
│   • Multi-device routing                │
│   • Async playback                      │
└─────────────────────────────────────────┘
```

## Core Components

### 1. Command Parser

**Responsibility:** Parse Emacspeak protocol commands from stdin/TCP

**Implementation:**
- Regex-based parser (similar to SwiftMac's `isolateCmdAndParams`)
- Support three command formats:
  - ID only: `s`, `d`, `version`
  - Block args: `q {text}`, `c {codes}`
  - Space args: `t 440 50`, `tts_set_speech_rate 225`

**Key Functions:**
```rust
struct Command {
    id: CommandId,
    args: Option<String>,
}

enum CommandId {
    Queue,        // q
    Code,         // c
    Dispatch,     // d
    Stop,         // s
    Letter,       // l
    Tone,         // t
    // ... etc
}

fn parse_command(line: &str) -> Result<Command, ParseError>
```

### 2. State Store

**Responsibility:** Thread-safe application state (like SwiftMac's StateStore actor)

**Implementation:**
- Use `tokio::sync::RwLock` or similar
- Or use actor pattern with message passing

**State:**
```rust
struct TtsState {
    // Voice settings
    current_voice: String,
    pitch_multiplier: f32,
    speech_rate: f32,

    // Punctuation
    punctuation_level: PunctuationLevel, // None, Some, All
    split_caps: bool,
    allcaps_beep: bool,

    // Volume controls
    voice_volume: f32,
    tone_volume: f32,
    sound_volume: f32,

    // Character rate
    character_scale: f32,

    // Delays
    pre_delay: Duration,
    post_delay: Duration,
    next_pre_delay: Duration,

    // Audio routing
    speech_routing: AudioRouting,
    notification_routing: AudioRouting,
    tone_routing: AudioRouting,
    sound_routing: AudioRouting,
}

struct AudioRouting {
    device_id: u32,  // 0 = system default
    channel_mode: ChannelMode,  // Left, Right, Both
}
```

### 3. Command Queue System

**Responsibility:** Queue commands for sequential processing

**Implementation:**
- Separate queues for different command types
- Process in FIFO order on dispatch

```rust
struct CommandQueue {
    speech_queue: VecDeque<SpeechItem>,
    code_queue: VecDeque<CodeItem>,
    tone_queue: VecDeque<ToneItem>,
    audio_queue: VecDeque<AudioItem>,
}

enum QueueItem {
    Speech(String),
    Code(String),  // Voice changes, pitch adjustments
    Tone { frequency: u32, duration: u32 },
    Silence { duration: u32 },
    AudioIcon { path: PathBuf },
}

impl CommandQueue {
    fn enqueue(&mut self, item: QueueItem);
    fn dispatch(&mut self) -> Vec<QueueItem>;
    fn clear(&mut self);
}
```

### 4. TTS Engine Abstraction

**Responsibility:** Platform-agnostic TTS interface

**Implementation:**
```rust
trait TtsEngine: Send + Sync {
    /// Synthesize text to PCM audio buffer
    async fn synthesize(&self, text: &str, settings: &TtsSettings)
        -> Result<AudioBuffer, TtsError>;

    /// List available voices
    fn available_voices(&self) -> Vec<VoiceInfo>;

    /// Set current voice
    fn set_voice(&mut self, voice: &str) -> Result<(), TtsError>;

    /// Stop current synthesis
    fn stop(&mut self);
}

struct TtsSettings {
    voice: String,
    rate: f32,
    pitch: f32,
    volume: f32,
}

struct AudioBuffer {
    samples: Vec<f32>,  // Interleaved PCM samples
    sample_rate: u32,
    channels: u16,
}

struct VoiceInfo {
    identifier: String,
    name: String,
    language: String,
    quality: VoiceQuality,  // Compact, Enhanced, Premium
}
```

**Platform Implementations:**
- `MacOsTtsEngine` - FFI to AVSpeechSynthesizer
- `LinuxTtsEngine` - Speech Dispatcher client
- `WindowsTtsEngine` - SAPI bindings
- `EspeakEngine` - Embedded eSpeak-ng fallback

### 5. Audio Processing Pipeline

**Responsibility:** Apply effects to PCM audio buffers

**Implementation:**
```rust
struct AudioPipeline {
    effects: Vec<Box<dyn AudioEffect>>,
}

trait AudioEffect: Send + Sync {
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), EffectError>;
}

// Core effects (Phase 1)
struct SilenceTrimmingEffect {
    threshold: f32,
}

struct PanningEffect {
    mode: ChannelMode,  // Left, Right, Both
}

struct VolumeEffect {
    gain: f32,
}

// Advanced effects (Phase 2)
struct ReverbEffect {
    room_size: f32,
    damping: f32,
}

struct EchoEffect {
    delay_ms: u32,
    decay: f32,
}

impl AudioPipeline {
    fn process(&self, buffer: AudioBuffer) -> Result<AudioBuffer, EffectError> {
        let mut buf = buffer;
        for effect in &self.effects {
            effect.process(&mut buf)?;
        }
        Ok(buf)
    }
}
```

### 6. Audio Output System

**Responsibility:** Route processed audio to devices

**Implementation:**
```rust
use cpal::{Device, Stream, StreamConfig};

struct AudioOutput {
    device: Device,
    stream: Stream,
    routing: AudioRouting,
}

impl AudioOutput {
    fn new(device_id: u32, routing: AudioRouting) -> Result<Self, AudioError>;

    async fn play(&mut self, buffer: AudioBuffer) -> Result<(), AudioError>;

    fn stop(&mut self);
}

// Multi-device support
struct MultiDeviceOutput {
    speech_output: AudioOutput,
    notification_output: Option<AudioOutput>,
    tone_output: Option<AudioOutput>,
    sound_output: Option<AudioOutput>,
}
```

### 7. Tone Generator

**Responsibility:** Generate pure tone beeps

**Implementation:**
```rust
struct ToneGenerator;

impl ToneGenerator {
    fn generate(frequency: u32, duration_ms: u32, sample_rate: u32)
        -> AudioBuffer {
        let num_samples = (sample_rate * duration_ms) / 1000;
        let mut samples = Vec::with_capacity(num_samples as usize);

        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * PI * frequency as f32 * t).sin();
            samples.push(sample);
        }

        AudioBuffer {
            samples,
            sample_rate,
            channels: 1,
        }
    }
}
```

## Concurrency Model

**Runtime:** Tokio async runtime

**Key Async Boundaries:**
- Command reading (stdin/TCP)
- TTS synthesis (may block on native APIs)
- Audio playback (cpal streams)

**Actor-like Pattern:**
```rust
// Main command processor
async fn process_commands(
    rx: mpsc::Receiver<Command>,
    state: Arc<RwLock<TtsState>>,
    queue: Arc<Mutex<CommandQueue>>,
    tts_engine: Arc<Mutex<dyn TtsEngine>>,
    audio_output: Arc<Mutex<AudioOutput>>,
) {
    while let Some(cmd) = rx.recv().await {
        match cmd.id {
            CommandId::Queue => {
                queue.lock().await.enqueue(QueueItem::Speech(cmd.args));
            }
            CommandId::Dispatch => {
                let items = queue.lock().await.dispatch();
                process_queue(items, &state, &tts_engine, &audio_output).await;
            }
            // ... handle other commands
        }
    }
}
```

## Error Handling

**Strategy:** Fail gracefully, log errors, continue operation

```rust
#[derive(Debug, thiserror::Error)]
enum OmnivoxError {
    #[error("TTS engine error: {0}")]
    Tts(#[from] TtsError),

    #[error("Audio output error: {0}")]
    Audio(#[from] AudioError),

    #[error("Command parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("Effect processing error: {0}")]
    Effect(#[from] EffectError),
}

// Log and continue on non-fatal errors
if let Err(e) = synthesize_and_play(text).await {
    eprintln!("Error: {}", e);
    // Don't crash, just skip this utterance
}
```

## Performance Optimizations

### Small Chunks
Split text into ~15 word chunks for faster initial audio output:
```rust
fn chunk_text(text: &str, max_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.chunks(max_words)
        .map(|chunk| chunk.join(" "))
        .collect()
}
```

### Aggressive Silence Trimming
```rust
fn trim_silence(buffer: &AudioBuffer, threshold: f32) -> AudioBuffer {
    let start = buffer.samples.iter()
        .position(|&s| s.abs() > threshold)
        .unwrap_or(0);

    let end = buffer.samples.iter().rposition(|&s| s.abs() > threshold)
        .unwrap_or(buffer.samples.len());

    AudioBuffer {
        samples: buffer.samples[start..=end].to_vec(),
        sample_rate: buffer.sample_rate,
        channels: buffer.channels,
    }
}
```

### Streaming Effects
Apply effects in-place during buffer processing to avoid copies:
```rust
impl PanningEffect {
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), EffectError> {
        match self.mode {
            ChannelMode::Left => {
                // Zero out right channel
                for i in (1..buffer.samples.len()).step_by(2) {
                    buffer.samples[i] = 0.0;
                }
            }
            ChannelMode::Right => {
                // Zero out left channel
                for i in (0..buffer.samples.len()).step_by(2) {
                    buffer.samples[i] = 0.0;
                }
            }
            ChannelMode::Both => {
                // No-op
            }
        }
        Ok(())
    }
}
```

## Testing Strategy

### Unit Tests
- Command parser
- State management
- Audio effects (input/output validation)
- Tone generation

### Integration Tests
- Full command sequences
- Queue processing
- Multi-device routing

### Platform Tests
- TTS engine on each platform
- Audio output on each platform

## Build & Distribution

### Cargo Workspace
```toml
[workspace]
members = [
    "omnivox-core",      # Core logic
    "omnivox-tts-macos", # macOS TTS
    "omnivox-tts-linux", # Linux TTS
    "omnivox-tts-windows", # Windows TTS
    "omnivox-tts-espeak", # eSpeak fallback
]
```

### Cross-Compilation
- Use `cross` for building Linux/Windows from macOS
- GitHub Actions for CI on all platforms

### Deployment
- Single binary per platform
- Drop-in replacement in `$EMACSPEAK_DIR/servers/omnivox`
