# Windows helper startup queue and listening profile

Functional correction, 2026-11-10, on the private lightweight branch. Omnivox
`ac9ee0a` fixes the serialized helper-creation queue. The paired Emacsvox client
remains `5fc22ec71`; its previous delayed-registration correction is unchanged.
This is not a published release or a new latency comparison.

## Confirmed incident

The user's 10:35 UTC development session ran the guarded Windows prototype,
runtime `5c330933b3325f9d`. Foreground selected eSpeak after Flite and Eloquence
failed to start. Notifications selected Flite. Foreground DECtalk subsequently
completed speech, matching the user's report. The earlier registry-generation
rejections were absent.

All six foreground helpers requested creation together. The serialized channel
gave every contender only six seconds to acquire its mutex. Healthy work ahead
of Eloquence and Flite exhausted that wait; they were marked unavailable with
`native helper creation channel busy or unavailable`. RHVoice subsequently
became ready 7,268 ms after its initialization began. This does not mean native
creation itself took 7,268 ms: the reported span includes queueing and helper
initialization. The worker preserved speech by falling back to eSpeak.

The [extracted metadata](data/2026-11-10-helper-queue/incident-metadata.json)
retains startup events without personal utterances. Full incident logs remain
local in `target/helper-queue/incident/`. A sound-file path error was also present:
the WSL profile sent Linux paths to the Windows worker. Failed mixed timelines
could suppress their associated spoken feedback.

## Bounded correction

Waiting for the creation channel now has its own bound: eight helper slots
times two six-second phases, or 96 seconds maximum. Successful acquisition
continues immediately; this adds no mandatory delay. The caller sends nothing
and owns no new native handles while waiting. The mutex still covers only
creation, handle transfer and start acknowledgement, and is released before
engine initialization. Ordinary synthesis and PCM use their existing direct
pipes.

Actual creation and acknowledgement retain their six-second deadlines. Failed
or uncertain handoff still retires the receiving worker through the independent
guard. No cleanup evidence, native resource bound or peer isolation was relaxed.
The [startup reference](../reference/engine-startup-snapshot.md#windows-creation-candidate)
records the distinction between queue wait and native phase deadlines.

The regression exercises eight contenders whose aggregate work exceeds one
phase deadline; all must acquire the channel. Another test holds the channel
past the separate queue deadline and requires a timeout. These use scaled
durations, not a 96-second test or a claim about worst-case product startup.
They directly exercise the helper client's lock acquisition function.

The [private listening profile](data/2026-11-10-helper-queue/listening/start.py)
also maps local sound resources through `wslpath` at the existing resource
boundary. A 256-entry cache avoids repeat subprocess calls for cached paths.
This mapping is confined to this WSL-to-Windows test profile. Ordinary clients
and remote icon identifiers retain their existing behavior.

## Verification

The [provenance](data/2026-11-10-helper-queue/provenance.json) pins source,
toolchain and private runtime `ea5d6dfb93dd575d`. The Windows main-only build
guard accepted this CLI change and verified reuse of the existing helper
payloads. Installed release runtimes and existing speech sessions were untouched.

| Check | Result |
| --- | --- |
| Native Windows guard suite, including explicitly enabled native tests | 30 passed; queue regression, queue expiry, rejection, pending creation, lost handoff, actual handle retirement, peer independence and bounded I/O. |
| Full interactive GUI, Eloquence and Flite | Both engines completed tracked source speech on foreground and notification lanes, then completed speech containing a mapped sound icon on both lanes. |
| Eloquence batch lifecycle | Preference reload, restart, retained recovery, minimal eSpeak emergency and return to Eloquence passed; foreground recovery retained the notification process. |
| Desktop batch route, Flite | The Windows development batch launcher reached the updated private profile and passed the same lifecycle sequence. |
| Rust checks | Locked Linux workspace tests, locked Linux workspace/all-target Clippy, Windows CLI/all-target Clippy and formatting passed with Rust 1.97.1. |

The [native results](data/2026-11-10-helper-queue/native-units/0.stdout.gz),
[GUI Eloquence result](data/2026-11-10-helper-queue/gui-eloquence-sound/gui-check.json),
[GUI Flite result](data/2026-11-10-helper-queue/gui-flite-sound/gui-check.json),
[Eloquence lifecycle output](data/2026-11-10-helper-queue/batch-eloquence/launcher.log)
and [desktop output](data/2026-11-10-helper-queue/desktop-batch/launcher.log)
are retained with controlled worker logs. All eight enabled engines registered
in the ordinary worker startups in these runs. No helper-unavailable warnings,
registry-generation rejections or bad sound paths occurred in those controlled
checks. Emergency startup intentionally exposes only eSpeak.

Reproduce the GUI checks using `xvfb-run -a python3` with the private launcher,
`--gui-check --engine eloquence` or `--gui-check --engine flite`. Use
`--batch-check --engine eloquence` for the lifecycle sequence. The retained
[native runner](data/2026-11-10-windows-creation-integration/fixtures/unit.py)
accepts the Windows test executable, private runtime, output directory and
`native_guard --ignored`; the test executable needs its C++ runtime DLLs beside
it. Checksums are in [SHA256SUMS](data/2026-11-10-helper-queue/SHA256SUMS).

## Handover and limits

The **Emacsvox Development** desktop shortcut now reaches this profile through
the existing Windows batch launcher and `/home/bart/bin/emacsvox-lightweight-test`.
The wrapper was backed up before changing its runner. The profile defaults to
Flite and retains all eight qualified engines for switching. Updating the
shortcut does not change an already running session.

The tests establish source completion with null output. They do not establish
audible Eloquence acceptance or acoustic responsiveness of this new build. The
user's positive listening report applies to the preceding runtime's working
voices. The original slow Windows launch was observed, not reproduced through
artificial native API delay; the deterministic contention regression covers its
queue-expiry mechanism. Fresh checks used warm payloads and do not establish
cold-start or tail-latency guarantees. Native macOS and additional engines retain
their existing qualification gaps.
