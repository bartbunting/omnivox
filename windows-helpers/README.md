# Omnivox Windows capture helpers

This component owns the 32-bit Eloquence and DECtalk capture executables used
by Omnivox on Windows. The helpers load native speech runtimes out of process,
capture mono PCM and markers, and speak the versioned
[engine helper protocol](../docs/protocols/HELPER-PROTOCOL.md) over standard
input and output. They do not play audio themselves.

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

Eloquence's ECI DLL and DECtalk's DLL, dictionary, and voices remain
user-supplied. See [environment variables](../docs/ENV-VARS.md) for explicit
runtime and helper paths. The stress procedure in
[tools/README.md](../tools/README.md#windows-helper-session-stress) exercises a
built helper against an installed runtime.

Explicit native DLL arguments and environment variables must contain absolute
paths. Otherwise Eloquence uses its documented Freedom Scientific 6.1
installation path; DECtalk checks only beside the helper and the sibling
`runtime` directory. Before any engine call, a helper validates that its DLL is
an x86 PE image with every required export, then uses restricted Windows loading
that resolves native dependencies only beside the selected DLL or from
System32. Missing, malformed, wrong-architecture, or incomplete runtimes are
reported as `not_available` through the helper protocol.

## Source and licensing

`common/OmnivoxHelperHost.cs` owns the bounded versions 1 through 4 protocol
loop. Each engine directory owns only its adapter, native capture boundary, and
entry point. These helper sources retain their original copyright and
`GPL-2.0-or-later` notices; [COPYING](COPYING) contains the applicable GPL
version 2 text. The repository's default MIT license does not relicense them.
