# Independent guard evidence, 2026-11-10

See the [report](../../2026-11-10-independent-guard.md). This is a Linux-only
extraction experiment, not a new Omnivox executable mode or production launcher.

Compile `guard.rs` with pinned `rustc +1.97.1 --edition=2021 -D warnings`, writing
the probe under an ignored private target directory. `platform-linux.rs` is an
exact copy of the repository's native tree implementation, identified by its
hash. `probe.py OMNIVOX_EXE GUARD_PROBE FRESH_DIRECTORY` starts only private
null-output processes, real Flite helpers and eSpeak replacements. `forward.py`
owns the fixture's single speech forwarding hop and completion lease.

The probe requires Linux pidfds, `/proc`, per-process subreaper support and the
retained adjacent resident/emergency fixture modules. It deliberately pauses or
kills only newly created processes for which actual pidfds were retained before
the fault. The production guard uses its actual child/group authority. After
guard loss the test client's cleanup is explicitly separate from product proof.
Never substitute a live profile, installed runtime or saved historical PID.

Runs 5–7 use the final seven-case fixture and pass. Runs 1–4 retain the earlier
checks and fixture corrections described in the report. The archive includes
synthetic settings, protocol observations, selected diagnostics, failed-run source
copies and build/check logs. Environment-bearing native startup journals remain
in their original private directories. No claims or historical evidence are
deleted. `archive-files.txt`, `provenance.json` and `SHA256SUMS` identify the inputs
and support full readback. Source-event speech completion is not acoustic output.
