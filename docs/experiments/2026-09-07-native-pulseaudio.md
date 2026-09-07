# Native PulseAudio output under WSLg — 2026-09-07

This is an **unreleased, opt-in development experiment** following the
[Linux helper parity work](2026-09-07-linux-helper-parity.md). It implements
the direct PulseAudio comparison proposed after testing Linux Eloquence and
DECtalk. The existing `device` backend remains the default.

## Implementation and launch

`--audio-output pulse` sends canonical stereo 44.1 kHz float PCM directly to
the system `libpulse.so.0`. Speech, tones and sound icons have independent
persistent streams, source workers and native event threads. PulseAudio mixes
them; Emacsvox foreground and notification remain separate Omnivox processes.
Engine helpers, voice policy, effects, progressive prebuffering and marker
semantics remain shared. See [ADR 0009](../adr/0009-native-pulseaudio-output.md).

The native backend requests 20 ms total latency, writes at most 220 frames
(4.99 ms), drains and corks idle streams, and flushes its own stream on a
stream-wide stop or backlog retirement. Keyed cancellation retains the shared
source fade without flushing unrelated requests. Automatic prebuffering at
one frame prevents an underrun from advancing the read cursor past the next
speech prefix. Failed streams retire their sources. Following the stall
investigation below, fresh audio can reopen a failed stream after a 250 ms
cooldown, without replaying interrupted speech.

A late short-tone capture exposed a roughly 3 ms gap after its first packet.
Startup now primes two writes (about 10 ms) before uncorking, within the same
buffer request. Shorter sources and smaller negotiated buffers start without
having to fill that reserve. The earlier capture and measurements are retained
under `before-startup-reserve/` rather than being treated as the final result.
Repeated capture then exposed a longer gap; inspection found that waiting for
resume acknowledgement could starve feeding. Resume now stays asynchronous, keeping PCM feeding while the
acknowledgement is in flight. Stop/flush acknowledgement still precedes any
replacement PCM. The intermediate capture is in `before-async-resume/`.
Further 20 ms repeats still showed occasional gaps. The WSL launcher therefore
starts at 40 ms, which retained all six short tones in the subsequent capture.
The general native CLI default remains 20 ms, and either setting is explicit
and adjustable. This does not establish freedom from underflows.

The local full-profile launchers are now:

| Command beside `evox.bat` | WSL equivalent | Output |
| --- | --- | --- |
| `evox.bat` | Existing Windows session launch | Windows Omnivox/WASAPI |
| `evox-linux.bat` | `~/bin/evox-linux` | Linux ALSA/PulseAudio, 50 ms request |
| `evox-linux-pulse.bat` | `~/bin/evox-linux-pulse` | Linux native PulseAudio, 40 ms WSL request |

Both Linux full-profile launchers use Emacs 31 and `~/.emacsvox.d`, including
the saved engine preferences. Native logs have their own directory,
`~/.local/state/emacsvox/omnivox-linux-pulse`. `--diagnose` works from both WSL
and the Windows batch launcher. See [WSL-AUDIO.md](../WSL-AUDIO.md) for commands
and the isolated comparison workflow. Existing user sessions were left running.

## Runtime and measurement scope

The Linux payload was staged using `make build`, Rust 1.97.1, and the locked
workspace at `942b51861a3dc2b92e34506cad4fb00fd4ea794b` plus the existing and new
unreleased changes. It reports version 1.8.0. WSLg is 1.0.73.2 and its
PulseAudio server reports `17.0-25-gc3305`, using `RDPSink`.

The installed Windows payload still reports 1.7.1. It was identified but not
rebuilt or used to claim matched-version latency parity. Runtime paths and
hashes are recorded in the local evidence directory.

There are three different clocks here:

- Omnivox markers and completion describe source consumption.
- PulseAudio buffer sizes, latency estimates and an owned stream's monitor
  describe software output before the WSLg RDP transport completes.
- Physical acoustic onset and stop-to-silence still need a Windows output or
  external capture. Neither of the first two measurements substitutes for it.

PulseAudio documents latency requests as hints and recommends inspecting
actual negotiated values in its
[latency-control guide](https://wiki.freedesktop.org/www/Software/PulseAudio/Documentation/Developer/Clients/LatencyControl/).
The installed headers and upstream
[stream implementation](https://github.com/pulseaudio/pulseaudio/blob/master/src/pulse/stream.c)
also explain why the native launcher removes `PULSE_LATENCY_MSEC`: that variable
overrides application buffer attributes. Direct native selection rejects the
conflict and directs users to `OMNIVOX_PULSE_LATENCY_MS`.

## Results

The final comparison used the same Linux executable for both backends,
with silent active tones, 48 stream observations per backend, and trials after
0.1, 2 and 5 seconds of idle. Other existing user sessions were left intact.

| Observation | ALSA, 50 ms request | Native PulseAudio, 40 ms request |
| --- | --- | --- |
| Active client buffer, median | 37.64 ms | 24.99 ms |
| Active client buffer, observed range | 0–100.00 ms | 0–25.71 ms |
| Shared WSLg sink estimate, median | 1,254.59 ms | 1,253.08 ms |
| Owned process shutdown | Clean | Clean |

This reduces the median client buffer by about 13 ms in this run. The much larger
shared sink estimate persists. Earlier experiments reported different WSLg
sink estimates, so these figures are neither a stable transport guarantee nor
an acoustic measurement. Native streams were corked after stop; the three
command-to-cork observations were 5.69, 8.61 and 4.61 ms, including `pactl`
inspection overhead. They do not measure when Windows speakers became silent.

A dedicated monitor of the **owned native tone stream only**, at a 40 ms
playback request, captured a long tone stopped after approximately 300 ms,
a 200 ms restart tone, and six 30 ms tones after idle. The long tone's span was
312.81 ms, the restart tone was 199.89 ms, and all six short tones were
29.93 ms. Command-to-first-monitored-data observations ranged from 2.23 to
51.79 ms. Recording requested 5 ms and itself affects PulseAudio scheduling.
This confirms software delivery and retention of the short sounds in that run;
it does not establish physical onset or glitch-free playback. An equivalent ALSA direct
monitor attempt was rejected with `Entity killed`, so there is no matched
monitor-latency comparison from that attempt.

The native stream was then deliberately disconnected through the PulseAudio
API using its recorded owned index. Its queued/future audio retired with a
clear error, the other two lanes remained present, and the server shut down
cleanly, including stop/drain after the disconnection. Missing server, invalid
native latency, and conflicting generic latency settings also failed promptly
with nonzero diagnostic exits. A listening Unix socket that never completed
the PulseAudio handshake failed within 3.04 seconds.

The full personal Emacs configuration completed 240 rapid Dired movements,
three letter checks, and six marked foreground/notification announcements
across Eloquence, DECtalk and eSpeak. Every marked announcement completed and
started progressively at the WSL launcher's 40 ms request. Timing logs still
reported underflows. This is lifecycle and protocol acceptance; listening
quality still needs the user's comparison.

### Load sensitivity

The concurrent run, which overlapped the full-profile test, buffer probing
and Windows compilation, completed without a crash but reported active-source
underflows. The native tone stream reported 24 by its last timing sample; a
stop-to-cork observation reached 70 ms. These outliers remain in the evidence.
An earlier quiet repeat reported no active-source underflows in its timing
samples. The final native buffer probe again reported underflows (23 observed
while a source was active), so neither request is claimed to be glitch-free.

Separate quiet four-second tests at 20, 30 and 40 ms requests also reported
zero active-source underflows. Their median client buffers were 15.84, 20.00
and 24.99 ms respectively. Repeated short-tone capture subsequently exposed
gaps at 20 ms; the captured 40 ms short tones were intact. The WSL launcher now
uses 40 ms while the general native backend retains its 20 ms default. Compare
20, 30 and 40 with `OMNIVOX_PULSE_LATENCY_MS` for the actual workload. Larger
requests have not proved freedom from underflows. Existing user sessions and
the observation tools were not isolated from the shared WSLg server.

## Verification and retained evidence

- Locked workspace tests: 628 passed, one existing ignored test.
- Locked workspace/all-target Clippy and CLI/Piper Clippy passed.
- Windows GNU audio-crate and CLI/Piper cross-checks passed; no Windows
  deployment was performed.
- Formatting and documentation-link checks passed.
- Seventeen WSL launcher/isolation tests passed, including native launch and
  reporting without ALSA plugins.
- Native regression tests cover bounded writes, drain/cork/resume, immediate
  replacement after stop, independent lanes, selective cancellation and cues,
  progressive prebuffering, stalled producers, failure and teardown. Stopping
  and draining an already-retired output cannot leave a waiter stranded.

Local, ignored evidence lives under `target/linux-pulse-20260907/`: build and
check logs; `provenance.json`; final `buffers.json`; the preserved
`concurrent-workload/` run; `tuning.json`; the owned monitor PCM and
`monitor-waveforms.json`; failure checks; and the full-profile Emacs proof,
results and owned speech logs. Probe scripts and the small owned-stream
failure injector are retained there too. They contain no redistributed engine
libraries. Intermediate captures are retained in `before-startup-reserve/`,
`before-async-resume/`, and `native-20ms/`. The retirement fix was covered by
the regression suite and the real disconnection/stop/shutdown check. Earlier
buffer measurements retain their measured executable identity separately.

## Foreground stall investigation, 7 September 2026

The subsequent user trial lost all three foreground streams to operation
timeouts at 06:17:33 UTC. Omnivox continued accepting commands, but every
foreground playback attempt failed. Notification speech still completed at
06:17:41. Playback errors then incorrectly opened runtime health circuits for
Eloquence, DECtalk and eSpeak. This explains the apparent hang and subsequent
voice fallback; it does not establish why the audio server stopped replying.
The logged loss-of-focus announcement followed the initial stream failures.

Six hundred stop/start cycles with varied pauses did not reproduce the original
trigger. An isolated Unix proxy then withheld replies from WSLg for four
seconds, only for a test Omnivox process. The old executable lost all three
streams permanently, with the same timeout messages. The proxy test disables
shared-memory transport in its own client configuration; it changes no user
or server settings and does not establish a shared-memory fault.

The fix separates a connection's cancellation lifetime from backend shutdown.
A failure discards its current and queued sources; after a 250 ms admission
cooldown, a new request can reopen that lane on the existing output worker.
There is no replay or idle reconnect loop. Cancellation remains responsive
while connecting, and shutdown remains final. Diagnostics identify the
operation, its state, callback result and whether its deadline expired.

Separately, routed progressive synthesis records errors returned by its output
consumer. These errors terminate the utterance without quarantining an engine
or attempting another voice through the same failed output. This correction
applies to all output backends, including the older ALSA path.

The staged fixed executable reopened speech, tone and sound streams after the
four-second reply stall, then reopened all three again after forced connection
loss. Each fresh stream was observed uncorked and both runs shut down cleanly.
The full-profile native Emacs check passed 240 paced Dired movements, three
letters and twelve tracked announcements across Eloquence, DECtalk and eSpeak,
including foreground/notification speech after four-second idle periods.
No output errors or engine circuit openings occurred in that check.
The same full-profile check also passed through the original ALSA backend:
240 paced movements, three letters and twelve completed announcements, with
no output errors or engine circuit openings.

The locked workspace suite passed 631 tests with one existing ignored test.
Locked workspace and Piper Clippy checks, Windows GNU CLI compilation, formatting
and complete Linux payload staging also passed. Retained local evidence is in
`target/linux-audio-stall-20260907/`, including before/fixed executable hashes,
proxy fault injection, test reports and isolated full-profile results.
The earlier ALSA symptoms and the trigger for the user's original stall remain
unconfirmed; this fixes the observed permanent failure and its engine-health
cascade, without claiming that WSLg cannot pause again.

## Next decision

Compare the three launchers by listening, especially first speech after idle,
short letters, rapid navigation and overlapping notifications. Keep the native
backend opt-in while collecting physical-output evidence and load behaviour.
If WSLg RDP remains the dominant delay, the next experiment is an optional
Windows WASAPI PCM-output helper with synthesis still on Linux. That bridge
is not part of this change.
