# Retained recovery evidence, 2026-11-10

See [the report](../../2026-11-10-retained-recovery.md) for behaviour and limits.
The native payloads are unchanged from the preceding host-preference delivery.
`provenance.json` identifies the source bases and executable digests.
`emacsvox-source.patch.gz` includes the final paired client changes and fixture.

`emacs.py EXE FRESH_DIRECTORY` runs the source fixture with private configuration,
voice records and null output. `--bytecode` requires the paired checkout's full
build and preflight. `--engine piper` selects the retained real companion/model;
absolute paths need adaptation elsewhere. For a native Windows executable under
WSL, use an output directory under the Windows temporary directory and pass
`--skip-crash`; the fixture converts configuration/voice paths explicitly.
`--lose-retirement-reply` drops one actual owner receipt in the isolated client,
checks that no replacement starts, and confirms that the healthy peer can speak.
The current native owner exits after replying, so receipt loss remains blocking;
the report identifies durable receipt verification as the next increment.

Linux crash injection uses executable/ancestry validation and retained pidfds for
the exact test worker. No live profile, installed runtime, unrelated process or
saved PID is a test target. Never reuse an output directory. Final source/compiled
runs print and verify actual loaded Lisp paths and actual synthesis engine IDs.
Use `--baseline --source-root OLD_CHECKOUT` with the Emacsvox base commit in
`provenance.json` to reproduce the pre-fix empty-owner failure. The current fixture
can load that old implementation without requiring its missing recovery functions.
Original failed fixture attempts and their logs remain in the archive.

`observations.tar.gz` contains the files listed in `archive-files.txt`: synthetic
configuration, client/native logs and check logs. Owner/startup journals can
contain captured environment and remain in their original private fixture roots;
they are not redistributed or replaced by invented cleanup evidence. No private
environment dump is included. `SHA256SUMS` covers this pack; run `sha256sum -c
SHA256SUMS` here. Archive contents were read back and checked against their manifest.

Timings are functional source observations, not an acoustic or matched release
comparison. Earlier reload packs and baseline payloads remain unchanged.
