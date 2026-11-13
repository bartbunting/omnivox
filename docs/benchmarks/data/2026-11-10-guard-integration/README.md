# Guarded owner/client integration evidence

See the [report](../../2026-11-10-guard-integration.md). This is isolated Linux
null-output qualification, not installed or acoustic acceptance.

`observations.tar.gz` retains raw runs, source/compiled client logs, process and
descriptor observations, management fixtures, timing records, local-socket
probes, failed attempts and verification logs. `archive-files.txt` enumerates it.
Final native/client runs are `native-pipes` and `client-compiled-pipes`; final
matched timing is `latency-pipes`. `latency-previous-after-submit` is the retained
deferred-start baseline, not release 1.18.0. `latency-after-submit` is the earlier
socket-pair draft. Earlier `latency-final`/`latency-1` names are historical run
labels, not final candidate designation. They submitted client tasks concurrently;
the final fixture explicitly waits for reload transmission before character input.

The source patches apply to the base commits in the provenance files. Payload
member hashes cover the final staged executable, helpers, data and notices.
`payload-pipes.sha256` identifies the final binary; `payload.sha256` is the retained
socket-pair draft. The complete payloads remain in the private target directory;
the archive does not duplicate native binaries. Both original release and earlier
benchmark evidence remain intact.

Fixtures use only private processes and null output. From an Omnivox checkout:

```sh
python3 docs/benchmarks/data/2026-11-10-guard-integration/native.py /absolute/omnivox /new/private/native-run
python3 docs/benchmarks/data/2026-11-10-guard-integration/latency.py /absolute/omnivox /new/private/timing-run
OMNIVOX_TEST_BYTECODE=1 python3 docs/benchmarks/data/2026-11-10-guard-integration/client.py /absolute/omnivox /new/private/client-run
```

The client fixture selects the isolated Emacsvox checkout and Emacs 31.0.90 in
its source. Adjust those explicit paths for reproduction and build its byte-code
first. The archived client driver originally lived below `target/guard-integration`;
its relative repository lookup is corrected in the copied reproducible fixture.
Historical failure logs retain their original paths and expectations: initial
source selection used an empty environment value, a kernel reported
`anon_pipe_write` rather than `pipe_write`, the new Lisp module initially lacked
a Make object entry, and completed-connection cleanup discarded received proof.
The final files reproduce corrected runs, not the exact source of every failed
intermediate fixture. No failed measurement was overwritten.

Every file except this checksum manifest is covered by `SHA256SUMS`.
