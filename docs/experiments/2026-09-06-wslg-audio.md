# Linux Omnivox audio under WSLg: initial experiment

**Date:** 2026-09-06

**Status:** Local configuration experiment; not a released audio preset or an
accepted backend migration.

**Follow-up:** The [repository trial tool](../WSL-AUDIO.md), added on
2026-09-07, makes launcher preparation and buffer/shutdown probing repeatable.
Interactive testing subsequently exposed an
[eSpeak interruption deadlock](2026-09-07-espeak-interruption.md), fixed in the
development checkout.
The observations below remain the original local experiment.

## Purpose and result

Unresponsive speech from the older Outloud and dtk-soft Emacspeak servers under
WSL was a motivation for Emacsvox and Omnivox. This experiment asks whether
Linux Omnivox can provide responsive speech through WSLg while retaining the
working Windows Omnivox route for comparison. It does not establish the cause
of the historical servers' latency.

The Linux route initially failed because the WSL distribution lacked the ALSA
PulseAudio plugin: Omnivox tried to open nonexistent ALSA card 0. A private
plugin and process-scoped configuration made playback available. A 50 ms
PulseAudio request reduced observed application buffering from 75–100 ms to
25–50 ms. The WSLg RDP sink still reported substantial delay. A 20 ms request
stalled and failed the bounded shutdown check.

Linux 1.8.0 passed synthesis/playback diagnostics and an Emacs check of both
speech lanes. Acoustic onset, stop-to-silence, full interactive acceptance, and
parity with Windows remain unmeasured.

## Environment and provenance

| Component | Recorded value |
| --- | --- |
| Linux | x86_64 WSL2, kernel `6.18.33.2-microsoft-standard-WSL2` |
| WSLg | `1.0.73.2` |
| PulseAudio | `17.0-25-gc3305`, `unix:/mnt/wslg/PulseServer` |
| Destination | `RDPSink`, signed 16-bit stereo 44.1 kHz |
| Application stream | Floating-point stereo 44.1 kHz; reported resampling method `copy` |
| Omnivox | `1.8.0`, commit `942b51861a3dc2b92e34506cad4fb00fd4ea794b` |
| Build | `make build`, pinned Rust `1.97.1`, locked dependencies |
| Output libraries | Rodio `0.19.0`, CPAL `0.15.3` |
| Private plugin package | Ubuntu `libasound2-plugins` `1.2.12-2build1`, amd64 |
| Emacs | Selected Emacs `31.0.90` |
| Windows reference | Existing staged Omnivox `1.7.1`; selection and version checked only |

The Linux executable SHA-256 is
`0c0fcd2d5a1c419c5bbe08a5cca09a563ad3ce2ad3842f5cb7fe4e20ea8e35f1`.
An older Linux 1.7.1 executable was used for preliminary probes, then rebuilt
through `make build`; the measurements below are exclusively from 1.8.0.
The Windows/Linux versions were not matched, so this is not a controlled
cross-platform performance comparison.

The Emacsvox checkout had unrelated concurrent work. The recorded post-check
commit is `96b8e634b25accc99057c9da48c4929991e2a5b4`, not a claim that its source
was frozen throughout the experiment. The experiment changed local trial
files and generated Linux build payloads, not either repository's tracked
implementation.

## Audio path and configuration

Windows Omnivox launched through WSL uses shared-mode WASAPI on Windows. Linux
Omnivox in this trial uses:

```text
Omnivox / Rodio / CPAL -> ALSA PulseAudio plugin -> WSLg PulseAudio
                     -> RDP audio transport -> Windows audio device
```

WSLg provides a PulseAudio server and forwards its audio through RDP. Running
the Linux executable therefore retains a Windows transport/output stage.
[Microsoft's WSLg architecture](https://devblogs.microsoft.com/commandline/wslg-architecture/)
describes that boundary.

The Ubuntu plugin archive was obtained with `apt-get download` and unpacked
with `dpkg-deb --extract` in a private test directory. No package was installed
in the distribution, and no system ALSA configuration or shell startup file
was edited. The archive digest is recorded in the
[measurement extract](data/2026-09-06-wslg-audio/measurements.json). The archive,
plugin binaries, and their original notices remain local; this documentation
does not add them to Omnivox release contents.

The trial uses this [ALSA configuration](data/2026-09-06-wslg-audio/alsa.conf):

```text
pcm.!default {
    type pulse
    hint.description "Omnivox WSLg audio test"
}
ctl.!default { type pulse }
```

The Linux launcher sets these values for its process tree:

| Variable | Trial value or purpose |
| --- | --- |
| `TTS_PROGRAM` | `omnivox`, preserving Emacsvox's existing adapter |
| `OMNIVOX_PROGRAM` | Linux `target/release/omnivox`, with data/notices staged by `make build` |
| `OMNIVOX_ENGINE` | `espeak` as the initial preferred engine |
| `OMNIVOX_AUDIO_OUTPUT` | `device` |
| `PULSE_SERVER` | `unix:/mnt/wslg/PulseServer` |
| `ALSA_CONFIG_PATH` | Private configuration above |
| `ALSA_PLUGIN_DIR` | Extracted package's `usr/lib/x86_64-linux-gnu/alsa-lib` directory |
| `PULSE_LATENCY_MSEC` | `50`, or unset for the baseline |
| `EMACSVOX_PLAY` | Linux `/usr/bin/paplay` for legacy cue-player selection |
| `OMNIVOX_LOG_DIRECTORY` | Separate trial log directory |

Inherited Windows companion paths are removed from the Linux trial's
environment. This prevents, for example, a Windows RHVoice DLL setting from
being treated as a usable Linux runtime. Emacsvox can subsequently apply its
own saved routing preferences; select the same usable voice on both sides for
a comparison.

CPAL 0.15.3's ALSA default requests a 100 ms buffer and approximately 25 ms
periods. PulseAudio's `PULSE_LATENCY_MSEC` override can modify the downstream
stream request. The experiment changes that request, not Omnivox's canonical
PCM or bounded progressive reserve. See the
[pinned ALSA implementation](https://raw.githubusercontent.com/RustAudio/cpal/v0.15.3/src/host/alsa/mod.rs)
and [PulseAudio override implementation](https://raw.githubusercontent.com/pulseaudio/pulseaudio/master/src/pulse/stream.c).

## Measurements and their limits

For each setting, a persistent Linux Omnivox device stream was opened without
queuing speech. Four `pactl --format=json list sink-inputs` snapshots were taken
approximately 0.5 seconds apart, selecting the entry matching the owned
Omnivox process ID. Settings were tested sequentially: unset, 50 ms, then
20 ms. Standard input was then closed, with a three-second shutdown deadline;
only the test process was terminated on timeout.

| PulseAudio request | Application-buffer snapshots | RDP sink estimate | Shutdown |
| --- | --- | --- | --- |
| Unset | 75–100 ms | 81–115 ms | Passed |
| 50 ms | 25–50 ms | 98–110 ms | Passed |
| 20 ms | 0 ms, stalled | 68–80 ms | Timed out; test process terminated |

The [retained measurements](data/2026-09-06-wslg-audio/measurements.json) keep
all twelve samples' measurement values and process exit results unchanged.
Unrelated PulseAudio client/process IDs, user/host names, machine IDs, and
properties are omitted. The extract records the original JSON's SHA-256;
the unmodified original remains in the local experiment directory. These are
introspection snapshots rather than a `benchmark_server.py` benchmark pack.

The microsecond values come from PulseAudio. No independent acoustic clock
or recording was collected. Four snapshots do not establish latency
percentiles, sustained-load stability, underrun rates, or a minimum achievable
latency. Buffer occupancy and sink estimates must not be presented as measured
command-to-sound or stop-to-silence times. A zero application queue during a
stalled run is not successful low-latency playback.

The 20 ms failure is an observed stall and shutdown timeout; its precise cause
was not established. The trial wrapper accepts 50–1000 ms or `default`. Its
50 ms lower limit is a conservative experiment setting, not a hardware limit.

## Launch the prepared local comparison

On the experiment workstation, run these in separate WSL terminals:

```sh
~/.local/bin/emacsvox-windows-test -nw
~/.local/bin/emacsvox-linux-test -nw
```

These are machine-local session launchers, not shipped binaries or new
`M-x tts-select-server` entries. Both retain the `omnivox` server identity.
They can run alongside the normal Windows session. The launchers use
Emacsvox's isolated `-Q` startup, but normal interactive startup can still
load Emacsvox's saved aural preferences; these are not separate persistent
preference stores.

Inspect executable selection or run the audible diagnostic:

```sh
~/.local/bin/emacsvox-linux-test --diagnose
~/.local/bin/emacsvox-windows-test --diagnose
~/.local/bin/emacsvox-linux-test --check
```

Compare the unmodified Linux buffer request:

```sh
OMNIVOX_WSL_LATENCY_MS=default ~/.local/bin/emacsvox-linux-test -nw
```

`OMNIVOX_WSL_LATENCY_MS`, `OMNIVOX_LINUX_PROGRAM`,
`OMNIVOX_WINDOWS_PROGRAM`, and `OMNIVOX_LINUX_ENGINE` are local wrapper
settings, not public Omnivox options. The executable overrides allow a later
matched-version test without replacing the ordinary installed runtime.
For an exact-engine backend diagnostic, each wrapper also supports
`--backend --engine espeak --check`.

The local files, plugin, original observations, and logs are under
`~/.local/share/emacsvox/wsl-audio-test-20260906/`. On another WSL machine,
recreate the private plugin/configuration and per-process environment above,
using its own staged Linux executable and Emacsvox checkout. These aliases
and the original machine's paths cannot be assumed to exist elsewhere.

## Checks completed

- `make build` staged Linux 1.8.0, generated eSpeak data/notices, and the
  standard companion builds using the pinned toolchain and locked resolution.
- `--engine espeak --check` reported successful synthesis, audio-device
  initialization, tone/speech playback, and sound-resource decoding.
- A fresh selected Emacs batch process explicitly initialized TTS and checked
  foreground and notification voice inventories, routing acknowledgements,
  and tracked speech completion. Its
  [result](data/2026-09-06-wslg-audio/emacsvox-smoke.txt) is retained.
- Both launchers passed shell syntax and executable-selection diagnostics.
- Windows executable selection/version was checked. Windows audio timing,
  full interactive Emacs acceptance, and acoustic playback were not measured.

The self-test and source-consumption completion events cannot prove acoustic
onset, final-device drain, or that a listener heard all samples. Verify the
retained extract and configuration from its data directory with
`sha256sum --check SHA256SUMS`.

## Follow-up and acceptance

1. Compare matching Omnivox versions, eSpeak voice, rate, text, and output
   device. Keep Windows available as the working reference.
2. Test rapid line/character navigation, cancellation of a long sentence,
   foreground/notification overlap, idle-to-speech, and ordinary system load.
3. Capture command-to-audible-output and stop-to-silence separately from mixer
   consumption. Retain raw repeated samples and underrun/recovery evidence.
4. Evaluate native PulseAudio for WSLg with coordinated application/server
   buffering. Native PipeWire is a separate ordinary-Linux-desktop candidate;
   it does not remove WSLg's RDP transport.
5. Evaluate the remaining RDP/Windows stage before attributing all latency to
   Omnivox. Do not claim Windows parity from a smaller application buffer.
6. Design maintained Emacsvox launch/profile selection and doctor output so
   users can identify and switch between native Linux and Windows execution
   while retaining the correct Omnivox adapter and platform-specific paths.

CPAL's newer [native PulseAudio/PipeWire support](https://github.com/RustAudio/cpal/releases/tag/v0.18.0)
makes a direct-backend experiment possible. Migrating the pinned stack remains
separate design/dependency work under the accepted ADRs. The
[roadmap](../plans/NEXT_STEPS.md) places this evidence and WSL responsiveness
first in the updated feature priorities.
