# Omnivox Architecture

## Overview

Rust-based cross-platform Emacspeak speech server with mandatory audio processing pipeline. All audio is captured to buffers, processed through effects, then played via rodio.

## Crate Structure

```
omnivox/
├── Cargo.toml              # Workspace root
├── omnivox-core/           # Commands, queue, state, pure presentation timeline
├── omnivox-tts/            # TTS trait + backends (macOS, Windows WinRT, espeak-ng)
├── omnivox-audio/          # Buffer, pipeline, effects, tone gen, file loader, rodio output
├── omnivox-cli/            # Main binary
└── elisp/                  # Emacs voice module (omnivox-voices.el)
```

## Data Flow

```
stdin (Emacspeak protocol)
    │
    ▼
┌──────────────────────────────┐
│  Command Parser (omnivox-core)│
│  parse_command() → Command    │
└──────────┬───────────────────┘
           │
           ▼
┌──────────────────────────────┐
│  Command Queue (omnivox-core) │
│  QueueItem: Speech, Code,     │
│  Tone, Silence, AudioIcon     │
└──────────┬───────────────────┘
           │ on dispatch 'd'
           ▼
┌──────────────────────────────────────────────────────┐
│  Three Input Sources → Common Buffer Format           │
│                                                       │
│  TTS Engine (omnivox-tts)                            │
│    ├─ macOS: AVSpeechSynthesizer.write(toBuffer)     │
│    ├─ espeak-ng: AUDIO_OUTPUT_RETRIEVAL mode         │
│    └─ (future: SAPI, Speech Dispatcher)              │
│                                                       │
│  Tone Generator (omnivox-audio)                      │
│    └─ Pure Rust sine wave + fade envelopes           │
│                                                       │
│  Audio File Loader (omnivox-audio)                   │
│    └─ OGG/WAV via rodio decoder, LRU cache           │
│                                                       │
│  All output: stereo f32 @ 44100Hz AudioBuffer        │
└──────────┬───────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────┐
│  Effects Pipeline             │
│  Vec<Box<dyn AudioEffect>>   │
│                               │
│  SilenceTrimmer (speech only) │
│  VolumeAdjust                 │
│  ChannelRouter (L/R/Both)     │
└──────────┬───────────────────┘
           │
           ▼
┌──────────────────────────────────────────┐
│  AudioStreams (3 concurrent rodio Sinks)  │
│                                           │
│  Speech Sink  (max depth 100, serialized) │
│  Tone Sink    (max depth 10, serialized)  │
│  Sound Sink   (max depth 10, serialized)  │
│                                           │
│  Different streams play concurrently      │
│  Overflow drops old items, keeps current  │
│  rodio auto-mixes all sinks together      │
└──────────────────────────────────────────┘
```

For a dispatched batch, speech and silence tickets form the primary
presentation clock. Legacy queued audio icons are buffered at their queue
boundary, mixed when several share that boundary, and scheduled on the sound
sink after all preceding primary tickets complete. Following speech can
already be queued, so it overlaps the icon instead of waiting for its tail.
The deferred icon ticket covers both its wait and full audible tail; tracked
completion waits for it and stop invalidates it before or during playback.
This is boundary-level lowering of the pure timeline model. Requested engine
anchors and the later bounded renderer provide sample-aligned in-span actions.
The bounded PCM renderer realizes each scheduled timeline one synthesis window
at a time. Insertions expand the primary buffer and its frame map; overlays mix
at their scheduled frame without moving the primary clock. Overlay tails carry
into the next speech or silence window, while the final remainder becomes a
separately tracked boundary overlay. This permits playback after one bounded
chunk and lets following speech continue under a long icon. Marker and anchor
frames are remapped through insertions before playback. Prepared resources are
canonical stereo PCM and must match their declared duration; file-backed
resources are limited to 16 MiB/30 decoded seconds and a 128-entry/64 MiB LRU.

Capitalization cues are the first live in-span consumer: preprocessing retains
their UTF-8 offsets, synthesis resolves them, and the renderer sample-mixes a
20 ms tone. Eloquence provides exact frames; DECtalk and WinRT approximate to
word boundaries; markerless engines place the tone at the chunk start. The
tone uses normal tone volume/channel processing and stop cancels all mixed or
unreached output. Structured resource transport is added at the Emacsvox
timeline-negotiation layer. Capability-gated compatibility lowering uses an
explicit presentation-tone mode; standalone legacy tones remain independent.

Semantic timeline actions render as zero-duration output-frame cues. Protocol
v2 carries only their bounded opaque IDs; the corresponding Lisp meaning stays
in Emacsvox. The audio source invokes the nonblocking event callback as it
consumes the frame, so stop/replacement drops every unreached event. Engine
markers and semantic events share one stable playback order and one stdout
reporter, whose flush barrier runs before tracked completion. Marker protocol
v1 remains available unchanged; v2 is enabled only with structured timeline
transport.

## Key Types

### omnivox-core

```rust
// Command parser
enum CommandId { Queue, Code, Dispatch, Stop, Letter, Tone, ... } // 27 commands
struct Command { id: CommandId, args: Option<String> }
fn parse_command(line: &str) -> Result<Command, ParseError>

// Queue
enum QueueItem { Speech(String), Code(String), Tone{freq,dur}, Silence{dur}, AudioIcon{path} }
struct CommandQueue { items: VecDeque<QueueItem> }

// State
struct TtsState { voice, pitch, rate, volume, punctuation_level, split_caps, ... }
enum ChannelMode { Left, Right, Both }
enum PunctuationLevel { None, Some, All }

// Engine-neutral presentation timeline
enum PresentationPosition { SpanBoundary{...}, TextOffset{...} }
enum AudioActionMode { Insert, Overlay }
enum TimelineActionKind { Audio{...}, SemanticEvent, EffectState{...} }
struct FrameMap { /* checked piecewise insertion map */ }
struct ScheduledTimeline {
    frame_map, actions, primary_output_frames, completion_frame
}
```

The timeline scheduler is deliberately pure. Engines resolve source positions
to primary speech-frame boundaries; the scheduler preserves stable same-frame
order and projects insertions, overlays, semantic events, and persistent effect
state onto the output clock. `primary_output_frames` excludes overlay tails,
while `completion_frame` includes them. Playback integration is layered on
this contract rather than inferred from the three legacy sinks.

### omnivox-tts

```rust
// TTS engine trait - all backends implement this
trait TtsEngine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError>;
    fn stop(&self);
    fn is_speaking(&self) -> bool;
    fn available_voices(&self) -> Vec<VoiceInfo>;
    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo>;
}

struct SynthesisRequest { text, settings, requested_voice, logical_voice_id, language, anchors }
struct SynthesisResult { audio, engine_id, actual_voice, markers, anchors, degraded_acss }
struct SynthesisMarker { kind, frame_offset, text_start, text_length, value }
struct RequestedAnchor { id, text_offset, affinity }
struct ResolvedAnchor { id, frame_offset, resolution }

// AudioBuffer (TTS output format)
struct AudioBuffer { samples: Vec<f32>, sample_rate: u32, channels: u16 }
// Helpers: empty(), from_i16(), to_stereo(), resample(), to_standard_format()

// Constants
const STANDARD_SAMPLE_RATE: u32 = 44100;
const STANDARD_CHANNELS: u16 = 2;
```

### omnivox-audio

```rust
// Canonical audio buffer
struct AudioBuffer { samples: Vec<f32>, sample_rate: u32, channels: u16 }
const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u16 = 2;

// Effects pipeline
trait AudioEffect: Send + Sync {
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError>;
    fn name(&self) -> &str;
}
struct AudioPipeline { effects: Vec<Box<dyn AudioEffect>> }

// Built-in effects
struct SilenceTrimmer { threshold: f32 }  // Default threshold 0.005
struct VolumeAdjust { scale: f32 }        // Clamps to [-1.0, 1.0]
struct ChannelRouter { mode: ChannelMode } // Left/Right/Both

// Tone generator
ToneGenerator::generate(freq_hz: f32, duration_ms: u32, volume: f32) -> AudioBuffer

// File loader
AudioFileLoader::load(path) -> Result<AudioBuffer>  // OGG/WAV, optional LRU cache

// Concurrent output streams
AudioStreams::new(speech_max, tone_max, sound_max) -> Result<Self>
AudioStreams::queue(stream, buffer) -> Result<bool>   // overflow drops old items
AudioControl::queue_overlay_after(buffer, barriers)   // nonblocking deferred overlay
AudioStreams::stop(stream)                            // clear + resume
AudioStreams::stop_all()
AudioStreams::is_playing(stream) -> bool
AudioStreams::pending(stream) -> usize
enum StreamType { Speech, Tone, Sound }

// Single-shot output (used by tests)
AudioOutput::new() -> Result<Self>  // wraps rodio
AudioOutput::play(buffer) -> Result<PlaybackHandle>
```

## macOS ObjC Bridge

The macOS TTS uses an Objective-C bridge (`macos_bridge.m`) because Rust's `block` crate produces blocks incompatible with AVSpeechSynthesizer's callback API.

```
omnivox-tts/src/macos_bridge.m  (compiled by cc crate via build.rs)
    │
    ├─ omnivox_synthesize(text, lang, name, rate, pitch, vol) → SynthResult
    │    Uses writeUtterance:toBufferCallback: with NSRunLoop pumping
    │    Returns malloc'd float buffer (freed by omnivox_free_samples)
    │
    ├─ omnivox_stop() → stops persistent singleton synthesizer
    ├─ omnivox_is_speaking() → bool
    ├─ omnivox_list_voices() → VoiceList (identifier, name, language)
    └─ omnivox_free_voice_list() / omnivox_free_samples() → memory cleanup
```

The bridge uses a persistent singleton AVSpeechSynthesizer (via dispatch_once) so that `stop()` can interrupt in-progress synthesis.

Completion detection uses a two-phase approach: wait for the explicit completion signal (frameLength==0 callback), with a 200ms idle timeout fallback for macOS versions that don't send it.

## espeak-ng Integration

espeak-ng is compiled from source via `espeak-rs-sys` crate (with `compile-espeak-intonations` feature). Uses `AUDIO_OUTPUT_RETRIEVAL` mode with `espeak_SetSynthCallback` for buffer capture.

Key details:

- Global mutex serialization (espeak-ng has global mutable state)
- Multi-tier data directory discovery (build dir → system paths → fallback)
- Parameter mapping: rate 0-1→80-450wpm, pitch 0.5-2.0→0-99, volume 0-1→0-200

## Engine Selection

The CLI selects the TTS engine at startup:

1. If `OMNIVOX_ENGINE=espeak`, use espeak-ng directly
2. On macOS: try AVSpeechSynthesizer first, fall back to espeak-ng
3. On other platforms: use espeak-ng (until native backends are added)

## Text Preprocessing

Before synthesis, text goes through:

1. **Punctuation replacement** (none/some/all levels) - converts punctuation characters to spoken words
2. **Split caps** - inserts spaces before uppercase in camelCase

## Concurrency

- Synchronous stdin command loop (no async needed for protocol)
- TTS synthesis is synchronous (blocking)
- Audio playback is async (rodio handles internally)
- espeak-ng uses global mutex for thread safety
- macOS ObjC bridge manages its own NSRunLoop for callback delivery

## Error Handling

Fail gracefully, log errors, continue operation. A failed synthesis skips the utterance rather than crashing.

## Build System

- `Makefile` wraps cargo commands
- `build.rs` in omnivox-tts compiles ObjC bridge (macOS) and discovers espeak-ng data paths
- All platform-specific code uses `#[cfg(target_os = "...")]` guards
- Stub implementations provided for non-native platforms
