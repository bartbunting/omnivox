# Omnivox Roadmap

**Reviewed:** 2026-09-27.
This is the outstanding backlog. [Status](STATUS.md) records current coverage,
[architecture](ARCHITECTURE.md) describes implementation, and the
[evidence index](benchmarks/README.md) links observations and comparisons.
Follow the [documentation guide](DOCUMENTATION-GUIDE.md) for proposal lifecycle.

## Direction

Keep interactive speech bounded and responsive across engines and platforms.
Preserve predictable fallback, independent speech lanes, cancellation, source
identity and truthful completion. Engine capabilities and qualification can
vary; a port or a passing unit suite is not complete native/audible acceptance.

## Feature backlog

Outstanding items are listed without a priority ranking:

- **Responsive Linux speech under WSL:** complete matched-build listening,
  physical command-to-sound/stop-to-silence comparisons and longer-running
  acceptance of the implemented native PulseAudio backend.
- **Streaming macOS system voices:** complete real Emacsvox listening and
  physical latency/cancellation acceptance of the implemented callback streaming.
- **Voice selection and installation assistance:** extend native and fresh-client
  acceptance, interrupted-operation recovery and operating-system voice setup.
- **Audio-device selection and recovery:** named devices, a deliberate
  follow-default policy, disconnect/reconnect recovery, and separate
  foreground/notification destinations.
- **Speech and audio doctor:** explain the selected executable, backend, device,
  engine, voice, fallback reason, buffer settings, and recovery action.
- **Reliable remote workstation setup:** extend outage, interactive,
  matched-release, and macOS coverage before graduating the preview.
- **Portable speech and output profiles:** switch voice/rate/routing preferences
  and Windows-versus-Linux launch choices without carrying incompatible runtime
  paths between platforms.
- **Omnivox pronunciation dictionaries:** per-language and per-application
  corrections that preserve original-text offsets for markers and navigation.
- **Pause and resume for long reading:** resume a bounded reading session with
  defined behavior for intervening navigation, cancellation, and engines without
  precise markers.
- **Linux ARM64 main-server distribution:** native runtime acceptance and
  main-server archives/Debian packages, beyond existing ARM64 companion coverage.
- **Another compact neural engine:** evaluate an isolated sherpa-onnx helper,
  including Kitten Nano, against latency, cancellation, memory, intelligibility,
  and model-licence requirements.

The earlier companion-manager proposal is part of voice installation assistance.
The earlier first-speech/navigation-latency proposal is part of responsiveness
and the cross-platform evidence work below. Existing engine hardening remains a
release requirement throughout this feature work.

### First delivery slice

The [WSLg comparison workflow](guides/wsl-audio-comparison.md) and opt-in native PulseAudio
backend are implemented. Remaining work is matched-build listening and physical
command-to-sound/stop-to-silence measurement, sustained navigation, competing
workload and long idle/resume transitions. Investigate recurrence of the
[shared WSLg bridge stall](experiments/2026-09-07-native-pulseaudio.md#shared-wslg-bridge-stall-after-the-recovery-fix).
Keep the backend opt-in. Consider a Windows WASAPI PCM-output helper only as a
separate proposal if the shared RDP path remains unreliable.

### macOS native voice streaming delivery slice

[Callback streaming](engines/macos.md) is implemented and has native
Intel/Apple Silicon and null-output checks. Complete real Emacsvox listening,
physical onset and stop-to-silence comparisons across voices, both lanes,
short navigation, long utterances and repeated cancellation. Retain bounded
backpressure, late-callback cleanup and no fallback after audio commitment.
Investigate native markers separately and advertise only verified capabilities.

### Remote workstation delivery slice

[Real SSH acceptance](experiments/2026-09-07-remote-ssh.md) covers Windows
DECtalk device/null and Linux eSpeak null output. Before graduating preview,
collect listening, longer outages, sleep/resume, interactive editing,
matched-release payload and native macOS evidence. Preserve two-lane recovery
without replay and the [documented setup](guides/remote-speech.md).

### Voice installation scope

[Managed installation, enablement, Apply/rollback and removal](guides/voice-management.md)
are implemented. Extend native and fresh-client acceptance across providers and
platforms; keep Windows component checks, full server/runtime checks and audible
acceptance distinct. Complete native macOS removal and broader spoken interaction
checks. Preserve disabled installation, imported ownership, active/rollback/session
references and both workers' correlated acknowledgements.

Further recovery work must reconcile incomplete worker records and interrupted
Apply using verified ownership, including provider-specific boot/process-tree
identity. Missing cleanup stays blocking; no force-clear or PID-based recovery.
Power-loss durability, partial publication reconciliation, retention/garbage
collection and download resume/update remain separate work.

Extend operating-system voice installation assistance through actual WinRT/macOS
inventory and synthesis. Keep RHVoice runtime acquisition separate from managed
data; proprietary engines retain user-supplied runtimes. Additional catalogues
need reviewed provenance and terms under [ADR 0007](adr/0007-managed-voice-lifecycle.md).

### Extensible engine registration

The [framework specification](plans/extensible-engine-framework.md) and
[accepted ADR 0008](adr/0008-extensible-engine-registration.md) cover both
independently maintained helpers and maintained adapters loading separately
supplied runtimes. They define strict JSON `helpers.d/` manifests, `config.json`
policy, unified discovery and lifecycle, explicit selection and coordinated
launch snapshots. Configuration v1 is authorized for implementation; runtime
integration and acceptance remain outstanding.

The first slice proves configuration, inventory, diagnostics, selection and
recovery with a redistributable fake helper. Real native integrations require
separate qualification. Language-routing enhancements remain a separate versioned
proposal; registration does not authorize them.

### Audio-output scope

Evaluate Windows shared-mode WASAPI latency/device recovery, native PulseAudio
under WSLg, and PipeWire/PulseAudio/ALSA on ordinary Linux. The pinned Rodio
0.19.0/CPAL 0.15.3 stack does not gain newer backends merely through a feature
flag; any dependency migration requires its own compatibility and packaging
review. Preserve canonical PCM, host mixing, bounded buffering and truthful
markers. Measure physical output, underruns, device changes and competing load.

## Latency and lifecycle evidence

- Select matched before/after baselines from [retained evidence](benchmarks/README.md).
  Extend character, word, line, dense-action, multipart and replacement workloads
  across real platforms. Preserve raw samples, exact voice/build identity and
  p50/p95/p99; qualify source timing separately from acoustic onset.
- Extend correlated client/admission/synthesis/playback observations to physical
  output where a truthful measurement is possible. Profile converter startup
  without weakening continuous-sinc quality or source mapping.
- Maintain replacement-domain, hard-stop, stale-event and helper-death stress.
  Investigate the Linux null-output Flite dispatch-fault timeout reproduced on
  v1.7.1 and a development build with `--iterations 10 --stop-every 4
  --fault-mode dispatch --fault-count 2`; idle-helper fault/recovery passes do
  not close this case.
- Measure multi-hour memory, helper working sets, handles, threads, CPU, decoded
  cache growth and quarantine capacity. Define workload-specific release
  thresholds from matched evidence rather than historical unit-test counts.
- Maintain malformed-input, saturation, multipart timeout and partial-write
  coverage with protocol changes. Never overwrite an earlier measurement pack.

## Engine hardening

- Broaden repeated Windows WinRT/Eloquence/DECtalk cancellation, crash, recovery
  and resource measurements.
- Maintain [Piper release gates](engines/piper.md#release-maintenance) for dependency
  updates; additional target architectures need native acceptance.
- Broaden macOS cancellation and investigate marker support without claiming
  precision unavailable from the native API.
- Extend RHVoice runtime qualification beyond Linux/Windows x64, prioritizing
  Linux ARM64. Compile-only platforms remain unqualified until native checks pass.
- Audit live multilingual routing and text repertoires, plus Linux ECI/DECtalk
  and macOS speech-rate calibration.
- Extend RuTTS cold-onset, high-rate intelligibility and multi-hour memory
  evidence beyond companion release gates. RuLex needs a separate provenance,
  database, licensing and cross-platform decision.
- Preserve eSpeak's Unicode fallback, native anchors and original UTF-8 mappings.

## Deployment and user diagnostics

- Decide Linux ARM64 generic-server runtime/artifact coverage and evaluate a
  broader Linux ABI baseline than Ubuntu 24.04.
- Define signing/provenance verification for Windows/macOS releases.
- Improve explanations of selected executable, engine, voice, routing, fallback,
  output device and recovery while keeping full speech text explicitly sensitive.
- Complete Homebrew upgrades between upstream versions and audible Emacs use;
  formula updates remain explicit. Automatic update PRs are only a future option.
- Complete fresh Voice Workbench Apply/undo, compatibility and divergent
  main/notification inventory acceptance in Emacsvox.

## Explicit future proposals

- **sherpa-onnx, Inflect Micro and Kitten Nano:** evaluate one isolated adapter,
  with model load/onset/completion latency, callback cadence, cancellation,
  memory, high-rate intelligibility and source-marker limits. Review runtime
  and model provenance/terms independently.
- **Multiple instances of one engine:** first establish repeated persistent
  helper failures through long-session evidence. Only then define per-instance
  identity, health, retry and duplicate-output rules.
- **Speech Dispatcher:** resolve external-playback capabilities and lifecycle
  in [the feasibility proposal](plans/speech-dispatcher.md) before implementation.
- **Multi-device output:** define explicit-device/follow-default ownership,
  notification separation, fallback and reconnect as part of audio-device work.
- **Remote expansion:** broader exposure needs a new boundary review; preview
  qualification alone does not authorize network, resource or multi-user changes.
- **Additional effects:** preserve marker and terminal semantics when duration
  changes or output repeats.
- **Configurable chunking:** add a control only after measurements show a useful
  trade-off beyond the current sentence/clause-aware limit.

## Experimental ideas

Feasibility work below is outside the feature backlog and does not establish
platform support or implementation approval.

### iOS remote speech receiver over Tailscale

Explore an iPhone app that synthesizes and plays speech for Emacsvox running
on another machine. Both devices join the user's tailnet; Emacsvox connects
over TCP to the phone, sending text and speech instructions while synthesis
and playback remain on iOS. Tailscale supplies the encrypted network path
without an SSH tunnel. Retain application authentication and restrict access
through tailnet policy.

Start with Apple's AVSpeechSynthesizer and investigate reuse of Omnivox's
macOS buffer-capture adapter, Rust protocol handling, scheduling, and effects.
Map logical voices to the phone's available voice inventory; desktop voice
identity or acoustic parity is not assumed. Additional engines are later,
separate porting and licensing investigations.

The first experiment should establish TCP connectivity over Tailscale, speech
playback, rapid replacement, and immediate cancellation with the app open.
Then lock the phone, leave it silent for several minutes, and request new
speech. Measure command-to-sound and stop-to-silence on a real iPhone, and test
connection loss and recovery without replaying stale speech. Tailscale's VPN
availability does not establish that iOS will keep the speech app running or
wake it for incoming TCP traffic. Reliable operation after locked-screen idle
is the main feasibility question, before committing to a full port.

A usable follow-up would cover foreground and notification lanes, bundled
icons, effects, truthful playback completion, VoiceOver coexistence, audio
interruptions, Bluetooth routing, and battery use. The desktop service's
separate worker processes need an iOS-compatible lifecycle design. Both the
current Omnivox listener and Emacsvox client enforce loopback addresses;
direct tailnet access requires an explicit revision of
[ADR 0004](adr/0004-workstation-service-and-worker-ownership.md) and review of the other
accepted process-boundary decisions before implementation.

Develop and test portable Rust and Emacsvox changes on Linux/WSL. Use a Mac
with Xcode and the iOS SDK to build and sign the app; a remote Mac is sufficient
for build access. Physical iPhone acceptance is required for the network,
audio, and background-lifecycle questions. No implementation or delivery date
is committed by this roadmap entry.

## Release acceptance

Release candidates retain all applicable locked checks and native scenarios for
startup/inventory, routing/preview, ordinary and keyed speech, independent lanes,
stop/replacement, missing-runtime fallback, circuit recovery and helper retirement.
Cover inserted/overlaid resources, effects and markers, malformed/large input,
queue pressure, multipart transport, cold/warm onset and long-session resources.
Exact release gates belong to [deployment](../.github/DEPLOYMENT.md).

Reports must identify the engine, platform, source/toolchain, sample count,
clock and instrumentation point. Mixer consumption is not first audible output.
