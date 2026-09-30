# Engine framework version-1 acceptance audit

Functional development evidence, 2026-09-28. The accepted first slice is complete:
an independently installed helper can be registered through configuration, used
for speech, and recovered with its original settings. Both speech workers share
the prepared settings and confirm what they received. This audit closes the
implementation checklist from the former extensible-engine plan.

The maintained contract is now [engine configuration version 1](../reference/engine-configuration.md).
[Language-routing changes](../plans/language-routing.md) remain a separate proposal.
Completion here does not qualify every native engine or platform, or publish a
release.

## New checks

Test commit `d17b12c` fills the final configuration coverage gaps:

- CLI, environment and default-folder precedence, including invalid and missing
  values, is checked against Windows, macOS and Unix path rules.
- Native Windows checks use real directory junctions: a redirected settings root
  works, while redirected manifest entries and a redirected `helpers.d` do not.
- Unix checks reject an unreadable selected root. The recorded run used a
  non-root account, so its permission checks executed.
- Disabled manifests still count toward the file limit.
- An explicit enable override can enable a disabled manifest, but cannot clear
  a local exclusion or a failed managed-asset check.

| Check | Result | Retained output |
| --- | --- | --- |
| Locked workspace tests | Passed | [Log](data/2026-09-28-engine-framework-audit/workspace-tests.log.gz) |
| Locked workspace all-target Clippy | Passed, warnings denied | [Log](data/2026-09-28-engine-framework-audit/workspace-clippy.log.gz) |
| Native Windows configuration tests | 40 passed, no skips | [Log](data/2026-09-28-engine-framework-audit/windows-configuration.log.gz) |
| Windows configuration all-target Clippy, no default features | Passed, warnings denied | [Log](data/2026-09-28-engine-framework-audit/windows-clippy.log.gz) |

The focused Linux configuration run also passed all 37 tests; its coverage is
included in the retained workspace log. Formatting and whitespace checks passed.
[Provenance](data/2026-09-28-engine-framework-audit/provenance.json) records the
source, pinned toolchain, host and exact commands; [checksums](data/2026-09-28-engine-framework-audit/SHA256SUMS)
cover the pack. Each listed suite ran once after the test additions. These are
pass/fail checks, not latency samples or acoustic measurements.

## Acceptance checklist

All 16 requirements were audited against implementation, automated regressions
and the retained process reports. The table identifies the evidence boundary:
some rules are exercised in isolated tests, while registration and shared startup
also run through real executables. It does not claim that every malformed-input
case ran through every native engine.

| Requirement | Coverage and result |
| --- | --- |
| Compatibility | Passed: absent roots/files and minimal configuration preserve defaults; shipped path, alias, environment, provider-argument and startup-order tests remain passing. See [reader tests](../../omnivox-tts/src/engine_configuration/tests.rs), [resolution tests](../../omnivox-tts/src/engine_configuration/resolved_tests.rs) and [CLI engine tests](../../omnivox-cli/src/engine.rs). |
| Root resolution | Passed: rules for all three platforms, explicit precedence/errors, absent defaults and unreadable Unix roots; native Windows process checks use Windows paths from WSL. See the new tests and [Windows process report](2026-09-28-windows-engine-snapshot.md). |
| Strict reader | Passed: malformed JSON, decoded duplicate keys, null/types/schema, UTF-8/BOM, nesting, file/aggregate sizes, ID/path/argument and routing/override bounds. Invalid and disabled candidates count toward limits. See [reader tests](../../omnivox-tts/src/engine_configuration/tests.rs). |
| Paths and arguments | Passed: drive/UNC/Unix validation, literal empty/spaced/shell-looking arguments, Unix links/special files and Windows junction rejection. Real helper launches preserve arguments on Linux and Windows. File-symlink creation on Windows was not separately exercised; the native reparse test uses junctions. |
| Identity | Passed: ordered inventory, duplicate and reserved IDs, descriptor ownership and wrong-engine rejection. See [reader tests](../../omnivox-tts/src/engine_configuration/tests.rs), [registry tests](../../omnivox-tts/src/engine_registry.rs) and [helper tests](../../omnivox-tts/src/helper_engine.rs). Reserved metadata is independent of compiled feature availability. |
| Overrides | Passed: field precedence, complete argument replacement, partial timeouts, unknown references, in-process restrictions, managed argument conflicts, enablement and retained exclusions. See [resolution tests](../../omnivox-tts/src/engine_configuration/resolved_tests.rs) and [CLI configuration tests](../../omnivox-cli/src/engine/configuration_tests.rs). |
| Timeouts and startup | Passed: timeout range edges and defaults; four-slot admission, batch expiry, retained unfinished work, cleanup and no late publication. [Initialization tests](../../omnivox-tts/src/helper_engine/initialization.rs) and the [deadline report](2026-09-27-engine-startup-deadline.md) cover bounded behavior. Review confirms selected-first ordering and separate shipped initialization in the shared startup path. |
| Registration path | Passed through real Linux and Windows executables: unknown helper ID, inventory, exact listing, preview and tracked ordinary speech after writing one manifest. See [process fixture](../../tools/verify_engine_configuration.py) and the [Windows report](2026-09-28-windows-engine-snapshot.md). |
| Selection matrix | Passed: exact/engine/policy scope, default exclusion from unrestricted matching and local automatic permission. See [resolver tests](../../omnivox-tts/src/resolver.rs), [routing policy tests](../../omnivox-tts/src/routing_policy.rs) and [startup resolution tests](../../omnivox-tts/src/engine_configuration/resolved_tests.rs). |
| Disablement | Passed: disabled engines avoid construction/rescan, local exclusions survive session replacement, and voice-library exclusions remain authoritative. See [registry tests](../../omnivox-tts/src/engine_registry.rs), [routing tests](../../omnivox-tts/src/routing_policy.rs) and [CLI library tests](../../omnivox-cli/src/engine/library_tests.rs). |
| Failure isolation | Passed: invalid optional registrations and failed helpers retain unrelated eligible choices; malformed main policy stops startup; exact requests fail without substitution. See reader/CLI/helper tests and the process fixture's missing-engine and invalid-new-activation checks. |
| Protocol and audio | Passed through the existing shared client: version negotiation, buffered/progressive/empty synthesis, bounded and correlated PCM/markers, cancellation and no replay after commitment. See [helper tests](../../omnivox-tts/src/helper_engine.rs), [wire tests](../../omnivox-tts/src/helper_protocol/wire_tests.rs) and the workspace suite. Fake-helper process acceptance uses synthetic PCM, not a speech runtime. |
| Recovery snapshot | Passed: file/environment mutation, complete retained records, fresh activation, renewed live negotiation and cleanup blocking replacement. See [snapshot tests](../../omnivox-tts/src/engine_configuration/snapshot_tests.rs), helper recovery tests and Linux/Windows process reports. |
| Two workers | Passed: shared preparation, independent acknowledgements, changed files between starts, independent lane recovery/retirement, failed-start rollback, reconnect and deliberate new activation. [Original integration evidence](2026-09-27-engine-framework.md) and [Windows evidence](2026-09-28-windows-engine-snapshot.md) retain compiled-client and remote process results. |
| Distribution independence | Passed: the same fake adapter has matching identity, descriptors and PCM through external discovery and a test-only shipped definition. Reserved-ID collisions reject old manifests; explicit main-file overrides remain allowed. See [CLI fixture](../../omnivox-cli/src/engine/configuration_tests.rs) and [process acceptance report](2026-09-27-engine-framework-acceptance.md). No private runtime is needed. |
| Diagnostics | Passed: activation/source/engine identity, unavailable policy references and exact/startup distinction, without exposing arguments or captured environment. Reader tests check no configuration writes; [status tests](../../omnivox-tts/src/engine_configuration/status_tests.rs) and process checks cover privacy. |

## Scope and remaining qualification

No production code changed in this final audit. It did not rebuild or redeploy
the runnable payloads. The previous [Windows qualification](2026-09-28-windows-engine-snapshot.md)
remains the process evidence for runtime `454e40235fb929ce`, with nine compiled
local-client tests and eight remote checks passing. Its full development staging
included eSpeak, Flite, RuTTS and TGSpeechBox checks; Piper was not included.
The existing Windows launcher default is unchanged.

macOS configuration rules have automated coverage, but this work adds no native
macOS process evidence. Each maintained engine still needs its own native runtime,
PCM, controls, cancellation and repeat-use qualification on its supported targets.
Nothing here establishes audible output, listening quality or performance.

The earlier full Emacs suite had one unrelated startup-inventory failure, recorded
in the [original report](2026-09-27-engine-framework.md). That suite was not rerun
or repaired by this test/documentation slice. Existing raw reports, including the
original deadline and Windows startup failures, remain unchanged.
