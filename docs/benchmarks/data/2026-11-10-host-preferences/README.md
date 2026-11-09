# Complete host preference evidence, 2026-11-10

See [the report](../../2026-11-10-host-preferences.md) for results and limits.

`observations.tar.gz` contains 291 selected raw files listed in `archive-files.txt`:
rotated comparisons, native Linux/Windows runs, fresh Emacs results and check logs.
Synthetic configuration, raw control/source/terminal timestamps and failed check
logs are retained. Owner startup journals are excluded because captured environment
can contain private values; the original journals remain in ignored fixture roots.
No substitute journal is presented as ownership evidence.

`summary.json` is produced by `summarize.py PATH/TO/matrix-1`; empirical p95 uses
nearest rank. `compare.py FRESH_OUTPUT_DIR` runs four rotated blocks against the
recorded release, punctuation and full-policy payloads. `host.py EXE FRESH_DIR`
runs whole-policy faults and speech, with optional `--engine piper` or
`--windows-runtime PATH`. `emacs.py` runs the fresh paired client fixture with
optional `--bytecode`. All use null output and isolated configuration/voice roots.
Absolute runtime, companion/model and checkout paths need adaptation elsewhere.
Never select installed speech or reuse an existing output directory.

Source patches apply to the base commits in `provenance.json`. The complete Linux
payload manifest, Windows staging provenance and checksums identify actual inputs.
The Emacs source patch includes the paired fixture; `run-resident-reload.el` is
also retained separately. `SHA256SUMS` covers this pack: run `sha256sum -c
SHA256SUMS` here. The raw archive was read back fully and its member list verified.
The prior punctuation and private-investigation packs remain unchanged.
