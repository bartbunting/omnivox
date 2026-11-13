# Owner authority evidence, 2026-11-10

See the [report](../../2026-11-10-owner-authority.md).

`probe.py EXE FRESH_DIRECTORY` is a Linux-only failure fixture. It creates private
ordinary eSpeak owners, uses null output and retains exact pidfds for injection
and cleanup. Never supply an installed speech process or reuse historical PIDs.
The executable needs the complete matching payload beside it. The fixture owns
every process it signals and never converts external test cleanup into a product
receipt. Its success flag means the observations completed, not that product
recovery succeeded.

`observations.tar.gz` retains three runs, including the unsuccessful service-query
fixture and the corrected run, with their exact scripts. Native startup journals
containing captured environment remain only in the original private test
directories. `archive-files.txt` records every archived member's digest;
`SHA256SUMS` covers the pack. `provenance.json` identifies the unchanged payload
and source commits. The earlier emergency and release evidence is unchanged.
