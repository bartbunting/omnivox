# Deferred emergency startup evidence, 2026-11-10

See the [report](../../2026-11-10-deferred-start.md) and
[native contract](../../../reference/engine-startup-snapshot.md#explicit-minimal-emergency-startup).

`native.py EXE FRESH_DIRECTORY` tests prepared owners, cancellation, exact identity,
duplicate release, unavailable data, post-preparation failure and actual old/peer
speech. Windows requires `--windows-runtime` naming its matching staged metadata
directory; the separate installed eSpeak data reference is retained there.

`emacs.py EXE FRESH_DIRECTORY` runs a fresh compiled Emacs 31 client. `--legacy`
loads the two saved modules from the previous client commit; `--profile` enables
client spans and explicitly forwards native startup timing through WSL.
`--engine piper` selects the retained Linux helper/model. `--expect-unsupported`
checks actual old-runtime rejection and continued main/notification speech.
Absolute isolated paths in fixtures/provenance need adjustment elsewhere.

`compare.py EXE FRESH_DIRECTORY` runs one warmup, eight unprofiled comparison pairs
and two profiled pairs, alternating client order. `analyze.py DIRECTORY` produces
per-run observations and medians/ranges. No tail percentile is claimed. Native
Windows timing in the first matrix lacked `WSLENV` forwarding; separate corrected
profiling runs retain those native spans. All comparison variants used the same
payload within their platform, so the missing diagnostic flag does not invalidate
the unprofiled comparison.

The archive retains selected synthetic configuration, native/client output,
profiles, failed pilots, checks and earlier engine-free launch measurements.
Its manifest supports complete readback. Native startup journals capture private
environment and remain only in the original fixture directories. Saved byte-code
is identified by hashes and source commit, not distributed here. Payload hashes,
Windows provenance and exact source patches identify the tested inputs. No live
profile, installed runtime, historical evidence or ownership claim is altered.
