# Saved speech defaults acceptance

This functional report covers the post-1.13.0 change at `f3328d6`: saved voice,
rate, pitch, volumes and text preferences apply at startup, and reset restores
the saved baseline. Emacs can override those settings again. See the
[decision](../adr/0010-saved-speech-defaults.md) and
[configuration contract](../reference/engine-configuration.md#saved-speech-defaults).

## Linux checks

The following checks passed on Linux under WSL2 with pinned Rust 1.97.1:

- Locked workspace tests, including sparse configuration, rejected malformed
  values, reset after client changes, and compatibility with private startup
  schemas 1 and 2. New schema-3 records require all saved-default fields.
- Locked workspace Clippy with all targets and the CLI Piper feature.
- `make dev`, formatting and documentation checks.
- One complete process acceptance run against the staged debug executable.
  The external fixture receives the saved voice, rate and pitch; command-line
  and client overrides take effect; reset restores the saved values. Exact
  preview and positional WAV voice selection retain their priority. Saved
  punctuation/CamelCase settings affect ordinary speech, and configured
  three-word chunks remain in use.
- The same process run verifies the second lane and recovery after configuration
  files and discovery environment are changed. Reset still uses the captured
  defaults. Independent retirement and blocked-startup-write cleanup also pass.
- One remote shared-snapshot/total-disconnect recovery test.

The volume checks cover state restoration and the diagnostic host gain setting,
and confirm that helper requests keep unit gain. They do not measure loudness
at a speaker.

## Windows checks

The full Emacsvox `windows-omnivox-dev` build passed its package and native
inventory checks. One complete process acceptance run then passed against its
native Windows release-profile executable, using Windows-local temporary files
and the C# fixture. It exercises the same configuration, override, reset,
recovery and cleanup assertions as Linux.

The package was staged under a separate runtime root and did not replace the
installed launcher or restart the user's speech. Build provenance identifies
`f3328d6` and Emacsvox `b7f0bd2`; the development build includes experimental
TGSpeechBox and omits Piper under the existing staging policy. This is a
development acceptance result, not a release publication.

## Method and limitations

The process harness uses an external Python fixture on Linux and a compiled C#
fixture on Windows. It records and asserts synthesis requests and tracks server
start/completion events. Null output is selected; the helper generates a tone.
These checks establish configuration, precedence and process behavior, not
native voice quality or physical sound. Test deadlines use monotonic clocks;
no latency samples or acoustic measurements were collected for this slice.
Temporary request traces are removed by the harness after success; its retained
logs report the assertion results, and the fixture/input generator is committed.

macOS was not exercised. Native listening and long-running engine stress remain
outside this configuration check. The previously recorded
[dense eSpeak marker failure](2026-09-28-host-chunk-configuration.md#pre-existing-dense-marker-failure)
was not re-tested or changed here. This work is unreleased.

## Reproduction and provenance

[Retained records](data/2026-09-28-saved-speech-defaults/) contain build/test logs,
source and executable identities, Windows staging provenance and checksums.
The fixture configuration and request sequence are in
[`verify_engine_configuration.py`](../../tools/verify_engine_configuration.py)
at the source commit above. Run from the Omnivox root with prepared native
inputs and the pinned toolchain:

```sh
cargo +1.97.1 test --locked --workspace
cargo +1.97.1 clippy --locked --workspace --all-targets --features omnivox-cli/piper -- -D warnings
make dev
python3 tools/verify_engine_configuration.py /path/to/staged/omnivox
OMNIVOX_REMOTE_TEST_PROGRAM=/path/to/staged/omnivox OMNIVOX_ENGINE=espeak \
  python3 tools/test_remote_service.py RemoteServiceTests.test_engine_snapshot_is_shared_and_survives_total_disconnect
```

For Windows, use the full sibling Emacsvox build with a separate staging root:

```sh
make -C ../emacsvox windows-omnivox-dev OMNIVOX_RUNTIME_DIR=/absolute/separate/runtime
python3 tools/verify_engine_configuration.py --windows /mnt/c/path/to/staged/omnivox.exe
```

Use the native Windows runtime named by the staged `windows-runtime.path`.
Create a new evidence directory for any rerun.
