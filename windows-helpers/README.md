# Omnivox Windows capture helpers

This component owns the 32-bit Eloquence and DECtalk capture executables used
by Omnivox on Windows. The helpers load native speech runtimes out of process,
capture mono PCM and markers, and speak the versioned
[engine helper protocol](../docs/protocols/HELPER-PROTOCOL.md) over standard
input and output. Under protocol v5 they progressively canonicalize and emit
callback PCM; versions 1 through 4 retain whole-result delivery. They do not
play audio themselves.

The two executables deliberately remain separate from the 64-bit Rust server.
Eloquence's ECI runtime is 32-bit, and the DECtalk callback ABI passes pointer
state through a 32-bit integer. Process isolation also lets Omnivox terminate
and replace a wedged native engine without losing the main speech server.

## Build

From WSL with Windows PowerShell and .NET Framework available:

```sh
make windows-helpers
```

The outputs are `windows-helpers/bin/OmnivoxEloquenceHelper32.exe` and
`windows-helpers/bin/OmnivoxDectalkHelper32.exe`. The default build uses the
Windows .NET Framework C# compiler. Emacsvox's reproducible Windows bundle
passes a checksum-pinned Roslyn compiler and .NET 4.0 reference assemblies
through `OMNIVOX_CSC` and `OMNIVOX_REFERENCE_DIR`; it does not maintain a
second copy of the helper source.

Run the source-contract checks without Windows or either proprietary runtime:

```sh
make windows-helpers-test
```

On Windows or WSL, build the helpers and verify that each one negotiates the
protocol and reports a deliberately absent runtime without exiting early:

```sh
make windows-helpers-startup-test
```

## Runtime requirements and installation

Generic Omnivox release archives do not contain these helper executables or
either native runtime. Build the helpers as above, then place
`OmnivoxEloquenceHelper32.exe` and/or `OmnivoxDectalkHelper32.exe` beside the
Windows `omnivox.exe`. An explicit helper path in the corresponding
[environment variable](../docs/ENV-VARS.md#optional-windows-helpers) may be
used instead.

The helpers target .NET Framework 4 and must run as 32-bit x86 processes. A
64-bit or ARM64 native speech DLL is not interchangeable with its IA32 build.

### Eloquence

Eloquence requires a complete, legitimately installed, licensed 32-bit ECI
6.1 runtime. Omnivox does not supply or copy it. The default location is:

```text
C:\Program Files (x86)\Freedom Scientific\Shared\Eloquence\6.1\ECI.DLL
```

Leave `ECI.DLL` with the installation's `ECI.INI`, dictionary, and `.SYN`
voice data. Copying a DLL without its matching installed data is not a
supported installation. If a licensed installation uses another location,
set `OMNIVOX_ECI_DLL` to the absolute Windows path of its 32-bit `ECI.DLL`.

### DECtalk

DECtalk requires a matched IA32 `DECtalk.dll` and `dtalk_us.dic` from the same
build. Keep both files together beside `OmnivoxDectalkHelper32.exe`, or set
`OMNIVOX_DECTALK_DLL` to the absolute Windows path of `DECtalk.dll`. The
Visual Studio 2022 build also requires the x86 Microsoft Visual C++ runtime
that supplies `VCRUNTIME140.dll`.

The durable reproducible default is upstream's
[`2023-10-30` release](https://github.com/dectalk/dectalk/releases/tag/2023-10-30).
Download `vs2022.zip`, whose SHA-256 is:

```text
4a778056c109b37f95ade4b3d3e308b9396b22a4b0629f9756ec0e5051b9636d
```

Extract only `IA32/DECtalk.dll` and `IA32/dtalk_us.dic` for the helper. The
repository's
[`LICENCE`](https://github.com/dectalk/dectalk/blob/2023-10-30/LICENCE) is
restrictive; public download availability does not replace the user's
responsibility to read those terms and ensure that their use is authorized.

#### Newer tested upstream build

For advanced testing, the Visual Studio 2022 job for upstream commit
[`69ebb459137a7a8d92ed41da8362233eaa173efc`](https://github.com/dectalk/dectalk/commit/69ebb459137a7a8d92ed41da8362233eaa173efc)
succeeded and produced an exact
[`vs2022` Actions artifact](https://github.com/dectalk/dectalk/actions/runs/29847218896)
(artifact ID `8501855998`). That artifact was still available when Omnivox
1.5.1 was prepared, but GitHub requires a signed-in account to download
Actions artifacts, and they expire. It is therefore not the reproducible
default. Its GitHub-reported archive digest is:

```text
sha256:793f4cf6751b5fb61b8e7fb01909e44c1911f06a81fc7b6078d0f5379f87903b
```

Use its `IA32` directory. The files exercised by Omnivox had these hashes:

```text
af25879d858846aaaa80b8f9626b1cbf4e57a1ab0467e8e2d82990558033c852  DECtalk.dll
72271446c6d656842b389ff67f2bfafd6e0d4732f8a3a37442133dfb206206e7  dtalk_us.dic
```

The current commit retains the `2023-10-30` export table, public API header,
callback signature, and memory-buffer structures used by the helper. Its IA32
artifact passed 100 persistent syntheses, word/sentence/phoneme/native-index
marker validation, cancellation, health pings, and clean shutdown. This result
does not guarantee compatibility with an arbitrary later commit; repeat the
same validation for each newer build.

To build that commit instead, install Visual Studio 2022 with the **Desktop
development with C++** workload, then run the maintained scripts from a Visual
Studio Developer Command Prompt:

```bat
git clone https://github.com/dectalk/dectalk.git
cd dectalk
git checkout 69ebb459137a7a8d92ed41da8362233eaa173efc
devops\vs2022\dt_buildall.bat
devops\vs2022\dt_copyfiles.bat
```

Use `dist\IA32\DECtalk.dll` and `dist\IA32\dtalk_us.dic`; do not use the
`AMD64` outputs with `OmnivoxDectalkHelper32.exe`.

### Emacsvox under WSL

Emacsvox's reproducible default downloads and verifies the durable DECtalk
release before staging the complete content-addressed Windows runtime:

```sh
cd /path/to/emacsvox
make -C servers/windows-dectalk runtime
make windows-omnivox
```

A newer source or Actions build is a local-development input until Emacsvox's
release lock names a durable asset. For local testing, place its matched IA32
DLL and dictionary under `servers/windows-dectalk/runtime/`, then use
`make windows-omnivox-dev` so provenance records a development build. Do not
overwrite a selected content-addressed runtime in place.

## Verification

The stress procedure in
[tools/README.md](../tools/README.md#windows-helper-session-stress) exercises a
built helper against a real runtime. For a source-built DECtalk checkout, run:

```sh
python3 tools/stress_helper.py \
  windows-helpers/bin/OmnivoxDectalkHelper32.exe \
  --engine-id dectalk --iterations 100 --cancel-probe \
  --helper-arg "$(wslpath -w /path/to/dectalk/dist/IA32/DECtalk.dll)"
```

After staging, start a fresh Omnivox process and verify that `dectalk` or
`eloquence` and its voices appear in live inventory. A process already running
during staging continues to use its previous content-addressed runtime.

For protocol-v5 verification, pass `--require-streaming` to
`tools/stress_helper.py`. Eloquence should emit its native word/sentence/index
markers ahead of the associated PCM. DECtalk intentionally retains one
512-sample native block because its runtime can report an index a few samples
after the callback containing that frame. The holdback is bounded and is
flushed at synthesis completion.

Explicit native DLL arguments and environment variables must contain absolute
paths. Otherwise Eloquence uses the Freedom Scientific 6.1
installation path; DECtalk checks only beside the helper and the sibling
`runtime` directory. Before any engine call, a helper validates that its DLL is
an x86 PE image with every required export, then uses restricted Windows loading
that resolves native dependencies only beside the selected DLL or from
System32. Missing, malformed, wrong-architecture, or incomplete runtimes are
reported as `not_available` through the helper protocol.

## Source and licensing

`common/OmnivoxHelperHost.cs` owns the bounded versions 1 through 5 protocol
loop. Each engine directory owns only its adapter, native capture boundary, and
entry point. These helper sources retain their original copyright and
`GPL-2.0-or-later` notices; [COPYING](COPYING) contains the applicable GPL
version 2 text. The repository's default MIT license does not relicense them.
