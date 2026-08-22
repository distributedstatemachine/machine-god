# Native file session store

This page is the normative contract for the eighth bounded Milestone 03 slice.
`FileSessionStore` is the candidate library implementation of core's
`SessionStore` contract in `machine-god-native`. Its exact feature-branch
review, CI, and benchmark evidence are green; the final documentation seal and
`main` integration remain pending. Review evidence is retained in the
[`native file session store review`](reviews/m03-session-store-review-01.md).
This is not a production-readiness claim.

## Root authority and platform scope

The host constructs `FileSessionStore` with one explicitly selected, existing,
absolute directory path. On the supported Linux and macOS Unix targets,
`FileSessionStore::open` opens the final component without following a final
symlink, authoritatively verifies that the opened object is a directory, and
retains that descriptor. Later record, lock, and temporary-file operations are
relative to the retained descriptor; replacing the host path afterward does not
redirect the store.

The store does not inspect `XDG_STATE_HOME`, `HOME`, the process environment, or
the current directory. It does not discover or create its root, list sessions,
delete or reset a session, or expose a path-based escape hatch. Root selection,
creation, ownership, permissions, quota, backup, and lifecycle remain host
responsibilities. The retained descriptor confines the store's fixed child
names to that host-selected directory, but the host and the filesystem beneath
the descriptor remain trusted. Hardened support for Windows and other non-Unix
targets is deferred; construction returns a fixed unsupported-platform error
there.

`FileSessionStore::open` has this complete fixed taxonomy:

| `FileSessionStoreOpenErrorKind` | Meaning |
| --- | --- |
| `UnsupportedPlatform` | The target has no supported hardened implementation. |
| `InvalidRoot` | The supplied root is not a valid absolute root path. |
| `InvalidFileType` | The opened root is not a directory. |
| `Unavailable` | The root could not be opened or inspected. |

Each category has fixed redacted `Display` and `Debug` output. The error retains
no root path, operating-system diagnostic, or raw error number.

## Stable v1 layout

For a `SessionId`, the store hashes this exact byte sequence:

```text
ASCII "machine-god:file-session:v1:" || SessionId UTF-8 bytes
```

The SHA-256 digest is encoded as 64 lowercase hexadecimal ASCII characters
`<digest>`. The flat v1 names are exactly:

```text
session-<digest>.json
session-<digest>.lock
session-<digest>.tmp
```

The domain separator, digest encoding, prefixes, and suffixes are fixed layout
for schema v1. Hashing keeps raw session IDs out of ordinary directory
listings and creates fixed safe child names. It is only a naming and modest
privacy-reduction measure: it is not encryption, authentication, an
unguessability claim, a filesystem capability, or confinement. A party that
can guess a session ID can compute its name, and a party that can read the
record learns the ID. Confinement comes from descriptor-relative operations
under the explicitly injected root.

Every successful load verifies that the decoded record's `id` exactly equals
the requested `SessionId`; a mismatch is corrupt state. A save derives the name
from the candidate record's own validated ID. Digest collisions are not given
merge semantics: the ID check fails closed instead of returning a record under
the wrong identity.

## Record format and bounds

`FILE_SESSION_SCHEMA_VERSION` is `1`. A stored file is one UTF-8 JSON document
with this exact envelope:

```json
{"schema_version":1,"record":{"id":"example","incarnation_id":"0198d2f9-ef9a-7d72-9c1d-6f6db8f3dd50","revision":1,"next_turn_sequence":1,"messages":[],"metadata":{}}}
```

Writes use the compact representation shown: the envelope has exactly
`schema_version` followed by `record`, and the record uses the core
`SessionRecord` JSON representation. Decoding is strict and versioned. Unknown
or duplicate typed schema fields, missing or wrong-typed fields, malformed or
non-UTF-8 documents, trailing data, and unsupported versions are rejected
rather than ignored or migrated. Duplicate keys inside arbitrary embedded JSON
values are not separately rejected. Revision zero and `next_turn_sequence`
zero are not accepted from a stored file.

`MAX_FILE_SESSION_BYTES` is exactly `8_651_165`. It includes the v1 envelope
and is sufficient for every `SessionRecord` that obeys all default
`EngineLimits`, including the default 8 MiB serialized-transcript and 256 KiB
serialized-metadata maxima and maximum valid identifiers. A configured engine
may impose smaller limits. Raising core limits does not raise this store limit;
an otherwise valid larger record is rejected before replacement.

A read retains at most `MAX_FILE_SESSION_BYTES + 1` bytes. The one additional
byte is only an exact overflow or concurrent-growth witness and is never
decoded. A save uses bounded compact serialization and rejects an over-limit
candidate before creating a temporary or replacing the data record, though a
polled save may first create its permanent lock sidecar. Independently of the
byte ceiling, the store iteratively applies core's default aggregate JSON
bounds of 64 container levels and 65,536 nodes to embedded values before
serialization and after decode. Direct `SessionStore` callers remain
responsible for configured engine limits outside those fixed structural
checks.

## Load behavior

Calling `load` constructs a future without filesystem effects. Dropping it
before its first poll performs no work. On first poll, the store performs its
complete bounded operation synchronously on the polling thread.

A missing record returns `Ok(None)` and does not create a record, lock, or
temporary artifact. After observing a present data file, load may create its
absent permanent `session-<digest>.lock` sidecar with mode `0600`; it creates no
lock for a missing record. The sidecar is opened without following a symlink,
authoritatively required to be a regular file, and held with an exclusive
advisory lock while the present record is read.
The record is then opened no-follow and nonblocking relative to the retained
root (`O_NOFOLLOW | O_NONBLOCK`) and required by `fstat` to be a regular file
before any bytes are read.
The open-then-`fstat` result is authoritative; a preliminary name observation
does not authorize reading a FIFO, device, directory, socket, or symlink.

The bounded read, UTF-8 and strict-envelope decode, schema-version check,
positive revision check, and exact record-ID check all complete while the
exclusive lock remains held. Malformed, oversized, wrong-version, wrong-ID, or
nonregular state fails closed. The store does not truncate, quarantine,
rewrite, unlink, migrate, or otherwise repair a corrupt record or nonregular
artifact.

## Save, compare-and-swap, and durability

Calling `save` likewise produces an inert future. Its first poll performs the
bounded serialization, locking, filesystem writes, and synchronization
synchronously on that executor thread.

Each session has one permanent regular no-follow lock sidecar. Saves hold its
exclusive advisory lock across comparison, write, rename, and directory sync.
Loads of an existing record hold an exclusive lock through validation and
decode.
The lock file is deliberately retained after operations: unlinking and
recreating it could split cooperating processes across different lock inodes.
The lock coordinates only store instances and other processes that honor the
same protocol. It neither excludes a process that ignores advisory locks nor
protects against an actor that can replace files or remove the lock sidecar.
The root's ownership and permissions and the filesystem's lock, descriptor,
rename, and sync semantics are therefore part of the trusted host boundary.

Under the exclusive lock, save implements core's optimistic concurrency
contract:

- `expected_revision: None` is a new-record compare-and-swap. It succeeds only
  when no record exists. Its revision base is
  `max(SessionRevision(0), candidate.revision)`, and it assigns the checked
  base plus one. An ordinary new core record therefore receives revision `1`,
  while an unusual direct caller with a positive candidate revision receives a
  strictly greater value.
- `expected_revision: Some(expected)` is an update compare-and-swap. The
  current record must exist, decode and validate, have exactly `expected`, and
  have the same session ID and incarnation as the candidate. Its revision base
  is `max(stored.revision, candidate.revision)`, and success assigns the checked
  base plus one.
- Existing-versus-missing state, an unexpected revision, or an incarnation
  change is a conflict. The store never merges messages or metadata.
- A maximum base of `u64::MAX`, whether supplied by current state or the
  candidate, fails revision assignment without wrapping or replacing the
  record.

The store, rather than the caller, assigns the successful revision and writes
that revision into the durable envelope. The returned `SessionRevision` is the
same value. A candidate's revision participates in selecting a strictly greater
assigned value; save never decreases or reuses the durable counter.

Before replacement, the complete next envelope is compact-serialized within
the byte cap. The fixed `session-<digest>.tmp` child is created relative to the
root with mode `0600`, exclusive creation, and no-follow semantics, then
authoritatively verified as a regular file. In flag terms, creation requires
`O_CREAT | O_EXCL | O_NOFOLLOW`. Under the exclusive lock, an existing
no-follow, `fstat`-confirmed regular temporary file is treated as stale crash
residue and unlinked before that exclusive creation. A symlink or nonregular
temporary artifact fails closed and remains in place. The store writes the
complete bytes, synchronizes the temporary file, atomically renames it over the
record within the same retained directory, and synchronizes that directory.

A failure before rename leaves the previous record authoritative. A successful
rename makes the complete old or complete new regular file visible to
cooperating readers; no partially written record is published. If the final
directory synchronization fails after rename, `save` returns a fixed error but
the new revision may already be visible and may or may not survive a crash.
That outcome is intentionally reported as ambiguous. A caller must load and
reconcile instead of blindly assuming either version or replaying external
effects.

These claims cover one record replacement within one retained directory on the
supported Unix filesystems whose advisory locking, `rename`, file sync, and
directory sync provide the assumed semantics. They do not claim a transaction
across multiple sessions, protection from noncooperating writers, NFS or other
remote-filesystem correctness, recovery from arbitrary filesystem corruption,
or survival of every full-system/sudden-power-loss failure mode.
The implementation requests `fsync` for the temporary file and directory. On
macOS it does not request `F_FULLFSYNC`, so success is not a claim that bytes
have reached physical media.

## Errors and polling behavior

Load and save expose only fixed `SessionStoreError` categories, diagnostics,
and retryability. Malformed, wrong-ID, unsupported-version, over-limit,
zero-counter, or structurally invalid stored bytes and detected symlink or
nonregular record, lock, or temporary artifacts are non-retryable `Corrupt`
failures. A revision compare-and-swap mismatch is a retryable `Conflict` because
core may load and reconcile; an incarnation mismatch is a non-retryable
`Conflict`. Revision exhaustion and an invalid, structurally unsafe,
unserializable, or oversized direct-save candidate are non-retryable `Other`
failures. Ordinary I/O, advisory-lock, write, pre-rename sync, and rename
failures are retryable `Unavailable`. A directory-sync failure after rename is
the distinct non-retryable `Unavailable` ambiguous-outcome category because
blind retry is unsafe. No missing load emits `NotFound`; absence is `Ok(None)`.

The exact fixed values are:

| Condition | Kind | Code | Message | Retryable |
| --- | --- | --- | --- | --- |
| Invalid stored schema, identity, counter, JSON, size, or file type | `Corrupt` | `file_session_corrupt` | `stored session is corrupt` | no |
| Missing/existing state or revision mismatch | `Conflict` | `revision_conflict` | `stored session revision did not match the expected revision` | yes |
| Incarnation mismatch | `Conflict` | `incarnation_conflict` | `stored session incarnation did not match the saved record` | no |
| Revision exhaustion | `Other` | `revision_exhausted` | `session revision counter was exhausted` | no |
| Candidate exceeds the serialized byte cap | `Other` | `session_too_large` | `serialized session exceeded the size limit` | no |
| Candidate turn sequence is zero, JSON exceeds structural bounds, or serialization fails | `Other` | `session_serialization_failed` | `session could not be serialized` | no |
| I/O, lock, pre-rename sync, or rename failure | `Unavailable` | `file_session_unavailable` | `file session store is unavailable` | yes |
| Directory sync fails after rename | `Unavailable` | `file_session_save_ambiguous` | `file session save outcome is ambiguous` | no |

Error values and their `Display` and `Debug` forms never include a session ID,
hash, root or child path, record bytes, JSON diagnostic, operating-system error
text, or raw error number. When errors cross the engine boundary, core
additionally reduces their code and message to `store_failed` / `session store
failed` while preserving the trusted kind and retryability fields.

All I/O, advisory-lock acquisition, `fsync`, and directory synchronization is
bounded in retained data and successful transfer work. Advisory-lock
acquisition, byte transfers, and `fsync` retry `EINTR`; other interrupted
operations surface through the fixed error taxonomy. Retry attempts and
wall-clock duration are not bounded. The returned futures do not
spawn a thread, task, timer, or runtime work. They have no effect before first
poll and detach no work on drop; once polled, dropping cannot preempt the
synchronous operation already executing. A host that must keep an async
executor responsive must poll these futures on a suitable blocking thread or
otherwise isolate the synchronous store.

## Deferred scope

This candidate adds no CLI command or existing CLI-byte change, provider or
transport wiring, credential discovery, permission prompt or new permission
mode, environment-based state-root discovery, directory creation, session
listing, deletion, reset, or automatic cleanup. Migration and legacy import,
schema upgrades, encryption at rest, authenticated records, secure erasure,
key management, backup/restore, multi-record transactions, cross-host
coordination, and non-Unix hardening remain deferred. It adds no compatibility,
upstream-equivalence, or product-performance claim and does not change the
pinned fx inventory, benchmark workloads or classification, workflows, or Zig
benchmark-only toolchain input.
