# Selected speech before optional helper discovery

Private prototype implemented in Omnivox `a40a930`, tested on 2026-11-10 with
the unchanged compiled Emacsvox client `5fc22ec71`. The listening shortcut still
uses the previously qualified runtime. This experiment is disabled by default
and is not a release or general platform qualification.

## Implementation and boundaries

The retained launch environment's `OMNIVOX_EXPERIMENTAL_EARLY_SPEECH=1` enables
the experiment only for an owned startup snapshot without managed-library inputs
or enabled external helpers. Remote workers and unowned launches retain eager
startup. A preferred TGSpeechBox also retains eager startup for this slice,
preserving its existing cached-inventory/prewarm behavior.

The first engine in resolved startup preference order is initialized before
optional helpers, alongside the existing native factories and eSpeak fallback.
If this produces no eligible working engine, ordinary discovery of the remaining
helpers completes before selecting a fallback; a potentially usable server is
not discarded merely because its first choice failed.

After choosing the initial engine, the prototype reuses the registry's existing
bounded per-engine asynchronous rescan mechanism for remaining shipped helpers.
Pending entries have no invented voices and report unavailable with a discovery
or rescan-in-progress reason. Success publishes validated descriptor and engine
handle atomically; failure retains its reason and the same native owner for
explicit retry. Inventory reads do not initialize engines or wait for discovery.
The initial preferred engine remains stable when later helpers appear.

This adds no process manager, speech forwarding hop, native helper protocol or
new public capability. Native handle ownership, creation/handoff deadlines,
resource limits, cleanup proof, cancellation and asset protections are unchanged.
Managed startup and Apply retain complete inventory/preflight. Existing client
inventory refresh and registration handle later discovery; automatic refresh
or a new pending-state UI is not supplied by this prototype.

## Delayed discovery and failure checks

The [native fixture probe](data/2026-11-10-early-speech/check.py) compares the
same guarded Windows executable with the flag off/on/on/off. Real Flite provides
foreground speech while a console-free test helper, registered as RuTTS, delays
its handshake by four seconds. This fixture supplies synthetic PCM and does not
measure the real RuTTS engine's initialization time.

| Observation | Eager startup | Early speech |
| --- | ---: | ---: |
| First source, two runs | 5,071 / 4,950 ms | 832 / 885 ms |
| Inventory while optional discovery is pending | Command admission waits | Unavailable, no voices, in-progress reason |
| Later explicit selection | Completes | Completes after validated publication |

The [fault results](data/2026-11-10-early-speech/faults/result.json) also establish:

- Repeated foreground speech and Stop remain usable while discovery is pending.
- Optional handshake timeout leaves foreground usable and publishes failure.
- Missing preferred Flite falls back to eSpeak while optional discovery proceeds.
- Retirement during pending discovery completes through the actual native guard;
  a separate notification worker continues speaking afterward.

An [additional fallback check](data/2026-11-10-early-speech/only-fallback/result.json)
disables native fallback and makes the preferred helper unavailable. Startup
correctly waits for the remaining delayed helper and speaks through it.
The [active cancellation check](data/2026-11-10-early-speech/active-stop/result.json)
starts a long utterance while optional discovery is delayed ten seconds, observes
source start, issues Stop, requires a cancelled terminal result, then completes
replacement speech. Foreground retirement finishes while discovery is pending;
notifications remain usable. These are native process/source checks, not acoustic
Stop-to-silence measurements.

## Normal startup and responsiveness

The [matched probe](data/2026-11-10-early-speech/matched.py) alternates the flag
over six blocks using the same executable, all eight real engines enabled,
Flite selected and null output. Each fresh owner supplies first speech and five
subsequent short utterances, then verifies retirement. All twelve runs pass.

| Measurement | Eager | Early |
| --- | ---: | ---: |
| First source median | 1,137 ms | 864 ms |
| First source range | 1,030–1,320 ms | 838–896 ms |
| Subsequent short-request median, 30 samples per path | 6.07 ms | 6.29 ms |

This is about 273 ms less warm startup time in this screening run. Subsequent
speech is measured immediately after first speech, so the early path can overlap
optional discovery. The roughly 0.22 ms median difference does not exceed the
existing proposed screening threshold. These sample counts establish neither
tail latency nor a universal steady-state guarantee. The observed slow user
launch from the [preceding investigation](2026-11-10-startup-investigation.md)
was not recreated as a controlled cold-cache benchmark.

## Compiled Emacs and independent streams

Full isolated GUI startup completes Eloquence speech and sound-icon feedback on
both lanes. A separate Flite batch profile passes preference reload, restart,
retained recovery, emergency eSpeak and return to saved Flite.

The [late-discovery GUI check](data/2026-11-10-early-speech/listening/check-late.el)
uses a sixteen-second helper delay. It confirms initial pending inventory while
both streams speak Flite; after discovery, normal client inventory refresh
preserves Flite speech. An explicit client configuration then selects the new
helper and completes speech on both lanes. The
[final result](data/2026-11-10-early-speech/gui-late-final/late-check.txt) is retained.

An earlier check incorrectly assumed notification discovery finished with
foreground discovery. Main speech used the new helper, while notification speech
correctly fell back to eSpeak until its own helper was ready. The test now observes
each stream's readiness independently. No cross-stream barrier was introduced.
Two initial native fixture failures were also corrected: .NET console encoding
setters were incompatible with redirected guarded pipes, and the copied fixture
descriptor needed protocol-v1 synthesis-cancellation support. Their failed
captures remain in the evidence pack; they are not counted as passing runs.

## Verification and reproduction

Locked Linux workspace tests and workspace/all-target Clippy pass, as do pinned
Rust 1.97.1 formatting and Windows CLI/all-target Clippy. The Windows main-only
build guard accepted the CLI-only change and staged private runtime
`4e1c1561f51908a6`; helper payloads were reused unchanged. Functional qualification
here is Windows through WSL. Linux functional and native macOS qualification
remain separate.

The [runtime manifest](data/2026-11-10-early-speech/listening/runtime.json), raw
measurements, controlled stderr, speech events and fixture inputs are retained
under [SHA256SUMS](data/2026-11-10-early-speech/SHA256SUMS). Private owner/control
captures remain local. Run `check.py`, `fallback-check.py`, `stop-check.py` and
`matched.py` with Python 3 on the recorded WSL setup. The GUI launcher accepts
`--gui-check --engine eloquence`; batch recovery uses `--batch-check --engine flite`.
The late-discovery launcher requires the compiled fixture and descriptor created
by the native probe; its retained path identifies the exact tested fixture.
All checks use isolated configuration and null output.

## Result

The experiment supports continuing with early selected speech. Keep it opt-in
until pending discovery is clearly exposed to clients, saved preferences aimed
at a still-pending engine are qualified, and the readiness/compatibility contract
is reviewed before adoption. In particular, a missing preferred engine that is
still attempting initialization can still delay fallback; this slice removes
unrelated optional discovery from readiness, not every startup delay. It does
not qualify automatic late inventory refresh, all engines as initial preferences,
managed activation, external helpers or every platform. No new ADR was created
and no accepted decision or installed deployment was changed.
