# Linux DECtalk and Eloquence/Outloud development trial

**Date:** 2026-09-07

**Status:** DECtalk and Eloquence/Outloud through Voxin work with the user's
installed Linux runtimes.
These are unreleased development changes following 1.8.0.

## Runtime discovery and build

The available DECtalk installation has `/usr/local/lib/libtts_us.so`, matching
headers under `/usr/local/include/dtk`, and
`/opt/dectalk/dic/dtalk_us.dic`. Its source checkout is `~/src/dectalk`.
The runtime reports `v4.99 Github NORMAL ACCESS32 US`. Direct native capture
passed before the Omnivox adapter was added.

The Emacspeak reference servers are under `~/src/emacspeak-reference/servers`;
Emacsvox also retains the old Linux Outloud and software DECtalk adapters.
Initially no `libibmeci.so` or Voxin runtime was installed. The user then
provided their licensed `voxin-enu-3.4.x86_64.tgz`. Its runtime and English
voice packages were extracted privately under `~/.local/share/voxin/rfs`.
The supplied configuration templates generated `eci.ini` with absolute paths
to this installation. No system libraries or Speech Dispatcher settings were
changed, and no licensed runtime files entered the repository or build payload.

This archive contains 64-bit libvoxin 1.6.3, which exports the ECI API and runs
the bundled 32-bit IBM engine through its supplied loader and libraries.
The helper now discovers the standard user installation automatically. Direct
native checks and the matching
[upstream implementation](https://github.com/Oralux/libvoxin/blob/49b02ca8c4411d7ba8592c13684f9084edb2cca4/src/voxind/main.c#L505)
confirmed that this version executes clear-input but omits its result from
the reply. The adapter tolerates that false result only when `voxGetVersion`
reports 1.6.3; it still checks stop and synthesis results. A regression test
retains failure for an unverified wrapper version.

`make build` stages two separate GPL-2.0-or-later helper executables alongside
the Linux server. They use the existing bounded helper protocol, dynamically
load the selected native library, and leave runtime files in their installed
locations. See the [Linux helper guide](../../linux-helpers/README.md) for
explicit path overrides, architecture requirements, and capabilities.

## Verification

The host is WSL2 Ubuntu 26.04 x86_64 with glibc 2.43. The existing Linux trial
launcher supplies the ALSA PulseAudio plugin, WSLg's PulseAudio socket, and
the 50 ms buffer request. It still uses the ALSA-to-PulseAudio output path.

- All nine DECtalk voices passed real native progressive synthesis and clean
  helper shutdown. Exact `--engine dectalk --check` completed synthesis and
  device playback with Paul.
- One real helper survived eight syntheses and four interleaved cancellation
  probes, with valid PCM framing and subsequent synthesis.
- The real audio server passed 16 replacement-speech iterations and eight
  hard stops through the Linux trial launcher.
- A separate terminal Emacs session passed 140 rapid/paced Dired arrow keys.
  Six tracked foreground/notification checks completed, including after hard
  stop. Three individually typed letters completed with DECtalk Paul, and
  Dired's directory profile synthesized with DECtalk Harry. Intermediate
  navigation speech was intentionally replaceable. The test session was
  closed afterward; the user's existing session was not changed.
- C ABI stub tests for both helpers exercise progressive framing,
  cancellation followed by synthesis with every advertised voice, missing
  runtimes, invalid callback lengths, protocol health, and clean shutdown.
  Unit tests cover Latin-1 rejection, mismatched native architecture, and
  cancellation while the native PCM queue is full. ECI's stub also verifies
  that owner operations remain on the thread that created its handle.
- After installation, all eight Eloquence presets passed real progressive
  synthesis. The `v1` and `v2` presets produced distinct WAV samples. A real
  helper passed eight syntheses and four cancellation probes; the audio server
  passed 16 replacement iterations and eight hard stops. Exact Eloquence
  diagnostics completed WSLg device playback.
- A fresh Emacs session using Eloquence passed another 140 Dired arrow keys,
  all six tracked foreground/notification checks, three individually typed
  letters, and speech after hard stop. Logs confirmed engine `eloquence` and
  voice `v1` for letter feedback. The test session was closed afterward.

Locked workspace tests and Clippy, the CLI's Piper feature Clippy check,
formatting, and WSL trial regression checks passed. Local raw evidence is
under `target/linux-legacy-probes` and `target/voxin-runtime-probes`; the
trial's separate Linux logs retain the Emacs speech-process results.

## Remaining acceptance

Only the installed DECtalk and Voxin builds have live runtime coverage. Other
runtime versions and Linux architectures remain unverified. Initial English
adapters expose rate, pitch, volume, progressive PCM, and cancellation;
native markers, requested anchors, and other ACSS dimensions are not yet
advertised. Linux rate mappings are provisional pending a retained audit.

Playback completion and mixer/protocol timing do not measure physical
command-to-sound or stop-to-silence latency. Listening comparisons and physical
capture remain part of the [WSLg experiment](../WSL-AUDIO.md). This work does
not establish the cause of the historical Outloud/dtk-soft WSL delays.
