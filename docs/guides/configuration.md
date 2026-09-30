# Omnivox Configuration

The command line configures a standalone server process. Runtime protocol
commands configure a process already launched by Emacs. Emacsvox and upstream
Emacspeak use different Lisp adapters; their variable names are documented
separately below.

## Command-line options

`omnivox --help` is the authoritative syntax summary. The supported value
ranges below are the values callers should supply; the parser rejects malformed
numbers but does not currently reject every out-of-range value before it reaches
the selected backend.

| Option | Meaning |
|---|---|
| `--help`, `-h` | Show help. |
| `--version`, `-V` | Print the workspace version. |
| `--check` | Run the diagnostic self-test; inspect each printed status and, with device output, confirm that its tone and speech are audible. |
| `--list-voices` | Print voices for the selected startup engine. |
| `--list-voices-alist` | Print the same list as Emacs-readable data. |
| `--engine NAME` | Prefer a registered external ID or `native`, `espeak`, `piper`, `rhvoice`, `flite`, `rutts`, or experimental `tgspeechbox`/`mbrola`; Windows also accepts `winrt`, Windows/Linux accept configured `eloquence` and `dectalk` helpers, and macOS accepts `macos`. Diagnostic actions select an explicit name exactly. |
| `--list-espeak-variants` | Silently list base voices and bundled variants from the selected speech host as JSON; see [eSpeak variants](../engines/espeak-variants.md). |
| `--voice ID` | Set the startup physical voice; copy an exact ID from `--list-voices`. |
| `--rate FLOAT` | Set normalized startup rate from 0.0 through 2.0; 0.5 targets the calibrated normal reference speed. |
| `--pitch FLOAT` | Set pitch multiplier from 0.5 through 2.0. |
| `--voice-volume FLOAT` | Set speech gain from 0.0 through 1.0. |
| `--tone-volume FLOAT` | Set tone gain from 0.0 through 1.0. |
| `--sound-volume FLOAT` | Set sound/icon gain from 0.0 through 1.0. |
| `--audio-target TARGET` | Route to `left`, `right`, or `both`. |
| `--audio-output MODE` | Use `device` (the default), opt-in `pulse` on Linux, or `null`; null consumes audio without opening a device or waiting for real-time playback. |
| `--piper-model PATH` | Supply a Piper `.onnx` model for the server or diagnostic actions. |
| `--config-dir PATH` | Select one absolute native directory containing optional `config.json` and `helpers.d/` manifests. |
| `--dump-wav VOICE OUTPUT [TEXT]` | Synthesize a canonical diagnostic WAV and a raw intermediate WAV. |
| `--play-wav FILE` | Play a WAV through the Omnivox audio path. |

Without an action option, Omnivox starts the stdin protocol server. Protocol
rate commands conventionally use Emacspeak's integer scale. Omnivox divides
values greater than 1 by 100 and then clamps the normalized value to `0.0..2.0`
(`50` becomes `0.5`, `150` becomes `1.5`, and `300` becomes `2.0`). Higher
values request faster speech; individual engines may impose a lower maximum.
Measured engines use monotonic per-engine calibration rather than treating
unrelated native midpoints as equal. A rate stops accelerating when an engine
reaches its real native limit. See [rate-calibration.md](../reference/rate-calibration.md)
for reference voices, audit procedure, and limitations.

An invalid `--audio-target` is logged and leaves the default `both` routing in
place. In server mode an unavailable initial preference can fall through the
registered engine order. Single-action diagnostics select an explicit engine
exactly: unknown or unavailable engines fail rather than silently reporting
measurements from a fallback.

`--check` exits nonzero if engine creation, synthesis, audio initialization,
processing, queueing, or an attempted sound-file check fails. Missing optional
test sound files remain harmless. Device output still needs an audible check.
`--dump-wav` also exits nonzero on synthesis, processing, or file-write failure.
It writes canonical audio to `OUTPUT` and a raw intermediate beside it, with
`_raw.wav` appended to the output filename stem. Both `--check` and `--dump-wav`
honor `--voice`, `--rate`, `--pitch`, `--voice-volume`, and `--piper-model`.
`--check` also honors `--audio-output`; null mode reports generated tone and
speech as consumed rather than audible.
`--play-wav` also honors `--audio-output` and waits for the file to finish;
`--check` waits for its queued tone and speech. Neither uses a fixed playback
timeout. Null output consumes the audio without waiting for its duration.
For `--dump-wav`, a nonempty positional `VOICE` takes precedence over
`--voice`; pass an empty positional string to use the flag or engine default.

## Local engine configuration

Standalone startup and exact diagnostic actions read configuration versions 1–3.
Version 2 is available from Omnivox 1.14.0; version 3 adds punctuation tables
in Omnivox 1.15.0.
Choose one root using `--config-dir`, then nonempty `OMNIVOX_CONFIG_DIR`, then
the platform default: `%APPDATA%\omnivox` on Windows,
`$HOME/Library/Application Support/Omnivox` on macOS, or
`$XDG_CONFIG_HOME/omnivox` (otherwise `$HOME/.config/omnivox`) on other Unix hosts.
Explicit roots must exist; an absent default root means no user settings.
Roots and nonempty platform directory variables must be absolute native paths.

The minimal `config.json` is `{"schema":1}`. Its optional `routing` object
supports `preferred_engine_ids`, `fallback_engine_ids`, `disabled_engine_ids`
and `automatic_engine_ids`. An explicitly empty preferred list keeps ordinary
startup defaults while preserving an empty policy list. Local disablement remains
effective after a client replaces its session routing policy.

Each direct `helpers.d/*.json` file registers one external helper using `schema:1`,
`engine_id`, an absolute `program`, and optional literal `arguments`, `enabled`
and `timeouts`. Shipped IDs cannot be shadowed. `engine_overrides` in the main
configuration can modify registered launch fields; existing CLI and engine-specific
environment overrides retain priority. In-process engines accept only `enabled`,
and managed voice-library arguments cannot be replaced.

External engines can be named by `--engine`, exact voice selectors or policy
lists. They enter unrestricted property matching only when locally named in
`automatic_engine_ids`; that permission does not choose a startup engine.
Edits apply to new processes, and recovery uses the retained launch definition.
Malformed main configuration rejects startup before native construction. Invalid
optional manifests are diagnosed independently unless a main override requires
the failed registration.

External startup has four admission slots and one 120-second batch budget,
independent of shipped engines. A timed-out attempt stays unavailable and keeps
its engine owner and slot while a launch, I/O or cleanup call remains unfinished.
Explicit retry waits for that attempt to finish and confirms cleanup before
replacement. A late descriptor cannot become available after startup returns.

See the [version-1 contract](../reference/engine-configuration.md#configuration-version-1)
for examples, precedence and bounds. Local owners retain complete startup records;
their workers expose negotiated `engine_configuration_status_v1` with activation
identity and configuration sources. Emacsvox prepares one shared record and
checks both workers' acknowledgements; recovery retains that activation. Remote
hosts freeze the record per authenticated session. Native platform qualification
is separate from framework tests.

### Change the speech chunk size

Omnivox normally sends at most 15 words to an engine at a time. To allow longer
phrases, use version 2 in `config.json`, for example:

```json
{
  "schema": 2,
  "speech": { "max_chunk_words": 30 }
}
```

Keep any existing routing and engine overrides in the same file. The allowed
range is 1–100 words. Smaller chunks can produce the first result sooner but
introduce more breaks; larger chunks can improve phrasing but increase the wait
for a result. Omnivox still prefers sentence and clause boundaries. The default
remains 15, and the 15-second silence limit is separate and fixed.

Restart speech deliberately to apply an edit. Both lanes use the same value,
and a worker recovering after failure keeps its previous value. A normal client
speech reset also keeps it. This setting does not divide `--dump-wav` output.
See the [exact rules](../reference/engine-configuration.md#configuration-version-2).

### Save your usual speech settings

Add a `defaults` object inside `speech` in version-2 `config.json`, for example:

```json
{
  "schema": 2,
  "speech": {
    "defaults": {
      "rate": 0.7,
      "voice_volume": 0.8,
      "tone_volume": 0.2,
      "punctuation": "some"
    }
  }
}
```

You can also save voice, pitch, sound volume, CamelCase splitting and character
speed. Keep existing engine and chunk settings in the same file. See the
[field names and ranges](../reference/engine-configuration.md#saved-speech-defaults).
Rate uses Omnivox's `0.0`–`2.0` scale: write `0.7`, not `70`.

Restart speech to load the file. Command-line settings can override it at startup,
and Emacs can change settings during the session. When Emacs resets speech,
Omnivox returns to your saved speech defaults. Later Emacs commands can change
them again. Editing the file alone does not change active or recovering workers.
Omitted settings retain their built-in values. Output choices have their own
`audio` section below.

### Punctuation pronunciations

In a supporting Emacsvox checkout, run `M-x omnivox-punctuation`, or choose
**Punctuation** in Aural Home's **Voices and speech** group. Choose a level with
`l`; `RET` edits a pronunciation, `a` adds a character, and `d` restores its
default. The host supplies the table, so the editor shows the defaults of the
selected executable. `s` saves all draft levels; `r` separately asks to restart
both speech workers. `q` retains your draft for later. File conflicts leave
edits available for review. This uses the existing bundled local management
provider; remote/native-direct editing is unavailable. The screen describes
saved settings, which may differ from active workers.

The punctuation extension uses version 3 of the same `config.json`.
Merge the following into your existing file, preserving any other settings:

```json
{
  "schema": 3,
  "speech": {
    "punctuation": {
      "some": { "'": "apostrophe", "’": "apostrophe" },
      "all": { "!": "exclamation mark" }
    }
  }
}
```

Omnivox 1.15.0 names straight, curly and modifier apostrophes at `all` by default;
no configuration file is needed for that fix. At `some`, they remain available
for natural pronunciation. The example opts into naming straight and right curly
apostrophes at `some` too, including inside contractions. Emacsvox uses `some` by
default in Org. To leave a character
to the engine, use null, for example `"$": null`. Omitted characters keep their
defaults, so you need only list changes. Each level is independent.

Edit the file on the machine running Omnivox. A Windows worker launched from WSL
uses the Windows configuration root. Restart both speech workers deliberately,
or use the existing coordinated Apply operation, to capture edits. Reset and
failure recovery keep the captured tables. Omnivox 1.14.0 and earlier do not
support this format. See the [complete tables and rules](../reference/engine-configuration.md#punctuation-tables).
Custom named profiles are the deferred second stage in the
[delivery plan](../plans/punctuation-configuration.md).

### Adjust the capital-letter cue

When reviewing characters, Omnivox normally speaks capitals at pitch `1.5`.
You can save another value and make exceptions for particular engines:

```json
{
  "schema": 2,
  "speech": {
    "capital_pitch": {
      "default": 1.5,
      "engines": { "espeak": 1.3, "piper": "off" }
    }
  }
}
```

These are illustrative values. Numbers from `0.5` to `2.0` set the capital's
pitch directly; they do not multiply your ordinary pitch. Use `"off"` to keep
ordinary pitch for capitals too. An engine without an override uses `default`.
User-added engines can have overrides under their registered IDs.

Keep this alongside existing settings in the same file, then restart speech.
Reset and recovery retain it. This controls character navigation; capitalization
announcements and tones within words or sentences keep their separate controls.
See the [exact rules](../reference/engine-configuration.md#capital-letter-pitch).

### Save audio output choices

Add `audio` alongside `speech` in version-2 `config.json`:

```json
{
  "schema": 2,
  "audio": {
    "backend": "device",
    "target": "both"
  }
}
```

`device` plays through your system's default output. On Linux, `pulse` selects
the existing native PulseAudio output. `null` is silent and useful for tests.
Choose `left`, `right` or `both` for the channel. These choices apply to speech,
tones and sounds from that process.

On Windows, `device` follows the system's default playback device, including
headphones selected after speech starts. Switching cancels interrupted speech
and queued sounds. Fresh speech uses the new output once it is ready; speech
engines remain running. If no output is available, new speech is discarded
instead of playing later. Recovery retries are bounded, and a device change or
fresh speech can trigger another attempt. Initial startup still requires a
working output device. This behavior is available from Omnivox 1.14.0.

PulseAudio also accepts `"pulse_latency_ms": 20`, from 10 to 200 milliseconds.
This requests buffering; it does not guarantee how soon sound reaches your ears.
Keep the default unless you have a reason to change it.

Command-line choices override launcher environment settings, which override
this file. Normal speech and notifications can therefore keep separate channels.
Reset restores each worker's startup channel, and recovery retains its output
settings. Restart speech deliberately to apply file edits. See the
[exact rules](../reference/engine-configuration.md#audio-output-settings).

### Add your own speech engine

Omnivox 1.13 adds registration for independently installed helpers. You need a
helper that speaks the [Omnivox helper protocol](../protocols/helper.md), plus
any runtime and voices it requires. An ordinary speech-engine DLL or NVDA
add-on is not itself an Omnivox helper. Registration does not install these files.

The following Windows example uses an illustrative helper called `wintalker`.
Replace its path and ID with those supplied by the helper's author.

1. Create `%APPDATA%\omnivox\helpers.d` if it does not exist. If you selected
   another root with `--config-dir` or `OMNIVOX_CONFIG_DIR`, use that root instead.
   When Windows Omnivox runs from WSL, use the Windows directory and Windows
   executable path, not the Linux configuration directory.
2. Save the following as `wintalker.json` in `helpers.d`. Use UTF-8 JSON and
   keep the doubled backslashes in the program path:

   ```json
   {
     "schema": 1,
     "engine_id": "wintalker",
     "program": "C:\\Users\\me\\speech\\omnivox-wintalker-helper.exe",
     "timeouts": {
       "synthesis_idle_ms": 60000
     }
   }
   ```

   The helper must report the same engine ID. Do not borrow a shipped ID such
   as `rutts`. Only register a helper you intend to run: Omnivox starts the
   program to discover its voices even before you select one for speech.
3. In PowerShell, use your new Omnivox executable to check the version and list
   that helper's voices. Replace the example executable path:

   ```powershell
   $omnivox = "C:\Speech\Omnivox\omnivox.exe"
   & $omnivox --version
   & $omnivox --engine wintalker --list-voices
   ```

   Copy a voice ID from the result, then test synthesis with that exact voice:

   ```powershell
   & $omnivox --engine wintalker --dump-wav "VOICE_ID" "$env:TEMP\omnivox-helper.wav" "Testing my speech engine."
   ```

   Open the resulting WAV file in an audio player to check what you hear.
   Successful file generation alone does not confirm audible voice quality.
4. Restart your speech server deliberately. For Emacsvox, use a client revision
   with engine-configuration support and restart the Emacsvox session so both
   foreground and notification speech receive the new settings. Select the new
   engine's voice through the client's voice selection interface. Existing
   processes keep their original configuration, including during recovery.

No `config.json` is needed just to register the helper. Registration alone does
not make it the default voice. For a standalone server, select it with
`--engine wintalker --voice VOICE_ID`; see the local routing settings above for
saved engine preferences. Client routing settings can also affect selection.

On Linux or macOS, use the configuration root listed above, a native absolute
program path, and the same manifest fields. Do not use shell variables such as
`$HOME` or `%APPDATA%` inside the JSON program path; they are not expanded.

If the helper does not appear, repeat the exact `--engine ... --list-voices`
command and read its error output. Check the configuration root, JSON spelling,
absolute executable path, matching engine ID, and required runtime installation.
A conflicting ID or invalid manifest is rejected. Timeouts outside the allowed
ranges are also rejected; the [configuration reference](../reference/engine-configuration.md#helper-manifest)
lists the bounds. To stop loading the helper, set `"enabled": false` in its
manifest and restart the speech session.

## Server environment

### Engine selection

`OMNIVOX_ENGINE`

- `native` or empty selects the platform default (`macos`, `winrt`, or eSpeak
  where no native engine exists).
- `espeak` selects eSpeak NG as the startup engine.
- `piper` selects the optional helper-backed Piper engine and requires a
  Piper-enabled build plus a model.
- `rhvoice` selects the helper-backed, user-installed RHVoice runtime.
- `flite` selects the source-built Flite companion and its compiled-in SLT
  voice.
- `rutts` selects the source-built RuTTS companion and its built-in Russian
  voices.
- `tgspeechbox` selects the experimental source-built TGSpeechBox formant
  companion.
- On Windows, `eloquence` and `dectalk` select their adjacent or explicitly
  configured helper and user-installed runtime; `winrt` explicitly selects the
  native engine.
- On macOS, `macos` explicitly selects AVSpeechSynthesizer.
- Equivalent startup option: `--engine`.

In server mode, the selected startup engine controls the initial preference;
it does not remove other available engines from inventory. Windows registers
WinRT and eSpeak plus adjacent or explicitly configured Eloquence and DECtalk
helpers. macOS registers AVSpeechSynthesizer and eSpeak, while Linux registers
eSpeak. Staged or explicitly configured RHVoice, Flite, RuTTS, and
TGSpeechBox companions register on every desktop platform. A build with Piper
support also registers Piper when `OMNIVOX_PIPER_MODEL` or `--piper-model`
supplies a model. Single-action diagnostics such as `--list-voices` continue
to create only the selected engine. An explicit diagnostic selection fails
when that exact engine is not available.

### RHVoice helper and runtime

`OMNIVOX_RHVOICE_HELPER`

- Optional path to `omnivox-rhvoice-helper`.
- Otherwise Omnivox first checks the `rhvoice/` directory beside itself, then
  accepts the legacy layout with the helper directly beside it.

`OMNIVOX_RHVOICE_LIBRARY`

- Absolute path to the user-installed RHVoice C API library. It overrides
  restricted platform discovery and is required on Windows.

`OMNIVOX_RHVOICE_DATA`

- Optional absolute RHVoice data directory containing installed languages and
  voices.

`OMNIVOX_RHVOICE_CONFIG`

- Optional absolute RHVoice configuration directory.

`OMNIVOX_RHVOICE_RESOURCES`

- Optional platform-separated list of absolute additional language/voice
  resource directories.

See [rhvoice.md](../engines/rhvoice.md) for compatible versions, installation paths,
platform status, and verification.

`OMNIVOX_FLITE_HELPER`

- Optional path to `omnivox-flite-helper`.
- Otherwise Omnivox checks `flite/` beside itself and then the legacy adjacent
  location.

`OMNIVOX_FLITE_VOICES`

- Optional platform-separated list of absolute `.flitevox` file paths (`:` on
  Linux/macOS, `;` on Windows).
- Only English Clustergen voices compatible with Flite v2.2 can load in the
  SLT-only companion. Invalid entries degrade the engine but do not remove the
  built-in `cmu_us_slt` voice.

See [flite.md](../engines/flite.md) for installation, build, voice-file, verification,
and licensing details.

`OMNIVOX_RUTTS_HELPER`

- Optional path to `omnivox-rutts-helper`.
- Otherwise Omnivox checks `rutts/` beside itself and then the legacy adjacent
  location.

The source-build wrapper additionally accepts `OMNIVOX_RUTTS_INPUTS_DIR` as a
verified-cache override. Advanced direct Cargo builds use
`OMNIVOX_RUTTS_SOURCE_DIR` to name an already verified RuTTS v6.3.3 source
tree; this is a build input, not a server runtime setting.

See [rutts.md](../engines/rutts.md) for installation, source build, text repertoire,
pronunciation, verification, and licensing details.

`OMNIVOX_MBROLA_HELPER`

- Absolute path to the privately staged [MBROLA prototype](../engines/mbrola.md)
  helper. Explicit opt-in only; it has no adjacent discovery or generic release
  payload. Each lane uses its own helper and the helper's adjacent manifest.

`OMNIVOX_TGSPEECHBOX_HELPER`

- Optional path to `omnivox-tgspeechbox-helper` (with `.exe` on Windows).
- Otherwise Omnivox checks `tgspeechbox/` beside itself and then the legacy
  adjacent location.

`OMNIVOX_TGSPEECHBOX_DATA`

- Optional absolute directory containing `packs/phonemes.yaml` and
  `packs/lang/default.yaml`.
- The staged helper normally finds these packs beside itself.

`OMNIVOX_TGSPEECHBOX_SAMPLE_RATE`

- Selects TGSpeechBox's native DSP rate: `44100` (the default) or experimental
  `22050` for controlled latency and audio-quality comparisons.
- The companion contains a validated inventory for each rate. Changing the
  value takes effect after restarting the speech server; no rebuild is needed.

The source preparer accepts `OMNIVOX_TGSPEECHBOX_INPUTS_DIR` as a verified
cache override. Advanced direct Cargo builds use
`OMNIVOX_TGSPEECHBOX_SOURCE_DIR` to name the verified pinned source tree;
these are build inputs rather than server settings. See
[tgspeechbox.md](../engines/tgspeechbox.md) for the experimental Windows x64 build,
profiles, controls, and limitations.

`OMNIVOX_PIPER_MODEL`

- Path to a Piper `.onnx` model. A matching configuration must be adjacent as
  either `<model>.onnx.json` or `<model>.json`.
- Overridden by `--piper-model` for the server and voice-list actions. The
  current `--check` and `--dump-wav` actions use this environment variable
  instead of the option.

`OMNIVOX_PIPER_HELPER`

- Optional path to `omnivox-piper-helper`.
- Otherwise the server first looks in a `piper/` companion directory beside
  its own executable, then accepts the legacy layout with the helper directly
  beside it.

`OMNIVOX_PIPER_ESPEAK_DATA`

- Optional Piper-specific path to `espeak-ng-data/` or its parent directory.
- Read by the Piper engine inside `omnivox-piper-helper`; the main server
  inherits this environment into the child. The helper otherwise prefers the
  companion data beside itself before checking `ESPEAK_NG_DATA`, its build-time
  path, and compatible system data.

`ESPEAK_NG_DATA`

- Parent directory containing `espeak-ng-data/phontab`, not the
  `espeak-ng-data` directory itself.
- The eSpeak TTS backend checks this value first, then an `espeak-ng-data`
  directory beside the executable, its staged Cargo-profile path, and common
  system data locations. The Piper helper also accepts this parent-directory
  convention after checking `OMNIVOX_PIPER_ESPEAK_DATA`. The TGSpeechBox helper
  uses the same convention after first checking its own companion data.
- Supported local builds and generic GitHub release archives package matching
  data beside the executable, so this variable is normally unnecessary for
  those layouts. Keep the packaged directory adjacent when relocating the
  binary.
- The Emacsvox WSL launcher forwards this value for its content-addressed
  staged Windows runtime.

`OMNIVOX_ESPEAK_VARIANTS`

- Legacy optional inventory rows for bundled [eSpeak variants](../engines/espeak-variants.md).
- Each entry has exact `base_voice_id`, `variant_id` and Boolean `enabled`.
  New workers retain enabled rows, but valid combinations are available on demand
  regardless of this list. Ordinary preview and palette use need no setting or restart.

### Audio routing

`OMNIVOX_AUDIO_TARGET`

- `left`, `right`, `both`, or empty; empty means both channels.
- Applies to every output stream owned by that process.
- Equivalent startup option: `--audio-target`.
- Notification isolation uses a second process with its own value; Omnivox has
  no hidden notification stream inside one process.

### Audio output backend

`OMNIVOX_AUDIO_OUTPUT`

- `device` uses the default operating-system audio device and is the default.
- `pulse` uses native PulseAudio on Linux, with independent speech/tone/sound
  streams. It needs system `libpulse.so.0` and a reachable PulseAudio-compatible
  server. Other platforms reject this selection. See [wsl-audio-comparison.md](wsl-audio-comparison.md).
- `null` opens no audio device and consumes queued speech, tones, sounds,
  playback cues, and tracked completions as quickly as possible.
- Equivalent startup option: `--audio-output`; the command-line value takes
  precedence over the environment.
- Null output exercises synthesis, the canonical audio pipeline, queueing,
  marker delivery, and completion plumbing. It does not exercise device
  buffering, real-time underruns, audible quality, or acoustic onset. Terminal
  latency from a null run is therefore not comparable with device playback.

`OMNIVOX_PULSE_LATENCY_MS`

- Native `pulse` backend only: integer 10–200 ms, default 20. Requests total
  sink-plus-stream latency; the server may negotiate different buffer sizes.
  The WSL comparison launcher starts at 40 ms unless explicitly overridden.
- Writes are at most approximately 5 ms. Idle streams drain and cork. A
  stream-wide stop discards local PCM and flushes that PulseAudio stream;
  selective cancellation never flushes unrelated requests.
- Unset `PULSE_LATENCY_MSEC` for native output: libpulse uses that variable to
  override application buffer attributes. The WSL native launcher removes it.
- `PULSE_SERVER` selects the native server; WSLg commonly uses
  `unix:/mnt/wslg/PulseServer`. No server is automatically started.
- Rate-limited informational logs include timing availability, the server
  latency estimate, total underflows and underflows observed while a source is
  active. End-of-source
  underflows can be normal; these counters are not acoustic measurements.

### Diagnostics

`OMNIVOX_LOG_SYNTHESIS_TEXT`

- Values `1`, `true`, `yes`, or `on`, ignoring case and surrounding whitespace,
  opt in to full synthesis-text logging.
- Disabled by default. Text may contain passwords, messages, documents, and
  other private content.

The following variables belong to the Emacsvox launcher rather than the Rust
process:

`OMNIVOX_PROGRAM`

- Absolute path to the Omnivox executable the launcher should run.
- Takes precedence over the content-addressed Emacsvox runtime and `PATH`.

`OMNIVOX_LOG_DIRECTORY`

- Linux directory for private stderr logs.
- Defaults to `$XDG_STATE_HOME/emacsvox/omnivox`, or
  `~/.local/state/emacsvox/omnivox` when `XDG_STATE_HOME` is unset.
- A session normally uses numbered `omnivox-...-partNNNNNN.log` files. The
  launcher falls back to one unnumbered log if its rotation helper is missing.

`OMNIVOX_LOG_MAX_FILE_BYTES`

- Approximate per-part rotation threshold; defaults to 16 MiB (`16777216`).
- Rotation occurs between complete log lines, so one oversized line may exceed
  the threshold.

`OMNIVOX_LOG_RETAINED_FILES`

- File-count target used when pruning retained log parts; defaults to 16.

`OMNIVOX_LOG_RETAINED_BYTES`

- Aggregate-byte target used when pruning retained log parts; defaults to
  256 MiB (`268435456`).
- The active target of each live session is protected from pruning, so live
  files can temporarily exceed the retention limits.

Nonpositive or nonnumeric log limits revert to their defaults. The launcher
creates its log directory with mode `0700` where possible and log parts with
mode `0600`. The Rust process does not read these launcher-only variables.

See [diagnostics.md](diagnostics.md) for collection and privacy
guidance.

### Optional Windows helpers

`OMNIVOX_ELOQUENCE_HELPER`

- Optional path to `OmnivoxEloquenceHelper32.exe`.
- Otherwise Omnivox looks beside its executable.

`OMNIVOX_ECI_DLL`

- Optional absolute path inherited and read by the Eloquence helper for a
  complete licensed 32-bit ECI 6.1 installation's `ECI.DLL`.
- Defaults to
  `C:\Program Files (x86)\Freedom Scientific\Shared\Eloquence\6.1\ECI.DLL`.
- Keep the DLL with its matching installed ECI configuration, dictionary, and
  voice data.

`OMNIVOX_DECTALK_HELPER`

- Optional path to `OmnivoxDectalkHelper32.exe`.
- Otherwise Omnivox looks beside its executable.

`OMNIVOX_DECTALK_DLL`

- Optional absolute path inherited and read by the DECtalk helper for a
  user-supplied 32-bit `DECtalk.dll`.
- A matching `dtalk_us.dic` from the same build must be in the same directory.
- Without an override, the helper checks beside itself, in the sibling
  `runtime` directory, and then in
  `%LOCALAPPDATA%\Omnivox\runtimes\dectalk\x86`.
- The standard per-user directory is the recommended installation location
  for the DLL and dictionary; it survives Omnivox upgrades.
- An explicit helper DLL argument takes priority over this variable; the
  legacy `EMACSVOX_DECTALK_DLL` is used when neither is supplied.

A missing helper or runtime removes only that engine from usable inventory;
normal fallback remains available. Proprietary runtimes are not distributed
by Omnivox or Emacsvox. See the
[Windows helper guide](../../windows-helpers/README.md#runtime-requirements-and-installation)
for acquisition, installation, architecture, dependency, and verification
details.

### Optional Linux Eloquence/Outloud and DECtalk helpers

`OMNIVOX_ELOQUENCE_HELPER` and `OMNIVOX_DECTALK_HELPER` also select Linux
helper executables. Without overrides, Omnivox discovers
`eloquence/omnivox-eloquence-helper` and `dectalk/omnivox-dectalk-helper`
beside the main executable (or flat beside it). Native Linux `make build`
and `make dev` stage these interfaces.

| Variable | Linux runtime input |
| --- | --- |
| `OMNIVOX_ECI_LIBRARY` | Absolute path to the installed ECI-compatible library: Voxin's `libvoxin.so`, or `libibmeci.so` matching the helper ABI. |
| `OMNIVOX_DECTALK_LIBRARY` | Absolute path to the English language library `libtts_us.so`. |
| `OMNIVOX_DECTALK_DICTIONARY` | Absolute path to its matching `dtalk_us.dic`. |

Explicit inputs take priority over the fixed installation paths listed in the
[Linux helper guide](../../linux-helpers/README.md), including Voxin's standard
user installation under `~/.local/share/voxin/rfs`. The library ABI must match
the helper executable. Windows DLL overrides do not select Linux libraries.
The user supplies each runtime and its data; unavailable engines retain normal
fallback and report their failure through inventory and exact diagnostics.

## Emacsvox adapter

Emacsvox provides its client settings, voice editor, saved palettes and launcher
configuration. Use its [speech-backend manual](https://github.com/bartbunting/emacsvox/blob/master/docs/manual/chapters/speech-backends.org)
for the current interface. The Omnivox CLI/environment options above configure
the speech host; client preferences are applied through negotiated runtime
operations. Do not copy the upstream Emacspeak adapter's variable names into
Emacsvox configuration.

## Upstream Emacspeak adapter

This repository's `elisp/omnivox-voices.el` is a separate compatibility module
for upstream Emacspeak. Its common customizations are:

| Variable | Default | Meaning |
|---|---:|---|
| `omnivox-speech-rate` | `60` | Initial integer rate on the 0--100 scale. |
| `omnivox-voice-id` | `""` | Empty means the engine default. |
| `omnivox-pitch` | `1.0` | Pitch multiplier. |
| `omnivox-voice-volume` | `1.0` | Speech gain. |
| `omnivox-tone-volume` | `0.1` | Tone gain. |
| `omnivox-sound-volume` | `0.5` | Sound/icon gain. |
| `omnivox-notification-channel` | `"left"` | Target for the separate notification process. |

Example:

```elisp
(add-to-list 'load-path "/path/to/omnivox/elisp")
(require 'omnivox-voices)
(setq omnivox-speech-rate 60
      omnivox-voice-id ""
      dtk-program "omnivox")
(require 'emacspeak-setup)
```

Use `omnivox-set-rate`, `omnivox-select-voice`, and the volume/pitch commands
for live changes. The `dtk-*` names in this example belong to upstream
Emacspeak and are intentionally not Emacsvox configuration names.

## Implementation details

CLI parsing lives in `omnivox-cli/src/cli.rs`. Process-wide channel selection
is applied during engine/audio initialization. Runtime state commands are
handled by `omnivox-cli/src/server.rs`. Logical voices and routing policy are
snapshotted at dispatch so later configuration changes affect later work only.

The deprecated `tts_set_notification_channel` command returns an explicit
unsupported-operation response; start a separately targeted process instead.
Legacy global language commands are likewise unsupported because language is a
property of each logical voice rather than process-global mutable state.
