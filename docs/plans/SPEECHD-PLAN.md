# Speech Dispatcher Backend Design Proposal

> **Status: unimplemented design proposal.** This retained proposal predates
> the current engine registry, structured synthesis result, tracked playback,
> and timeline capability contracts. Reconcile those interfaces and resolve the open
> questions below before treating any file list or code sketch as an
> implementation plan. Current Linux builds use eSpeak NG.

> **Roadmap status:** This is a backend-specific design note, not the canonical
> project roadmap. See [NEXT_STEPS.md](NEXT_STEPS.md). Before implementation,
> revise this design to use the common engine descriptor, voice identity,
> fallback, cancellation, and completion contracts. Because Speech Dispatcher
> normally owns playback instead of returning PCM, it must advertise external
> playback and the resulting limits on centralized mixing, effects, and marker
> handling rather than claiming buffered-engine parity.

## Overview

The proposed backend would expose Speech Dispatcher (`libspeechd`) on Linux and
make its configured output modules available through Omnivox. Exact module,
voice, language, and stable-ID behavior still needs real-system validation.

## Architecture

Speech Dispatcher normally owns audio playback instead of returning PCM to its
client. A backend would need to submit speech, wait for a truthful terminal
notification, and advertise external playback. Returning an empty
`AudioBuffer` alone is not sufficient: current tracked completion, timeline
ordering, marker, cancellation, and effects contracts would otherwise report a
false result. Tones and audio icons would still use Omnivox's rodio path.

```
SD backend: text -> spd_say() -> END/CANCEL notification -> daemon audio output
rodio:      tones + audio icons -> rodio -> configured audio output
```

Process-level sink selection for a separate notification server is a deployment
hypothesis, not an established backend contract. The effective PulseAudio,
PipeWire, or ALSA routing variables and ordering between Speech Dispatcher and
rodio must be validated on each supported path.

## Provisional Capability Assessment

| Feature | Status | Mechanism |
|---------|--------|-----------|
| Text synthesis | Candidate | Submit through `spd_say()` and prove terminal notification semantics. |
| Rate, pitch, volume | Candidate | Map common settings and verify native ranges/defaults. |
| Voice and language | Open | Define stable IDs and request-local module/voice/language selection. |
| Stop/cancel | Open | Prove callback and cross-thread cancellation behavior. |
| Chunking and generation | Candidate | Reuse common preparation and stale-work checks. |
| Split caps and punctuation | Candidate | Reuse common text preprocessing before submission. |
| Capitalization cues | Open | Coordinate caller-supplied cues and isolated-letter pitch with external playback. |
| Tones and audio icons | Open | Rodio remains separate; ordering and completion need a bridge. |
| Channel routing | Open | Validate daemon/backend-specific sink selection. |
| Silence trimming and PCM effects | Unavailable | Speech Dispatcher owns the speech PCM. |
| Markers and tracked completion | Open | Require truthful notifications integrated with current tickets/events. |
| Priority | Optional | Define policy before mapping Speech Dispatcher priority classes. |

## Historical Implementation Sketch

The file list and code below predate current interfaces and are illustrative,
not compile-ready.

### `omnivox-tts/src/speechd.rs`

Main engine implementation. Key parts:

```rust
pub struct SpeechDispatcherEngine {
    connection: SpeechDispatcherConnection,
    terminals: PendingTerminalMap,
}

impl TtsEngine for SpeechDispatcherEngine {
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        self.connection.apply_settings(&request.settings)?;
        let message_id = self.connection.say(&request.text)?;
        // Wait with a deadline for the matching (client_id, message_id) END or
        // CANCEL notification. Connection loss is also terminal.
        self.terminals.wait_for(message_id)?;
        // Current SynthesisResult cannot yet represent externally owned audio
        // plus a correlated terminal notification.
        todo!("define an external-playback result contract")
    }

    fn stop(&self) {
        self.connection.cancel();
        // The correlated callback or timeout retires the pending request.
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        // Convert spd_list_synthesis_voices() into stable physical voice IDs.
    }
}
```

The connection wrapper and terminal map above are placeholders, not existing
Omnivox types. In particular, do not hold one mutex across the terminal wait:
`stop()` and the callback path must remain able to make progress concurrently.

Control mappings are deliberately left TBD. A revised rate mapping must
preserve Omnivox's `0.5` normal point and `0.0..2.0` host range, then clamp to
the verified Speech Dispatcher range; the old `rate - 1.0` sketch did not do
that.

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

Completion detection requires a threaded connection, END and CANCEL callback
registration, and a bounded pending-request table keyed by the callback's
client and message IDs. A condition variable or channel can wake the waiter,
but the notification identity and terminal kind—not the wakeup alone—determine
the result. Connection failure and timeout need explicit terminal paths.

## Changes to Existing Files

### `Cargo.toml` (workspace)

Decide workspace placement together with CI policy. Adding a Linux-only sys
crate to `members` makes `cargo test --workspace` build it even when it is not a
default member, so the locked workspace gate would then require development
headers. An optional dependency plus a dedicated feature job may be preferable.

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

The current C API returns a message ID from `spd_say()`. Notifications require a
threaded connection: `spd_set_notification_on()` rejects
`SPD_MODE_SINGLE`. Speech Dispatcher's own `spd-say --wait` client registers
both END and CANCEL callbacks and waits on a semaphore.

A backend can use a condition variable or channel for the same synchronization,
but it must correlate callback message and client IDs with the submitted
request, distinguish END from CANCEL, handle connection loss and timeout, and
prevent a late callback from completing a newer request. There is no
`spd_say_sync()` API in the current header, and `SPD_MODE_SINGLE` must not be
treated as synchronous playback.

Reference the upstream
[`libspeechd.h`](https://github.com/brailcom/speechd/blob/master/src/api/c/libspeechd.h)
and [`spd-say` wait implementation](https://github.com/brailcom/speechd/blob/master/src/clients/say/say.c)
when revising this design; both are more authoritative than the historical FFI
sketch above.

## Stop Behavior

`engine.stop()` would call `spd_cancel(conn)` and rely on a correlated CANCEL or
END notification to retire external playback. The host generation still blocks
later stale chunks. A bounded timeout and connection-recovery path are required
if the daemon never supplies a terminal callback; locally signalling a wait
primitive is not by itself evidence that Speech Dispatcher stopped output.

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
(printf 'tts_say {Hello world}\n'; sleep 3) | OMNIVOX_ENGINE=speechd omnivox

# List voices
OMNIVOX_ENGINE=speechd omnivox --list-voices

# Test stop
(printf 'tts_say {A very long sentence}\ns\n'; sleep 2) | OMNIVOX_ENGINE=speechd omnivox
```

## Linux Prerequisites

- `speech-dispatcher` package installed and daemon running
- `libspeechd-dev` (headers + `.so`) for compilation
- At least one SD output module configured (espeak-ng is the typical default)

## What We Do NOT Implement

- `spd_set_punctuation()` — we preprocess punctuation in text layer before SD sees it
- `spd_set_spelling()` — we handle letter-by-letter in our Letter command
- Process-global `spd_set_language()` state — current Omnivox language belongs
  to each logical synthesis request. A revised backend design must decide how
  to apply that request-local language without reintroducing global language
  commands.
- `spd_set_output_module()` — user configures this in SD's own config; we don't override it

## Open Questions (resolve at implementation time)

1. Which callback and connection-loss sequences occur after `spd_cancel()`, and
   which terminal status should each sequence produce?
2. What synchronization does `SPDConnection*` permit while one host thread is
   waiting and another requests cancellation? Verify this from upstream code
   and stress it against supported daemon versions.
3. Can request-local module, language, voice, and prosody settings be isolated
   reliably on one serialized connection, or are multiple connections needed?
4. How should externally owned playback participate in Omnivox tickets,
   timeline ordering, marker degradation, and notification-channel routing?
5. How does Speech Dispatcher handle empty strings? Guard against sending empty
   text regardless.
