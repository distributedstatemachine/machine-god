# Native process-local file undo

## Principal isolation and shared capacity

`NativeUndoBudget` is an inert aggregate allocation shared by independent
`FileUndoTracker::for_principal` histories. Each tracker permanently binds its
admitted native owner and nonzero generation; `check_principal` rejects a foreign
owner or stale generation. Those labels are not execution authority: the native
host must select the tracker from its actual admitted turn, never from current
UI selection. Standalone `FileUndoTracker::new` uses this same implementation
with a private default budget, not a second tracking path.

Workspace-routed mutation tools obtain the tracker from the original native
conversation's pinned turn scope. The shared tool allocation contains no undo
selection. Registration checks actual session/turn witnesses, and replacing or
retiring a route cannot move an existing operation into another history. The
reference host binds its selected tracker before conversation admission;
managed principals supply their independently budgeted trackers through that
same scope boundary.
Interactive undo controls and transition clears likewise capture the original
conversation runtime's tracker before accepting work; they do not look it up
again through the shared host when the control later executes.

Validated `NativeUndoLimits` bound combined retained and in-flight entries,
bytes, and descriptors. Defaults allow 101 entries, 101 times (10 MiB plus
128 KiB) of byte reservations, and 3,232 descriptor reservations across the whole
domain, not per child. This preserves one principal's 100 maximum preimages
plus forward-operation headroom. Constructors reserve no file descriptors or
preimages. `usage` is a momentary accounting observation, not an admission lease.

Before locating paths, opening snapshot descriptors or reading a preimage, a
forward transaction reserves one entry, 10 MiB plus 128 KiB of allocation
capacity, and 32 descriptors. Inverse operations and explicit replacement-copy
source inspection separately reserve 64 KiB and 16 descriptors of scratch
headroom before their effects. Retained charges include actual preimage vector
capacity, path labels, snapshot storage and retained descriptors. Allocation
growth is explicitly capped at the existing 10 MiB preimage limit. Ordinary
unavailable/oversized-preimage markers retain their existing forward behavior;
aggregate budget exhaustion instead returns `ResourceLimit` before capture or
dispatch (tool error code `file_undo_resource_limit`). It never silently disables
accounting or evicts a sibling's history.

The nonclone reservation stays with the synchronous transaction on its actual
worker, not a caller response wrapper. Commit transfers retained charges to the
original tracker's entry; transaction scratch is released afterward. Each entry
drops its descriptors/preimages before releasing its reservation. Ambiguous
forward publication retains a separately fenced, unconfirmed entry and its
charges until explicit clear; it is not reported as a committed inverse. An
ambiguous inverse retains its original entry. Failed or cancelled pre-dispatch
transactions release their temporary resources and reservations. Tracker drop
and owner-local clear discharge only that tracker's retained allocations.

The aggregate accounting mutex is held only while updating counters, never
across descriptor operations, capture, mutation, callbacks or asynchronous
waits. Existing per-tracker exclusion remains independent: one busy principal
does not lock another principal's history. Clear, capacity eviction and undo
affect only the selected owner; the manager still owns retirement, execution
witness validation and overall principal residency bounds.

## Workspace endpoint ownership

Each retained undo location owns its own root descriptor, canonical private
relative path, parent descriptor, and logical result label. A workspace-routed
rename can therefore be reversed across two retained roots on the same filesystem;
a routed copy retains only its destination inverse. Both rename roots and both
parent identities are revalidated before undo effects. Replacement by an object
with identical bytes does not satisfy a retained identity proof.

Logical root-qualified labels are used in routed undo outcomes, while existing
single-root callers continue receiving relative paths. Labels are never used as
execution paths. Existing preimage limits, quarantine handling, history ownership,
nonblocking reservations, and ambiguous-effect barriers are unchanged.

Workspace-routed write, edit, and delete also bind their exact endpoint into the
shared tracker, preserving logical labels for both creation-removal and preimage
restoration receipts. There is no shared mutable path-alias map. Forward mutation
rechecks its exact turn's live scope immediately before publication, with or without
file-approval injection. Undo entries retain root and object proofs rather than an
expired turn's execution permission; later undo remains subject to the explicit
tracker owner and its normal session/reset lifecycle.

## Tracker contract

`FileUndoTracker` is explicitly injected, shared native authority for inverse
file mutations. Its inert `new()` constructor performs no filesystem work.
The host may share an `Arc<FileUndoTracker>` through the five tools'
`with_undo_tracker` builders. Existing constructors and untracked execution
retain their established contracts. Injection additionally authorizes bounded
preimage reads of the selected mutation endpoints; ordinary `Write`, `Delete`,
or rename authority alone does not silently acquire those reads.

This is an in-memory history, not a durable journal, transcript undo, or a
promise that undo history is restored on resume. The host must scope/reset the
tracker when changing its command-session lifetime. Core has no filesystem
authority and the CLI owns neither snapshots nor mutation truth.

## Effects and bounds

Tracked writes, exact edits, deletes, no-replace renames, and no-replace copies
register only after their actual publication syscall reports success. A later
directory-sync failure or tool-level cancellation does not erase a committed
entry. An interrupted publication establishes a separate uncertainty barrier,
not a falsely committed inverse. Unsafe postpublication observation also blocks
replay. Precommit failure/cancellation registers nothing.

The tracker retains at most 100 committed operations, evicting only the oldest
entry when a 101st entry commits. Each operation retains at most one regular-file
preimage of at most 10 MiB, giving a derived aggregate logical bound of 1,000 MiB.
Oversized preimages are rejected by metadata before content reads; a retained
capture never reads beyond 10 MiB and verifies stable metadata around its read.
Digest-only source/postimage observation is separate, retaining no content and
bounded to 16 MiB plus one overflow witness. Tracking therefore preserves the
copy tool's complete 16 MiB source boundary. Replacement copy streams from its
pinned source descriptor; source bytes are not retained as undo preimages.
Larger rename sources do not need reconstructed bytes: undo moves the exact
pinned source object. These are observed using descriptor identity and stable
size, mode, mtime, and ctime, without content hashing. The intentional quarantine
rename changes ctime; its subsequent identity check retains size, mode, and mtime
comparison. This preserves unbounded forward rename sizes without inventing
unbounded hashing or a byte preimage.
Reads and hashes use 8 KiB scratch chunks, fixed SHA-256 state, and at most 16
cumulative interruptions. Snapshot vectors retain bounded allocator capacity;
capture can coexist with the previous retained history. These are byte/work
bounds, not filesystem latency promises.

Unavailable or oversized preimages do not prevent the existing forward tool's
authorized dispatch. They are distinct from an observed absent file. After a
proven successful publication, such an operation retains an explicit
non-undoable history marker. Later forward operations and their inverses may
proceed; undo stops at that marker with `NotUndoable(PreimageTooLarge)` or
`NotUndoable(SnapshotUnavailable)` until history is explicitly cleared (or the
entry eventually ages out under the 100-entry bound).
`latest_unavailable_reason()` exposes the latest marker's reason without effects
or consumption, so the host can report that forward success lacks undo support.
Cancellation before dispatch records nothing. Publication uncertainty remains a
separate stricter barrier that also stops later tracked mutations.

The tracker uses nonblocking mutex acquisition: simultaneous cooperating
mutations/undo report `Busy`, never detach work or synchronously wait for the
other operation. Roots, parents, and relevant leaf objects are descriptor-pinned.
Operations rewalk the original retained root and compare exact parents before
publication. Captures verify regular type, identity, mode, size, stable
modification/change timestamps, and (within the observation bound) content digest. The final leaf is
never followed through a symlink. Directory deletion captures a typed directory
identity and ordinary mode, without reading/enumerating its contents.

`undo_last` restores prior bytes and ordinary rwx bits, removes a newly created
file, restores a deleted regular file or empty directory, or moves a renamed
source back and restores any overwritten destination preimage. Restored files
are new inodes; ownership, ACLs, extended attributes, timestamps, special bits,
and hard-link topology are not reconstructed. Rename reversal moves the actual
retained source object. Only exact predecessor observations are rebased after
successful undo, allowing interleaved multi-operation history without granting
permission to overwrite an externally replaced same-content file.
When a later tracked inverse reconstructs a renamed destination, reversal moves
the exact rebased postimage that passed current identity checks. It retains the
original content/mode checks without requiring the historical source inode to
survive reconstruction. External replacement still fails before inverse work.

`rename_replace` and `copy_replace` are separate explicit native APIs taking an
already-open workspace directory. They exercise actual replacement effects and
retain destination preimages; they do not widen the default tools' schema or
no-replace semantics. Source/destination same-path and same-inode hard-link
aliases are rejected. Replacement copy preserves the source. Both endpoints
remain regular-file-only, confined, bounded, and existing-parent-only.
When the destination preimage is unavailable, explicit replacement copy uses
ordinary atomic replacement and records the non-undoable marker; it does not
invent an absent destination or quarantine content it cannot restore.

## Safe replay and failure ownership

Before inverse work, all observed postimages must still match. Modified,
replaced, missing, symlinked, or redirected targets fail closed. Undo moves a
current postimage into an unpredictable same-parent quarantine name using
`NOREPLACE`, verifies its identity/content, and publishes each inverse only into
an absent name. A competing new entry is never intentionally overwritten by
inverse publication. A mismatched quarantined entry is moved back only with
`NOREPLACE`; a conflicting recreated name is preserved.

Successful inverse work cleans the verified quarantine entry and syncs affected
parents. Failed unpublished file stages have descriptor-owned RAII cleanup:
restore private mode, compare exact held/named identity, then best-effort unlink.
Failures after inverse work begins return `Ambiguous`, retain a non-replayable
barrier, and may leave `.machine-god-undo-…` recovery artifacts. Those artifacts
are deliberately not recursively deleted or replayed on tracker drop. Dropping
the tracker releases descriptors and in-memory snapshots. `clear()` explicitly
forgets authority/barriers but does not remove filesystem recovery artifacts.
The host must not describe an ambiguous result as successful undo or retry it
automatically.

`Arc<FileUndoTracker>::reserve_clear()` reserves exclusive clear admission and
returns an owned, nonclone `Send` `FileUndoClearReservation`. It performs no
filesystem work and retains no mutex guard across awaits. Entries, unavailable
markers and uncertainty barriers remain intact until `commit(self)`. Dropping
the reservation, including unwinding an abandoned handoff, preserves history.
While reserved, cooperating clear, latest-marker queries, mutation admission and
undo return `Busy` (existing cancellation/argument checks may still reject first).
An already active mutation/undo prevents reservation acquisition.

`commit` is synchronous and infallible: it clears retained authority/barriers,
releases admission, and drops old descriptors/preimages outside the tracker
mutex. It never undoes a file or deletes recovery artifacts. Commit and abort
briefly recover the mutex solely to finish reservation ownership; they do not
clear mutex poison or make subsequent poisoned-tracker calls succeed. Thus a
host can reserve before a fallible terminal handoff, abort without losing undo
history, or commit after handoff without another fallible undo acquisition.

Portable Linux/macOS rename/unlink is not inode compare-and-swap. The shared
tracker serializes cooperating injected tools, not unrelated actors, editors,
untracked tools, or other processes. A parent can move after the last rewalk;
an entry can change after the last comparison. Identity-checked cleanup has the
same final name-to-unlink race disclosed by the existing native staging tools.
Quarantine protects against observed stale clobber, not adversarial filesystem
snapshot isolation. Residue after cleanup/mode-restoration failure can retain
its prior mode. These limitations must not become stronger security claims in
the interactive host.

Before inverse publication, cancellation is honored. Once inverse mutation
starts, cancellation is ignored through owned completion/ambiguity reporting;
there is no detached cleanup task. Every parent sync permits at most 16 calls.
Temporary names have at most eight collision attempts; Linux entropy uses
nonblocking native acquisition under a 31-call bound and macOS uses the pinned
one-call entropy adapter. Successful observations are `Empty`, `Restored(path)`,
or `Removed(path)`. Failures are fixed/redacted `Busy`, `Rejected`, `Changed`,
`ResourceLimit`, `Unavailable`, `Cancelled`, `Ambiguous`, or `NotUndoable(reason)`; tool tracking failures
use fixed `file_undo_tracking_failed` without reflecting paths/content/OS errors.

## Pinned input

The native interactive owner's `UndoLast` control runs this same inverse through
the actual reference host's shared owned-worker scope. It retains a runtime
lifecycle permit through execution, accepts active model turns without cancelling
them, and leaves canonical transcript/history unchanged. Its exact `Undone`
receipt or `Undo` error is retained independently of output and later session
transitions. Accepted inverse work settles before deferred cancellation or
shutdown; worker destruction is joined by the full host completion observer,
not inferred from response readiness. The control does not create a new tracker,
broaden file authority, skip unavailable entries or retry ambiguous effects.

Pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` owns its 100-entry
change stack and bounded preimage capture in
`src/core/workspace/change_tracker.zig`; `src/core/tooling/tracked_file_mutations.zig`
captures delete/rename/copy state and appends after successful dispatch.
Its rename and copy trackers include overwritten-destination bytes. The slash
handler is `src/core/app/app_commands.zig:998`. Machine-god deliberately does not
adopt upstream's pathname-only restoration, swallowed restoration failures, or
silent loss of failed entries. Native control integration does not by itself
constitute interactive CLI integration or full feature acceptance.
