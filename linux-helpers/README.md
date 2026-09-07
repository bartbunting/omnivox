# Linux Eloquence/Outloud and DECtalk helpers

These development helpers load user-installed Linux runtimes and send bounded
progressive PCM to Omnivox. Each engine runs in a separate process using the
existing helper protocol. Native libraries, voice data and dictionaries are
not copied, installed or distributed by this build.

Build both with `make linux-helpers` (release) or `make linux-helpers-dev`.
Native Linux `make build` and `make dev` also stage them. Omnivox discovers
`eloquence/omnivox-eloquence-helper` and `dectalk/omnivox-dectalk-helper` beside
its executable. Existing `OMNIVOX_ELOQUENCE_HELPER` and
`OMNIVOX_DECTALK_HELPER` overrides select another absolute helper path.

The library ABI must match the helper, independently of the main server.
An old 32-bit ECI installation needs a helper built for
`i686-unknown-linux-gnu`, with that Rust target and native development libraries
available. The build tool accepts `--target`; point `OMNIVOX_ELOQUENCE_HELPER`
to the resulting executable. The loader rejects a mismatched ELF architecture.
Voxin 3.4 supplies an ECI-compatible **64-bit `libvoxin.so`** that runs its
bundled legacy engine in a child process. Use that library with the 64-bit
helper; its underlying IBM `libibmeci.so` is still 32-bit.

## Installed runtimes

An explicit override must name an existing absolute file and takes priority.
Missing libraries or symbols remain reportable engine unavailability; exact
engine diagnostics must fail instead of silently testing eSpeak.

| Setting | Purpose and automatic locations, in order |
| --- | --- |
| `OMNIVOX_ECI_LIBRARY` | First `~/.local/share/voxin/rfs/opt/oralux/voxin/lib/libvoxin.so`; then `libibmeci.so` under `/usr/lib/x86_64-linux-gnu`, `/usr/local/lib`, `/usr/lib64`, `/usr/lib`; then `/opt/oralux/voxin/lib/libvoxin.so`; then `libibmeci.so` under `/opt/IBM/ibmtts/lib`, `/usr/share/ibmtts/lib` |
| `OMNIVOX_DECTALK_LIBRARY` | English `libtts_us.so` under `/usr/local/lib`, `/opt/dectalk/lib`, `/usr/lib/x86_64-linux-gnu`, `/usr/lib` |
| `OMNIVOX_DECTALK_DICTIONARY` | `dtalk_us.dic` under `/opt/dectalk/dic`, `/usr/local/share/dectalk`, `/usr/share/dectalk` |

DECtalk loads the language library directly, avoiding the generic `libtts.so`
dispatcher's additional library search. Its dictionary filename is passed
explicitly. ECI uses the selected installation's own English voice data.
Neither helper searches the current working directory for its primary library.

Voxin's normal user installation is discovered automatically. Keep its `rfs`
tree intact, including the supplied 32-bit loader/libraries and the generated
`var/opt/IBM/ibmtts/cfg/eci.ini` with paths to the installed voice data. A custom
installation can select its absolute `libvoxin.so` through
`OMNIVOX_ECI_LIBRARY`. The user's licensed Voxin 3.4 English archive passed
live synthesis, cancellation, WSLg playback, and Emacs navigation tests with
libvoxin 1.6.3. That exact wrapper version has a scoped workaround for its
incorrect `eciClearInput` return value; other versions retain strict checks.

## Capabilities and platform parity

The Linux adapters expose the eight American English ECI presets and
nine English DECtalk voices, using the existing engine IDs and voice IDs.
They support progressive PCM, rate, average pitch, pitch range, stress,
richness, volume and cancellation. Voice-expression controls use the Windows
adapters' established native parameter tables, including ECI richness gain.
They declare ISO-8859-1 text support; unsupported Unicode can route to eSpeak.

ECI publishes native word/sentence indexes and exact requested anchors while
synthesizing. DECtalk publishes word/sentence/phoneme markers and resolves
requested anchors at native word boundaries, using the same affinity and
span-boundary fallback as Windows. It holds one 512-sample native audio block
so delayed index records precede their PCM. Both adapters preserve original
UTF-8 text offsets and map native sample positions through the continuous
canonical converter. Anchored speech can therefore use progressive playback.
Missing requested indexes fail synthesis rather than inventing a timestamp.

Differences that remain: Windows ECI accepts Windows-1252 while the Linux
adapter guarantees Latin-1; Windows DECtalk accepts raw inline commands and
native index requests while Linux retains its literal-text handling. Linux
does not advertise caller-supplied native-index support. See the
[parity audit](../docs/experiments/2026-09-07-linux-helper-parity.md) for runtime
evidence, other platform differences, and remaining work.

Rate mappings begin with the Windows mappings and are **provisional on Linux**
until retained rate-audit evidence supports calibration. Compilation and stub
coverage alone do not establish live support for a runtime or architecture.

Use `omnivox --engine dectalk --list-voices` and `--engine dectalk --check`
(or `eloquence`) through a configured Linux audio launcher to test the actual
installation. Restart an existing Emacsvox speech session to refresh inventory.

These integration sources are GPL-2.0-or-later; retain `COPYING`. They adapt
native capture concepts and mappings from the GPL Windows helpers. The engine
libraries remain governed by their own terms. This is a local development
build, not a change to published generic release archives.
