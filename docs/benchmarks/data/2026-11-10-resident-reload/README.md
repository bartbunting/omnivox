# Resident reload evidence, 2026-11-10

See [the report](../../2026-11-10-resident-reload.md) for interpretation and limits.
This is the first negotiated punctuation implementation, before broader host
preferences. Historical private investigation evidence is a separate unchanged pack.

- `observations.tar.gz`: 855 selected raw files, listed in `archive-files.txt`.
  Linux/Windows results, timestamped control/marker/terminal records, synthetic
  configuration inputs and check logs include failed runs and successful reruns.
- `summary.json`: eight rotated blocks, 1,024 requests per case/workload,
  nearest-rank empirical p95. Recompute with `summarize.py PATH/TO/matrix-1`.
- `omnivox-source.patch.gz` and `emacsvox-source.patch.gz`: exact executable-source
  differences from the base commits recorded in `provenance.json`.
- `linux-payload.sha256`, `windows-PROVENANCE` and `windows-SHA256SUMS`: native
  payload identities. Earlier Linux runs record their own executable hashes.
- Python and Lisp fixtures reproduce isolated tests. Absolute fixture/model/runtime
  paths must be replaced for another checkout. Use fresh output directories;
  never point these tests at installed speech or an existing voice root.
- `SHA256SUMS`: checksums of the pack's artifacts. Verify with `sha256sum -c
  SHA256SUMS` from this directory.

Raw fixture owner journals remain in ignored test directories because retained
startup captures can contain environment data. They are deliberately excluded
from this shareable pack; no synthetic replacement journal is presented as raw
ownership evidence. Reported owner status, timings and controlled failure inputs
are retained. The archive was read back in full and its member list verified.

Matched matrix: `python3 compare.py FRESH_OUTPUT_DIR`. Other fixtures take the
isolated executable and fresh output directory; inspect their arguments first.
`resident.py --help` documents engine, reload, workload and Windows runtime options.
All audio uses null output. The protocol response, observed source playback,
first PCM and acoustic onset are distinct; only the first two are recorded here.
