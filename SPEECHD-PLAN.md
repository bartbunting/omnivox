# Speech Dispatcher Backend Implementation Plan

> **Roadmap status:** This is a backend-specific design note, not the canonical
> project roadmap. See [NEXT_STEPS.md](NEXT_STEPS.md). Before implementation,
> revise this design to use the common engine descriptor, voice identity,
> fallback, cancellation, and completion contracts. Because Speech Dispatcher
> normally owns playback instead of returning PCM, it must advertise external
> playback and the resulting limits on centralized mixing, effects, and marker
> handling rather than claiming buffered-engine parity.

## Overview

Add Speech Dispatcher (`libspeechd`) as a TTS backend for Linux. Gives users access
to any SD-configured voice (espeak-ng, Festival, Flite, Piper via SD module, etc.)
without additional omnivox configuration.

## Architecture

SD does not return PCM buffers — it outputs audio through PulseAudio/PipeWire/ALSA
directly. `SpeechDispatcherEngine::synthesize()` calls `spd_say()` (blocking), waits
for completion via callback, then returns `AudioBuffer::empty()`. The worker queues
nothing to rodio for speech; tones and audio icons still go through rodio as normal.

```
SD backend:  text -> spd_say_sync() -> SD daemon -> PulseAudio/PipeWire -> speakers
rodio:       tones + audio icons -> rodio -> same audio device
```

Channel routing for notification mode: set `PULSE_SINK=tts_right` (or `ALSA_DEFAULT`)
on the notification omnivox process. SD inherits it and outputs to the virtual sink.
No code changes needed — the existing Linux audio infrastructure handles it.

## Feature Support

| Feature | Status | Mechanism |
|---------|--------|-----------|
| Text synthesis | ✅ Full | `spd_say()` |
| Rate control | ✅ Full | `spd_set_voice_rate()` (-100 to 100) |
| Pitch control | ✅ Full | `spd_set_voice_pitch()` |
| Volume control | ✅ Full | `spd_set_volume()` |
| Voice selection | ✅ Full | `spd_set_synthesis_voice()` |
| Stop/cancel | ✅ Full | `spd_cancel()` in `engine.stop()` |
| Chunking | ✅ Full | Each chunk calls `spd_say_sync()` |
| Generation counter | ✅ Full | Worker checks staleness between chunks |
| Split caps | ✅ Full | Text preprocessed before SD sees it |
| Punctuation expansion | ✅ Full | Text preprocessed before SD sees it |
| Capital letters | ✅ Full | Semantic timelines carry queued cues; isolated letters use the selected capitalization presentation |
| Tones | ✅ Full | Still through rodio (unchanged) |
| Audio icons | ✅ Full | Still through rodio (unchanged) |
| Channel routing | ✅ Via env | `PULSE_SINK` / `ALSA_DEFAULT` on process, SD inherits it |
| Silence trimming | N/A | SD owns audio output |
| Multi-client | ✅ Built-in | SD daemon supports concurrent connections |
| Priority system | ✅ Bonus | Use `SPD_TEXT` for main, `SPD_NOTIFICATION` for notify server |

## New Files

### `omnivox-tts/src/speechd.rs`

Main engine implementation. Key parts:

```rust
pub struct SpeechDispatcherEngine {
    conn: Mutex<*mut SPDConnection>,
    done: Arc<(Mutex<bool>, Condvar)>,
}

impl TtsEngine for SpeechDispatcherEngine {
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let conn = self.conn.lock().unwrap();
        self.apply_settings(&conn, &request.settings)?;
        // Set end-of-speech callback to signal done
        // Call spd_say() with request.text and SPD_TEXT priority
        // Wait on condvar for callback to fire
        Ok(SynthesisResult::audio("speechd", actual_voice, AudioBuffer::empty()))
    }

    fn stop(&self) {
        let conn = self.conn.lock().unwrap();
        unsafe { spd_cancel(*conn) };
        // Signal condvar so blocking synthesize() unblocks
    }

    fn available_voices(&self) -> Vec<String> {
        // spd_list_synthesis_voices() + format as "module:name"
    }
}
```

Rate mapping: `spd_rate = ((rate - 1.0) * 100.0).clamp(-100.0, 100.0) as i32`
Pitch mapping: `spd_pitch = ((pitch - 1.0) * 100.0).clamp(-100.0, 100.0) as i32`
Volume mapping: `spd_vol = ((volume - 0.5) * 200.0).clamp(-100.0, 100.0) as i32`

### `omnivox-speechd-sys/` (new crate)

Thin FFI wrapper for `libspeechd`, same pattern as espeak-rs-sys.

```
omnivox-speechd-sys/
  Cargo.toml
  build.rs          # pkg-config for speech-dispatcher
  src/
    lib.rs          # extern "C" bindings
```

`build.rs` uses `pkg-config` to find `speech-dispatcher`:
```rust
pkg_config::probe_library("speech-dispatcher").unwrap();
```

Key bindings needed:
```rust
extern "C" {
    fn spd_open(client: *const c_char, conn: *const c_char,
                user: *const c_char, mode: SPDConnectionMode) -> *mut SPDConnection;
    fn spd_close(conn: *mut SPDConnection);
    fn spd_say(conn: *mut SPDConnection, priority: SPDPriority,
               text: *const c_char) -> c_int;
    fn spd_cancel(conn: *mut SPDConnection) -> c_int;
    fn spd_set_voice_rate(conn: *mut SPDConnection, rate: c_int) -> c_int;
    fn spd_set_voice_pitch(conn: *mut SPDConnection, pitch: c_int) -> c_int;
    fn spd_set_volume(conn: *mut SPDConnection, volume: c_int) -> c_int;
    fn spd_set_synthesis_voice(conn: *mut SPDConnection,
                                voice: *const c_char) -> c_int;
    fn spd_set_capital_letters(conn: *mut SPDConnection,
                                mode: SPDCapitalLetters) -> c_int;
    fn spd_set_notification_on(conn: *mut SPDConnection,
                                event: SPDNotification) -> c_int;
    fn spd_list_synthesis_voices(conn: *mut SPDConnection) -> *mut *mut SPDVoice;
    fn free_spd_voices(voices: *mut *mut SPDVoice);
}
```

Completion detection: SD is async by default. To make `synthesize()` blocking:
1. `spd_set_notification_on(conn, SPD_END)` at connection open
2. Set `conn->callback_end` to a C callback that signals a `Condvar`
3. `synthesize()` waits on the condvar after `spd_say()`
4. `stop()` calls `spd_cancel()` AND signals the condvar to unblock

Note: callback runs on SD's internal thread — condvar is the right primitive here.

## Changes to Existing Files

### `Cargo.toml` (workspace)

Add `omnivox-speechd-sys` to `members` (NOT `default-members` — Linux only).

### `omnivox-tts/Cargo.toml`

```toml
[target.'cfg(target_os = "linux")'.dependencies]
omnivox-speechd-sys = { path = "../omnivox-speechd-sys", optional = true }

[features]
speechd = ["omnivox-speechd-sys"]
```

### `omnivox-tts/src/lib.rs`

Add:
```rust
#[cfg(all(target_os = "linux", feature = "speechd"))]
pub mod speechd;
```

### `omnivox-cli/src/engine.rs`

Add `"speechd"` case in `create_engine()`:
```rust
if forced == "speechd" {
    #[cfg(all(target_os = "linux", feature = "speechd"))]
    match SpeechDispatcherEngine::new() {
        Ok(engine) => { info!("Using Speech Dispatcher engine"); return Ok(Arc::new(engine)); }
        Err(e) => warn!("Speech Dispatcher not available: {}, falling back", e),
    }
}
```

### `Makefile`

```makefile
build-speechd:
    cargo build --release --features speechd

install-speechd:
    cargo install --path omnivox-cli --features speechd
```

## Completion Detection Detail

This is the trickiest part. SD is async — `spd_say()` returns immediately.
Options:

1. **Callback + Condvar** (recommended): Register `SPD_END` notification callback
   at connection open. Callback signals a `(Mutex<bool>, Condvar)`. `synthesize()`
   waits on condvar. `stop()` calls `spd_cancel()` + signals condvar. Clean and
   correct.

2. **spd_say_sync via SPDConnectionMode**: SD has `SPD_MODE_SINGLE` connection mode
   which may block — needs testing, not documented well.

3. **Polling**: Poll `spd_get_client_id()` or similar — bad idea, don't do this.

Use option 1.

## Stop Behavior

`engine.stop()` must:
1. Call `spd_cancel(conn)` — stops SD output immediately
2. Signal the condvar — unblocks any `synthesize()` waiting for END callback
3. Worker's generation counter discards subsequent chunks as usual

This mirrors how `espeak_Cancel()` works but with the added condvar signal.

## Voice List Format

`spd_list_synthesis_voices()` returns `SPDVoice[]` with fields:
- `name`: voice identifier (e.g., "en-US")
- `language`: language code
- `variant`: variant name

Format for `--list-voices` output: `"speechd:{module}:{name}"` or just `"{name}"`.
TBD based on what SD actually returns at runtime.

## Testing

On a Linux system with SD installed:
```bash
# Verify SD is running
spd-say "hello"

# Test omnivox SD backend
OMNIVOX_ENGINE=speechd (printf 'tts_say {Hello world}\n'; sleep 3) | omnivox

# List voices
OMNIVOX_ENGINE=speechd omnivox --list-voices

# Test stop
OMNIVOX_ENGINE=speechd (printf 'tts_say {A very long sentence}\ns\n'; sleep 2) | omnivox
```

## Linux Prerequisites

- `speech-dispatcher` package installed and daemon running
- `libspeechd-dev` (headers + `.so`) for compilation
- At least one SD output module configured (espeak-ng is the typical default)

## What We Do NOT Implement

- `spd_set_punctuation()` — we preprocess punctuation in text layer before SD sees it
- `spd_set_spelling()` — we handle letter-by-letter in our Letter command
- `spd_set_language()` — language switching not yet implemented in omnivox generally
- `spd_set_output_module()` — user configures this in SD's own config; we don't override it

## Open Questions (resolve at implementation time)

1. Is `SPD_MODE_SINGLE` actually synchronous? If yes, simplifies implementation.
2. Does the END callback fire after `spd_cancel()`? If not, need to track cancelled state.
3. Thread safety of `SPDConnection*` — is it safe to call `spd_cancel()` from reader
   thread while `spd_say()` is blocking in worker thread? Likely yes (designed for this),
   but verify.
4. How does SD handle empty strings? Guard against sending empty text.
