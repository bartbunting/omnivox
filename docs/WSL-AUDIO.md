# Compare Windows and Linux speech under WSLg

This development workflow makes the [initial WSLg experiment](experiments/2026-09-06-wslg-audio.md)
repeatable from a checkout. It prepares two opt-in Emacsvox session launchers
and a Linux buffer/shutdown probe. An opt-in native PulseAudio backend is also
available in development builds. Acoustic latency measurement and selection of
a new default audio backend remain future work.

## Full personal configuration: native PulseAudio trial

The local full-configuration launchers beside `evox.bat` are:

| Windows command | Speech output |
| --- | --- |
| `evox.bat` | Existing Windows Omnivox |
| `evox-linux.bat` | Linux Omnivox through ALSA/PulseAudio, 50 ms request |
| `evox-linux-pulse.bat` | Linux Omnivox directly through PulseAudio, 40 ms WSL request |

From WSL, use `~/bin/evox-linux-pulse` (or add `-nw`). It loads the same full
personal configuration as `~/bin/evox-linux`, including the saved voice policy.
Use `~/bin/evox-linux-pulse --diagnose` to verify selection, or
`~/bin/evox-linux-pulse --backend --engine eloquence --check` for an exact-engine
audible check. Its separate logs live in
`~/.local/state/emacsvox/omnivox-linux-pulse`. Existing sessions retain the
backend with which they started; open a new trial session to test this build.

The native client needs the installed `libpulse.so.0` and the WSLg socket,
without the ALSA PulseAudio plugins. The WSL launcher requests 40 ms, submits chunks of about
5 ms, and corks streams after they drain. Stream-wide stop flushes the selected
lane's queued PulseAudio audio. Selective cancellation does not flush unrelated
announcements. Speech, tones and sound icons remain independent, as do the
foreground and notification processes. Native timing diagnostics report actual
buffer attributes and underflows; the requested latency is not a guarantee.

To compare the more aggressive general native-backend default:

```sh
OMNIVOX_PULSE_LATENCY_MS=20 ~/bin/evox-linux-pulse -nw
```

The accepted native range is 10–200 ms. The direct backend defaults to 20 ms;
the WSL launcher starts at 40 because repeated 20 ms short-tone capture exposed
gaps. The launcher removes the
ALSA-specific configuration and inherited `PULSE_LATENCY_MSEC`. Direct CLI use
is `env -u PULSE_LATENCY_MSEC omnivox --audio-output pulse`. Do not use the
existing idle ALSA buffer probe to characterize this backend: idle native
streams are corked. Compare active speech and repeat after idle, including
rapid Dired movement, letters, capitalization cues and overlapping notifications.
The WSLg RDP output remains in the path, so lower client buffering alone cannot
prove Windows-like responsiveness. See the
[native PulseAudio experiment](experiments/2026-09-07-native-pulseaudio.md).

The concurrent test produced underflows, although quiet 20/30/40 ms sustained
trials did not report active-source underflows. Repeated short-tone capture
needed the larger WSL request. Compare 20, 30 and 40 ms for your workload;
the experiment does not promise freedom from underflows. A failed native
PulseAudio connection discards interrupted speech. After a 250 ms cooldown,
fresh speech reopens the affected stream on its output worker. It does not
replay the failed backlog or repeatedly reconnect while idle. This recovery
does not change the existing ALSA backend's device lifecycle.

Reopening a stream requires a responsive server. In the next live trial,
WSLg's shared PulseAudio/RDP bridge stalled and even an independent control
query timed out. Both Linux output choices use that bridge. Check it without
starting speech:

```sh
python3 tools/wsl_audio.py health
```

The query has a three-second deadline and returns nonzero for a timeout,
connection failure, or unavailable `pactl`. The JSON report also includes this
check as `linux_server`; the local full-profile launchers include it in
`--diagnose`. A reachable control connection does not prove audible playback.
A socket's presence alone does not establish server health.

If this independent query times out, changing engines or restarting Omnivox
cannot repair the shared server. Preserve the speech and WSLg logs first.
Resetting the identified WSLg Windows RDP client restored this incident: the
bridge reconnected, and both existing speech processes resumed without an
Emacs or Omnivox restart. Audio-channel setup took about 21 seconds. This
interrupts shared Linux GUI connections and does not prevent recurrence; save
work and coordinate the interruption before attempting recovery. Do not
automatically restart Weston, PulseAudio or WSL from a speech launcher. The
[recurrence investigation](experiments/2026-09-07-native-pulseaudio.md#shared-wslg-bridge-stall-after-the-recovery-fix)
records the blocked server threads and the remaining uncertainty.

For an isolated generated trial below, prefix its Linux launcher with
`OMNIVOX_WSL_AUDIO_OUTPUT=pulse` to select the same native backend.

## Prepare a trial

You need WSLg with its PulseAudio socket, a working Emacsvox checkout and Windows
Omnivox installation, and a staged Linux Omnivox payload. Use `make build` in
Omnivox to build and stage Linux executable data and notices when needed.
The trial tool uses Python's standard library.

Linux also needs the ALSA PulseAudio PCM and control plugins. On Ubuntu these
come from `libasound2-plugins`. The tool looks in ordinary system ALSA plugin
directories. If the plugins are missing, either install the compatible package
through your distribution's package manager or privately extract it and pass
its `alsa-lib` directory with `--alsa-plugin-dir`. The original experiment
documents private extraction. The tool does not install or download packages.

From the Omnivox checkout, choose a **new** trial directory:

```sh
trial_dir="$HOME/.local/share/emacsvox/wsl-audio-trial"
python3 tools/wsl_audio.py prepare "$trial_dir" \
  --emacsvox-dir ../emacsvox \
  --linux-program target/release/omnivox
```

For a privately extracted plugin, append:

```text
--alsa-plugin-dir /absolute/path/to/plugin/usr/lib/x86_64-linux-gnu/alsa-lib
```

Use the package and directory for your distribution and architecture. Preparation
checks that the plugins exist; the real probe and `--check` exercise loading.
It defaults to `unix:/mnt/wslg/PulseServer`; `--pulse-server unix:/absolute/path`
selects another WSLg socket.

Windows selection uses the existing Emacsvox launcher, including its installed
runtime configuration and staged metadata. To test another complete Windows
payload without changing that installation, pass
`--windows-program /mnt/c/path/to/payload/omnivox.exe`. Supply any external voice
or runtime settings that payload needs explicitly.

The generated files reference your checkout, Python interpreter, and payloads
by absolute path. Recreate the trial after moving those inputs. Preparation
refuses to overwrite an existing directory.

## Launch either session

In separate WSL terminals:

```sh
"$trial_dir/emacsvox-windows-test" -nw
"$trial_dir/emacsvox-linux-test" -nw
```

Both use the existing `omnivox` speech-server identity and prefer eSpeak for the
initial comparison. Normal interactive startup can still apply Emacsvox's saved
aural preferences. These are session launchers, not separate persistent
preference stores or new `M-x tts-select-server` entries.

The Linux process tree uses the trial's ALSA configuration, PulseAudio socket,
plugin directory, and 50 ms buffer request. It removes inherited Windows drive,
UNC, executable and DLL settings from the known engine runtime path variables,
including mixed voice-path lists. Compatible Linux paths remain available.
Each platform uses its own log directory inside the trial.

Inspect selection and run an audible exact-engine self-test:

```sh
"$trial_dir/emacsvox-linux-test" --diagnose
"$trial_dir/emacsvox-windows-test" --diagnose
"$trial_dir/emacsvox-linux-test" --backend --engine espeak --check
```

Compare Linux with the default buffer request:

```sh
OMNIVOX_WSL_LATENCY_MS=default "$trial_dir/emacsvox-linux-test" -nw
```

The wrapper accepts `default` or 50–1000 ms. `default` unsets an inherited
`PULSE_LATENCY_MSEC`; it does not pass a numeric zero. Smaller requests stalled
in the initial ALSA experiment. The lower limit is a conservative trial
constraint, not a hardware limit or a public Omnivox CLI setting.

## If eSpeak stops responding during navigation

The first interactive trial exposed an eSpeak cancellation deadlock in 1.8.0:
rapid Dired movement could leave later speech, including letters, queued
indefinitely while the process remained alive. The development fix and its
[interruption checks](experiments/2026-09-07-espeak-interruption.md) are separate
from buffer tuning. Rebuild with `make build` and restart the Linux trial
session to use the corrected executable. An already-running speech process
continues using its old binary until restarted.

## Try Linux DECtalk or Outloud

Native Linux `make build` also stages the
[DECtalk and Eloquence/Outloud interfaces](../linux-helpers/README.md). Restart
the Linux session, then select the engine and voice in
`M-x emacsvox-aural-voice-workbench`. The same Linux launcher handles all these
engines. Saved aural preferences can override its initial eSpeak preference.

Test the installed DECtalk runtime independently first:

```sh
"$trial_dir/emacsvox-linux-test" --backend --engine dectalk --list-voices
"$trial_dir/emacsvox-linux-test" --backend --engine dectalk --voice paul --check
```

Use `--engine eloquence` and `--voice v1` for Outloud. Voxin's standard user
installation is discovered automatically. Its 64-bit `libvoxin.so` wrapper
and bundled legacy engine/voice data must be installed; compilation alone
does not supply them. Exact diagnostics fail if that runtime is absent.
The [local Linux legacy-engine experiment](experiments/2026-09-07-linux-legacy-engines.md)
records DECtalk and Voxin playback/navigation acceptance. The subsequent
[parity work](experiments/2026-09-07-linux-helper-parity.md) adds native markers,
requested anchors and voice-expression controls so anchored announcements can
stay progressive. Both interfaces still have provisional Linux rate mappings.

## Retain measurements

Inspect both binaries before comparing them:

```sh
python3 tools/wsl_audio.py report "$trial_dir"
```

Either launcher also accepts `--report`. The JSON records resolved executables,
versions, hashes, configured output paths, Linux buffer settings, configuration
and plugin hashes, and whether versions match. It does not open an audio
stream. Configured output paths are not confirmation of physical-device
routing. Match source/build provenance, exact eSpeak voice, rate, text, and
final output device before drawing performance conclusions; equal version
strings alone are insufficient.

Repeat the idle Linux buffer probe into a **new** evidence directory:

```sh
python3 tools/wsl_audio.py probe "$trial_dir" "$trial_dir/buffers-01" --samples 4
```

This needs `pactl`. It sequentially opens the default and configured trial
streams, waits for each owned process to appear, settles for 0.5 seconds, and
retains samples at 0.5-second intervals. It queues no speech. Each run closes
stdin with a three-second shutdown deadline; a timeout retires only that
trial's process group and fails the probe. Missing streams or measurement
fields also fail. Evidence includes `runtimes.json`, `buffers.json`, and
per-run stderr files; Emacsvox's normal server diagnostics go to the separate
trial log directories. Keep these machine-specific paths and logs private
unless deliberately preparing a shareable extract.

PulseAudio application-buffer occupancy and sink estimates **are not
command-to-sound or stop-to-silence measurements**. Probe success means stream
observation and bounded shutdown passed, not that speech was heard or that
smaller buffers improve responsiveness.

For speech workloads, reuse the [server lifecycle benchmark](../tools/README.md#server-lifecycle-benchmarks).
The generated launchers forward protocol-tool arguments after `--backend`:

```sh
python3 tools/benchmark_server.py "$trial_dir/emacsvox-linux-test" \
  --server-arg=--backend --engine espeak --expected-engine-id espeak \
  --voice-id espeak:gmw/en-US --mode warm --iterations 5 --warmups 1 \
  --case character --case word --case line --case replacement \
  --json-output "$trial_dir/linux-lifecycle.json"
```

Copy an exact voice ID from `--backend --engine espeak --list-voices` if your
inventory differs. Repeat with the Windows launcher and a separate output file
after matching builds and voice selection. For the default Linux buffer run,
prefix the command with `OMNIVOX_WSL_LATENCY_MS=default`. Retain the runtime
report with each benchmark. Device runs play speech; do not add `--null-audio`
for a responsiveness comparison. Lifecycle markers measure mixer consumption
and protocol cancellation, which can precede physical sound and silence.

Next acceptance work is matched-build listening and physical-output capture
during idle onset, rapid navigation, long-sentence stopping, overlapping
foreground/notification speech, and competing workload. Native PulseAudio
evaluation should use this baseline and preserve the existing bounded mixer
and helper contracts.
