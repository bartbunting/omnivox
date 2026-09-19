# DECtalk custom controls on the ordinary speech path

Implementation: `8119e02`. DECtalk now queues the selected preset, common controls,
native overrides and plain text together. For a previously checked preset there
are no pre-text `TextToSpeechSync` calls. Actual native readback still precedes
all public markers and PCM.

## Application and isolation

Each qualified native instance retains at most nine verified preset baselines.
The first native request for a preset selects it, synchronizes once and checks
its limits. Later requests combine the existing qualified common projection
with that baseline and the immutable native patch, including explicit defaults
and contextual masking. The actual submitted common commands are unchanged,
including legacy values that DECtalk clamps internally.

The first nonempty native callback checks all 28 actual fields, publishes the
application receipt, then permits leading anchors, other markers and PCM.
A mismatch or failed receipt suppresses subsequent callbacks until cleanup.
Silent text completes the same verification after synthesis drains. Exceptions
and cancellation preserve cleanup and invalidate cached baselines. Receipt
callbacks run without the capture state lock; the native runtime serializes its
callbacks, and the synthesis owner drains them before completing the request.

Text containing brackets or non-whitespace control characters retains the
conservative pre-text verification path and invalidates every cached baseline,
including when it arrives through ordinary speech. Such text can contain native
commands that change speaker settings ahead of the audio callback. This avoids
claiming the later state is the originally applied plan.

Cleanup still resets the native marker origin, restores the selected preset and
verifies restoration. Failed cleanup blocks reuse. These checks happen after
streamed audio; this change does not remove their completion cost.

## Validation

The final helper passes 1,135 native captures over all nine voices and 28
controls, 45 cancellation cases, 18 delivery failures, 27 progressive cases,
nine reset/restoration overlaps and seven independent composition fixtures.
The five existing reset lifecycle cases and eight new batching cases pass.
New cases check cold/warm waits, per-voice caching, embedded-command invalidation,
readback and receipt failures before output, cancellation during the receipt,
recovery and silent text. The batching audit fails against the previous package.
All 121 helper-6 wire acceptance cases pass with the final helper.

Source-contract, documentation and whitespace checks passed. Rust and Lisp
sources, dependencies and wire formats are unchanged. The DLL remains the same
qualified user-installed Windows x86 build; no other DECtalk runtime or platform
is qualified by this work.

## Matched server measurements

Before: development runtime `34e353095d7cfd73`. After: `b98c08411e68a656`.
Each cell contains 90 warm observations, three shuffled blocks of 30 with three
warmups per worker. Both use Windows GNU release builds, Rust 1.97.1, the same
installed DLL and dictionary, exact Perfect Paul, rate 225 and null audio.
Compilation and acceptance finished before timing; benchmark jobs ran serially.
Each custom request also verifies the applied parameter through the public
readback operation, outside the measured interval.

All values below are milliseconds from dispatch. These are software first-source
and terminal timings, not physical acoustic onset.

| Workload / measure | Before median | After median | Before p95 | After p95 |
|---|---:|---:|---:|---:|
| Ordinary first source | 3.105 | 2.978 | 4.305 | 3.878 |
| Custom `sm=61` first source | 70.096 | 3.023 | 90.495 | 3.805 |
| Ordinary completion | 40.349 | 41.217 | 51.047 | 56.057 |
| Custom `sm=61` completion | 135.734 | 67.798 | 161.159 | 81.322 |

Warm custom startup now matches ordinary startup in this workload. These results
exclude the first per-preset defaults check and do not claim that embedded native
commands use the fast path. Custom completion still includes verified preset
restoration after streaming; the extra completion cost remains visible above.

## Development deployment

Full `make windows-omnivox-dev` staging passed deterministic helper compilation,
payload checksums, inventories and companion synthesis checks. Staging used an
isolated runtime directory and preserved the existing default runtime pointer.
The packaged helper exactly matches the audited bytes:
`c4e605f39977781b30e867f61929fa3215a0d34d77afe56af3bf2dcd0cf7f258`.

Fresh compiled Emacs acceptance passed DECtalk and Eloquence editor preview,
comparison, applied readback, palette save/reload, main and notification speech,
and reconnection. These were private null-audio workers. The desktop development
launcher now pins `b98c08411e68a656`, and its local repeat plan compares this
package with the previous one. Existing Emacs sessions were not restarted.

## Evidence and repetition

[Provenance and original-byte hashes](data/2026-09-20-dectalk-batched-parameters/index.json),
[server summary](data/2026-09-20-dectalk-batched-parameters/server-comparison/summary.json),
[execution order](data/2026-09-20-dectalk-batched-parameters/server-comparison/manifest.json)
and [checksums](data/2026-09-20-dectalk-batched-parameters/SHA256SUMS) accompany the
raw observations, native/wire audits, build logs and fresh Emacs acceptance.
Compressed files decompress to their recorded original hashes.

The retained [comparison script](data/2026-09-20-dectalk-batched-parameters/compare-server.py)
uses the maintained Emacsvox server benchmark. Pass a new output directory,
`--emacsvox <checkout>`, `--plan <saved plan>`, two `--target name=program`
arguments, `--iterations 30 --repeats 3`. The
[recorded invocation](data/2026-09-20-dectalk-batched-parameters/benchmark-command.json)
identifies the exact local run. Retained raw JSON and the summary remain directly
readable; logs and larger acceptance transcripts are gzip compressed.

Run `tools/check_dectalk_execution.ps1` in x86 Windows PowerShell with `-Sta`,
`-Helper <candidate>` and `-RuntimeDll <installed DLL>` for the native regression
matrix. `tools/test_helper6_dectalk.py` supplies the separate wire acceptance.
The benchmark does not install a runtime, edit palettes or play audio.
