# MBROLA prototype

This is a local engineering experiment, not a supported companion or a release
payload. ADR 0012 reserves production integration for a separate engine-boundary
decision. The prototype uses the existing helper protocol, mixer, cancellation
and fallback contracts without changing their wire formats.

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

The helper reads its adjacent `prototype.json`, verifies required private files,
and accepts only this exact voice. Set `OMNIVOX_MBROLA_HELPER` to the absolute
staged helper path to register it; use `--engine mbrola` to prefer it. Merely
placing a helper next to Omnivox does not enable it. Native Windows needs a
Windows path in that variable. Ordinary eSpeak fallback remains available.

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
