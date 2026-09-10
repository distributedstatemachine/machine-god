# Top-level background command

The command exposes a bounded read-only observation of persisted background
history for the canonical current workspace. The CLI owns only grammar,
presentation, exit codes, and output writes. Native code owns environment,
current-directory, state-root, descriptor, record, and hashing effects. This
slice starts no work and deliberately has no process-control authority.

The native [background supervisor](background-supervisor.md) writes legacy
numeric records; the complete [terminal host](terminal.md) writes committed
terminal histories. This top-level command observes both and remains a
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
machine-god background <terminal-32-lowercase-hex> [--json]
machine-god background --json <terminal-32-lowercase-hex>
```

At most one target and one `--json` flag are accepted. IDs use nonempty ASCII
decimal digits and must fit `u64`; leading zeroes are accepted within the
20-byte token cap and do not change numeric identity. Terminal IDs use the
exact generated `terminal-` prefix followed by 32 lowercase hexadecimal digits;
the 41-byte token is an opaque string, never a numeric alias, path or display
index. `stop`, `open`, `logs` and an explicit `list` token are not top-level
operations. `last` is exact and
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

No target lists a union of at most 100 validated legacy records and 128
committed terminal histories for the canonical workspace. Ordering compares
legacy `updated_at_ms` and terminal `last_output_ms` losslessly, descending.
Equal timestamps put terminal IDs before legacy IDs, then sort each identity
domain descending. Interactive selection independently uses creation time.
`last` returns the first record in this observed order, retaining its detail
from the same scan rather than reopening the selected record. There is no
atomic cross-store snapshot. If scan or aggregate truncation could hide a newer
record, `last` fails `ResourceLimit`; it never returns a possibly false latest
record. An exact numeric ID reads only the legacy namespace and canonical
record name; an exact terminal ID reads only the terminal namespace. Missing
list hierarchy is an empty complete result; missing `last` or exact ID is
`NotFound`.

List rows expose only `id`, recorded `state`, `updated_at_ms`, and a UTF-8
command preview of at most 256 bytes (empty for a commandless terminal).
Numeric legacy IDs remain JSON numbers; terminal IDs are JSON strings.
For terminal rows, `updated_at_ms` is the saved last-output timestamp.
Preview truncation occurs at a character
boundary and is explicit. List JSON uses kind `background` and fixes top-level
key order `kind,count,truncated,records`; each row fixes key order
`id,state,updated_at_ms,command_preview,preview_truncated`. Human mode starts
with `[background] no persisted background records` for a complete empty result
or `[background] N saved`, then one bounded row per record. A truncated list
ends with `[background] listing incomplete: a resource limit was reached`.

Legacy detail remains unchanged: numeric ID, recorded state, start and update timestamps,
optional PID, full bounded command, canonical recorded working directory,
optional exit code, optional server URL, and optional diagnostic. JSON kind is
`background_detail`; human mode labels every field. `running` and all other
states are explicitly recorded history, not a current-liveness assertion.

Terminal detail uses kind `background_terminal_detail` and fixed JSON key order
`kind,id,state,created_at_ms,last_output_ms,command,cwd,exit_code,signal,earliest,latest,facts_cursor,recorded_only`.
Command may be null for a commandless session. Exit code and signal are nullable
and mutually exclusive. Each cursor contains `segment,offset`. The final
`recorded_only` field is always true. Human mode labels the same fields.
Terminal states are `starting`, `running`, `exited`, `lost` and `closed`;
saved state and outcomes do not establish live ownership. No PID or server URL
is invented from terminal history, and raw output is not rendered here.

Strings are JSON-escaped in machine mode and terminal-control-sanitized in
human mode. Both modes have exactly one final LF. The complete representation
is validated and rendered before the first success write. Its ceiling is six
times the command, cwd, URL and diagnostic byte limits plus 1,024 bytes for
fixed fields and framing, including that LF. It accommodates worst-case
escaping of every valid detail and every 228-row union. A violated snapshot invariant, checked-size overflow, or
one-byte excess becomes `ResourceLimit` with no partial success output. Writer
failure uses only the fixed global output diagnostic.

## Native persisted schema

Linux and macOS select nonempty `XDG_STATE_HOME`, otherwise nonempty `HOME`
plus `.local/state`, and then the fixed `machine-god` hierarchy. Legacy records
use `background-v1`; complete terminal histories use `terminal-v1`.
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
and is at most 64 KiB. Workspace and cwd are absolute canonical Unicode paths,
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

### Committed terminal histories

The read-only terminal inspector validates the existing profile topology,
catalog ownership, terminal identity and committed facts/output cursors. It
reads committed state only; it never prepares a profile, recovers a journal,
repairs a torn tail, recreates a missing lock or converts saved process metadata
into live authority. All terminal namespace entries are validated before
workspace or exact-ID filtering. Malformed topology or committed facts therefore
fail the query even when they would not appear in its result. Duplicate matching
terminal identities fail closed instead of selecting an arbitrary owner.

Terminal inspection is complete-or-error, bounded to 128 rows, 1 MiB of retained
text (including identity, workspace, command and cwd) and 64 MiB of input.
A malformed, unavailable or over-budget
terminal namespace fails union queries; it never yields a partial-success
legacy-only list. Legacy list truncation retains its explicit existing contract.

## Bounds, effects, and concurrency

The legacy portion of a list processes at most 1,024 non-dot directory entries plus one name-only
overflow witness, accepts at most 100 records, and retains at most 479,744 bytes per
record (six times bounded text fields plus 512 bytes of framing), plus one
transient overflow byte used only to reject an oversized or
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
create, repair, rewrite, delete, probe, signal, or explicitly change timestamps.
Legacy observation takes no lock. Terminal observation takes a shared lock on
the existing profile lock descriptor; a busy or missing required lock is a
fixed operational failure, never permission to create one. Linux record and
directory descriptors request `O_NOATIME`. macOS
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
On Linux/macOS, the public combined native facade requires its existing
`ai-gateway-http` terminal graph for list, last and terminal queries. Without
that feature those queries are inert until polled and return `Unsupported`,
not a misleading complete legacy-only list. Exact numeric queries retain the
unchanged legacy API behavior without that feature. Production Linux/macOS CLI
builds already enable it. Existing `NativeBackgroundQuery` and inspection APIs
remain unchanged for legacy terminal-tool consumers.

## Interactive background commands

### Interactive request and URL-handoff contracts

`NativeBackgroundCommand` parses the payload following `/background` without
environment, filesystem or process access. An empty payload requests the list;
`stop`, `open` and `logs` accept one optional target, with an omitted target or
`last` selecting the latest entry. Space and tab separate tokens. The parser
rejects more than 256 UTF-8 bytes, other controls, extra tokens, unknown commands
and invalid targets before any effect.

Interactive targets use the complete terminal host's existing
`terminal-<32-lowercase-hex>` identities. They are not numeric indices into a
changing display. This is an intentional spelling difference from pinned fx's
numeric interactive background IDs; legacy numeric top-level inspection remains
distinct. Identifiers are descriptive, never process or workspace authority.

The interactive control lane resolves the current conversation's exact native
terminal generation. Its complete list contains at most 128 rows and 1 MiB of
command/cwd text, newest creation timestamp first and descending terminal ID on
ties. Unknown owners are errors, not empty successes. Selection resolves `last`
once; later reads, close and URL admission retain that same sealed target.
Conversation handoff and host shutdown revoke stale targets. Saved PIDs never
reconstruct control authority. Listing distinguishes retained backend ownership
from readable history; command previews are explicitly shortened at 256 bytes.

`stop` requests native graceful close and retains its exact receipt. A
history-only result does not assert a process was stopped. `/cancel` and shutdown
cancel background controls while continuing to own their completion futures;
committed or uncertain effects are not reported as rollback and are not retried.

`logs` reads separate head and tail windows of at most 16 KiB each, with at most
1,024 native pages per window. Each window freezes its first observed end
cursor; the pair is not an atomic snapshot. Gaps and incomplete windows are
explicit, and discontinuous spans are not concatenated into synthetic output.
Presentation selects the first 40 head lines and last 40 tail lines, marks omitted
lines and prints source/next/end cursors. Invalid UTF-8 bytes use visible hex
escapes; terminal controls are escaped. The bounded 256 KiB renderer completes
before the output write and never changes the native receipt on output failure.

URL selection examines at most 64 KiB of captured output. It accepts only
validated HTTP(S) candidates whose raw and canonical spellings each fit 2,048
bytes. Invalid and oversized candidates are skipped, not truncated into valid
URLs; an oversized input capture is an error. Parsed-host ranking prefers
loopback and local-network addresses, uses pinned line hints, and chooses the
latest candidate on a score tie. Credentials, missing authority, backslashes,
invalid UTF-8 and embedded whitespace/control characters are rejected.
Canonicalization may normalize host spelling, default ports and path encoding.
Uppercase HTTP(S) schemes are accepted. Candidate text in a path or query cannot
masquerade as a loopback host.

`NativeBackgroundUrlOpener` receives explicit executable, environment and native
worker-scope authority. Construction does not discover a browser, read the
environment or launch a process. The caller protects the executable installation
for the capability lifetime; retained-file identity checks do not replace that
prerequisite. A polled request passes one validated URL argument directly to the
launcher, with no shell interpolation, null standard streams and a fixed root
working directory. The exact target's generation and cancellation are checked
at launch admission. No network probe or claim of continued server availability
follows from a URL observed in output.

Interactive startup captures the optional fixed desktop launcher on its existing
blocking startup worker: `/usr/bin/open` on macOS, `/usr/bin/xdg-open` on Linux.
There is no PATH search, workspace fallback or shell. The retained descriptor
must name the canonical regular executable; symlinks and parent-symlink paths
are rejected. Failure disables URL opening without failing interactive startup.
The launcher receives fixed `PATH=/usr/bin:/bin` and only the bounded desktop
keys `HOME`, `USER`, `LOGNAME`, `DISPLAY`, `WAYLAND_DISPLAY`, `XAUTHORITY`,
`XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, `XDG_CURRENT_DESKTOP`,
`XDG_SESSION_DESKTOP`, `DESKTOP_SESSION`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
`XDG_CONFIG_DIRS`, `XDG_DATA_DIRS`, `LANG`, `LC_ALL`, `LC_CTYPE` and `TMPDIR`.
Capture does not enumerate the environment or forward unrelated credentials,
loader settings or `BROWSER`.

Clones share one operation admission through actual direct-child reaping. The
launcher has a ten-second observation deadline, and cancellation or caller drop
retains native cleanup ownership. A successful launcher exit reports `Opened`,
not browser lifetime or network reachability. A nonzero exit reports
`LauncherFailed` without promising rollback. Once launch begins, lost completion
or cancellation reports `Indeterminate`; it cannot be represented as proof that
no browser opened. None of these contracts adds process-control authority to the
top-level inspection command. Full feature integration and gate state belongs
only in the [implementation plan](implementation-plan.md).

### Supervisor and terminal ownership

Pinned fx also connects its native supervisor to interactive `/background
stop`, `open`, and `logs` commands. Machine-god's process-local supervisor is
not exposed through those commands. Durable cross-process control still
requires workspace/session leases, process-instance tokens, bounded managed
logs, authenticated control, and crash recovery. A recorded PID alone is never
stop or liveness authority. None of those capabilities, terminal background
input, arbitrary log paths, URL probing, `/proc` inspection, repair, or
migration is part of this read-only command.

The separately injected terminal `list`, `inspect`, and bounded exit-only
`wait` actions reuse this descriptor-confined persisted-record reader. Terminal
`list` returns at most 100 ordered rows containing only background ID, recorded
state, and update timestamp; it deliberately omits the command preview exposed
by this human-facing command. `inspect` and `wait` retain their compact exact-ID
recorded-state projections. `wait` observes atomic record replacements through
an independently injected monotonic delay boundary. None of these actions
broadens this top-level CLI or adds process or supervisor authority.

The compatibility scenario moves only from unimplemented to
implemented-but-non-equivalent and remains not measured and claim-ineligible.
No sample, threshold, performance result, or upstream-equivalence claim is
introduced.
