# Native `memory` tool

This document defines machine-god's bounded durable `memory` tool. The tool
stores explicit user preferences that should survive across sessions. It is not
for task notes, repository facts, credentials, secrets, temporary context, or
facts the user did not ask the agent to retain.

## Input and authority

The exact input object has one required `action` and an action-dependent
`fact`:

```json
{"action":"save","fact":"Prefer concise commit messages"}
{"action":"list"}
{"action":"clear"}
```

`action` is exactly `save`, `list`, or `clear`. `save` requires exactly one
nonempty UTF-8 `fact`; `list` and `clear` forbid `fact`. Unknown fields,
duplicate typed fields, wrong types, explicit `null`, an empty fact, and an
unsupported action are invalid. A fact is preserved byte-for-byte without
trimming, case conversion, or Unicode normalization.

Preparation is synchronous, bounded, nonblocking, and effect-free. It strictly
decodes the call and returns both the canonical arguments and an exact
`Capability::Custom` named `memory` whose `details` are those same arguments.
All three actions require a permission decision because they observe or change
cross-session state. A grant for one exact action/fact is not authority for a
different action or fact. Direct execution repeats strict validation.

The model-visible description tells the model to use `save` only after an
explicit user request and not to retain secrets or transient/project-specific
context. That instruction reduces accidental misuse; it is not a classifier or
content filter.

## Bounds

The public limits are:

| Limit | Value |
| --- | ---: |
| `MAX_MEMORY_FACT_BYTES` | 4,096 |
| `MAX_MEMORY_FACTS` | 128 |
| `MAX_MEMORY_TOTAL_FACT_BYTES` | 32,768 |
| `MAX_MEMORY_FILE_BYTES` | 49,152 |
| `MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES` | 32,768 |
| `MAX_MEMORY_SERIALIZED_RESULT_BYTES` | 65,536 |
| `MAX_MEMORY_IO_ATTEMPTS` | 65,536 |

Fact count and aggregate fact bytes are checked with overflow-safe arithmetic.
The argument, compact stored document, and complete serialized `ToolOutput`
limits include JSON escaping and their envelopes. A save that would make the
state valid by raw counts but exceed either serialized-state or future-list
result capacity fails before publication. Reads retain at most one byte beyond
the file limit as an overflow witness. Every retried or short native I/O call is
charged before dispatch; exhaustion is a fixed resource-limit failure rather
than an unbounded retry loop. Once rename or unlink has crossed the commit
point, attempt exhaustion is instead the fixed `memory_commit_ambiguous`
failure because rollback can no longer be claimed. Filesystem calls may still
block for a kernel-dependent duration.

## State-root authority and layout

On Linux and macOS, public construction accepts one existing absolute state
directory, opens its final component without following a symlink, verifies it
is a directory, and retains that descriptor. Construction creates no file,
lock, future, task, runtime, or background worker. Later access uses only these
fixed descriptor-relative children:

```text
memories.json
memories.lock
memories.tmp
```

The tool never inspects `HOME`, XDG variables, the current directory, the
workspace, or a session-record path. Prepared reference-host composition gives
it an identity-preserving clone of the exact state-root descriptor already
retained by `FileSessionStore`; replacing the selected pathname cannot redirect
either component.

Child opens use no-follow, close-on-exec, and nonblocking flags and require
regular files before reading, writing, locking, or unlinking. New lock and
temporary files have mode `0600`. After exclusive creation makes the permanent
lock visible, owner read/write access is established before cancellation is
observed again, including under an owner-masking process umask. The permanent
lock sidecar is never unlinked or recreated after successful acquisition, so
cooperating processes keep one lock identity. The state-root owner and
processes that ignore or replace advisory-lock artifacts remain trusted host
boundaries.

The compact version-one document is:

```json
{"schema_version":1,"memories":["Prefer concise commit messages"]}
```

It is strict and fail-closed. Malformed or non-UTF-8 bytes, trailing data,
unknown/duplicate typed fields, a future version, invalid fact values,
duplicate facts, or any count/byte/serialized bound violation is corrupt state.
The tool does not silently treat corrupt state as empty, migrate it, partially
return it, or overwrite it. A missing document is an empty memory set. Facts
retain insertion order and exact duplicate saves are idempotent.

## Concurrency and durability

Every operation opens the permanent lock sidecar. `list` takes a nonblocking
shared advisory lock; `save` and `clear` take a nonblocking exclusive lock.
Contention returns `memory_busy`, so tool execution never waits indefinitely
for another cooperating process. Independent tool instances therefore cannot
silently lose a cooperating concurrent save.

While holding the exclusive lock, `save` validates the current document and
validates the fixed temporary child before deduplicating the requested fact. A
changed document is compact-serialized within all limits, written to the fixed
exclusive temporary child, synchronized, atomically renamed over
`memories.json`, and followed by state-directory synchronization. A stale
regular temporary file from an interrupted operation may be removed under the
lock before any save result; a symlink, directory, FIFO, socket, or other
unexpected artifact fails closed and is left untouched, including for an exact
duplicate save.

`clear` validates the current document so it can report the exact removed
count, removes a stale regular temporary child when present, unlinks the live
document, and synchronizes the state directory. Missing state succeeds without
creating a document. It provides logical deletion, not secure erasure: storage,
backups, allocator history, and filesystem remnants are outside the contract.

Before rename or unlink, failure and cancellation preserve the previous live
document and best-effort cleanup removes a temporary file owned by the call.
After the live rename or unlink succeeds, cancellation cannot report rollback;
the call completes directory synchronization and returns success or the fixed
nonretryable `memory_commit_ambiguous` error. No operation is a transaction
with session persistence.

## Results

Success returns one of these structured values:

```json
{"action":"save","stored":true,"count":1}
{"action":"list","memories":["Prefer concise commit messages"],"count":1}
{"action":"clear","cleared":1}
```

An exact duplicate save returns `stored: false` with the unchanged count.
Listing missing state returns an empty array and count zero. Clearing missing
state returns `cleared: 0`. Values are assembled and measured completely before
success; output is never truncated into invalid JSON.

## Lifecycle and errors

`execute` is inert until its returned future is polled and creates no detached
work. The first poll performs the bounded operation synchronously on the
polling thread. Cancellation is checked before state access, after lock
acquisition, during bounded transfer work, and immediately before an
irreversible mutation. Dropping the future stops further work; it cannot undo a
rename or unlink that already succeeded.

Public construction and tool failures are fixed and redacted. Tool codes are:

| Code | Meaning | Retryable |
| --- | --- | --- |
| `memory_invalid_arguments` | The strict object/action shape is invalid. | no |
| `memory_invalid_fact` | The fact is empty or exceeds its byte bound. | no |
| `memory_resource_limit` | A count, byte, serialization, or I/O-attempt bound was exceeded. | no |
| `memory_unsupported_platform` | Hardened native persistence is unavailable. | no |
| `memory_busy` | A cooperating operation currently owns an incompatible lock. | yes |
| `memory_state_corrupt` | Existing state or a fixed child has an invalid shape. | no |
| `memory_unavailable` | The retained state root or required child operation is temporarily unavailable. | yes |
| `memory_read_failed` | Bounded state reading could not complete. | yes |
| `memory_write_failed` | A precommit mutation or synchronization failed. | yes |
| `memory_commit_ambiguous` | Publication/removal succeeded but directory durability could not be confirmed. | no |
| `memory_cancelled` | Cancellation won before the commit point. | no |

`MemoryToolOpenErrorKind` separately distinguishes unsupported platform,
an invalid root path, invalid root type, and unavailable root, with fixed
messages that contain no path or operating-system diagnostic. On targets other
than Linux and macOS, construction and execution expose only the fixed
unsupported behavior and perform no filesystem effect.

## Deferred scope

Encryption, authentication, synchronization across machines, expiry, selective
deletion, editing, search, metadata, semantic deduplication, automatic memory
extraction, non-Unix hardened persistence, interactive memory management, and
permission caching are separate future work.
