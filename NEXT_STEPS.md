# Next Steps for Omnivox

## Immediate: Windows SAPI TTS Backend

### 1. Create Windows TTS Engine

Create `omnivox-tts/src/windows.rs` implementing `TtsEngine`:

```rust
#[cfg(target_os = "windows")]
pub struct WindowsTtsEngine { ... }

#[cfg(target_os = "windows")]
impl TtsEngine for WindowsTtsEngine {
    fn synthesize(&self, text: &str, settings: &TtsSettings) -> Result<AudioBuffer, TtsError>;
    fn stop(&self);
    fn is_speaking(&self) -> bool;
    fn available_voices(&self) -> Vec<VoiceInfo>;
    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo>;
}
```

**Approach options:**

- **windows-rs crate** with SAPI COM interface (ISpVoice)
- **windows-rs** with Windows.Media.SpeechSynthesis (UWP API)
- Key: Must synthesize to memory buffer, not direct audio output

**SAPI buffer capture pattern:**

1. Create `ISpVoice` COM object
2. Create `ISpStream` backed by memory (`IStream` or `HGLOBAL`)
3. Set the stream as output: `ISpVoice::SetOutput(stream)`
4. Call `ISpVoice::Speak(text, SPF_IS_NOT_XML)`
5. Read PCM data from the memory stream
6. Convert to f32 stereo @ 44100Hz AudioBuffer

**Files to modify:**

- `omnivox-tts/src/windows.rs` - New file, main implementation
- `omnivox-tts/src/lib.rs` - Add `pub mod windows;` behind `#[cfg(target_os = "windows")]`
- `omnivox-tts/Cargo.toml` - Add `windows` dependency behind `[target.'cfg(windows)'.dependencies]`
- `omnivox-tts/build.rs` - May need Windows-specific build steps
- `omnivox-cli/src/main.rs` - Add Windows branch in `create_engine()` function

### 2. Wire into CLI

In `omnivox-cli/src/main.rs`, the `create_engine()` function currently has:

```rust
fn create_engine() -> Result<Box<dyn TtsEngine>> {
    let forced = std::env::var("OMNIVOX_ENGINE").unwrap_or_default();
    if forced != "espeak" {
        #[cfg(target_os = "macos")]
        { /* try MacOsTtsEngine */ }
    }
    // fallback to EspeakTtsEngine
}
```

Add a `#[cfg(target_os = "windows")]` block to try `WindowsTtsEngine` before espeak-ng fallback.

### 3. Test on Windows

- Build: `cargo build --release`
- Test speech: `echo "tts_say {Hello world}" | target\release\omnivox.exe`
- Test espeak-ng fallback: `set OMNIVOX_ENGINE=espeak && echo "tts_say {Hello}" | target\release\omnivox.exe`
- List voices: `target\release\list-voices.exe`
- Run tests: `cargo test`

## After Windows

### Linux Speech Dispatcher Backend

- Create `omnivox-tts/src/linux.rs` implementing `TtsEngine`
- Use `speech-dispatcher` or `ssip-client` crate for Speech Dispatcher
- Or use libspeechd C bindings via FFI
- espeak-ng already works as fallback on Linux

### Additional Features (Lower Priority)

- Network mode (-p flag for TCP listener)
- Multi-device audio routing
- Sox-style effects (reverb, echo, chorus)
- Language switching tables
- Caps beep (currently using pitch raise)
- Unify the two AudioBuffer types (omnivox-tts vs omnivox-audio)

## Reference

See CLAUDE.md for full architecture details and implementation patterns.
