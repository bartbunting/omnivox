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

The memory limit defaults to 4096 MiB, adjustable from 256 to 65536 MiB:

- Windows limits committed memory for the complete private job.
- Linux limits each worker/helper's virtual address space with inherited
  `RLIMIT_AS`. This is not an aggregate RAM limit, and shared mappings and native
  thread stacks also consume address space.

These limits have different operating-system meanings. They do not measure model
RAM, and a low limit may reject an otherwise valid large model. macOS currently
rejects this development command; ordinary speech behavior is unchanged.

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
hazards. A process group is not a sandbox against native code deliberately
escaping it; an inherited pipe that prevents confirmed cleanup blocks progress.

## Verification and remaining work

[The Linux probe](../tools/verify_voice_validation.py) uses deterministic Piper
fixtures and, optionally, a temporary export of bundled Flite SLT. It checks
native results, bad hashes/models, deadlines, cancellation, supervisor death,
descendant reaping, inherited memory limits and a fresh check after failures.
It also verifies refusal to continue when an escaped child retains a pipe; the
test owner explicitly retires that deliberately escaped child afterward.
No trained voice download is required. Native Windows component tests exercise
job termination, descendant pipes, closure of the last job handle and refusal of
a native memory commit above the configured budget.

Full Windows server/companion validation, MSVC acceptance and macOS supervision
remain separate acceptance work. The existing Windows GNU main staging limitation
is recorded in [ADR 0012](adr/0012-voice-library-and-model-lifecycle.md).

The storage service must still bind durable validation evidence to executable and
companion provenance, persist interrupted operation ownership, reconcile failed
cleanup across manager invocations, and validate full candidate startup/status
with the exact overrides before activation. This command validates managed native
loads; it does not implement those transaction guarantees or two-lane rollback.
