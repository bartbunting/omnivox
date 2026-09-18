# ADR 0017: Preserve speech when a managed engine is unavailable

Status: Accepted under the maintainer's explicit fallback requirement on 2026-09-18.

This refines ADR 0012's main-server startup checks and restores ADR 0001's
engine-isolation rule. A valid active library containing Piper voices must not
prevent ordinary speech when the executable was built without Piper support.
The same rule applies to missing companions, rejected runtimes, bad provider
assets, startup failures and incomplete native voice inventories.

Ordinary server startup retains the immutable generation and its administrative
voice exclusions. It verifies assets before loading each provider, registers
failed or absent providers as unavailable with their reason, and selects an
available engine through existing routing. No failed provider contributes
invented voices to inventory or eligibility. Rescanning a configured failed
helper repeats both asset and exact-inventory checks. An undiscovered companion
or missing compiled feature requires configuration/build repair and restart.

Exact diagnostics and exact voice previews still fail for an unavailable target.
Native download validation remains strict. Apply still compares both workers'
actual eligible voice sets with the candidate and cannot acknowledge missing
voices as successfully applied. Degraded startup alone is not complete Apply.

Malformed or unreadable generation metadata remains an explicit startup error:
silently dropping it could re-enable excluded voices. The fallback change does
not bypass asset verification, rewrite the active library, enable disabled
engines, replay audio after PCM commitment, or hide device failures. Startup
still fails if no eligible engine can synthesize. No protocol or dependency is
added, and all preceding runtime-supply and native acceptance rules still apply.
