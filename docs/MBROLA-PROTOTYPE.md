# MBROLA prototype

This is a local engineering experiment, not a supported companion or a release
payload. [ADR 0013](adr/0013-mbrola-voice-library.md) adds managed English voice
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

## Acceptance evidence, 2026-09-16

The [Linux report](experiments/2026-09-16-mbrola-linux.json),
[native Windows report](experiments/2026-09-16-mbrola-windows.json) and
[Windows server report](experiments/2026-09-16-mbrola-windows-server.json) retain
artifact hashes and results. Windows used the fully verified development
runtime `6aaa061807e433bc`, with the prototype staged separately on its native
drive. All server probes owned their workers and used null audio output.

`tools/verify_mbrola_prototype.py` checks native rate progression, pitch changes
and exact mute. It injects a blocked frontend into a temporary bundle, confirms
three cancellations and successful replacements, changes/restores the database,
rejects an added unverified voice alias, kills the helper while a child is
blocked, confirms child retirement and starts a working replacement helper.
The real frontend/runtime also pass simultaneous two-lane inventory, exact
preview, wrong-voice rejection and eSpeak fallback checks.

For the retained 22-word corpus, both platforms produced these durations:

| Host rate | Native frontend control | Audio seconds |
| --- | --- | --- |
| 0.0 | 80 | 14.104 |
| 0.5 | 175 | 6.679 |
| 1.0 | 350 | 2.958 |
| 1.5 | 400 | 2.181 |
| 2.0 | 450 | 1.457 |

These are single-run smoke measurements of canonical helper PCM, not an
Eloquence calibration, latency benchmark or intelligibility assessment.
The ordinary CLI also generated a WAV through the main conversion/effects
pipeline. Linux helper stress passed 12 varied syntheses. Verification included
830 locked workspace tests (one pre-existing ignored test), workspace Clippy,
Windows helper Clippy, formatting, documentation links and supported main builds.

To repeat the complete Linux probe after `make dev` and the private build:

```sh
python3 tools/verify_mbrola_prototype.py \
  target/mbrola-prototype/linux/runtime/omnivox-mbrola-helper \
  --server target/debug/omnivox --report /tmp/mbrola-linux.json
```

For native Windows use its staged `.exe` paths, `--scratch-dir` on the Windows
drive, and `--espeak-data` with the native parent of the launcher's shared
`espeak-ng-data` tree. `--server-only` verifies a newly staged main server while
reusing previously completed helper acceptance. Listening, extended soak,
calibration, progressive output, broader databases, installation UX and release
licensing/packaging remain separate from this development companion.

## Complete text and startup latency, 2026-09-17

The pinned eSpeak frontend's bulk stdin reader overwrites the final input byte
with NUL. The helper now supplies an explicit terminator: previously `focus`
was synthesized as `focu`, and a final multibyte character could also be damaged.
Native acceptance compares unterminated words against newline-terminated
controls, requires different audio for genuinely shortened words, and covers
`focus`, `lost focus`, `test`, `testing`, `hello`, and `café`. The regression fails
against the original helper.

The private bundle previously included 516 files, mostly unrelated languages.
The new builder retains only en1 dependencies and all notices, and regenerates
the data directory so obsolete languages do not survive a rebuild. A controlled
native Windows comparison with the same helper and five texts reduced median
synthesis time from 494.5 ms to 160.0 ms; canonical PCM was byte-identical for
each text. These are local warm-run measurements, not a general latency promise.
The prototype still buffers each utterance and starts its two native subprocesses
for each request.

The [updated Linux report](experiments/2026-09-17-mbrola-linux.json) and
[updated Windows report](experiments/2026-09-17-mbrola-windows.json) record rebuilt
artifact identities, complete-text checks and timing, native rate/pitch/mute,
cancellation/replacement and forced retirement, plus two simultaneous silent
server lanes exercising exact preview and fallback. Repeat with the verification
commands above. `python3 tools/test_build_mbrola_prototype.py` also checks stale
language removal and preserves the prior staging data if a required input is
missing. Audible acceptance of the original focus interaction remains a separate
listening check.

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

### Library acceptance, 2026-09-17

The [Linux library report](experiments/2026-09-17-mbrola-library-linux.json) and
[Windows library report](experiments/2026-09-17-mbrola-library-windows.json) record
all three real HTTPS downloads, retained notices, disabled installation, native
validation, independent per-request voices, two simultaneous streams, pinned
workers and disabled exact previews. Fresh compiled Emacs passed paired Apply,
injected notification replacement failure with rollback, and independent US1
disablement on both platforms. These used isolated voice roots and null audio.

Windows short-word synthesis was 146–149 ms for en1 and 159–167 ms for the US
voices in this small check; en1 retains the previous latency improvement. The
existing full-text, rate/pitch/mute, tamper rejection, native cancellation,
replacement and forced-retirement probes also passed on both platforms.
Workspace tests passed 835 tests with one existing ignored test; Linux workspace
and affected Windows Clippy passed. Emacs focused tests passed 85 with four
GUI-only skips, and all 18 graphical tests passed separately.

The subsequent [combined Windows report](experiments/2026-09-17-mbrola-piper-windows.json)
records a fresh native Piper companion from commit `1d613c2` and complete Windows
development staging. Its [Piper CI run](https://github.com/bartbunting/omnivox/actions/runs/35162878166)
passed on Windows, Linux and both Mac architectures; this does not establish
macOS MBROLA support. Native staging checked the Kristin Piper model. In a fresh
compiled Emacs, a schema-2 library containing a Piper speaker fixture, Flite SLT
and the four MBROLA voices passed paired Apply, injected notification failure
with rollback, and US1 disablement. Exact synthesis from Piper, Flite, en1 and
enabled US1 passed on both streams before and after those transitions, using
null audio and an isolated library.

To repeat the combined Emacs check, first produce a fresh private root with
`verify_mbrola_library.py`, then run Emacsvox's `test/run-live-library-tests.py`
with `--server`, `--omnivox-source`, `--emacs`, `--mbrola-root` and
`--mbrola-helper`. Use the root before another Apply test commits its active
pointer. The runner imports a validated Piper fixture and adds Flite SLT only
to that private library.
