# Retained platform and companion observations

Extracted on 2026-09-27 from documentation at source `ca38781`. These are
retained observations, not new test runs or current universal qualification.
Each section identifies its original document; the Git revision preserves that
account. Original raw files and CI links remain where recorded. Missing local
logs, samples or provenance have not been reconstructed. Historical outstanding
work describes that observation, not necessarily the current implementation.

## macOS progressive synthesis

Source: `docs/MACOS-STREAMING.md` at `ca38781`.

The final [native acceptance run](https://github.com/bartbunting/omnivox/actions/runs/35167821767)
passed on Intel and Apple Silicon running macOS 15.7.9 at source commit
`382c338812e2f6af1f8ca2f923225763d9b04363`:

- Each architecture passed five native queue/lifecycle regressions, ten Rust
  adapter checks and native Clippy. Gordon, Karen and Catherine each passed
  progressive delivery, full-result collection, repeated cancellation,
  consumer failure and subsequent synthesis.
- Each complete server payload passed two simultaneous speech processes with
  exact Gordon routing and null audio output. Each process exercised all six
  benchmark workloads with two measured samples per workload, six replacement
  iterations and three hard stops. The workflow artifacts retain the reports,
  voice identifier, host details, source commit and executable hashes.
- The Linux locked workspace suite passed 840 tests, with one existing ignored
  test; workspace Clippy and formatting checks also passed.

## MBROLA prototype measurements

Source: `docs/MBROLA-PROTOTYPE.md` at `ca38781`.

### Acceptance evidence, 2026-09-16

The [Linux report](../experiments/2026-09-16-mbrola-linux.json),
[native Windows report](../experiments/2026-09-16-mbrola-windows.json) and
[Windows server report](../experiments/2026-09-16-mbrola-windows-server.json) retain
artifact hashes and results. Windows used the fully verified development
runtime `6aaa061807e433bc`, with the prototype staged separately on its native
drive. All server probes owned their workers and used null audio output.

`tools/verify_mbrola_prototype.py` checks native rate progression, pitch changes
and exact mute. It injects a blocked frontend into a temporary bundle, confirms
three cancellations and successful replacements, changes/restores the database,
rejects an added unverified voice alias, kills the helper while a child is
blocked, confirms child retirement and starts a working replacement helper.
The real frontend/runtime also pass simultaneous two-lane inventory, exact
preview, wrong-voice rejection and eSpeak fallback checks.

For the retained 22-word corpus, both platforms produced these durations:

| Host rate | Native frontend control | Audio seconds |
| --- | --- | --- |
| 0.0 | 80 | 14.104 |
| 0.5 | 175 | 6.679 |
| 1.0 | 350 | 2.958 |
| 1.5 | 400 | 2.181 |
| 2.0 | 450 | 1.457 |

These are single-run smoke measurements of canonical helper PCM, not an
Eloquence calibration, latency benchmark or intelligibility assessment.
The ordinary CLI also generated a WAV through the main conversion/effects
pipeline. Linux helper stress passed 12 varied syntheses. Verification included
830 locked workspace tests (one pre-existing ignored test), workspace Clippy,
Windows helper Clippy, formatting, documentation links and supported main builds.

To repeat the complete Linux probe after `make dev` and the private build:

```sh
python3 tools/verify_mbrola_prototype.py \
  target/mbrola-prototype/linux/runtime/omnivox-mbrola-helper \
  --server target/debug/omnivox --report /tmp/mbrola-linux.json
```

For native Windows use its staged `.exe` paths, `--scratch-dir` on the Windows
drive, and `--espeak-data` with the native parent of the launcher's shared
`espeak-ng-data` tree. `--server-only` verifies a newly staged main server while
reusing previously completed helper acceptance. Listening, extended soak,
calibration, progressive output, broader databases, installation UX and release
licensing/packaging remain separate from this development companion.

### Complete text and startup latency, 2026-09-17

The pinned eSpeak frontend's bulk stdin reader overwrites the final input byte
with NUL. The helper now supplies an explicit terminator: previously `focus`
was synthesized as `focu`, and a final multibyte character could also be damaged.
Native acceptance compares unterminated words against newline-terminated
controls, requires different audio for genuinely shortened words, and covers
`focus`, `lost focus`, `test`, `testing`, `hello`, and `café`. The regression fails
against the original helper.

The private bundle previously included 516 files, mostly unrelated languages.
The new builder retains only en1 dependencies and all notices, and regenerates
the data directory so obsolete languages do not survive a rebuild. A controlled
native Windows comparison with the same helper and five texts reduced median
synthesis time from 494.5 ms to 160.0 ms; canonical PCM was byte-identical for
each text. These are local warm-run measurements, not a general latency promise.
The prototype still buffers each utterance and starts its two native subprocesses
for each request.

The [updated Linux report](../experiments/2026-09-17-mbrola-linux.json) and
[updated Windows report](../experiments/2026-09-17-mbrola-windows.json) record rebuilt
artifact identities, complete-text checks and timing, native rate/pitch/mute,
cancellation/replacement and forced retirement, plus two simultaneous silent
server lanes exercising exact preview and fallback. Repeat with the verification
commands above. `python3 tools/test_build_mbrola_prototype.py` also checks stale
language removal and preserves the prior staging data if a required input is
missing. Audible acceptance of the original focus interaction remains a separate
listening check.

## MBROLA managed-library acceptance

Source: `docs/MBROLA-PROTOTYPE.md` at `ca38781`.

The [Linux library report](../experiments/2026-09-17-mbrola-library-linux.json) and
[Windows library report](../experiments/2026-09-17-mbrola-library-windows.json) record
all three real HTTPS downloads, retained notices, disabled installation, native
validation, independent per-request voices, two simultaneous streams, pinned
workers and disabled exact previews. Fresh compiled Emacs passed paired Apply,
injected notification replacement failure with rollback, and independent US1
disablement on both platforms. These used isolated voice roots and null audio.

Windows short-word synthesis was 146–149 ms for en1 and 159–167 ms for the US
voices in this small check; en1 retains the previous latency improvement. The
existing full-text, rate/pitch/mute, tamper rejection, native cancellation,
replacement and forced-retirement probes also passed on both platforms.
Workspace tests passed 835 tests with one existing ignored test; Linux workspace
and affected Windows Clippy passed. Emacs focused tests passed 85 with four
GUI-only skips, and all 18 graphical tests passed separately.

The subsequent [combined Windows report](../experiments/2026-09-17-mbrola-piper-windows.json)
records a fresh native Piper companion from commit `1d613c2` and complete Windows
development staging. Its [Piper CI run](https://github.com/bartbunting/omnivox/actions/runs/35162878166)
passed on Windows, Linux and both Mac architectures; this does not establish
macOS MBROLA support. Native staging checked the Kristin Piper model. In a fresh
compiled Emacs, a schema-2 library containing a Piper speaker fixture, Flite SLT
and the four MBROLA voices passed paired Apply, injected notification failure
with rollback, and US1 disablement. Exact synthesis from Piper, Flite, en1 and
enabled US1 passed on both streams before and after those transitions, using
null audio and an isolated library.

## Homebrew packaging

Source: `docs/STATUS.md` at `ca38781`.

macOS core installation is also available through the
[official Homebrew tap](https://github.com/bartbunting/homebrew-omnivox), initially
packaging Omnivox 1.12.0. Its
[native CI run](https://github.com/bartbunting/homebrew-omnivox/actions/runs/35784117656)
passed on Apple Silicon and Intel: archive verification, formula style and audit,
installation, eSpeak and Apple WAV synthesis, reinstall, a packaging-revision
upgrade, and removal. This does not establish audible Emacs acceptance or an
upgrade between different upstream releases. Formula updates remain an explicit
post-release step; optional engines are separate. See
[macOS installation](../../.github/DEPLOYMENT.md#macos-with-homebrew).
