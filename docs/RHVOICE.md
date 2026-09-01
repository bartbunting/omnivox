# RHVoice Companion

Omnivox uses RHVoice through the isolated `omnivox-rhvoice-helper` process. The
helper is portable Rust code, but it does not contain RHVoice itself. Install a
compatible RHVoice native library, at least one language, and at least one
voice separately.

This boundary is deliberate:

- an RHVoice crash or hang cannot corrupt the main speech server;
- Omnivox does not redistribute RHVoice code or voice data;
- users choose the voices and accept their individual licences; and
- a missing runtime removes only RHVoice while the normal platform and eSpeak
  engines remain available.

## Compatibility and platform status

The helper uses the public callback-based C API present in the accepted
RHVoice 1.14.0 runtime and the inspected current 1.18.4 source. It accepts later
1.x versions only when the required symbols are present, and rejects an older
release or another major version with a protocol-visible diagnostic.

| Platform | Status |
|---|---|
| Linux x64 | Native discovery, synthesis, PCM, word/sentence markers, ACSS, cancellation, and shutdown tested with RHVoice 1.14.0 |
| Linux ARM64 | Helper compiles; live native acceptance is pending |
| Windows x64 | Helper supported with an explicit compatible `RHVoice.dll`; live acceptance is pending |
| Windows ARM64 | Helper compiles; no compatible upstream runtime has passed acceptance |
| macOS Intel/Apple Silicon | Helper compiles and accepts explicit paths; RHVoice does not claim macOS support and live acceptance is pending |

Upstream currently documents GNU/Linux, Windows, and Android. Android is not
an Omnivox target. Compile coverage is not a claim that an RHVoice runtime is
available for that platform.

## Build and layout

Supported Omnivox builds stage the helper in its own directory:

```text
omnivox or omnivox.exe
rhvoice/
└── omnivox-rhvoice-helper or omnivox-rhvoice-helper.exe
```

From a source checkout, `make build` creates the release server payload and
the RHVoice helper. `make dev` does the same in the debug profile. The focused
aliases are `make build-rhvoice` and `make install-rhvoice`.

Keep the `rhvoice/` directory beside the main executable. Do not copy an
arbitrary RHVoice library into that directory: the helper requires either a
known system installation or an explicit absolute path, and the selected
library must retain access to its own matching dependencies and data.

## Install RHVoice on Linux

Distribution packages are the simplest option when they provide the shared C
library as well as language and voice data. On Debian-family systems, install
the RHVoice library/module and a language package. For example, the Ubuntu
24.04 packages used for Omnivox's minimum-version acceptance are:

```sh
sudo apt install librhvoice5 rhvoice-english
```

Package names and versions vary. Upstream's
[Linux installation page](https://rhvoice.org/linux/) and
[packaging status](https://github.com/RHVoice/RHVoice/blob/master/doc/en/Packaging-status.md)
list current alternatives. The upstream Snap is primarily exposed through
Speech Dispatcher and is not searched automatically by the Omnivox helper; use
the explicit paths below if that installation exposes the C library and data.

To build the current tested release from source, follow upstream's
[Linux build instructions](https://github.com/RHVoice/RHVoice/blob/1.18.4/doc/en/Compiling-on-Linux.md).
A dedicated prefix makes the Omnivox paths unambiguous:

```sh
git clone --recursive --branch 1.18.4 https://github.com/RHVoice/RHVoice.git
cd RHVoice
scons prefix=/opt/rhvoice
sudo scons prefix=/opt/rhvoice install
```

If the prefix is not in the platform loader's normal search path, configure
all relevant paths explicitly:

```sh
export OMNIVOX_RHVOICE_LIBRARY=/opt/rhvoice/lib/libRHVoice.so
export OMNIVOX_RHVOICE_DATA=/opt/rhvoice/share/RHVoice
export OMNIVOX_RHVOICE_CONFIG=/opt/rhvoice/etc/RHVoice
```

Use the actual `lib` or `lib64` directory selected by that build. Dependent
RHVoice libraries must also be resolvable by the operating-system loader; fix
the installation or loader configuration rather than copying isolated shared
objects beside Omnivox.

## Install or build RHVoice on Windows

RHVoice's ordinary Windows voice installers expose SAPI 5. They do not promise
to install the public C API `RHVoice.dll` required by Omnivox. Pointing Omnivox
at `RHVoiceSvr.dll` will not work.

Use a compatible C API build and matching data. Upstream's
[Windows build instructions](https://github.com/RHVoice/RHVoice/blob/1.18.4/doc/en/Compiling-on-Windows.md)
describe the required Python, SCons, WiX, and NSIS tools. A recursive 1.18.4
checkout built with `scons` normally produces the 64-bit API library and staged
data under these paths:

```powershell
$env:OMNIVOX_RHVOICE_LIBRARY = "C:\src\RHVoice\build\windows\x86_64\lib\RHVoice.dll"
$env:OMNIVOX_RHVOICE_DATA = "C:\src\RHVoice\build\windows\data"
$env:OMNIVOX_RHVOICE_CONFIG = "C:\src\RHVoice\build\windows"
```

Confirm those paths against the selected RHVoice build rather than assuming a
different checkout has the same layout. The library architecture must match
the Omnivox helper architecture. The helper loads dependencies only from the
selected DLL directory and Windows System32; it does not search the current
working directory.

## Explicit runtime configuration

All path overrides must be absolute and must exist before the helper starts.
They are inherited by the helper from Omnivox.

| Variable | Meaning |
|---|---|
| `OMNIVOX_RHVOICE_HELPER` | Override the helper executable; otherwise Omnivox checks `rhvoice/` and then beside itself. |
| `OMNIVOX_RHVOICE_LIBRARY` | Exact `libRHVoice`/`RHVoice.dll` file. This takes priority over automatic discovery. |
| `OMNIVOX_RHVOICE_DATA` | Directory containing RHVoice's `languages/` and `voices/` data. |
| `OMNIVOX_RHVOICE_CONFIG` | RHVoice configuration directory, containing `RHVoice.conf` or `RHVoice.ini`. |
| `OMNIVOX_RHVOICE_RESOURCES` | Additional language/voice resource directories, separated with the platform path-list separator (`:` on Unix, `;` on Windows). |

Linux automatic discovery checks normal multiarch, `/usr/lib`, `/lib`, and
`/usr/local/lib` locations. macOS checks common Homebrew, `/usr/local`, and
MacPorts library directories but remains an unverified runtime target. Windows
requires `OMNIVOX_RHVOICE_LIBRARY` because a SAPI installation is not evidence
that the C API library is present.

## Verify the installation

List the physical RHVoice voices first:

```sh
omnivox --engine rhvoice --list-voices
```

Copy an exact `rhvoice:...` ID from that output and synthesize without opening
an audio device:

```sh
omnivox --engine rhvoice --dump-wav "rhvoice:Alan" rhvoice-test.wav \
  "RHVoice is working through Omnivox."
```

The command should create nonempty `rhvoice-test_raw.wav` and
`rhvoice-test.wav` files. In server mode, `--engine rhvoice` or
`OMNIVOX_ENGINE=rhvoice` makes RHVoice the initial preference while retaining
other registered engines for explicit runtime routing and fallback.

If discovery fails, run the helper directly with a protocol probe or inspect
Omnivox's stderr. A missing library, unsupported version, missing symbol,
invalid data path, or empty voice inventory is returned as `not_available`;
it is not expected to terminate Omnivox.

## Licensing, upgrades, and removal

RHVoice's main library and combined build have upstream licence obligations,
and individual voices use different licences. Review
[RHVoice's licence summary](https://github.com/RHVoice/RHVoice/blob/1.18.4/doc/en/License.md)
and the licence shipped with every selected voice. Omnivox does not copy those
components into its archives.

After upgrading RHVoice, restart Omnivox so the helper reloads the library,
configuration, and voice inventory. Repeat voice discovery and WAV synthesis.
To disable RHVoice without uninstalling it, remove the `rhvoice/` companion
directory or set runtime routing policy to disable the `rhvoice` engine. To
remove it completely, uninstall the user-supplied RHVoice runtime and voice
data with the mechanism that installed them.
