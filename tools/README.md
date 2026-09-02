# Developer Tools

## Documentation links

`check_markdown_links.py` resolves repository-local links in every tracked
Markdown file and rejects missing targets, unsupported URI schemes, and links
that leave the repository. It does not make network requests for external
URLs. Run it directly or through:

```sh
make docs-check
```

## Build and runtime staging

`build.py` is the supported wrapper for distributable Cargo builds. It keeps
Cargo's locked dependency resolution, reads the exact `espeak-rs-sys` output
from Cargo's JSON build messages, and stages `espeak-ng-data` plus applicable
project and third-party licensing files beside the executable. The Makefile and
release workflow invoke it; additional Cargo build arguments pass through
unchanged:

```sh
python3 tools/build.py --release
python3 tools/build.py --release --target aarch64-apple-darwin
```

The wrapper fails rather than choose between non-identical eSpeak data outputs.

`build_rhvoice.py` builds the portable RHVoice helper with locked Cargo
dependencies and atomically stages it as `rhvoice/` beside the selected Cargo
profile output. It deliberately stages no RHVoice library or voice data; see
the [RHVoice companion guide](../docs/RHVOICE.md) for the user-installed
runtime contract:

```sh
python3 tools/build_rhvoice.py --release
```

`prepare_flite_inputs.py` downloads, safely extracts, and verifies the exact
Flite v2.2 release source and complete tree digest recorded in
`omnivox-flite-sys/source-inputs.json`. `--check` performs an offline cache
audit. `build_flite.py` then builds the helper with the target C compiler and
atomically stages `flite/` with the executable, Flite licence, Cargo lock,
source provenance, and exhaustive payload checksums:

```sh
make prepare-flite
python3 tools/prepare_flite_inputs.py --check
python3 tools/build_flite.py --release
```

Set `OMNIVOX_FLITE_INPUTS_DIR` to use a different verified cache. See the
[Flite companion guide](../docs/FLITE.md) for installation and optional local
voice configuration.

`package_flite.py` and `verify_flite_release.py` create and verify a
deterministic native `.tar.gz` or `.zip`, including safe relocation and real
SLT synthesis. `package_flite_source.py` and `verify_flite_source.py` create
the platform-neutral upstream-source/build-integration artifact and recheck
its exhaustive manifest, Git tree, input lock, and offline preparation:

```sh
make verify-flite
make verify-flite-source
```

`prepare_rutts_inputs.py` performs the equivalent locked archive and complete
tree verification for RuTTS v6.3.3. `build_rutts.py` compiles the portable C
core and Rust adapter, then atomically stages `rutts/` with both built-in
Russian voices, licensing, provenance, and exhaustive payload checksums:

```sh
make prepare-rutts
python3 tools/prepare_rutts_inputs.py --check
python3 tools/build_rutts.py --release
```

Set `OMNIVOX_RUTTS_INPUTS_DIR` to use a different verified cache. The
[RuTTS companion guide](../docs/RUTTS.md) documents the KOI8-R text boundary,
built-in voices, RuLex exclusion, and installation.

`package_rutts.py` and `verify_rutts_release.py` create and verify the
deterministic native `.tar.gz` or `.zip`, including safe relocation and real
male/female synthesis. `package_rutts_source.py` and
`verify_rutts_source.py` create and audit the platform-neutral upstream-source
and build-integration artifact:

```sh
make verify-rutts
make verify-rutts-source
```

`build_piper.py` builds the optional helper in relocatable mode, selects the
native Linux x64, Windows x64, or macOS ARM64/x64 library layout, and
atomically stages the companion as `piper/` beside the Cargo profile output.
The directory contains the helper, only the required native libraries, its
separately generated eSpeak data, project and third-party licensing files,
source/input provenance, and a SHA-256 manifest:

```sh
python3 tools/build_piper.py --release
```

Linux staging validates `$ORIGIN` lookup and resolved libraries; macOS staging
validates thin Mach-O architecture and `@loader_path` lookup; Windows staging
validates x64 PE identities. Native runner synthesis remains the final dynamic
loading check on every platform. Before Cargo runs,
`prepare_piper_inputs.py` downloads the target's checked-in eSpeak NG, Sonic,
and ONNX Runtime archives, verifies their SHA-256 digests, safely extracts
them, and records the extracted-tree digests. Downloads and extraction are
bounded to 100,000 members and 4 GiB compressed/downloaded or uncompressed
data, preventing a validly named pathological input from exhausting local
storage. It detects Linux x64, Windows
x64, and macOS ARM64/x64 native targets; `--target` remains available for an
explicit cache. A prepared cache can be checked without network access:

```sh
python3 tools/prepare_piper_inputs.py \
  --target x86_64-unknown-linux-gnu --check
```

`prepare_piper_test_model.py` separately downloads the English model used by
upstream libpiper tests at a locked repository revision, verifies the model,
configuration, and `MODEL_CARD` sizes and SHA-256 digests, and records the
prepared state under `target/piper-test-model/`:

```sh
make prepare-piper-test-model
python3 tools/prepare_piper_test_model.py --check
```

This model is for native acceptance only. Its checked-in lock records the
model card's LibriVox, public-domain, and trained-from-scratch declarations,
approves that exact revision for CI-only use, and requires that the model stay
out of release artifacts.

Create and verify the deterministic companion candidate for the current native
platform with:

```sh
make package-piper
make verify-piper
```

The result is one of the following, with its outer digest written to
`target/release/piper-sha256sums.txt`:

| Native platform | Companion candidate |
| --- | --- |
| Linux x64 | `omnivox-VERSION-piper-linux-x64.tar.gz` |
| macOS ARM64 | `omnivox-VERSION-piper-macos-arm64.tar.gz` |
| macOS x64 | `omnivox-VERSION-piper-macos-x64.tar.gz` |
| Windows x64 | `omnivox-VERSION-piper-windows-x64.zip` |

Each archive extracts as one `piper/` directory, ready to place beside the
matching generic `omnivox` executable. To add real synthesis through the
relocated main binary, supply a licence-reviewed model and adjacent
configuration:

```sh
PIPER_MODEL=/path/to/voice.onnx make verify-piper
```

Create and verify the platform-neutral corresponding-source artifact with:

```sh
make package-piper-source
python3 tools/verify_piper_source.py
```

`package_piper_source.py` archives the exact committed Omnivox and vendored
libpiper tree, every Cargo registry source selected by `Cargo.lock`, the
checksum-locked eSpeak NG and Sonic sources, all four ONNX Runtime binary build
inputs, and the exact ONNX Runtime source revision. The deterministic result is
`target/release/omnivox-VERSION-piper-source.tar.gz`. Its exhaustive manifest
records every file's mode, size, and SHA-256 digest. The verifier compares the
Omnivox tree to its recorded Git commit, rechecks every input against the
archived locks, and resolves the Cargo graph offline from the included vendor
directory. CI voice-model payloads are excluded.

`verify_piper_release.py` checks the outer and exhaustive inner checksums,
exact payload and notice sets, clean source provenance, native binary
architecture, platform library lookup, and optional model-backed voice
discovery and WAV output. Linux uses ELF/RUNPATH and `ldd` checks, macOS uses
Mach-O, `@loader_path`, and `otool` checks, and Windows validates the PE import
chain from the helper through `piper.dll` to ONNX Runtime. When a model is
supplied, it also confirms that missing and corrupt Piper models fail exact
Piper diagnostics while an explicit eSpeak diagnostic remains usable.
Model-backed archive acceptance passes on all four initial native runners. The
tag workflow packages and re-verifies the four companions and source artifact.
Piper archives are published beginning with v1.6.4. Release code remains
unsigned.

## Release archive verification

`verify_release.py` checks an exact `.tar.gz` or `.zip` release asset against
`sha256sums.txt`. It safely extracts into a relocated path, validates the
published payload and binary architecture, and can exercise real headless
engine synthesis through `--dump-wav` without requiring an audio device.
Extraction rejects encrypted or special entries, duplicate or unsafe paths,
more than 100,000 members, and more than 4 GiB of declared uncompressed data.
The same bounds protect companion and corresponding-source verification.

For example:

```sh
python3 tools/verify_release.py \
  --archive omnivox-1.4.1-linux-x64.tar.gz \
  --checksums sha256sums.txt \
  --version 1.4.1 \
  --platform linux \
  --arch x86_64 \
  --engines espeak
```

The tag workflow uploads draft release assets first, downloads them on the
native platform runners, and publishes only after this verification passes.
Every current release target performs native synthesis verification. An empty
engine list remains available for an intentional structural and architecture
check that does not exercise synthesis.

`verify_release_asset_set.py` independently requires the complete release to
contain exactly the 24 documented archives and `sha256sums.txt`. Directory mode
also requires the checksum manifest to name every archive exactly once; names
mode protects the uploaded draft and the final publication step from missing,
duplicate, or stale cached assets.

## Speech-rate audit

`audit_speech_rates.py` measures both the raw synthesized WAV and the canonical
WAV after Omnivox silence trimming and output processing. Reported WPM uses the
canonical duration. It uses exact diagnostic engine selection, so an absent
helper fails instead of contributing fallback samples. Repeated targets may
select an exact voice after `=`:

```sh
python3 tools/audit_speech_rates.py target/debug/omnivox \
  --target espeak --target flite=cmu_us_slt \
  --rates 0.3,0.4,0.5,0.6,0.8 --repetitions 3 \
  --json-output /path/to/private/english-rate-audit.json
```

Run language-specific engines separately with a representative corpus. For
example, audit RuTTS with Russian text and an exact built-in voice rather than
comparing its Russian duration with English engines:

```sh
python3 tools/audit_speech_rates.py target/debug/omnivox \
  --target rutts=male --text 'Проверка скорости русской речи.' \
  --rates 0.3,0.4,0.5,0.6,0.8
```

The JSON report records the corpus hash, byte and word counts, executable hash,
raw samples, and median duration/WPM, but not the corpus text or absolute
program path. Existing reports are never overwritten. Use `--work-dir` to
retain audio for listening; existing WAVs are never overwritten. When WSL
launches a Windows executable directly, add `--windows-output-paths`; for
reliable Windows-local I/O, choose a `--work-dir` below `/mnt/c`.

The tool invokes the one-shot `--dump-wav` action separately for each sample,
so every sample starts a fresh Omnivox process and engine. That is a consequence
of the current exact diagnostic interface, provides no benefit to the duration
calculation, and makes a sweep slower. Startup and synthesis wall-clock time
are not measurements: only the completed WAV frame count contributes to
duration and WPM. See the
[speech-rate calibration guide](../docs/RATE-CALIBRATION.md) for the current
reference procedure.

## Server lifecycle benchmarks

`benchmark_server.py` measures cold and warm lifecycle latency through the
public Omnivox protocol using only the Python standard library. It negotiates
capabilities, sends structured presentations, timestamps the first
`utterance_started` mixer-source marker and tracked terminal record with a
monotonic clock, verifies the realized engine and physical voice when requested,
and reports nearest-rank p50/p95/p99 values. Default workloads cover character,
word, line, dense semantic actions, multipart assembly, and a rapid same-key
replacement burst:

```sh
python3 tools/benchmark_server.py ../emacsvox/servers/omnivox \
  --engine native --expected-engine-id winrt \
  --mode both --iterations 20 --warmups 2 \
  --provenance ../emacsvox/servers/omnivox-bin/current/PROVENANCE \
  --json-output /path/to/private/winrt-latency.json
```

Repeat `--case` to select workloads. Cold mode starts a new server for every
sample; warm mode keeps one process alive. The JSON report includes raw samples,
host and Python identity, server version, command, actual engine and physical
voice counts, and an optional bounded `KEY=VALUE` provenance file. Marker
receipt measures mixer-source consumption and may precede audible device output.

Eloquence and DECtalk are accepted Windows startup preferences when their
helpers and user runtimes are installed. To exercise routing-policy replacement,
start the ordinary native server and apply one strict session preference:

```sh
python3 tools/benchmark_server.py ../emacsvox/servers/omnivox \
  --engine native --preferred-engine-id dectalk \
  --expected-engine-id dectalk --iterations 20
```

The harness applies the public routing policy to every cold process and once to
the warm process. It configures no fallback engines, and the expected-engine
check prevents a different realized engine from contaminating results.

Use the Russian profile and an exact voice for meaningful RuTTS measurements:

```sh
python3 tools/benchmark_server.py ../emacsvox/servers/omnivox \
  --engine rutts --expected-engine-id rutts --voice-id male \
  --text-profile rutts-ru --mode both --iterations 20 --warmups 2 \
  --json-output /path/to/private/rutts-male-latency.json
```

Repeat with `--voice-id female`. The profile contains only ASCII and Russian
characters that RuTTS can convert losslessly to KOI8-R. Exact-voice selection
uses the public logical-voice registry and rejects fallback at registration or
dispatch. Reports carrying physical-voice fields use report schema version 2.

For cross-engine comparison, put common settings and strict per-engine routes
in a bounded JSON plan, then let `benchmark_suite.py` randomize their order for
every repeat using a recorded seed:

```json
{
  "plan_version": 1,
  "server": "../../emacsvox/servers/omnivox",
  "provenance": "../../emacsvox/servers/omnivox-bin/current/PROVENANCE",
  "seed": 20260901,
  "repeats": 2,
  "benchmark": {"mode": "both", "iterations": 20, "warmups": 2},
  "runs": [
    {
      "id": "winrt",
      "engine": "native",
      "expected_engine_id": "winrt"
    },
    {
      "id": "rutts-male",
      "engine": "rutts",
      "expected_engine_id": "rutts",
      "voice_id": "male",
      "text_profile": "rutts-ru"
    }
  ]
}
```

Paths are resolved relative to the plan. Run it with a new output directory:

```sh
python3 tools/benchmark_suite.py /path/to/plan.json \
  /path/to/new-benchmark-evidence
```

The runner refuses an existing output directory. It writes one raw schema-v2
benchmark report per run plus an atomically updated `suite.json` containing the
plan hash, seed, realized execution order, report hashes, and completion state.
If a benchmark fails, partial evidence remains labelled `failed` for inspection
instead of being overwritten or mistaken for a complete suite.

## Server cancellation and recovery stress

`stress_server.py` uses the same public protocol client to interleave two
replaceable domains with ordered and urgent survivors. It validates contiguous
marker sequence, completed semantic callbacks, expected cancellation, exactly
one terminal record, absence of post-terminal events, periodic hard stops, and
successful speech after each stop. Schema-v2 reports retain the exact physical
voices observed on completed dispatches:

```sh
python3 tools/stress_server.py ../emacsvox/servers/omnivox \
  --engine flite --expected-engine-id flite \
  --iterations 25 --stop-every 5 \
  --provenance ../emacsvox/servers/omnivox-bin/current/PROVENANCE \
  --json-output /path/to/private/flite-stress.json
```

The optional helper fault probe takes an exact process name, snapshots the
process table before starting its dedicated server, and refuses ambiguous,
pre-existing, or unrelated targets. It kills only the one validated child PID,
then requires the named fallback, requests an engine recovery probe, and
requires a later dispatch to return to the recovered engine. Dispatch mode
submits a bounded long request before the kill, hard-stops that request, and
checks its exactly-once cancellation before separately proving fallback:

```sh
python3 tools/stress_server.py ../emacsvox/servers/omnivox \
  --engine flite --expected-engine-id flite --iterations 5 \
  --fault-helper-process omnivox-flite-helper.exe \
  --fault-engine-id flite --fallback-engine-id espeak \
  --fault-mode dispatch --fault-count 3
```

On POSIX, use the helper executable's exact basename without `.exe`. Fault
injection is never enabled by default. `--fault-mode idle` retains the original
kill-before-dispatch probe. `--fault-count` bounds repeated crash cycles from 1
through 100; each cycle re-resolves one current child before acting. Optional
`--fault-delay-ms` applies only after dispatch submission and is capped at five
seconds.

The stress harness accepts the benchmark tool's strict routing controls. Use
`--preferred-engine-id` for registered-only DECtalk or Eloquence, and combine
`--voice-id` with `--expected-engine-id` to reject a different physical voice.
RuTTS needs its Russian profile:

```sh
python3 tools/stress_server.py ../emacsvox/servers/omnivox \
  --engine rutts --expected-engine-id rutts --voice-id female \
  --text-profile rutts-ru --iterations 25 --stop-every 5 \
  --json-output /path/to/private/rutts-female-stress.json
```

The exact logical route is used for ordinary, replaceable, urgent, and hard-stop
recovery speech. During an explicit helper-fault probe, the fallback voice is
recorded without being mistaken for the requested voice; the recovered helper
must return to the requested engine and physical voice.

Long runs can sample the complete server/helper process tree. A direct native
server needs only `--resource-sample-every N`; when WSL uses a launcher script,
also name the exact Windows root:

```sh
python3 tools/stress_server.py ../emacsvox/servers/omnivox \
  --engine flite --expected-engine-id flite --iterations 100 \
  --resource-sample-every 5 --resource-process-name omnivox.exe \
  --json-output /path/to/private/flite-server-soak.json
```

The report aggregates the root and its current descendants and also groups
metrics by executable basename. It includes a complete launch-to-shutdown
summary and a separate steady-state summary that excludes the initial process
sample. Exact new-root resolution is mandatory on Windows; ambiguity disables
observation instead of including an unrelated process.

## Failure diagnostics

`collect_diagnostics.sh` creates a bounded archive containing recent Omnivox
session logs, build/runtime identity, process inventory, and relevant Windows
events. It strips synthesis-text log records, redacts checkout and common
user-home paths, omits process command lines, and stores runtime hashes by
basename. It does not include Windows memory dumps; inspect the result before
sharing because native error text can still carry unexpected private data. See
[`docs/DIAGNOSTICS.md`](../docs/DIAGNOSTICS.md) for the failure workflow and the
opt-in `configure_windows_crash_dumps.ps1` helper.

## Windows helper session stress

`stress_helper.py` keeps one protocol-v4 helper process alive across repeated
synthesis calls. It validates negotiation, descriptor identity, realized
voice, audio sequence and frame totals, all marker kinds advertised by the
engine (including a DECtalk native-index probe), periodic pings, and clean
shutdown. With `--cancel-probe`, it also
cancels one long in-flight synthesis, rejects stale output after the
acknowledgement, and checks that the helper remains responsive. It records
process resources through `/proc` for native POSIX helpers and Windows CIM for
`.exe` helpers launched from WSL. The native Piper workflow uses both paths on
every initial platform.

From WSL, after running `make windows-helpers` in the Omnivox checkout:

```sh
python3 tools/stress_helper.py \
  windows-helpers/bin/OmnivoxEloquenceHelper32.exe \
  --engine-id eloquence --iterations 100

python3 tools/stress_helper.py \
  windows-helpers/bin/OmnivoxDectalkHelper32.exe \
  --engine-id dectalk --iterations 100 --cancel-probe \
  --helper-arg "$(wslpath -w /path/to/IA32/DECtalk.dll)"
```

Use `--voice-id` to select a non-default voice and repeat `--helper-arg` when a
helper needs an explicit native DLL argument. Use `--cancel-every N` for
repeated in-flight cancellation, `--health-every N` for liveness probes, and
`--resource-sample-every N` for long-session measurements. A successful
`--json-output FILE` report contains elapsed samples and first/last/minimum/
maximum/growth summaries for available working-set, private-byte, virtual-byte,
handle, thread, and CPU counters.

Both the complete resource summary and a steady-state summary excluding the
initial process sample are retained, so runtime initialization is not silently
reported as long-session growth.

When WSL launches a Windows `.exe`, the tool snapshots Windows processes first
and binds only one newly created process with the exact executable name. It
reports metrics as unavailable rather than guessing when resolution is
ambiguous. Reports retain only the helper basename and argument count, not
local runtime paths. No memory-growth limit is implied; release or experiment
plans must state any threshold explicitly.

The in-process eSpeak counterpart is ignored during ordinary unit tests and
can be run explicitly:

```sh
cargo test --locked -p omnivox-tts stress_repeated_synthesis_session \
  -- --ignored --nocapture
```

## Audible feature smoke test

[test-all-features.sh](../test-all-features.sh) exercises legacy protocol
commands, the selected native and eSpeak startup engines, integer speech rates,
and left/right/both channel routing. It also exercises Piper when both
`PIPER_MODEL` and an adjacent or explicitly selected helper are available:

```sh
OMNIVOX_BIN=./target/release/omnivox ./test-all-features.sh
```

The script requires Bash and `timeout`. Its pass/fail result detects process
errors and timeouts; a listener must still verify the audible rate and channel
claims. This is a manual smoke test, not a CI latency or audio-quality gate.

## WAV comparison

`compare_wavs.py` compares two PCM16 or float32 WAV files after silence
trimming and reports RMS difference, correlation, SNR, and per-segment values.
`tts_reference.swift` captures a macOS AVSpeechSynthesizer reference WAV.
