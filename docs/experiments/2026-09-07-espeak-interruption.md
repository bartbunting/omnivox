# eSpeak interruption stall during the WSLg trial

**Date:** 2026-09-07

**Status:** Reproduced and fixed in the development checkout after 1.8.0.
Listening acceptance remains open.

## Failure and cause

Interactive use exposed missing foreground speech, including letters, followed
by a repeatable stall during rapid up/down movement in Dired. The user reported
that TGSpeechBox survived the same movement and eSpeak stalled again after
switching back.

An isolated terminal Emacs session using the [Linux trial launcher](../WSL-AUDIO.md)
reproduced the problem. Omnivox stayed alive and admitted new requests, but the
speech worker stopped finishing them. A debugger backtrace showed:

```text
omnivox-synth
  EspeakTtsEngine::synthesize_stream
  espeak_Cancel
  espeak_ng_Cancel
  fifo_stop
  pthread_cond_wait
```

The eSpeak FIFO thread was waiting for another start request. This identified
a native cancellation deadlock; the foreground and notification processes
could reach it independently. Earlier short synthesis and buffer probes had
not exercised this interruption failure reliably.

The fix replaces native cancellation with a stop epoch observed by the capture
callback. A full callback queue checks cancellation while retaining its fixed
capacity. A rejected stream disconnects its consumer so the callback aborts
the utterance. Stop does not take the capture lock or wait for the native FIFO.

## Verification

- A subprocess regression test fills the bounded stream queue, exercises
  request cancellation, repeated hard stops and rejected audio delivery, then
  checks subsequent streaming and buffered synthesis across 12 iterations.
  The original implementation exceeded its 15-second deadline. The fixed
  implementation completed in approximately 3.4 seconds.
- Through a real terminal Emacs command loop, 340 Dired arrow keys at the
  50 ms trial setting and another 80 at the default buffer setting left
  foreground and notification speech usable. Rapid keys intentionally cancel
  intermediate requests; this is not a claim that every entry was spoken.
- All ten subsequent tracked foreground/notification checks completed. Three
  individually typed letters completed synthesis, and speech completed after
  an explicit hard stop. Logs confirmed eSpeak handled these requests.
- Locked workspace tests and Clippy passed, including the CLI's `piper`
  feature configuration. Formatting checks passed. `make build` rebuilt and
  staged the Linux executable with its matching data and notices.

The trial retained its existing Emacsvox profile and ALSA/PulseAudio route.
The rebuilt executable is still versioned 1.8.0 but contains an **unreleased
development fix**; it is not the published 1.8.0 payload. SHA-256 identities:

| Payload | SHA-256 |
| --- | --- |
| Before fix | `0c0fcd2d5a1c419c5bbe08a5cca09a563ad3ce2ad3842f5cb7fe4e20ea8e35f1` |
| After fix | `4466449570c74f75bb09500688347422a018863027a53bcf7e583f05c908f5df` |

Machine-local backtraces, logs and completion results remain in the private
trial directory. Physical command-to-sound and stop-to-silence latency are
still unmeasured. This diagnosis does not establish the cause of the initial
20 ms idle-buffer shutdown failure or the historical Outloud/dtk-soft issues.
