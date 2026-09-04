# Top-level background command

The command exposes a bounded read-only observation of persisted background
history for the canonical current workspace. The CLI owns only grammar,
presentation, exit codes, and output writes. Native code owns environment,
current-directory, state-root, descriptor, record, and hashing effects. This
slice starts no work and deliberately has no process-control authority.

The separately reviewed native [background supervisor](background-supervisor.md)
is the production writer for this store. This top-level command remains a
strict read-only observer: invoking it never starts, stops, adopts, or probes a
job. A host that has not used the supervisor may therefore still observe an
empty list.

## Grammar and exits

The accepted invocations are:

```text
machine-god background
machine-god background --json
machine-god background last
machine-god background last --json
machine-god background --json last
machine-god background <unsigned-decimal-u64> [--json]
machine-god background --json <unsigned-decimal-u64>
```

At most one target and one `--json` flag are accepted. IDs use nonempty ASCII
decimal digits and must fit `u64`; leading zeroes are accepted within the
20-byte token cap and do not change numeric identity. `last` is exact and
case-sensitive. Empty, signed, whitespace-padded,
non-Unicode, duplicate-flag, duplicate-target, option-assignment, and unknown
arguments are invalid. Parsing completes before environment, current-directory,
or filesystem access. Invalid syntax uses the fixed global diagnostic, empty
standard output, and exit 2.

Success exits 0 with empty standard error. Operational or rendering failure
exits 1. Human failures use standard error and empty standard output. JSON
failures use one compact `kind,error,code` object on standard output and empty
standard error. Closed categories are `NotFound`, `Corrupt`, `ResourceLimit`,
`Unavailable`, and `Unsupported`. Diagnostics never reflect paths, commands,
record contents, environment values, filenames, OS errors, or raw numbers.

## Queries and output

No target lists at most 100 validated records, ordered by `updated_at_ms`
descending and then numeric ID descending. `last` returns the first record in
that authoritative order. If scan or aggregate truncation could hide a newer
record, `last` fails `ResourceLimit`; it never returns a possibly false latest
record. An exact ID derives and opens only its canonical record name. Missing
list hierarchy is an empty complete result; missing `last` or exact ID is
`NotFound`.

List rows expose only `id`, recorded `state`, `updated_at_ms`, and a UTF-8
command preview of at most 256 bytes. Preview truncation occurs at a character
boundary and is explicit. List JSON uses kind `background` and fixes top-level
key order `kind,count,truncated,records`; each row fixes key order
`id,state,updated_at_ms,command_preview,preview_truncated`. Human mode starts
with `[background] no persisted background records` for a complete empty result
or `[background] N saved`, then one bounded row per record. A truncated list
ends with `[background] listing incomplete: a resource limit was reached`.

Detail exposes the numeric ID, recorded state, start and update timestamps,
optional PID, full bounded command, canonical recorded working directory,
optional exit code, optional server URL, and optional diagnostic. JSON kind is
`background_detail`; human mode labels every field. `running` and all other
states are explicitly recorded history, not a current-liveness assertion.

Strings are JSON-escaped in machine mode and terminal-control-sanitized in
human mode. Both modes have exactly one final LF. The complete representation
is validated and rendered before the first success write and is capped at 64
KiB including that LF. A violated snapshot invariant, checked-size overflow, or
one-byte excess becomes `ResourceLimit` with no partial success output. Writer
failure uses only the fixed global output diagnostic.

## Native persisted schema

Linux and macOS select nonempty `XDG_STATE_HOME`, otherwise nonempty `HOME`
plus `.local/state`, and then the fixed `machine-god/background-v1` hierarchy.
The selected raw Unicode environment base and canonical Unicode current
workspace are each limited to 4,096 bytes; an over-limit base is `ResourceLimit`.
A domain-separated SHA-256 of that exact path selects
`workspace-<64-lowercase-hex>`. Each record uses a domain-separated SHA-256 of
its big-endian numeric ID as `record-<64-lowercase-hex>.json`. Decoded workspace
and ID must reproduce both names.

The strict compact schema-v1 record is a closed object containing:

- `version` equal to 1;
- `workspace`, `id`, `started_at_ms`, `updated_at_ms`, `command`, `cwd`, and
  `state`;
- nullable `pid`, `exit_code`, `server_url`, and `diagnostic`.

States are `running`, `exited`, `failed`, `stopped`, `dead`, and `stale`.
Timestamps are unsigned and update time cannot precede start time. PID, when
present, is a nonzero `u32`. `running` has no exit code, `exited` has exactly
zero, and `failed` has a nonzero code. Command is nonempty, contains no NUL,
and is at most 32 KiB. Workspace and cwd are absolute canonical Unicode paths,
contain no NUL, and are each at most 4,096 bytes. Optional URL and diagnostic
are at most 2,048 and 4,096 UTF-8 bytes respectively and contain no NUL.

Only exact canonical record names are candidates. Temporary, lock, uppercase,
wrong-width, nested, and unrelated entries consume scan budget but are not
records. A selected canonical symlink, directory, special file, oversized or
malformed document, unsupported version, unknown/duplicate/missing field,
invalid value, or filename/content/workspace mismatch is `Corrupt`.

On macOS, each selected canonical record's opened descriptor must have zero
ACL-level flags and either no extended ACL entries or only zero-flag `DENY`
entries whose sole permission is `DELETE`. An ACL outside that closed policy is
`Corrupt`; failure to read the descriptor-bound ACL is `Unavailable`.

## Bounds, effects, and concurrency

One list processes at most 1,024 non-dot directory entries plus one name-only
overflow witness, accepts at most 100 records, and retains at most 64 KiB per
record plus one transient overflow byte used only to reject an oversized or
concurrently growing file. It accepts at most 8 MiB aggregate canonical record
bytes plus one transient overflow byte. JSON is limited to four container
levels and 64 nodes before typed decoding. A complete
list proves that every observed canonical candidate within the hierarchy and
budgets validated; truncation is bounded incomplete observation, not a cursor
or pagination promise. Exact lookup retains the same per-record bounds.

Future construction performs no environment read, current-directory access,
canonicalization, hashing, filesystem operation, allocation proportional to
store contents, runtime construction, task, thread, timer, watcher, provider,
permission, network, or process operation. All synchronous bounded work starts
on first poll. Dropping before first poll is effect-free. The operation does not
create, lock, repair, rewrite, delete, probe, signal, or explicitly change
timestamps. Linux record and directory descriptors request `O_NOATIME`. macOS
has no per-open equivalent, so filesystem-managed access times may advance
according to the mounted filesystem's policy; inspection never restores them
with a metadata write.

Existing hierarchy components are opened descriptor-relatively without
following symlinks and must satisfy the native owner/mode and macOS ACL policy.
The cooperating native writer publishes complete private regular files by
atomic replacement; a reader may observe the complete old or new inode but no
multi-record snapshot is promised. Concurrent disappearance may omit a list
candidate or yield `NotFound` for exact lookup. Other I/O ambiguity is redacted
as `Unavailable`. Filesystem calls have no universal wall-clock guarantee.

FreeBSD, Windows, WASI, and other unsupported targets return the active fixed
`Unsupported` category and never pretend the history is empty.

## Deferred pinned-fx surface

Pinned fx also connects its native supervisor to interactive `/background
stop`, `open`, and `logs` commands. Machine-god's process-local supervisor is
not exposed through those commands. Durable cross-process control still
requires workspace/session leases, process-instance tokens, bounded managed
logs, authenticated control, and crash recovery. A recorded PID alone is never
stop or liveness authority. None of those capabilities, terminal background
input, arbitrary log paths, URL probing, `/proc` inspection, repair, or
migration is part of this read-only command.

The separately injected terminal `inspect` and bounded exit-only `wait` actions
reuse the same exact-ID persisted-record validation but return only compact
recorded-state projections. `wait` observes atomic record replacements through
an independently injected monotonic delay boundary; neither action broadens
this top-level CLI or adds supervisor authority.

The compatibility scenario moves only from unimplemented to
implemented-but-non-equivalent and remains not measured and claim-ineligible.
No sample, threshold, performance result, or upstream-equivalence claim is
introduced.
