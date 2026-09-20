# macOS native voice streaming

The development AVSpeechSynthesizer adapter implements ADR 0006's existing
progressive synthesis contract. It remains built in under ADR 0001, uses the
existing playback pipeline and requires no new dependency or wire protocol.

## Ownership and bounds

Each request owns a Cocoa capture, eight windows of at most 512 native frames
and an explicit terminal state. Mono and stereo float PCM use the same continuous
sinc converter as other streaming engines. A full queue applies backpressure;
Stop closes the capture and wakes its producer independently of the native
synthesis queue. Request-token cancellation is also checked while polling and
before every converted window. Sink failure and unwinding retire the capture.

Native callbacks never borrow Rust memory. An executing callback retains its
capture, and a later callback must upgrade a weak reference and pass the closed
state check. A delegate completion must match the request's utterance. The
engine serializes requests; Stop also invalidates requests already waiting for
that ownership. Future requests capture a fresh stop generation.

An empty native completion buffer or the matching finish delegate closes
production. A 200 ms pause is no longer a completion heuristic. Thirty seconds
without native/queue progress fails the request. Rust also watches progress
independently of Cocoa, including a stalled native startup call. It allows two seconds for
the native capture owner to retire; missing acknowledgement quarantines this
in-process engine until Omnivox restarts. Retained native ownership keeps late
callbacks memory-safe even in that failure case.

Cumulative native and collected canonical PCM each use the existing 128 MiB
synthesis limit. The native queue itself retains at most 32 KiB of sample data;
this excludes Apple-owned buffers, native speech services, converter state and
the downstream playback reserve. It is not a whole-process memory bound.

## Compatibility and limits

The full-result API collects the same stream with the same limits. Ordinary
supported speech can start playback before native completion. Requests requiring
unsupported anchors or whole-waveform effects retain their existing buffered
path. The adapter advertises no word, sentence, phoneme or exact-anchor support,
and retains the existing native rate mapping.

Exact routed voices use their native identifier, rather than a potentially
ambiguous display name. Missing exact voices fail; ordinary routing can select
fallback only before the existing PCM commitment boundary. Main and notification
workers continue owning independent adapters and queues.

## Repeatable checks

On Linux, `cargo test --locked -p omnivox-tts macos::` checks callback-boundary
independent conversion, cancellation, invalid PCM and honest omitted anchors.
It does not execute the Cocoa bridge.

On a Mac, compile and run the native queue regressions:

```sh
clang -fobjc-arc -fmodules -framework Foundation -framework AVFoundation \
  tools/test_macos_streaming.m -o /tmp/omnivox-macos-bridge-tests
/tmp/omnivox-macos-bridge-tests
cargo run --locked -p omnivox-tts --no-default-features --example macos_streaming_probe
```

The probe owns the main Cocoa run loop and does not open an audio device. It
checks native voices, early windows, full-result compatibility, repeated stop,
consumer failure, successful subsequent requests, omitted-voice system defaults
and rejection of missing explicit voices. Its deliberate consumer
pauses exercise backpressure; reported timings are not a performance benchmark.
The [native workflow](../.github/workflows/macos-streaming.yml) runs these checks
on Intel and Apple Silicon without publishing a release. It also stages the
complete main-server payload and uses the existing benchmark and stress tools
on two simultaneous null-output processes. Those checks cover all six workload
types, exact physical routing, replacement and hard stops.

The final [native acceptance run](https://github.com/bartbunting/omnivox/actions/runs/35167821767)
passed on Intel and Apple Silicon running macOS 15.7.9 at source commit
`382c338812e2f6af1f8ca2f923225763d9b04363`:

- Each architecture passed five native queue/lifecycle regressions, ten Rust
  adapter checks and native Clippy. Gordon, Karen and Catherine each passed
  progressive delivery, full-result collection, repeated cancellation,
  consumer failure and subsequent synthesis.
- Each complete server payload passed two simultaneous speech processes with
  exact Gordon routing and null audio output. Each process exercised all six
  benchmark workloads with two measured samples per workload, six replacement
  iterations and three hard stops. The workflow artifacts retain the reports,
  voice identifier, host details, source commit and executable hashes.
- The Linux locked workspace suite passed 840 tests, with one existing ignored
  test; workspace Clippy and formatting checks also passed.

Listening through Emacsvox, physical command-to-sound and stop-to-silence
measurements, and comparison with buffered playback remain open. No audio device
was exercised by the native CI probes. Queue and PCM checks do not establish
audible quality, navigation comfort or device latency.
