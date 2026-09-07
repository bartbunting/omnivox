# Omnivox Roadmap

**Priorities reviewed:** 2026-09-07

This is the current project backlog. It intentionally does not repeat shipped
architecture or a chronological implementation diary; see
[STATUS.md](../STATUS.md), [ARCHITECTURE.md](../ARCHITECTURE.md), and Git
history for
those records.

## Direction

Omnivox should remain a bounded, responsive multi-engine host for interactive
accessibility. Speech must degrade predictably when an optional engine, voice,
marker, effect, or resource is unavailable. Ordered and urgent output must not
be sacrificed to navigation replacement, and completion must describe actual
mixer-source lifecycle rather than command acceptance.

Keep platform implementations at parity where the installed native APIs can
provide the same capability. Test shared behavior and document remaining
runtime or integration differences explicitly; an initial port is not the
finished platform contract.

The structured identity model remains:

- an **engine** is a synthesizer implementation;
- a **physical voice** is the pair `(engine_id, voice_id)`;
- a **logical voice** is a portable Emacs style with ordered selectors;
- engine-rendered ACSS and Omnivox-rendered post-synthesis effects are distinct;
- exact machine-local IDs never replace portable fallback policy.

## Feature backlog

This open-ended backlog follows the audio-output, WSL, and additional-voice
discussion, with entries ranked in proposed delivery order. Add further
features as needs emerge. These entries describe future work; local
experiments and existing preview features are identified explicitly. They do
not change the accepted architecture, helper boundaries, or release policy.

| Rank | Feature | First useful outcome |
| --- | --- | --- |
| 1 | Responsive Linux speech under WSL | Reproducible Windows/Linux comparison launches, measured command-to-sound and stop-to-silence, and a native PulseAudio experiment against the current ALSA bridge. |
| 2 | Voice selection and installation assistance | Browse, preview, install/import, test, and select additional voices through Emacsvox's Voice Workbench, with engine-specific installation support. |
| 3 | Audio-device selection and recovery | Named devices, a deliberate follow-default policy, disconnect/reconnect recovery, and separate foreground/notification destinations. |
| 4 | Speech and audio doctor | Explain the selected executable, backend, device, engine, voice, fallback reason, buffer settings, and recovery action. |
| 5 | Reliable remote workstation setup | Complete real SSH-host and native-device acceptance, improve setup diagnostics and reconnection, and graduate the existing preview when evidence supports it. |
| 6 | Portable speech and output profiles | Switch voice/rate/routing preferences and Windows-versus-Linux launch choices without carrying incompatible runtime paths between platforms. |
| 7 | Omnivox pronunciation dictionaries | Per-language and per-application corrections that preserve original-text offsets for markers and navigation. |
| 8 | Pause and resume for long reading | Resume a bounded reading session with defined behavior for intervening navigation, cancellation, and engines without precise markers. |
| 9 | Linux ARM64 main-server distribution | Native runtime acceptance and main-server archives/Debian packages, beyond existing ARM64 companion coverage. |
| 10 | Another compact neural engine | Evaluate an isolated sherpa-onnx helper, including Kitten Nano, against latency, cancellation, memory, intelligibility, and model-licence requirements. |

The earlier companion-manager proposal is part of feature 2. The earlier
first-speech/navigation-latency proposal is part of feature 1 and the
cross-platform evidence work below. Existing engine hardening remains a
release requirement throughout this feature work.

### First delivery slice

The [WSLg experiment](../experiments/2026-09-06-wslg-audio.md) provides a working
local comparison setup, a 50 ms PulseAudio request, and successful Linux 1.8
speech-lane checks. It does not demonstrate acoustic latency or parity with
Windows. The unreleased [WSLg comparison workflow](../WSL-AUDIO.md) now prepares
separate session launchers, reports executable and configuration identities,
and repeats the Linux buffer/shutdown probe. The local trial also fixed
eSpeak's interruption deadlock and added Linux legacy-engine interfaces:
DECtalk and Outloud through the user's Voxin 3.4 runtime pass local
playback/navigation checks. See the
[Linux legacy-engine experiment](../experiments/2026-09-07-linux-legacy-engines.md).
The subsequent [helper parity work](../experiments/2026-09-07-linux-helper-parity.md)
adds native timing and voice-expression controls to both Linux legacy engines.
Keep Linux rate calibration, ECI text-repertoire verification, and the DECtalk
raw-command contract visible as remaining platform work.
The [native PulseAudio experiment](../experiments/2026-09-07-native-pulseaudio.md)
now adds direct Linux output and a full-profile comparison launcher, with a
20 ms native default (40 ms in the WSL trial after short-tone tests), bounded
writes, idle corking, stream-wide flushing and recovery of failed native lanes
when fresh audio arrives. Output failures no longer quarantine healthy voices.
A subsequent live recurrence identified a shared WSLg stall: Weston's audio
packet semaphore was exhausted, PulseAudio's output thread blocked sending to
it, and independent control queries timed out. Reconnecting Omnivox cannot
repair that shared server. An approved reset of the identified Windows RDP
client restored both existing speech processes without restarting Emacs or
Omnivox. Next, investigate the idle/resume acknowledgement path and recurrence,
then collect matched-build listening and physical-output
evidence, including long idle transitions and stop-to-silence. Keep the native
backend opt-in. Evaluate an optional Windows WASAPI PCM-output helper while
keeping synthesis on Linux if the shared RDP path remains unreliable. That
bridge is not implemented.

Separately, start voice management with Piper model discovery/selection, Flite
voice import, and platform-native installation guidance. Emacsvox owns the
accessible UI and guided installation workflow; Omnivox owns engine/voice
inventory, capability reporting, test synthesis, and useful failure details.
This milestone does not promise a particular release date or version.

### Voice installation scope

The intended flow is browse, preview where a sample is available, install or
import, run real test synthesis, then select the usable voice. Show language,
engine, download size, installed state, and voice-specific terms. Keep runtime,
language data, model/voice data, and configuration identifiable so update and
removal can preserve shared dependencies and the working fallback.

| Engine family | Proposed assistance and current constraint |
| --- | --- |
| Piper | Catalog and verified download of the model plus its JSON configuration and model card. The current integration has one configured model; selecting among several requires bounded loading, memory, and eviction behavior. Models remain separate from companion releases. |
| Flite | Import and validate local `.flitevox` files first. The current v2.2 companion accepts only English Clustergen files with `eng`/`usenglish` initializers and reports `flitevox:INTERNAL_NAME`; it has no runtime voice downloader. |
| Windows/macOS native speech | Guide the user through supported operating-system voice installation, then rescan the actual WinRT/AVSpeechSynthesizer inventory and test it. A voice appearing in Narrator or another application is not proof that Omnivox's synthesis API can use it. |
| RHVoice | Guide compatible C API runtime, language-data, and voice-data installation separately, with per-voice terms. A Windows SAPI installation does not establish compatibility with the helper's C API loader. Keep runtime and data user-installed under ADR 0002. |
| eSpeak NG, RuTTS, TGSpeechBox | Expose the choices actually reported by the bundled/staged engine. Do not imply that every engine provides independently downloadable voices. |
| Eloquence and DECtalk | Diagnose the user-supplied vendor runtime and available voices; provide vendor installation guidance within the existing helper boundary. |

Preserve per-engine provenance and licensing decisions in
[ADR 0001](../adr/0001-speech-engine-process-boundaries.md),
[ADR 0002](../adr/0002-rhvoice-and-flite-companions.md), and the later companion
ADRs. Installation assistance is not permission to redistribute arbitrary
models or runtimes, or to make a speech engine download assets automatically.

### Audio-output scope

| Platform | Evaluation direction |
| --- | --- |
| Windows, including Windows Omnivox launched from WSL | Keep event-driven shared-mode WASAPI as the default direction. Evaluate supported low-latency processing periods, device selection, and recovery while retaining coexistence with other audio applications. |
| Linux inside WSLg | Evaluate native PulseAudio against the current ALSA-to-PulseAudio bridge. Retain separate Windows and Linux comparison launches; WSLg's RDP audio transport remains part of the Linux path. |
| Ordinary Linux desktops | Evaluate native PipeWire, with PulseAudio and ALSA alternatives appropriate to the available sound server and device configuration. |

Omnivox currently pins Rodio 0.19.0 and CPAL 0.15.3. Newer CPAL releases offer
native PulseAudio/PipeWire support, but those capabilities are not present in
the pinned stack. A dependency migration is a separate implementation decision,
with cross-platform API, runtime, and packaging checks; adding a Cargo feature
to the current version is not sufficient.

Measure idle-to-first-sound, sustained navigation, stop-to-silence, underruns,
device changes, and competing workload. Retain canonical PCM, host mixing,
bounded progressive buffering, and truthful marker/completion semantics under
the accepted ADRs. Device callbacks and sink estimates must remain distinct
from acoustic measurements. Smaller buffers alone do not establish better
speech responsiveness, as the WSLg stalled-buffer result demonstrates.

## Latency and lifecycle evidence

1. Extend the correlated Emacs submission, protocol admission, synthesis, and
   mixer-source telemetry to first audible device output where a platform
   exposes a truthful measurement callback.
2. Maintain and extend `tools/benchmark_server.py` across real platforms for
   character, word, ordinary line, dense-action timeline, multipart timeline,
   and rapid keyed replacement workloads. Preserve raw samples and compare
   p50, p95, and p99 rather than averages alone. The initial
   [Windows x64 development baseline](../benchmarks/2026-09-01-windows-x64-c9458361eb57b94a.md)
   records all six workloads for WinRT, eSpeak NG, RHVoice, Flite, DECtalk,
   and Eloquence. The later
   [null-output pre-optimization baseline](../benchmarks/2026-09-03-windows-x64-null-f7204ac69b6010f1.md)
   adds RuTTS and TGSpeechBox with exact representative voices, a seeded order,
   and no audible playback; other platforms and repeat runs remain outstanding.
   The later anchored-streaming follow-up shows that Eloquence and DECtalk
   dense timelines no longer add a whole-result wait. Profile and reduce the
   continuous-sinc startup cost only with matched audible-quality and waveform
   evidence; do not restore per-chunk linear upsampling.
3. Maintain `tools/stress_server.py` across real engines and platforms for
   interleaved replacement keys, ordered and urgent traffic, hard stops,
   queued/buffered audio, late completion, and helper restart. Keep verifying
   that stale markers, semantic callbacks, and duplicate or late terminal
   history cannot escape; add physical-output observation where available.
   Investigate the Linux null-output Flite dispatch-fault timeout reproduced
   on both the published v1.7.1 payload and the current development build with
   `--iterations 10 --stop-every 4 --fault-mode dispatch --fault-count 2`.
   Idle-helper fault, fallback, and recovery pass after correcting the stress
   tool's handling of zombie processes; the dispatch-fault case remains open.
4. Measure long-session memory, decoded-resource cache behavior, helper working
   sets, and quarantined native-call capacity on real platforms. The helper
   soak tool now records working-set/private-byte, handle, thread, and CPU
   samples on POSIX and native Windows helpers; multi-engine evidence and
   explicit release thresholds remain outstanding. Server stress can also
   group the root and all helper descendants by executable, separating startup
   growth from steady-state growth.
5. Keep malformed-input, queue-saturation, multipart timeout, and partial-write
   tests aligned with every protocol change.

## Engine hardening

1. Expand real Windows repetition, cancellation, crash, and recovery testing
   for WinRT, Eloquence, and DECtalk, including helper working-set measurement.
2. Maintain Piper's published companion gates and audit dependency updates
   against the [Piper release plan](PIPER-RELEASE.md). Publication began with
   v1.6.4 on Linux x64, Windows x64, and macOS ARM64/x64. Keep checking
   corresponding source, relocation, model-free packaging, and real synthesis
   on every release; wider platform support remains separate work.
3. Improve macOS marker and cancellation coverage without overstating what
   AVSpeechSynthesizer exposes.
4. Extend RHVoice live-runtime acceptance beyond Linux x64 and Windows x64,
   prioritizing Linux ARM64 where upstream runtime support is available. Keep
   macOS and Windows ARM64 labelled compile-only until compatible native
   runtimes pass discovery, synthesis, marker, cancellation, and shutdown
   acceptance.
5. Verify logical-language and text-repertoire routing against live
   multilingual voices on every supported engine.
6. Extend RuTTS evidence beyond the native release gates that passed on all six
   companion targets for v1.7.1. Windows x64 GNU development acceptance also
   covers both voices, exact routing, cancellation latency, helper resources,
   hard stops, repeated helper death, fallback, and recovery. Prioritize cold
   onset, high-rate intelligibility, and multi-hour helper memory evidence on
   real user machines.
   Evaluate RuLex later as its own licensing, provenance, database, and
   cross-platform decision rather than silently adding it to the companion.
7. Keep eSpeak NG as the reliable Unicode-capable final fallback and retain
   regression coverage for its exact native anchors and source-accurate UTF-8
   word/sentence mappings.

## Deployment and user diagnostics

1. Decide whether Linux ARM64 should join the Linux x64 GitHub artifact and
   runtime-test matrices, and evaluate a broader Linux ABI baseline than the
   current Ubuntu 24.04 build.
2. Add signing and provenance verification appropriate to Windows and macOS
   release artifacts.
3. Improve user-facing route, fallback, cancellation, and audio-device
   diagnostics while keeping full synthesis text opt-in and visibly sensitive.
4. Complete real-machine Voice Workbench apply/undo, migration, and divergent
   speaker/notification inventory coverage in the Emacsvox repository.
5. Reconcile README, status, protocol, and deployment documents as a release
   gate instead of storing completed phases in the roadmap.

## Explicit future proposals

These are not current features and require design or scope approval:

- **sherpa-onnx with Inflect Micro and Kitten Nano:** evaluate one optional,
  isolated sherpa-onnx adapter rather than model-specific integrations. Measure
  model load, first-audio and complete-synthesis latency, PCM callback cadence,
  cancellation and replacement without helper restart, working set, high-rate
  intelligibility, and the consequences of absent source-accurate word
  markers. Keep runtime and model assets separately auditable, with explicit
  per-model licence and provenance records.
- **Multiple instances of one engine:** do not add a second Eloquence helper
  merely as a precaution. First collect long-session failure evidence for the
  persistent ECI owner-thread implementation. Revisit per-instance identity,
  health, retry, and duplicate-output rules only if failures remain frequent
  enough to justify the added state and resource cost.
- **Speech Dispatcher:** start from the capability and lifecycle contract in
  [SPEECHD-PLAN.md](SPEECHD-PLAN.md), then revise it for the current engine
  registry. External playback cannot claim buffered mixing/effects parity.
- **Multi-device audio:** promoted to feature 3 above. Define device ownership,
  explicit-device versus follow-default behavior, fallback, restart, and
  notification separation before extending channel routing.
- **Remote workstation follow-up:** the loopback, authenticated single-session
  service is available as a preview in 1.8.0; finish native-device and real
  SSH-host acceptance before removing preview status. Broader network access
  still requires protocol exposure review, and explicit privacy documentation
  before implementation.
- **Additional effects:** new duration-changing or repeating effects must
  preserve marker semantics and truthful tracked completion.
- **Configurable chunking:** add a public control only if benchmarks show a
  useful trade-off beyond the current sentence/clause-aware hard limit.

## Release acceptance

A release candidate should satisfy all applicable locked checks and then pass
real-platform scenarios for:

- startup, voice discovery, route registration, preview, and clean shutdown;
- ordinary, replaceable, ordered, and urgent speech;
- hard stop and repeated keyed replacement during cancellable and
  uncancellable synthesis;
- mixed engines, missing voice/engine fallback, circuit recovery, and helper
  replacement;
- inserted/overlaid resources, effect-state continuity, marker/action ordering,
  and truthful terminal status;
- long input, multipart timelines, malformed records, saturation, and bounded
  resource failure;
- warm and cold onset distributions plus long-session resource stability.

Platform-dependent claims must name the engine, OS, toolchain, sample count,
clock, and instrumentation point. Mixer consumption is a useful proxy but must
not be labelled as first audible output.
