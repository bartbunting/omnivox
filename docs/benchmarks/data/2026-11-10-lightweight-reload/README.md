# Lightweight reload evidence

See the [report](../../2026-11-10-lightweight-reload.md) for interpretation and
limits. `SHA256SUMS` covers this pack. `observations.tar.gz` contains unmodified
selected result JSON, native stderr, client logs, exact probe copies from the
matched runs and gate/build logs. Startup records, environment snapshots, random
management tokens and disposable voice-library state are deliberately excluded.

`provenance.json`, the payload manifests and `candidate-source.patch.gz` identify the
measured Linux code and inputs. The executable was preserved before building the
maintenance control into the shared Cargo target: `target/release/omnivox` afterward
was **not** the measured candidate. After final Windows-path correction and its
build, it contains the final corrected executable instead. Use the preserved candidate payload or rebuild
the source patch; do not identify a binary solely by its workspace version.
`final-source.patch.gz` includes the subsequent capture-only canonical-path fix;
the final Linux binary hash is separately recorded. The original latency matrix
belongs to the earlier hash. Both gzip files preserve exact applyable patch bytes.

## Reproduction on the recorded workstation

All commands run in `/home/bart/src/omnivox-lightweight-reload` unless stated.
Use new output directories; probes refuse to overwrite a previous run.
The scripts contain explicit workstation paths for the release, companion/model
and Emacs. Review and adapt those inputs on another host, preserving the recorded
hashes when claiming a matched reproduction. No script contacts a live Emacs.

```sh
make fmt-check
make build
# Preserve the complete runnable payload before building a comparison revision.
python3 docs/benchmarks/data/2026-11-10-lightweight-reload/compare.py target/new-ordered
python3 docs/benchmarks/data/2026-11-10-lightweight-reload/compare.py target/new-letter --workload letter
python3 docs/benchmarks/data/2026-11-10-lightweight-reload/compare.py target/new-replacement --workload replaceable
python3 docs/benchmarks/data/2026-11-10-lightweight-reload/overlap.py target/new-overlap
```

`probe.py` additionally accepts `--experimental --faults`, `--engine flite` or
`--engine piper`, `--reloads N` and `--samples N`. `faults.py`, `crash.py`,
`prepared.py`, `pressure.py`, `availability.py` and `total.py` each take an explicit
executable and a new disposable output directory. They require owned workers;
the crash probe signals only its own verified live child using a retained pidfd.
The availability probe temporarily denies directory writes and restores modes
before ordinary owned cleanup. Native output is null.

`emacs.py OUTPUT` invokes a fresh selected Emacs 31 loading the copied
`run-lightweight-reload.el` from the paired worktree. The Lisp verifies actual source
paths, installs independent policies and retires both processes normally. It is a
functional fixture, not a delivered settings command.

For the full optional workspace gate, the successful command was:

```sh
env CARGO_TARGET_DIR=/home/bart/tmp/ovlw-check \
  OMNIVOX_PIPER_INPUTS_DIR=/home/bart/src/omnivox/target/piper-inputs/1.7.0/x86_64-unknown-linux-gnu \
  cargo +1.97.1 test --locked --workspace
cargo +1.97.1 clippy --locked -p omnivox-cli --all-targets --features piper -- -D warnings
```

The ordinary long worktree target first failed native Piper preparation/phoneme
compilation; the shorter isolated target passed without changing dependencies or
source. Logs retain both attempts. `make build` supplied runnable companions and
data; a bare Cargo executable was not used as a distribution payload.

## Analysis

Extract `observations.tar.gz` into a new directory, then run
`python3 summarize.py EXTRACTED_DIRECTORY` to recreate `summary.json`. Percentiles
use the nearest-rank empirical observation; all samples are retained. Main matched
runs have eight rotated blocks; overlap runs have five alternating blocks and
timestamped management intervals. The overlap subset tests interval intersection
on the same lane, not merely whether the test was labelled reload. Operations
include preparation-independent RPC timing; `total-1` separately measures the
whole capture/install operation instead of adding overlapping measurements.

`pilot-1` predates monotonic live policy revision. `pilot-2` and the matched/native
runs use the final measured SHA. The early letter fixtures used incorrect protocol
shapes before `letter-5`; the original comparison used a relative configuration
root before `matched-ordered-2`. The first Emacs fixture had a parse error before
`emacs-2`; `emacs-3` additionally verifies loaded paths. These diagnostic failures
are retained, not counted as successful workload qualification. Earlier pilot
source bytes were not all captured; their observations cannot establish exact
reproduction. Matched roots contain their actual `probe.py` copy.

The previous investigation and plan are copied as `.txt` to preserve context
without importing branch-only links or maintaining a second plan. The accepted
0011/0013–0016 records are exact text from session commit `7879419`, not new or
amended decisions. Current lightweight follow-up lives in the linked maintained
plan.

## Windows fixture isolation

Full development staging used the new Emacsvox worktree with a copied release
tool directory under ignored `dist/lightweight-release`. The retained patch
redirects Windows payload/cache storage from LocalApplicationData to
`GetTempPath()/omnivox-lightweight-20261110`. The launcher was copied alongside it.
No installed runtime directory was used for staging. The Docker wrapper mounts
the linked worktree's Git metadata read-only so native companion provenance can
resolve the source commit inside `/workspace/omnivox`.

The standard toolchain verification initially rebuilt the normal Docker cache
tag from its pinned inputs. The recorded older image ID was unavailable for
restoration. Subsequent builds use the separate `lightweight-20261110` image tag;
this build-cache change does not replace an installed speech payload. Initial
copy-order, missing-PowerShell-PATH and unmapped-Git-metadata failures are retained.
They are staging-fixture failures, not successful Windows qualification.

`windows.py RUNTIME_METADATA_DIRECTORY OUTPUT` exercises WSL stdio to native
Windows owned workers using isolated configuration and null output. It records
whether WSL can reach the private loopback endpoint directly. If it cannot, a
bounded PowerShell native TCP client performs management for functional checks;
that mode is explicitly labelled and its process-start overhead is not a reload
latency result. See the report's final platform coverage for the actual outcome.
`windows-6` passed with native Windows state roots and the management bridge;
preceding attempts retain UNC-state and candidate-path failures. The final fixture
asserts that canonical capture differs from owned startup only in punctuation and
activation ID. Windows main-only staging followed successful complete staging,
passed its existing path guard and reused that isolated base's verified companions.
