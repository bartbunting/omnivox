# Rodio 0.23 migration checks

These checks cover the dependency migration before device-selection behavior is
added. They establish source and audio-component compatibility, not a complete
Windows payload, desktop deployment, listening result or latency improvement.
The [delivery plan](../plans/audio-device-selection.md) tracks those later gates.

## Candidate and method

The candidate starts at Omnivox `b34f103` and upgrades Rodio 0.19.0 to 0.23.0 and
CPAL 0.15.3 to 0.18.2. Enabled Rodio features are `playback`, `lewton` and `hound`;
the existing Vorbis/WAV decoders remain in use. Optional recording, dithering,
alternative CPAL backends and effects are not enabled.

The migration adapts source spans, nonzero format metadata, players and mixers,
while retaining the canonical 44.1 kHz stereo queue wrapper. Windows still opens
its selected default device directly. Rodio's configuration fallback may try
formats on that device, but this path does not use arbitrary-device fallback.

[Provenance](data/2026-11-13-rodio-upgrade/provenance.json) records the exact base,
source patch hash, lock hash, pinned Rust toolchain, native input locations and
commands. The [source patch](data/2026-11-13-rodio-upgrade/source.patch.gz) and
[checksums](data/2026-11-13-rodio-upgrade/SHA256SUMS) preserve the tested code and
raw results independently of later routing changes.

## Results

- Formatting baseline and final formatting checks passed. No untouched source
  formatting changed.
- Linux audio tests: 172 unit and 31 integration tests passed.
- Locked workspace tests: 1,218 passed, zero failed, two ignored.
- Locked workspace/all-target Clippy passed with warnings denied.
- Windows GNU audio/all-target compilation and Clippy passed with warnings denied.
- Native Windows audio integration tests: all 31 passed. The unit run passed
  154 tests, failed one fixture lookup and ignored the real-device test. The
  cross-compiled `CARGO_MANIFEST_DIR` pointed at a Linux path unavailable on
  Windows. Staging the unchanged Ogg fixture at its expected Windows path made
  that exact test pass; temporary fixture files and directories were removed.
  The original failure log is retained alongside the successful follow-up.
- The explicitly invoked native real-device test passed. It started zero-valued
  PCM on the actual Windows default device, forced owner-thread reconnection,
  observed cancellation of the old ticket and completion of fresh audio, and
  rejected admission after shutdown. It did not change the Windows default
  endpoint or test physical unplugging.
- Documentation and paired-link checks passed for the migration and proposal.

Raw Linux/native logs are compressed beside the provenance. Native executable
paths, hashes and initial exit codes are retained in
[native results](data/2026-11-13-rodio-upgrade/native-results.json); the provenance
also records the separate successful fixture follow-up.

## Limits

These native checks execute newly built audio test binaries, not an older staged
Omnivox runtime. Full development staging and real Eloquence/DECtalk speech checks
remain necessary before deploying the dependency change. No Emacsvox source or
desktop shortcut was changed for this migration. macOS, physical device switching,
acoustic timing and binaural rendering were not tested.
