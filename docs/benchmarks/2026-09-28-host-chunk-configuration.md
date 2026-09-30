# Host chunk-size configuration acceptance

This report covers the post-1.13.0 change at `e7293af`: configuration version 2
can choose 1–100 words per synthesis request while retaining 15 as the default.
See the [contract](../reference/engine-configuration.md#configuration-version-2)
and [decision](../adr/0009-local-speech-preferences.md).

## Functional evidence

Linux checks with pinned Rust 1.97.1 passed:

- Locked workspace tests: 1,103 passing results, including one child-process
  test result, and two ignored tests. New cases cover strict parsing, historical
  snapshot compatibility, configured action-window admission/preparation,
  Unicode boundary mapping and reset retention.
- Locked workspace Clippy with all targets and the CLI Piper feature.
- `make dev`, formatting and documentation checks.
- The process acceptance harness with a real external fixture process. Actual
  synthesis requests split seven words as 3/3/1 for previews, ordinary speech
  after reset, the other lane after files/environment change, and recovery from
  the retained owner record. Independent retirement and blocked-write cleanup
  checks also passed.
- The remote shared-snapshot/total-disconnect recovery acceptance test.

These checks establish configuration and ownership behavior. The fixture
generates a tone and does not establish native speech quality.

Native Windows also passed the same process acceptance harness, using its C#
fixture and Windows-local storage. The candidate came from the full Emacsvox
`windows-omnivox-dev` build, staged under a separate runtime root. That build's
package and native inventory checks passed. It did not replace the installed
launcher or restart the user's speech. Windows build provenance and logs are
retained; this development package omits Piper under the existing build policy.

## Exploratory Linux eSpeak timing

The same debug executable and exact voice `espeak:gmw/en-US` ran with limits
15, 5, 30, 30, 5, 15. Each run had two warmups and ten samples per workload.
Null output consumed PCM without opening a device or waiting for its duration.
The existing line and rapid-replacement workloads completed; replaced requests
reported cancellation. Combined medians from twenty samples per setting:

| Words | Line: first source / terminal | Replacement: first source / terminal |
| --- | --- | --- |
| 5 | 7.51 / 425.10 ms | 27.91 / 828.21 ms |
| 15 | 7.06 / 365.93 ms | 27.74 / 730.20 ms |
| 30 | 7.89 / 406.06 ms | 28.62 / 817.06 ms |

A Windows build ran concurrently, so scheduling load is a confound. These are
exploratory source/throughput observations, not evidence of an optimal setting
or an audible improvement. A debug build and null playback do not represent
normal listening. Keep the default unchanged. Other engines, macOS, sustained
reading and physical onset/stop-to-silence remain unmeasured here.

## Windows eSpeak timing

The native Windows release-profile executable passed the same six-run sequence,
with twenty samples per setting/workload and exact voice `espeak:gmw\en-US`.
The Python harness ran in WSL; each speech process ran on Windows with its
matching staged eSpeak data. Compilation had finished before these runs.

| Words | Line: first source / terminal | Replacement: first source / terminal |
| --- | --- | --- |
| 5 | 1.99 / 29.61 ms | 30.05 / 87.03 ms |
| 15 | 1.91 / 26.10 ms | 28.57 / 80.12 ms |
| 30 | 1.96 / 26.54 ms | 29.56 / 80.37 ms |

Every measured request completed or was cancelled as expected. These small
samples support operation at each setting, without showing a reason to change
the default. Do not compare the absolute Windows/Linux timings: build profiles
and background load differ. Both runs use null output, with no listening or
physical sound measurement.

## Pre-existing dense-marker failure

The dense-action eSpeak workload failed at the default 15-word limit, reporting
14 of 15 requested anchors after partial output. The pre-change release binary
reproduced that same failure. Both logs and executable hashes are retained.
This establishes that the observed failure predates this configuration change;
it does not establish its root cause. The failed dense workload is excluded
from the passing timing table and remains native marker work to investigate.

## Reproduction and provenance

[Raw records](data/2026-09-28-host-chunk-configuration/) retain logs, complete
sample reports, configuration files, a reproduction script, executable hashes
and source identity. The existing harness supplies workload text and exact voice
checks; `reproduce.py` records the sequence and options. Reruns must create a new
directory. Timing comparisons should use an idle machine and matched builds.

Functional commands from the Omnivox root, with the pinned toolchain and prepared
Piper inputs selected:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets --features omnivox-cli/piper -- -D warnings
make dev
python3 tools/verify_engine_configuration.py /path/to/staged/omnivox
OMNIVOX_REMOTE_TEST_PROGRAM=/path/to/staged/omnivox OMNIVOX_ENGINE=espeak \
  python3 tools/test_remote_service.py RemoteServiceTests.test_engine_snapshot_is_shared_and_survives_total_disconnect
```
