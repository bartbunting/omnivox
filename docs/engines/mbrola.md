# MBROLA prototype

This is a local engineering experiment, not a supported companion or a release
payload. [ADR 0007](../adr/0007-managed-voice-lifecycle.md) adds managed English voice
downloads to this explicitly configured companion. It retains the existing
helper protocol, mixer, cancellation and fallback contracts. Library documents
selecting MBROLA use schema 2; existing schema-1 libraries remain supported.

The first voice is `mbrola:v1/mb-en1/en1` on engine `mbrola`: British English en1
(Roger). `tools/build_mbrola_prototype.py` fetches SHA-256-checked source archives
and this one database from immutable revisions, builds private native programs,
and retains the source URLs, hashes and notices in an ignored runtime directory.
It changes no supported release manifest or companion installer.

The private eSpeak frontend comes from the already locked `espeak-rs-sys` 0.1.9
archive. Its generated build copy replaces `mbrowrap.c` with the checked
`tools/mbrola/frontend-only.c` overlay. This removes upstream DLL/PATH runtime
loading: the frontend only emits `.pho` instructions with `--pho -q -v mb-en1`.
Unexpected native PCM calls fail. The separate pinned MBROLA executable consumes
those instructions and returns 16-bit little-endian mono PCM at 16 kHz. A small
generated-source Windows overlay switches stdout to binary mode. Pristine
archives are retained. A direct Linux/Windows probe produced identical 54,890
PCM bytes for the same utterance after these overlays.

MBROLA is AGPL-3.0; the frontend is GPL-3.0-or-later. Database licensing is
separate: retain the en1 README's Edinburgh/FPMs provenance and restrictions,
and the voices repository's licence notice. This experiment does not establish
redistribution approval or bundle a model into a generic release. Production
work still needs a recorded licensing review, packaging policy and boundary ADR.

Build on Linux (Windows uses the installed x64 MinGW cross compiler):

```sh
python3 tools/build_mbrola_prototype.py
python3 tools/build_mbrola_prototype.py --platform windows
```

The helper reads its adjacent `prototype.json` and verifies required private
files. Legacy startup exposes en1. Managed startup accepts `--voice-library PATH`
and the parent's `--voice-library-sha256 DIGEST`, reporting only the selected
voices. The builder stages the English dictionary, phoneme tables, four reviewed
English voice definitions, their translation tables and the included en1 database. Each request still hashes every staged manifest file and rejects
unlisted data files; reducing unrelated language data avoids repeated Windows
filesystem work without weakening those checks. Set `OMNIVOX_MBROLA_HELPER` to the absolute
staged helper path to register it; use `--engine mbrola` to prefer it. Merely
placing a helper next to Omnivox does not enable it. Native Windows needs a
Windows path in that variable. Ordinary eSpeak fallback remains available.
Keep the Windows runtime on a Windows drive, including when launching from WSL.
The private file validation on a WSL UNC path was slow enough to exceed the
fault probe's startup allowance. Copy the complete `windows/runtime` directory
to a private Windows directory; retain its adjacent manifest, data and notices.
The Emacsvox launcher forwards `OMNIVOX_MBROLA_HELPER` to native Windows workers.
After an explicit restart of both lanes, existing inventory, exact preview and
palette editing can select the compound voice without a new client protocol.

Each speech lane owns its own helper. Calls serialize inside that helper;
frontend and runtime processes are sequential and use bounded pipe capture.
Text is limited to 8 KiB, phoneme instructions to 1 MiB and native PCM to 16 MiB.
The frontend deadline is 10 seconds and the runtime deadline is 20 seconds.
Cancellation kills and reaps the active native process before the next request.
Unconfirmed retirement after two seconds exits the helper so its host can
recover it. Linux children receive a parent-death signal; the pinned native
programs spawn no descendants. On Windows the helper joins a private
kill-on-close job before creating any descendants, which inherit ownership.

The descriptor advertises buffered PCM, native rate/pitch/volume, serialized
synthesis and cancellation. It advertises no word/sentence/phoneme markers or
exact requested anchors. There is no extra post-synthesis stretching. The
provisional native rate map is 80, 175, 350 and 450 WPM controls at host rates
0, 0.5, 1 and 2 respectively; these are controls, not measured output WPM or an
Eloquence calibration. High-rate intelligibility and audible acceptance require
listening. Linux and native Windows are the only intended prototype targets.

## Verification

Run `tools/verify_mbrola_prototype.py` against the private helper and a staged
main server. It checks rate, pitch, mute, complete final text, tamper rejection,
cancellation, child retirement, two-lane inventory, exact previews and fallback.

```sh
python3 tools/verify_mbrola_prototype.py \
  target/mbrola-prototype/linux/runtime/omnivox-mbrola-helper \
  --server target/debug/omnivox --report /tmp/mbrola-linux.json
```

For native Windows use staged `.exe` paths, `--scratch-dir` on a Windows drive,
and `--espeak-data` naming the native parent of the staged eSpeak data tree.
`--server-only` checks a new main server against a previously qualified helper.
Retain the report with its exact runtime and build identities. The helper must
preserve the last input byte, including an unterminated multibyte character.

[Retained prototype and library evidence](../benchmarks/2026-09-27-retained-platform-results.md#mbrola-prototype-measurements)
records the original PCM durations, controlled startup comparison and native
acceptance. Listening, extended soak, calibration, progressive output and release
qualification remain separate from these null-output development checks.

## Managed English downloads

Emacsvox's separate reviewed MBROLA catalogue offers US1, US2 and US3 from the
same pinned voice repository revision. Each download retains that voice's
`license.txt` as `LICENSE` and `README.txt` as `README`. These database licences
permit MBROLA use and require the owner's permission for sale or incorporation
in a sold product. Do not treat the runtime's AGPL terms as the database licence.
The US1 README calls it male while the pinned frontend says female; the catalogue
uses the unambiguous US1 name without asserting gender.

Acquisition resolves the explicit `OMNIVOX_MBROLA_HELPER`, verifies its complete
`SHA256SUMS` and `SOURCE-PROVENANCE.json`, synthesizes without playback in the
bounded validator, and installs disabled. en1 keeps its existing physical ID
and is retained in the index when the first optional database is installed.
Enable or disable individual voices and use the existing two-lane Apply.
Different palette choices and streams can select different enabled databases.
Each request opens only its chosen database in the native synthesizer; the
helper retains metadata rather than resident native database handles.

`tools/verify_mbrola_library.py --server PATH --helper PATH --catalogue JSON
--report JSON` runs explicit HTTPS acquisition in a retained private root,
checks cancellation, all three downloads and notices, native validation,
disabled installation, en1 preservation, alternating voices, two simultaneous
streams, generation pinning, disabled exact previews and the final-text fix.
Native Windows runs need `--scratch-dir` on the Windows drive. These checks do
not prove audible acceptance or constitute release publication.

### Library verification

The [retained library results](../benchmarks/2026-09-27-retained-platform-results.md#mbrola-managed-library-acceptance)
include native Linux/Windows acquisition, paired Apply, rollback and exclusions.
These do not establish macOS MBROLA support or acoustic acceptance.

To repeat the combined Emacs check, first produce a fresh private root with
`verify_mbrola_library.py`, then run Emacsvox's `test/run-live-library-tests.py`
with `--server`, `--omnivox-source`, `--emacs`, `--mbrola-root` and
`--mbrola-helper`. Use the root before another Apply test commits its active
pointer. The runner imports a validated Piper fixture and adds Flite SLT only
to that private library.
