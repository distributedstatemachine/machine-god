# MCP profile persistence

`NativeMcpConfigStore` owns an explicitly supplied native profile directory on
Linux and macOS. Its constructor admits at most 4096 raw directory-path bytes
and 64 components, compacts the retained path, and performs no filesystem or
environment operation. `path()` reports the selected `mcp.json`; selection does
not discover or write fx roots and does not widen native settings schema v7.

`load()` creates nothing. It retains the nearest existing ancestor descriptor,
its observed identity, missing descendant components and an existing private
profile-root descriptor. Missing roots/files yield an empty configuration.
Present `mcp.json` files must be owned, singly linked regular files with no
group/other permission bits. Root directories, locks and publication temps are
also private; macOS rejects granting ACLs through the shared private-entry
validator. Existing ordinary ancestors keep their existing policy, including
0755 user parents. Files are opened no-follow, close-on-exec and nonblocking;
the raw read admits at most 1 MiB plus one detection byte, followed by the
[bounded configuration codec](mcp-cli.md#profile-configuration-codec).

## Observation and publication

The snapshot binds its originating store instance, parent/root identities,
exact data-file incarnation and bytes. MCP retains the opened source file and
its device/inode, size, mode, link count, ownership and modification/change-time
revision. Named links and stable metadata are checked around reads. An
equal-byte replacement by another inode is stale, as are observed in-place
revision changes. Modification-time observations exclude access time, so reads
do not invalidate themselves. This is conservative observation evidence, not
proof against a malicious writer with authority to modify the same namespace.

`validate_unchanged(snapshot)` exposes that same exact-observation check without
applying a mutation. It performs bounded synchronous filesystem observation on
the caller's owned worker, creates no directory, lock or temp, and rejects
foreign snapshots and changed ancestor/root/source identities, revisions or
bytes. Still-missing safe namespaces remain valid. Success is a point-in-time
observation, not a reservation against subsequent writers or a runtime activation
receipt. Disk changes do not themselves revoke a separately retained active
runtime configuration; failed reload must leave that runtime generation intact.

`apply(snapshot, mutation)` accepts explicit `Insert`, `Replace` and `Remove`.
Insertion never silently replaces an alias. Replacement preserves an existing
position and appends absent aliases. Removal selects an exact validated alias;
unknown aliases and identical replacement values are observational no-ops.
No-ops revalidate the snapshot but create no directory, lock or temp and do not
canonicalize or rewrite existing bytes.

The borrowed future is inert until polled, then owns one synchronous,
input-bounded transaction without detached work. Complete candidate codec and
serialization bounds precede creation effects. Publication resolves retained
parents, creates only missing required directories with mode 0700 and syncs
new directory entries. Failed setup may leave empty directories; it does not
claim rollback of those directories.

Changed publication takes a private `.mcp.lock` with nonblocking exclusive
flock, returning `Busy` for contention. The lock persists and is never unlinked.
MCP lock/unlock operations allow at most sixteen interrupted attempts, with
explicit unlock even when another descriptor could retain the open description;
descriptor close remains the fallback after an unlock failure. Reads and writes
also reject sixteen cumulative interruptions rather than resetting the count
after partial progress. These bounds do not impose hard wall-clock deadlines
on filesystem syscalls such as fsync.

Under the cooperative lock, the source is reread and compared with the original
observation. The shared transaction exclusively creates `.mcp.tmp` with mode
0600, writes/fsyncs it, rechecks parent/root/lock/temp identities and source
observations, then renames it over `mcp.json` and syncs the root. Post-rename
MCP checks also confirm the published file link and intended bytes. Only owned,
unpublished temps are cleaned up. Existing foreign staging artifacts remain
untouched. Snapshot CAS is atomic among cooperating writers using this lock;
it is not an atomic compare-and-rename defense against arbitrary external
writers racing the final checks.

## Receipts and settings reuse

A commit receipt carries the admitted `before` configuration, `intended`
configuration, `changed` flag and `Confirmed` or `Ambiguous` durability. It is
not a fresh observation or a runtime reload/activation result. Failures before
rename leave prior data authoritative. After rename, failed durability or
source/link checks return an `Ambiguous` receipt: the caller must reload and
reconcile, never automatically retry the mutation or claim rollback.

Settings and MCP use one crate-private descriptor/transaction implementation,
with distinct fixed filenames, locks, staging files and bounds. Existing
settings retain their 64 KiB limit, legacy-readable-file policy, exact-byte CAS,
settings interruption retry behavior and model/permission error mapping.
Workspace mutations still merge latest settings under their lock and return
their existing before/after durability receipts. The extraction does not change
those public APIs or native schema versions. MCP does not adopt settings' weaker
legacy-readable-file policy or byte-only source equality.

Environment/header values can contain secrets. Store, snapshots, mutations,
receipts and errors have redacted diagnostic output. Persistence grants no
process, network, OAuth, tool or conversation authority; those remain separately
admitted runtime responsibilities.
