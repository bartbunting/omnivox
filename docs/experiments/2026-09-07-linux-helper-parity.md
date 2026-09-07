# Linux legacy-helper parity, 2026-09-07

The initial Linux adapters returned progressive PCM but omitted native timing
and several ACSS controls already implemented on Windows. As a result,
capitalization cues and other anchored presentation actions selected the
buffered fallback. A voice or pitch change by itself does not require anchors.

This development change uses the existing helper protocol v5 and the same
separate, user-installed runtime boundary. No dependency or runtime packaging
policy changes. It is unreleased work after v1.8.0.

## Implemented parity

| Capability | Windows | Linux after this change |
| --- | --- | --- |
| ECI requested anchors | Exact native indexes | Exact native indexes |
| ECI word/sentence timing | Native indexes | Native indexes |
| DECtalk requested anchors | Native word boundaries, with span fallback | Same affinity and fallback rules |
| DECtalk word/sentence/phoneme timing | Native capture records | Native capture records |
| Rate, average pitch, pitch range, stress, richness, volume | Supported | Supported |
| Progressive PCM and cancellation | Bounded callbacks and queues | Bounded callbacks and queues |

Text bookmarks retain the caller's UTF-8 byte positions; timestamps come only
from native callbacks. ECI handles `eciInsertIndex`/`eciIndexReply`. DECtalk
maps private indexes to word/sentence markers and requested anchors, preserving
the native phoneme sample positions. It retains one 512-sample native PCM
block to accommodate late records, as the Windows helper does. Pending markers
are ordered before their PCM and converted with the continuous sample clock.
Requested-anchor capacity is reserved ahead of optional phoneme records.
Missing or duplicate requested native indexes fail instead of producing
fabricated timing. Buffered callers retain the collected markers and anchors.

Pitch-range, stress and richness tables match the Windows adapters, including
ECI breathiness/volume compensation and DECtalk's paired voice parameters.
Native runtime versions can still sound different with the same settings.

## Verification

The installed Voxin 3.4 English runtime (`libvoxin` 1.6.3 and its bundled
32-bit engine) and Linux x64 DECtalk 4.99 each passed 12 persistent helper
syntheses and four cancellations. Markers were monotonic, ahead of published
PCM, and within the completed frame count. Separate acceptance sessions
covered all eight ECI voices and nine DECtalk voices, each added ACSS control,
multiple anchors, accented text, empty/whitespace/punctuation-only input,
and literal control characters. These are local runtime checks, not claims
about untested library versions or Linux architectures.

An owned terminal Emacs session launched through `~/bin/evox-linux` loaded the
full personal profile and passed 160 rapid Dired arrow keys and two letter
submissions. All four subsequent marked announcements (each engine on both
foreground and notification lanes) completed with a progressive
`utterance_started.frame_count = 0` and native timing events. The announcements
enabled capitalization tones and a styled voice. The test session exited;
existing user sessions were not restarted.

The rebuilt release payload also passed 12 server replacement iterations and
four hard stops per engine using the null backend. These verify routing,
timeline and cancellation behavior independently of device buffering.

Native ABI tests cover UTF-8 positions, repeated requests, deliberately delayed
DECtalk index callbacks, missing/duplicate ECI indexes, cancellation and
recovery, invalid PCM, unavailable libraries, and the scoped Voxin clear-input
workaround. The locked workspace run passed 619 tests with one ignored;
workspace/all-target Clippy and the Piper-enabled CLI Clippy check passed.

Raw local evidence is under `target/linux-parity-20260907/`, including helper
soaks, native acceptance reports, and the pre-change working-file snapshot.
PCM hashes are diagnostic only: repeated identical requests need not produce
identical bytes in these persistent native engines. First-PCM timings describe
helper output, not physical sound onset.

## Remaining differences

- **Audio output:** Linux still uses ALSA through WSLg PulseAudio/RDP;
  Windows uses WASAPI. This change does not eliminate that downstream delay.
- **Speech-rate calibration:** Linux starts with the Windows tables but needs
  its own retained acoustic-duration audit before claiming calibrated parity.
- **ECI text repertoire:** Windows advertises Windows-1252; Linux guarantees
  Latin-1. Verify the installed Linux runtime's extended character handling
  before broadening that claim. Unsupported text retains eSpeak fallback.
- **DECtalk raw commands and native index requests:** Windows accepts inline
  native commands; Linux keeps its existing literal-text handling and does
  not advertise caller-supplied native indexes. Exposing those commands needs
  an explicit input-contract decision; speech-position anchors work without it.
- **Platform-only engines and acceptance:** WinRT is Windows-only and
  AVSpeechSynthesizer is macOS-only. eSpeak, RHVoice, Flite, RuTTS, Piper and
  TGSpeechBox share their respective Rust synthesis adapters across platforms;
  loader paths, available runtimes/voices and release acceptance differ.
  There was no analogous platform-specific anchor omission in that shared
  synthesis code. Markerless RuTTS/Piper limitations apply on both platforms.

The existing Windows launcher currently resolves to a different Omnivox build
from the Linux development launcher. Match builds, voices and configuration
before attributing a listening difference solely to the operating system.
