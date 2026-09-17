# Managed voice uninstallation

The development local service removes reviewed Piper, Flite and MBROLA
downloads under the ownership rules in [ADR 0012](adr/0012-voice-library-and-model-lifecycle.md)
and [ADR 0013](adr/0013-mbrola-voice-library.md). Emacsvox supplies the accessible
review and confirmation. The remote speech socket has no removal operation.

Removal is package-wide: all speakers sharing a Piper model appear in the
review, and all must be disabled. Apply that disablement before uninstalling.
Built-in Flite SLT, MBROLA en1, system voices, imported files, engine runtimes
and the MBROLA frontend are outside removal's ownership. Saved palette choices
and disabled physical IDs remain; downloading the same voice again preserves
its physical identity and starts disabled.

## Reference and ownership checks

The review binds an immutable operation, original index revision and digest,
package/revision UUIDs, catalogue and file hashes. Execution repeats the checks
under native storage and profile leases. Other profiles' references, active
generations, incomplete validation or Apply (including rollback), and startup
snapshots lacking retirement evidence retain the files. Unknown or malformed
references block cleanup. Native startup snapshot publication and removal share
a permanent storage lock, so new managed owners cannot appear between the
reference check and deletion.

An owned speech process records retirement only after its entire worker tree
and output readers have finished. An idle process, disconnected client or
released OS lock is insufficient. Prepared startup snapshots are separately
identified; using one starts a new independently recorded owner. Snapshots from
older binaries have no retirement receipt and conservatively retain their
referenced files. There is no PID-based force cleanup or automatic restart.

Package paths must be the exact installer-owned location and contain only the
catalogue's fixed filenames plus its retained catalogue. Content hashes and
sizes are rechecked; unexpected files, links and changed content retain the
package. These checks protect normal managed operations, not arbitrary external
programs concurrently rewriting a user's private storage.

## Interruption and reporting

Execution first publishes an index without the package and its voice rows,
then unlinks verified files individually. Original index revisions, review and
deletion receipts remain. Interruption before index publication preserves the
installed package; interruption afterward leaves an explicit resumable cleanup.
Retry uses the same retained plan and rechecks current references, including
any new installation sharing files. It never recursively deletes unexpected
content or deletes files outside the managed revision.

Results distinguish blocked, partial and complete cleanup, confirmed removed
file bytes, remaining bytes and absent bytes without a confirmed deletion
receipt. A lost receipt after unlinking does not become claimed savings on
retry. Logical file bytes are not a measurement of filesystem free space,
compression, retained external file handles or RAM. Interrupted or incomplete
metadata writes may require inspection; stronger power-loss guarantees remain
separate work.

## Private local interface and checks

`host` advertises `removal_version: 1`. Older services remain usable for their
existing commands; clients require this capability before offering removal.
The private stdio commands are:

- `uninstall-preview`: engine, physical voice and expected index digest; returns
  the frozen review and current blockers without deleting or detaching anything.
- `uninstall`: operation UUID and reviewed plan digest; repeats reference and
  ownership checks and returns the cleanup outcome.
- `uninstall-pending`: retained operations whose index detachment began and
  whose cleanup has no completion receipt, with fresh blocker information.

Run the focused removal tests with
`cargo test --locked -p omnivox-tts voice_library::installation::removal`.
`tools/verify_voice_removal.py` uses reviewed catalogues in private native roots
to check actual acquisition, two independently owned speech processes, blocked
cleanup until both retire, removal and verified reinstallation. Supply the
explicit development MBROLA helper for MBROLA checks. On Windows it also holds
a file open against deletion, checks the partial result and detached index,
then releases the handle and resumes the same operation. Its speech output is
null; these are storage/lifecycle checks, not listening acceptance.

Development checks on 2026-09-17 passed for the reviewed Piper Kristin, Flite
AWB/RMS and MBROLA us1/us2/us3 downloads on Linux, and Flite AWB/RMS and all
three MBROLA downloads on native Windows. Shared Piper speakers, active and
rollback generations, other profiles, ownership failures and interrupted
metadata/unlink handling have regression coverage. Native macOS uninstallation
and spoken Emacsvox acceptance remain unverified.
