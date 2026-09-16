# Disposable native voice validation

The development `--validate-voice-library` command checks the managed Piper and
Flite load sets in one runtime generation without playing audio or changing the
installed index, active pointer, or running speech workers. It is a prerequisite
for the voice manager, not an activation command. The `voice_library_v1`
capability remains unadvertised.

## Running a check

Use a staged Omnivox executable and the exact intended companion executables:

```sh
omnivox --validate-voice-library /absolute/path/generation.json \
  --piper-helper /absolute/path/piper/omnivox-piper-helper \
  --flite-helper /absolute/path/flite/omnivox-flite-helper \
  --validation-timeout-seconds 60 --validation-memory-mib 4096
```

Supply the helper for each provider with a nonempty projection. Paths are native
paths on the machine running Omnivox. Windows accepts the corresponding `.exe`
paths. Helpers are explicit; discovery and legacy model-file overrides cannot
silently select different data for this check. A null provider contributes no
managed validation work; it does not validate that provider's legacy settings.

Keep the command's stdin open while validation runs. EOF or any input cancels
owned work. A calling application should retain its pipe until the command exits,
and close it to cancel. Cancellation, native failure, timeout, invalid output,
or unconfirmed cleanup ends the command with failure before another load starts.
Progress lines are not a final success result; callers must also check the exit
status. This development text output is not a versioned management API.

Before loading, the command reports the load count, memory budget and deadline.
The default deadline is 60 seconds for each load, including verification,
initialization and synthesis. It can be set from 1 to 600 seconds. Cleanup gets a
separate five-second attempt and reports failure rather than waiting forever;
drop makes one further bounded attempt if necessary.

The memory budget defaults to 4096 MiB, adjustable from 256 to 65536 MiB:

- Windows limits committed memory for the complete private job.
- Linux limits each worker/helper's virtual address space with inherited
  `RLIMIT_AS`. This is not an aggregate RAM limit, and shared mappings and native
  thread stacks also consume address space.
- macOS samples the sum of `ri_phys_footprint` for the private process group
  during each supervisor wait, normally every 10 milliseconds. Exceeding the
  budget terminates the group and fails validation. This is a sampled cutoff,
  not a hard allocation cap: transient spikes or allocations between samples
  can exceed it. Missing accounting or a full 256-process snapshot fails closed.

These budgets have different operating-system meanings. They do not measure
model RAM, and a low budget may reject an otherwise valid large model. macOS
uses footprint accounting because its virtual address-space limit can reject
a worker whose existing mappings already exceed the requested limit. The
implementation uses the fixed V0 record in Apple's
[resource header](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.81.4/bsd/sys/resource.h)
and the process-group enumeration in
[libproc](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.81.4/libsyscall/wrappers/libproc/libproc.c).

## Saving and comparing evidence

Add `--validation-report /absolute/path/result.json` to save a completed run.
The destination must not already exist. Omnivox verifies the generation's assets,
the validator executable and every file in each selected staged companion before
native loading and again after successful native checks and cleanup. Changed
inputs, failed checks or cancellation prevent publication.

Use the same generation, helpers, working directory and limits with
`--check-validation-report /absolute/path/result.json` to compare saved evidence
with current observations. This performs file verification without loading native
voices. A matching report does not authorize activation or skipping future native
validation. Saving and comparing are separate operations.

Reports currently require bundled companion data and native libraries. Nonempty
Piper/eSpeak data overrides or native-loader overrides are rejected with an
explanation. Ordinary validation without a report retains its existing behavior.
Both checksum observations run as owned workers with the configured memory
budget and deadline, separately from each native load.

Reports contain local paths, generation JSON, per-load voice identities,
checksums, search configuration, limits and completion time. They are bounded
local observations, not authenticated attestations. Publication uses a complete
temporary file and a non-overwriting hard link in the destination directory;
filesystems without hard links fail. This establishes complete-file visibility,
not power-loss recovery or ownership of interrupted operations. Cancellation
after publication cannot retract a report. See the
[evidence design](voice-validation-evidence-design.md) for the precise scope,
limits and excluded runtime dependencies.

## What is checked

The supervisor parses the original generation once and retains its exact digest.
It writes private temporary projections, each containing one native load:

- One Piper model with all its enabled speakers. A fresh helper loads that model
  and produces a short PCM result for each speaker using exact physical IDs.
- One external Flite voice, with compiled-in SLT omitted. SLT, when enabled,
  receives its own separate probe.

Each projection has its own byte digest; it is not an acknowledgement that the
original generation was applied. The worker and helper check the projected bytes
and asset hashes before loading. The worker requires the expected inventory,
nonempty PCM and the exact requested identity. Samples are discarded without
creating an audio output. Validation establishes compatibility with this probe,
not speech quality, acoustic output or permanent validity of mutable imports.

The supervisor admits the next projection only after the preceding worker tree
and pipe readers are confirmed finished. Successful checks remove their private
projection files. Failures retain scratch inputs and identify the directory in
probe diagnostics; they never delete voice assets. Scratch is not an installed
revision or a durable operation journal.

## Process ownership

The supervisor runs as a dedicated process, outside either speech lane. Its
worker waits for a startup gate before constructing any native helper.

Windows assigns that worker to a private job before opening the gate. The job
also owns descendants, has a memory limit, and kills them when its last owning
handle closes, including supervisor exit. Cleanup explicitly terminates the job,
checks its active-process count, reaps the direct worker and joins its readers.
The remote service's existing job configuration retains its prior limits.

Linux starts the worker in a private process group. Only the dedicated supervisor
becomes a child subreaper, so it can reap orphaned helper descendants without
changing speech-worker lifecycle rules. A worker thread watches the supervisor's
pipe and kills its own group on parent exit. Normal retirement signals the owned
group before reaping its leader, then checks group absence and reader completion.
It never sends another group signal after reaping begins, avoiding PID-reuse
hazards.

macOS uses the same private-group startup gate and parent-pipe watcher. It
reaps its direct worker and waits for group absence as the system reaps orphaned
descendants; it does not claim Linux subreaper behavior. Cleanup requires both
group absence and reader completion and never signals a group again after
reaping starts. Darwin can report `EPERM` for a group containing only zombies;
this remains pending until group absence is observed within the cleanup deadline.
It is never treated as cleanup success. This follows the zombie filtering in
[Darwin's group signalling](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.81.4/bsd/kern/kern_sig.c).
On either Unix platform, a process group is not a sandbox against
native code deliberately escaping it; an inherited pipe that prevents confirmed
cleanup blocks progress.

## Verification and remaining work

[The Linux/macOS probe](../tools/verify_voice_validation.py) uses deterministic Piper
fixtures and, optionally, a temporary export of bundled Flite SLT. It checks
native results, bad hashes/models, deadlines, cancellation, supervisor death,
descendant exit and a fresh check after failures. Linux also checks inherited
address-space limits. macOS component tests exercise the footprint cutoff with
a real allocation in a helper descendant and require confirmed cleanup.
It also verifies refusal to continue when an escaped child retains a pipe; the
test owner explicitly retires that deliberately escaped child afterward.
No trained voice download is required. Native Windows component tests exercise
job termination, descendant pipes, closure of the last job handle and refusal of
a native memory commit above the configured budget.

The [macOS Voice Validation workflow](../.github/workflows/voice-validation-macos.yml)
runs native component tests and the full Piper/Flite probe on Intel and Apple
Silicon, on manual dispatch or pushes to `ci/macos-voice-validation-*` branches.
It builds the supported staged payloads and uses only bundled/generated
voice fixtures. Native Intel and Apple Silicon checks passed in
[verification run 35035356056](https://github.com/bartbunting/omnivox/actions/runs/35035356056)
at source commit `9107e6f578094c6dfba90a75d4a62a6a390c2179`. Each host passed six
supervision tests five times, including a deterministic zombie-group regression,
then the full Piper/Flite probe and its ownership fault checks. Native Clippy
also passed for the validator and its prepared dependencies. These are silent
validation checks, not acoustic or coordinated-activation acceptance.

Saved-evidence checks subsequently passed on both native Mac architectures in
[run 35042369039](https://github.com/bartbunting/omnivox/actions/runs/35042369039)
at source commit `7ff386693701d8a7a50cb10be615f455062516ca`. Each host passed seven
shared evidence tests, repeated supervision tests, native Piper/Flite report
creation and comparison, stale-input rejection, and refusal to publish after
cancellation or supervisor death. Linux passed the same integration probe and
the workspace/Clippy gates. Native Windows x64 GNU component tests passed six
shared evidence checks and eight supervision/command checks, including report
creation, non-overwriting publication and cancellation on its native filesystem.

Full Windows server/companion validation and MSVC acceptance also remain separate
work. The existing Windows GNU main staging limitation is recorded in
[ADR 0012](adr/0012-voice-library-and-model-lifecycle.md).

The validator can save and compare observed executable, companion and voice
inputs. The [installed-voice store](VOICE-INSTALLATION.md) now registers admitted,
validated local imports, persists desired enablement and prepares immutable
activation candidates. Apply must still validate full candidate startup/status
with the exact overrides and coordinate both speech lanes with rollback. These
validation commands do not activate a generation. Further recovery and
power-loss hardening are recorded separately and do not block that integration.

The [operation-journal foundation](voice-operation-journal-design.md) now provides
separate development preparation and inspection commands. It preserves frozen
requests and detects interrupted or damaged state under an exclusive lease.
The separate `--run-voice-validation-operation` command now executes a frozen
request under profile and operation leases. It records intent before spawn,
ownership before opening each native gate, and confirmed cleanup before further
work; its report binds the exact operation and plan. An interrupted profile stays
blocked even when a fresh operation ID is supplied. Ordinary
`--validate-voice-library` remains a standalone diagnostic. Provider-specific
reconciliation of workers without saved cleanup is still pending; stored PIDs are
never authority to signal processes or restart speech.

`--recover-voice-validation-operation ROOT PROFILE_UUID OPERATION_UUID` can abandon
an interrupted validation when every worker has a complete saved cleanup record.
It preserves the journal and any report, adds a separately verified abandonment
receipt, and allows a fresh operation through normal admission. It does not count
the old validation as successful. A damaged final write can also be abandoned
when the verified journal prefix ends at `validating` and worker cleanup is
complete; the damaged bytes are preserved. Missing cleanup and other damaged
history remain blocked; see the operation-journal guide for the exact boundary.

The development run command keeps native ownership in a separate supervisor. If
its manager process dies or disconnects, that supervisor can still cancel the
workers, confirm cleanup and record the result while retaining both leases. A new
manager must inspect history and use normal admission; manager exit alone does
not prove cleanup. If the supervisor also dies, missing cleanup remains blocked.
