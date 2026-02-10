# Omnivox Environment Variables

Complete list of all environment variables recognized by omnivox.

## Engine Selection

**OMNIVOX_ENGINE**

- Values: `espeak` or empty
- Default: empty (use platform-native TTS)
- Forces espeak-ng engine on platforms with native TTS (macOS/Windows)
- Example: `(setenv "OMNIVOX_ENGINE" "espeak")`

## Volume Controls

**OMNIVOX_VOICE_VOLUME**

- Range: 0.0 to 1.0 (1.0 = 100%)
- Default: 1.0
- Controls speech synthesis volume
- Example: `(setenv "OMNIVOX_VOICE_VOLUME" "0.8")`

**OMNIVOX_TONE_VOLUME**

- Range: 0.0 to 1.0
- Default: 1.0
- Controls generated tone volume (beeps)
- Example: `(setenv "OMNIVOX_TONE_VOLUME" "0.1")`

**OMNIVOX_SOUND_VOLUME**

- Range: 0.0 to 1.0
- Default: 1.0
- Controls audio icon/sound file volume
- Example: `(setenv "OMNIVOX_SOUND_VOLUME" "0.1")`

## Channel Routing

**OMNIVOX_AUDIO_TARGET**

- Values: `left`, `right`, `both`, or empty
- Default: empty (both channels)
- Controls channel routing for all audio output
- Used by Emacspeak for dual-server notification mode
- Example: `(setenv "OMNIVOX_AUDIO_TARGET" "left")`

When Emacspeak runs in dual-server mode, it spawns two omnivox processes:

1. **Main speech process**:
   - No OMNIVOX_AUDIO_TARGET set (uses both channels)
   - Handles primary speech output

2. **Notification process**:
   - OMNIVOX_AUDIO_TARGET set to `left` by Emacspeak
   - Handles notifications, audio icons, and status updates
   - Allows notifications to play in left ear while main content continues in both ears

## How Emacspeak Uses These

When `dtk-set-notification-mode` is enabled in Emacspeak, it spawns two omnivox processes:

```elisp
;; Main process (no special env vars)
(start-process "omnivox-main" ...)

;; Notification process
(let ((process-environment (cons "OMNIVOX_AUDIO_TARGET=left" process-environment)))
  (start-process "omnivox-notification" ...))
```

This enables concurrent audio streams. For example, while reading a long document (both channels), notification messages like "50 percent" play in the left channel without interrupting the main content.

## Complete Example

```elisp
;; Volume settings
(setenv "OMNIVOX_VOICE_VOLUME" "1.0")   ; Full volume for speech
(setenv "OMNIVOX_TONE_VOLUME" "0.1")    ; Quiet tones (10%)
(setenv "OMNIVOX_SOUND_VOLUME" "0.1")   ; Quiet audio icons (10%)

;; Force espeak-ng engine (useful for testing or if native TTS has issues)
(setenv "OMNIVOX_ENGINE" "espeak")

;; Note: Don't set OMNIVOX_AUDIO_TARGET manually
;; Emacspeak sets it automatically when spawning the notification process
```

## Technical Details

The `OMNIVOX_AUDIO_TARGET` environment variable is read once at startup in `omnivox-cli/src/main.rs`. It configures the `ChannelRouter` audio effect in the pipeline:

- `left` - Routes all audio to left channel (right channel silent)
- `right` - Routes all audio to right channel (left channel silent)
- `both` or empty - Normal stereo output

This routing applies to all audio sources: TTS speech, generated tones, and audio icon files.
