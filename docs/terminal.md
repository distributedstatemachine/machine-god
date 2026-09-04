# Native `terminal` command contract

The native tool executes one bounded foreground shell command, starts one
noninteractive background shell command after explicit process authorization,
reads bounded process-local output from a command started by the same session
incarnation, sends one explicit signal to that exact live process tree after
separate authorization, or lists, inspects, or boundedly waits on persisted
background records without process authority. It is registered by the
reference host and has no top-level CLI command.

## Boundary

The reference-host tool implements the `exec`, bounded `start`, bounded
process-local `read`, bounded persisted-record `list`, bounded persisted-record
`inspect`, bounded persisted-record `wait`, and bounded process-local `signal`
subsets of fx's `terminal` tool.
`exec` captures bounded standard output and error and waits for the direct
child. `start` durably records and releases one noninteractive command through
the native background supervisor, starts capturing its merged output, and
returns its display identity without waiting for command completion. `read`
pages that captured output only for the exact session incarnation that started
it. `list` returns a compact bounded catalog of recorded history.
`signal` delivers exactly one of `hangup`, `interrupt`, `quit`, `terminate`, or
`kill` to the identity-checked live process tree owned by the exact session
incarnation, then acknowledges delivery without waiting for exit or escalating
to another signal. It does not derive authority from the displayed PID.
`inspect` reads the validated record for one display identity without claiming
current liveness. `wait` observes bounded atomic replacements of that exact
record until it contains a supported recorded exit or reaches the requested
safety ceiling. Standalone public terminal constructors remain exec-only;
injecting only a trusted background starter adds only `start`, `list` appears
only when a trusted lister is explicitly injected, `inspect` appears only when
a trusted inspector is explicitly injected, and `wait` appears only when its
separately bounded waiter is also injected. `read` appears only when a trusted
process-local output reader is explicitly injected alongside a starter.
`signal` appears only when a trusted process-local signal controller is
explicitly injected alongside a starter.

The model-facing input is:

```json
{
  "action": "exec",
  "command": "cargo test --workspace",
  "cwd": ".",
  "profile": "clean"
}
```

`action` and `command` are required for the closed `exec` and `start` forms.
The reference host also accepts the separate closed forms `{"action":"list"}`
and `{"action":"inspect","background_id":7}`, where `background_id` is a
nonzero JSON `u64`, plus this exact wait form:

```json
{
  "action": "wait",
  "background_id": 7,
  "return_when": { "kind": "exit" },
  "wait_ceiling_ms": 30000
}
```

All four wait fields are required. `return_when` accepts exactly the closed
object shown above, and `wait_ceiling_ms` is an integer from 1 through 30,000.
The separate read form is:

```json
{
  "action": "read",
  "background_id": 7,
  "cursor_segment": 1,
  "cursor_offset": 0
}
```

`action`, nonzero `background_id`, and `cursor_segment` are required.
`cursor_segment` is exactly `1`. `cursor_offset` is an optional JSON `u64` and
canonicalizes to zero when omitted. The numeric background ID is only a display
and lookup value; the host additionally binds every read to the caller's exact
session ID and session-incarnation ID.
The separate signal form is:

```json
{
  "action": "signal",
  "background_id": 7,
  "signal": "terminate"
}
```

All three fields are required. `background_id` is a nonzero JSON `u64`, and
`signal` accepts exactly `hangup`, `interrupt`, `quit`, `terminate`, or `kill`.
The host privately supplies the current engine session ID and incarnation; the
model cannot select or spoof either owner field.
An exec-only construction accepts only `exec`; a starter-only construction
accepts `exec` and `start`.
`cwd` is optional and defaults to `"."`. `profile` is optional and accepts
only `"clean"`; omission has the same meaning. Unknown or duplicate fields,
mistyped values, an empty command, and a command over 32 KiB reject. The
complete canonical argument object is bounded by 64 KiB.

`cwd` is a canonical workspace-relative directory spelling. `.` selects the
workspace root. Otherwise it contains at most 256 slash-separated components,
4,096 UTF-8 bytes in total, and 255 UTF-8 bytes per component. Empty
components, repeated or trailing separators, absolute and platform-prefix
paths, components spelled exactly `~`, `.` or `..`, NUL, C0/C1
controls, U+2028 LINE SEPARATOR, U+2029 PARAGRAPH SEPARATOR, and bidi-control
characters reject rather than normalize. Unicode is not normalized or
case-folded. A literal component such as `~cache` is an ordinary valid name,
whether leading or nested; only the exact component `~` rejects. Preparation
performs no filesystem, environment, process, thread, or network effect. For
`start`, the captured canonical workspace, one separator when needed, and a
non-`.` relative `cwd` must additionally fit the background request's 4,096-byte
absolute-cwd limit. This can make the accepted relative limit smaller than the
common 4,096-byte parser limit; preparation checks the combined byte length
without constructing the absolute path or a background request.

The reference-host tool description is:

```text
Run a foreground command, start a background command, read bounded same-session background output, signal one live same-session background process tree, list persisted background records, inspect one persisted background record, or wait for its recorded exit
```

An exec-only construction retains its earlier foreground-only description and
schema. All forms deliberately exclude `screen`, `write`, `monitor`, `resize`,
`close`; list filters and pagination; PTYs; interactive stdin;
durable or restart-safe output; output tail retention; separate background
stdout/stderr channels; artifacts; custom or login shells; user shell profiles;
retries; external working directories; and benchmark workloads. They make no
fx-equivalence or product-performance claim.

## Permission and exact execution agreement

Core extends `Capability::Process` so permission policy receives the complete
immutable execution identity:

```text
Capability::Process {
    program: "/bin/sh",
    arguments: ["-c", canonical_command],
    working_directory: authorized_cwd,
    environment: {
        profile: "construction_snapshot",
        sha256: lower_hex_digest,
    },
}
```

For `exec`, `authorized_cwd` is the canonical workspace-relative argument and
the profile is `construction_snapshot`. The environment digest identifies the
exact bounded snapshot retained when the tool is constructed; it never exposes
raw keys or values. Construction accepts at most 512 entries, 1,024 bytes per
key, 16 KiB per value, and 256 KiB in aggregate. Keys must be nonempty, contain
neither `=` nor NUL, and values must contain no NUL. Entry validity plus
individual and aggregate sizes are checked
before sorting, so a rejectable snapshot cannot trigger sort work outside the
stated construction bounds. Valid entries are then sorted by raw platform
spelling before length-prefixed SHA-256 hashing, so insertion order cannot
change permission identity. The system executor clears its environment and
installs exactly that snapshot. The model cannot add, remove, or replace an
entry.

For `start`, `authorized_cwd` is the same validated workspace-relative argument
used by `exec`, interpreted against the terminal's retained workspace identity.
Its profile is `background_fixed`, and its digest identifies exactly the
supervisor's fixed `LANG=C`, `LC_ALL=C`, and `PATH=/usr/bin:/bin` environment.
Preparation verifies without allocation that the eventual absolute canonical
workspace/cwd fits the background request bound. The absolute string used by
background persistence is derived privately only during allowed execution. The
injected background constructor binds its canonical path to the retained
workspace descriptor by device and inode before accepting the starter. Renaming
that retained directory or placing a replacement at its former pathname
therefore cannot make the process permission describe the replacement
directory.

The stable serialized process capability therefore contains the fixed program, exact
two arguments, authorized cwd, profile name, and digest. Successful preparation
returns those same canonical model arguments. Direct `execute` reparses and
revalidates all fields and rejects any canonical-argument, program, argument,
cwd, profile, or digest divergence before filesystem access, worker creation,
or process spawn. The existing engine presents terminal execution as critical
risk. Denial has zero terminal-owned effects. `signal` instead prepares the
exact custom capability
`{"name":"terminal_signal","details":{"background_id":7,"signal":"terminate"}}`;
the requested identity and signal cannot change after authorization, and
denial performs no registry lookup, process-table scan, or signal syscall.
`read`, `list`, `inspect`, and `wait` prepare with no authority because they can read only through
explicitly injected owner-scoped output or persisted-history boundaries; none
requests process permission.

The capability authorizes a process, not a sandbox. The retained workspace
descriptor constrains only the child's starting-directory identity. Once
approved, `/bin/sh` and the command can use absolute paths, inherited
credentials in the approved snapshot, child processes, and network or other
host authority available to the machine account. Hosts requiring isolation
must add an external sandbox; this contract claims none.

## Workspace and platform boundary

The trusted host supplies one already-opened workspace-root descriptor. On
Linux, execution walks each selected cwd component descriptor-relatively with
directory, no-follow, nonblocking, and close-on-exec opens. Symlinks, missing
components, and non-directories reject. The final retained directory descriptor
is converted only from trusted process state to
`/proc/<machine-god-parent-pid>/fd/<directory-fd>` and used as the child's
starting directory. The injected pathname is never reopened as authority.
Rename or unlink after descriptor retention cannot redirect the starting
directory.

Safe standard Rust has no descriptor-relative `Command` cwd primitive on
macOS, and the audited
[Darwin process-query exception](decisions/0003-bounded-darwin-process-query-ffi.md)
does not authorize command-launch FFI. The production system executor is
therefore Linux-only. Public `TerminalTool::open` and
`TerminalTool::open_with_limits` construction on macOS, FreeBSD, WASI, and other
non-Linux targets fails with the fixed unsupported category before filesystem
lookup, environment inspection, thread creation, or spawn.

Reference-host composition retains the workspace authority and advertises
`terminal` as defined by the canonical
[tool catalog](native-reference-host.md#tool-catalog). On macOS, `exec` returns
its fixed unsupported error after strict preparation and permission. `start`,
descriptor-confined `list` and `inspect`, and persisted-record `wait` are
supported on Linux and macOS. Process-local `read` is supported there for
commands started through that same composed host and session incarnation. On
other platforms the complete reference host
is unavailable. The exported exec contract remains portable through a trusted
injected `TerminalExecutor`, and a trusted injected
`TerminalBackgroundStarter` may implement the documented background ownership
contract. A trusted injected
`TerminalBackgroundInspector` may implement the exact persisted-record read
contract, a trusted injected `TerminalBackgroundCatalog` may implement the
bounded persisted-record catalog contract, and a separately injected waiter
may implement the bounded
persisted-record wait contract. A trusted injected
`TerminalBackgroundOutputReader` may implement the owner-scoped process-local
read contract.

## Background start protocol

`start` accepts exactly the common `action`, `command`, `cwd`, and `profile`
fields. Shell, backend, return condition, wait ceiling, dimensions, initial
monitors, caller-selected session IDs, and every interactive or control field
reject as unknown. Preparation is effect-free, checks the eventual combined
absolute-cwd byte length without allocation, and does not build a background
request or clone the command for one. Execution revalidates the exact canonical
object, reuses that checked length, checks cancellation, moves the owned command
into a request with the privately derived bounded absolute cwd, and then
delegates to the one host-owned supervisor. It does not consume a foreground
execution slot or create a foreground deadline, guardian, output buffer, pipe,
reader, or executor call.

The supervisor owns the start commit, cleanup, capacity, persistence, worker,
and cancellation protocol defined by
[background-supervisor.md](background-supervisor.md). Cancellation before
delegation has zero starter effects. A supervisor-reported cancellation returns
the fixed terminal cancellation error. Once the supervisor returns success,
the durable running record and released process are committed; cancellation
observed afterward cannot relabel that success.

The successful output is exactly:

```json
{
  "action": "start",
  "background_id": 7,
  "pid": 1234,
  "status": "started"
}
```

`background_id` is a nonzero durable display identifier. `pid` is a nonzero
display-only process identifier or `null`; neither value grants process-control
authority. Success means the supervisor completed its release contract, not
that the shell is still running when the result is observed. Capacity and
clock failures are retryable fixed unavailable errors. Persistence, process,
and invariant failures are fixed redacted execution errors.

## Process-local background output read

For a start carrying output ownership, the helper keeps one pipe whose bytes
combine the final shell's standard output and standard error. The private
readiness marker is consumed before capture begins and can never appear in the
stream. Standard input remains `/dev/null`. The supervisor continuously drains
the pipe while the command runs, including after the retained prefix is full,
so an output flood cannot block command completion.

The process-local registry admits at most 16 live captured streams and retains
at most 100 closed streams, evicting only the oldest closed stream. It retains
the first 64 KiB per stream and counts all observed bytes with saturation. One
read returns at most 7 KiB. That page bound keeps the complete result below the
48 KiB serialized-result limit even when every byte needs a six-byte JSON
control escape. Pages retreat from their raw size boundary rather than split a
potentially valid UTF-8 scalar. A trailing partial scalar in an open stream is
temporarily withheld with an unchanged cursor and `lossy: false`; once its
remaining bytes arrive it is returned intact, while an incomplete scalar at
stream close is invalid UTF-8 and is replaced lossily. Other invalid UTF-8 is
likewise replaced and reported. Captured bytes, owner identities, commands,
paths, and native errors are absent from Debug and error values.

The registry entry is hidden until process release commits. A failed or dropped
pre-release start removes it. After release, reads require the exact
`(session_id, session_incarnation_id, background_id)` tuple; a wrong owner and
an evicted or unknown ID all fail identically as `terminal_read_not_found`.
The strict tool schema accepts only cursor segment one; another segment is a
`terminal_invalid_arguments` error before reader dispatch. An offset beyond
observed output bytes returns `terminal_read_invalid_cursor`. Registry or
adapter failure returns retryable
`terminal_read_unavailable`; four reads may be pending concurrently and further
reads fail retryably as `terminal_read_busy`.

The successful result is exactly:

```json
{
  "action": "read",
  "background_id": 7,
  "cursor_segment": 1,
  "cursor_offset": 12,
  "output": "hello\nworld\n",
  "output_bytes": 12,
  "retained_bytes": 12,
  "truncated": false,
  "lossy": false,
  "stream_closed": true
}
```

`cursor_offset` is the next offset. `output_bytes` counts bytes observed from
the pipe, whereas `retained_bytes` reports the readable prefix. Once output
exceeds the prefix, `truncated` stays true. A bounded final drain that closes
before EOF also sets `truncated`; in that case `output_bytes` is a lower bound
because an unread suffix was discarded. Reading from within the prefix advances
by the returned page. Reading at or beyond the retained boundary of a truncated
stream returns an empty page and advances directly to `output_bytes`, making
known discarded bytes explicit without creating a retry loop. `stream_closed`
means this process-local producer closed; it is not a persisted process-state
or liveness claim.

The read future is inert until poll. Pre-cancellation has no reader effect; a
registered cancellation wake resolves and drops a pending injected reader even
if it ignores the token. Cancellation is rechecked after the reader and before
publication. The four-slot permit is released on success, error, cancellation,
drop, or unwind. Output exists only in this host process: restart, host exit,
closed-entry eviction, or use from another composed host loses it. Persisted
records remain independently inspectable but cannot reconstruct these bytes.

## Process-local background signal

Signal control is available only for a process started by this host with an
output owner. A separate fixed-capacity registry binds the nonzero background
ID to the exact session and session-incarnation owner plus a clone of native
process authority; it never stores or reopens the display PID as authority.
Registration is hidden before process release. Core invokes the owned
process's bounded retain-time activation hook after the release commit and
before returning the public handle or transferring the process to its retainer.
Activation failure synchronously drops and cleans the released process,
best-effort records `dead`, and returns the fixed process failure. The registry
lease stays with retained process ownership and is removed only after the
signal gate has closed before terminal reap, so a completed or reused numeric
identity cannot regain control.

The Linux/macOS controller validates the retained root identity, takes one
bounded process ancestry snapshot, delivers the selected signal deepest-first
to identity-revalidated descendants outside the original process group, and
then signals the original group. Linux uses the retained procfs mount authority
and pidfds for individual outside-group delivery. macOS uses the safe
fixed-buffer Darwin wrapper's kernel unique-process and parent identities. A
vanished descendant is harmless, but an incomplete snapshot, identity
ambiguity, bound overflow, non-vanished delivery failure, or original-group
delivery failure rejects the operation; partial delivery is never reported as
success. One descendant failure does not suppress later descendant attempts or
the original-group attempt. One request never waits for exit, repeats, or
escalates its chosen signal.

Traversal runs on the supervisor's existing fixed worker pool rather than the
engine poll thread. Terminal admits at most four signal actions independently
of foreground executions, output reads, record reads, and supervisor process
capacity. Registry and per-process lifecycle lock contention fail fast as
retryable `terminal_signal_busy`. An unknown, completed, or wrong-owner ID is
indistinguishable as `terminal_signal_not_found`; a process-table or delivery
failure is the fixed non-retryable `terminal_signal_failed` error. Signal
future submission is the ordered mutation commit boundary: cancellation before
submission has no signal effect, while cancellation after submission cannot
relabel a completed or partially attempted native delivery.

Success acknowledges delivery only:

```json
{
  "action": "signal",
  "background_id": 7,
  "signal": "terminate",
  "status": "signaled"
}
```

It makes no claim that the process has exited. Callers may use the separate
persisted-record `wait` action when they need a bounded recorded-exit
observation.

## Persisted background inspection

`inspect` accepts exactly `action` and `background_id`; command, cwd, profile,
session, control, and list fields reject. Preparation canonicalizes to the same
two-key object and uses no authority. The execution future is inert until first
poll. Pre-cancellation has zero inspector effects, and cancellation is checked
again after the one injected read before any result is published. The tool
also registers its own cancellation wake, so cancellation resolves and drops
a pending inspector future even when the injected inspector does not observe
the supplied token.

The inspector performs exact `NativeBackgroundQuery::Id(background_id)`
semantics over the frozen workspace identity. It does not scan a listing,
probe a PID, reconcile or initialize the supervisor, claim liveness, or signal,
wait for, restart, or otherwise control a process. Every decoded recorded state
is a successful historical result. The compact output is:

```json
{
  "action": "inspect",
  "background_id": 7,
  "recorded_state": "exited",
  "started_at_ms": 1000,
  "updated_at_ms": 1200,
  "pid": 1234,
  "exit_code": 0
}
```

The result is checked against the terminal 48 KiB serialized-result ceiling.
Record-not-found, corrupt, resource-limit, unavailable, and unsupported
failures have fixed redacted mappings. The reference adapter retains the
injected state-root descriptor and canonical workspace spelling, so ambient
cwd/environment changes or replacement of the original state-root pathname
cannot redirect a read.

This is intentionally not equivalent to upstream fx interactive-session
inspection. machine-god's `background_id` identifies one persisted start
record; it is not a session authority and grants no access to interactive
terminal state or control.

## Persisted background listing

`list` accepts exactly `{"action":"list"}`. Every other field, including
`background_id`, command, cwd, profile, task, workspace, backend, lifecycle,
cursor, and pagination fields, rejects. Preparation preserves that canonical
one-field object, requests no authority, and has no lister or filesystem
effect.

Execution invokes only the separately injected persisted-history lister over
the frozen workspace identity. A missing background hierarchy or workspace
directory is a complete empty success:

```json
{
  "action": "list",
  "count": 0,
  "truncated": false,
  "records": []
}
```

A nonempty success contains at most 100 compact rows:

```json
{
  "action": "list",
  "count": 2,
  "truncated": false,
  "records": [
    {
      "background_id": 9,
      "recorded_state": "exited",
      "updated_at_ms": 1200
    },
    {
      "background_id": 7,
      "recorded_state": "running",
      "updated_at_ms": 1100
    }
  ]
}
```

`count` is exactly the returned row count. Rows are ordered by
`updated_at_ms` descending and then numeric `background_id` descending. IDs
are nonzero and unique, and recorded states use the same closed six-state
vocabulary as inspection. The projection deliberately omits command previews,
cwd, PID, exit code, server URL, and diagnostics. It therefore exposes no
process authority or present-liveness assertion and keeps the complete
100-row shape within the terminal 48 KiB serialized-result ceiling.

The lister reuses the persisted reader's existing bounds: one call processes
at most 1,024 non-dot directory entries plus one name-only overflow witness,
accepts at most 100 records, retains at most 64 KiB per record, and accepts at
most 8 MiB of aggregate canonical record bytes. Each record retains the
four-container-level and 64-node JSON bounds. A bounded incomplete scan returns
its validated partial set with `truncated` equal to `true`; that flag is not a
cursor or pagination promise. A complete list proves only that every observed
canonical candidate within those bounds validated. Concurrent atomic
replacement may expose an old or new complete record, concurrent disappearance
may omit a candidate, and no multi-record snapshot is promised.

Four list calls may be active. Further calls fail immediately with the fixed
retryable `terminal_list_busy` result before invoking the lister, opening a
file, or creating a queue, worker, thread, timer, process, or supervisor
effect. The list slots are independent of foreground-execution and wait slots.
The execution future is inert until first poll. Pre-cancellation has no lister
effect; cancellation has its own wake path, drops a pending lister future, and
releases its slot exactly once. Cancellation is checked after the read and
bounded rendering and immediately before publication, so a cancelled call
does not publish a stale success.

Corrupt, resource-limit, unavailable, and unsupported reader failures map to
fixed redacted terminal categories. An impossible `NotFound` list result or an
invalid injected shape, identity, bound, uniqueness, or ordering is the fixed
`terminal_lister_failed` invariant result; missing production state must have
returned the empty success. No failure reflects a path, ID, timestamp, command,
record content, environment value, filename, or native diagnostic.

Listing never probes a PID, infers liveness, initializes, reconciles, or calls
the background supervisor, or signals, waits for, restarts, adopts, or controls
a process. It is intentionally a bounded machine-god persisted-history
projection, not pinned-fx's interactive terminal-session catalog, and makes no
fx-equivalence or performance claim.

## Persisted background wait

`wait` accepts exactly `action`, `background_id`, `return_when`, and
`wait_ceiling_ms` in the closed form shown above. Unknown, omitted, mistyped,
zero, negative, fractional, or out-of-range values reject. Preparation
canonicalizes that same object and requests no authority. The action is an
intentional persisted-background subset: it does not wait on an interactive
session or an owned process handle and makes no fx-equivalence or performance
claim.

Execution observes only the exact persisted record selected by
`background_id`. The first observation is immediate when the ceiling still
permits it. The ceiling is checked immediately before every observation or
delay poll, and no such work begins after it has elapsed. If the ceiling
expires before the first observation begins, there is no snapshot and
execution returns the fixed retryable `terminal_wait_unavailable` error. While
an observation is pending, it is raced against one persistent injected timer
for the absolute ceiling, so an inspector that does not wake itself is still
dropped when that timer expires. The same absolute timer races a pending
backoff delay. While the record remains `running`, subsequent observations are
separated by delays
of 16, 32, 64, and 128 milliseconds, then 250 milliseconds, always clipped to
the caller's absolute `wait_ceiling_ms`. After an in-flight observation
returns, elapsed-ceiling and cancellation checks win before any newly observed
exit is published. Whenever the ceiling wins after a snapshot has been
accepted—including a snapshot returned by an observation that raced the
ceiling—that snapshot produces the bounded ceiling result without another
observation. One call
performs at most 128 exact observations, never scans a listing, and retains
only the latest running snapshot between attempts. If that observation cap is
reached first, the same latest snapshot produces the bounded ceiling result.
Four wait calls may be active; further calls fail immediately without an
observation, timer, queue, thread, process, or supervisor effect. Repeated
polling neither probes a PID nor initializes, reconciles, calls, or controls
the background supervisor. The numeric PID in a record remains display-only
and is never used as liveness evidence.

A validated `exited` record with exit code 0, or a validated `failed` record
with an exit code from 1 through 255, succeeds with the upstream-compatible
tagged-object outcome union:

```json
{
  "action": "wait",
  "background_id": 7,
  "outcome": { "exited": 0 },
  "recorded_state": "exited",
  "started_at_ms": 1000,
  "updated_at_ms": 1200,
  "pid": 1234,
  "exit_code": 0
}
```

The same complete shape is returned for a validated `failed` record, with
`recorded_state` equal to `failed` and both `outcome.exited` and `exit_code`
equal to the recorded code from 1 through 255.

If an observation that began before the ceiling returns only after it, the
ceiling wins even when that returned record is `exited` or `failed`. The
response uses `outcome: { "safety_ceiling": {} }` while `recorded_state`, both
timestamps, `pid`, and `exit_code` remain the exact projection of that returned
record; the newly observed exit is not published as `outcome.exited`.

If the latest accepted snapshot is still `running` when the absolute ceiling
or observation cap wins, the successful bounded result is:

```json
{
  "action": "wait",
  "background_id": 7,
  "outcome": { "safety_ceiling": {} },
  "recorded_state": "running",
  "started_at_ms": 1000,
  "updated_at_ms": 1100,
  "pid": 1234,
  "exit_code": null
}
```

Recorded `stopped`, `dead`, or `stale` states, and recorded exit codes outside
the supported ranges, return the fixed redacted lost-wait result rather than an
exit or liveness claim. Missing, corrupt, resource-limit, unavailable, and
unsupported observations retain the exact fixed inspection mappings; timer and
capacity failures use fixed redacted wait-unavailable categories. No record
contents or native diagnostics are reflected. Every successful output is
checked against the terminal 48 KiB serialized-result ceiling.

The wait future is inert until first poll. Pre-cancellation performs no waiter
effect. Once active, cancellation has its own wake path and drops any pending
observation and timers before releasing the wait slot. Inspector, timer, and
caller-Waker destruction occur before the final cancellation check, and
cancellation is checked again after bounded rendering and immediately before
publication. Cancellation raised by any of those destructors therefore wins
instead of publishing an exit or safety-ceiling result. Dropping the outer
future performs the same deregistration and releases the slot exactly once; no
timer, waiter, or retained record may outlive its owning wait future.

Inspection and timer readiness times are captured before their futures and the
caller Waker are torn down. Time spent in that teardown cannot turn an early
timer into a valid one or a timely observation into an overrun. Cancellation
raised during teardown retains precedence over the captured readiness result.

The absolute ceiling bounds controllable userspace waiting, not an
uninterruptible filesystem syscall, arbitrary trusted future poll or drop, or
Waker callback. Once such work returns, cancellation and the elapsed ceiling
are checked before publishing a newly observed exit or any other output. The
wait does not initiate another controllable operation after either the ceiling
or observation cap wins. At most one 64 KiB record, one bounded decoded detail,
and one persistent absolute-ceiling timer are live per admitted wait. During a
backoff, its shorter delay is the only second timer; aggregate decoded input is
at most 8 MiB across 128 maximum-size observations and does not accumulate in
memory.

## Foreground execution protocol

Production launches exactly `/bin/sh` without `PATH` lookup and supplies the
exact argument vector `[/bin/sh, -c, command]`. It is not a login shell and no
machine-god-selected startup file is loaded. Standard input is null. Standard
output and error are independent pipes and are never claimed to preserve their
cross-stream interleaving.

The execution future and injected executor are inert until first poll. The tool
acquires one fail-fast concurrency permit before cwd lookup and creates no
worker when saturated. The default active limit is four and the public hard
maximum is sixteen; saturation returns a retryable busy result without a queue,
thread, or child. `TerminalTool::open` selects the 120-second/four-active
defaults. `open_with_limits` selects the same fixed system executor with public
validated bounds, and `with_executor` supplies a trusted executor plus those
bounds. Accepted deadlines are 1 millisecond through 600 seconds.

One successful admission creates exactly one execution activity and consumes
exactly one active slot. One activity-backed coalescing notifier is shared,
never incremented, by the outer call, its owned request and executor, every
terminal-owned Waker registration, and the native worker and deadline threads
through their actual returns. The notifier supplies the outer cancellation
future, injected or system executor polling, and deadline notification. Every
retained notifier or Waker clone owns the same activity, but at most one
underlying caller-Waker callback is in flight for the admitted execution;
concurrent notices before a re-poll coalesce into that callback. If a poll
observes the in-flight callback and a later notice arrives before it returns,
the notifier preserves one serialized replay to the latest bound caller Waker
so that notice is not lost. The supplied notifier Waker may itself be used to
re-poll the outer future. It is recognized in that outer `Context` and is never
installed as its own notification target; such a self-re-poll preserves the
last external caller Waker instead of forming an `Arc` cycle or recursively
notifying itself. No notifier lock is held while an arbitrary Waker is cloned,
dropped, or invoked. Retaining any request, executor, notifier, or supplied
Waker keeps the same activity alive, so later calls fail fast as busy while the
configured capacity remains occupied.

The outer call retains that activity through bounded output rendering, the
final cancellation check, and public function return. A frame-owned RAII guard
closes the notifier on every exit from the await frame: normal return, drop of a
pending outer future, and unwind. Close marks delivery closed, cancels queued
replay, and takes the external target while holding the state lock, then detaches
and destroys that target outside the notifier lock. No notice through a supplied
Waker retained past frame destruction may reach the stale external task. An
independently retained supplied-Waker clone still owns the activity and capacity
until it is dropped, and a callback already in flight likewise owns the activity
until it returns. The slot is released exactly once, only after the last outer,
request, executor, notifier, Waker, callback, or native-thread activity owner
returns or is dropped.

The timeout deadline begins on first poll before capacity admission or cwd
validation. After admission, one tool-owned condition-variable guardian wakes
the outer future at that deadline independently of the executor. It enforces the
same deadline around every controllable userspace phase, including a permanently
pending injected executor, which is destroyed at expiry. Failure to create that
bounded guardian fails before executor construction or process spawn.
Cancellation is rechecked after executor and guardian destruction and again
immediately before a `ToolOutput` is returned.

Before registering outer cancellation, polling either the built-in or a public
injected executor, or arming deadline notification, TerminalTool supplies an
opaque Waker from the shared activity-backed notifier. A public executor
therefore needs and receives no access to the private activity counter. Every
retained clone and the single coalesced callback in flight keeps the originating
slot until it returns. Using that supplied Waker to re-poll the outer future does
not replace the notifier's external delivery target.

This timeout is not an unconditional wall-clock ceiling. Safe Rust cannot
preempt a host thread blocked inside a filesystem lookup, `Command::spawn`, a
kernel wait, another uninterruptible syscall, a trusted executor's synchronous
`poll` or `Drop`, or an arbitrary blocking `Waker` callback. Cancellation and
the deadline are checked immediately around controllable boundaries, and
execution resumes the authoritative timeout path when control returns, but
elapsed wall-clock time can exceed the requested duration while one of those
synchronous operations remains blocked.

The child starts in a new process group. One system worker owns it and two
bounded readers drain stdout and stderr concurrently to prevent pipe deadlock.
Across both streams, execution:

- retains at most 64 KiB of raw output using deterministic head-and-tail
  retention;
- continues draining and counting through 1 MiB of produced bytes;
- attempts to publish `output_limit` once either reader observes an aggregate
  produced count beyond 1 MiB. The shared final-cause close described below
  decides whether that observation or a concurrent deadline is authoritative;
  and
- promptly stops both readers after that observation, with fixed chunk and
  post-stop read-count ceilings that deterministically bound overshoot rather
  than claiming termination on the first byte beyond 1 MiB.

Cancellation is checked first. Output-limit observation and deadline expiry
then use one linearized final-cause close. If a reader's overflow observation
linearizes first, its output-limit claim closes timeout competition. Successful
cleanup then publishes a valid `output_limit` outcome. The claim does not
fabricate that outcome when cleanup fails: the specific fixed typed wait, pipe,
or other executor cleanup error is preserved regardless of whether the deadline
passes before the error reaches the outer tool. If the timeout close linearizes
first, timeout wins and any overflow observed afterward cannot change that
closed cause. Final publication never exposes a contradictory status/counter
pair, rewrites a validated executor outcome inconsistently, or converts a
specific cleanup failure into a deadline-dependent generic invariant.

Invalid UTF-8 is replaced lossily in presentation and identified by per-stream
`*_lossy` flags; the output makes no byte-round-trip claim. Head/tail omission
is identified by per-stream `*_truncated` flags and total byte counters.
Final rendering trims retained text on UTF-8 boundaries as needed so the
complete serialized `ToolOutput` never exceeds 48 KiB, including JSON escaping.

The public successful protocol object is:

```json
{
  "action": "exec",
  "cwd": ".",
  "status": "exited",
  "exit_code": 0,
  "signal": null,
  "stdout": "",
  "stderr": "",
  "stdout_bytes": 0,
  "stderr_bytes": 0,
  "stdout_truncated": false,
  "stderr_truncated": false,
  "stdout_lossy": false,
  "stderr_lossy": false,
  "duration_ms": 1
}
```

`status` is one of `exited`, `signaled`, `timed_out`, or `output_limit`.
`exit_code` is an integer from 0 through 255 only for ordinary exit and
`signal` is an integer from 1 through 255 only when the direct child terminated
by signal. Reported duration is bounded by 600 seconds. Exit zero is the sole
`ToolOutput` success. Nonzero exit, signal, deadline, and output limit return
the same bounded structured object with `is_error: true`; these are command
outcomes, not reflected operating-system diagnostics. Spawn, wait, pipe,
invariant, and unsupported failures use fixed redacted tool-error categories.

## Foreground cancellation, timeout, and ownership

Cancellation is checked before cwd acquisition, before capacity and worker
creation, in the serialized final-spawn gate, after spawn failure, while
waiting, after executor and guardian destruction, and immediately before final
`ToolOutput` publication. Cancellation wins any same-poll race and returns the
fixed cancelled tool error without partial command output. The final spawn
attempt and abort transition share one state gate: abort recorded first
guarantees zero child; successful spawn recorded first is the command-effect
commit point. The outer cancellation registration uses the same coalescing
activity notifier as executor and deadline notification, so an inline or
blocking cancellation callback cannot outlive activity accounting.

After commit, cancellation, timeout, output overflow, or future drop sends
`SIGTERM` to the owned process group, waits for a bounded grace, sends `SIGKILL`
if necessary, and observes for another bounded grace. An already-exited
foreground leader instead receives group `SIGTERM` followed immediately by the
final group `SIGKILL`; it does not impose the termination grace on every normal
command. Both paths retain the direct-child leader identity until the final
group signal has been dispatched, avoiding numeric PID/PGID reuse between
reaping and signalling, then reap the leader and observe group disappearance.
Cleanup distinguishes an absent group from permission or other signal
ambiguity, closes pipes, and joins readers. Worker joining and active-slot
release follow the notification-tail contract below.

Successful cleanup guarantees that no observed, signalable member of the
original process group remains. If disappearance cannot be established, the
tool returns fixed redacted `terminal_wait_failed` when it has a result channel,
rather than claiming success. Future drop has no result channel and completes
the same bounded cleanup path without upgrading ambiguity to a disappearance
claim. Without subreaper or elevated authority, this foreground tool cannot
prove that an adopted zombie has been reaped or that a credential-escaped or
otherwise unsignalable member has disappeared. A descendant that deliberately
escapes the group with `setsid` is also outside containment. A process or host
operation stuck in an uninterruptible kernel wait remains outside the wall-
clock claim.

Reader shutdown uses deterministic fixed read-count and chunk-size bounds, so
an escaped descendant that retains and continuously writes a pipe cannot make
join unbounded. Owned descriptors, pipes, readers, and the direct child are
cleaned before a result is published. The one admitted execution activity is
shared by the outer call, its owned request and executor, the single coalescing
notifier behind every terminal-owned Waker registration, and each native worker
or deadline thread. It is never incremented for a publisher or callback. A
native thread retains that activity through its actual return even when it
observed no Waker. A retained request, notifier, or Waker likewise retains the
slot. Concurrent executor, cancellation, and deadline notices may race, but at
most one underlying caller-Waker callback is in flight. Notices preceding a
re-poll coalesce into it; one notice following an observing poll is retained as
a serialized replay to the latest bound target. No notifier lock spans
arbitrary Waker clone, drop, or wake behavior. An inline or blocking callback
retains the activity through callback return. The frame-owned RAII guard closes
delivery on normal return, pending outer-future drop, and unwind. It cancels
replay and removes the external target under the notifier lock, destroys the
taken target outside that lock, and suppresses subsequent notices to the stale
task without releasing activity owned by an independently retained supplied-
Waker clone or a callback already in flight.

Joining a native notification thread from the consuming path could self-join or
cross-thread deadlock, so its handle may be released only while the same
activity continues to own the slot. Notification tails, OS threads, and stacks
are consequently bounded by configured capacity; further calls fail fast as
busy rather than accumulating work outside admission accounting. This ownership
rule makes no wall-clock bound for executor destruction, native thread return,
or callback completion. The outer activity remains retained through bounded
rendering, the final cancellation check, and public return.

## Required acceptance coverage

Focused and workspace evidence must cover strict schema and
canonical arguments; exact capability serde and policy/execution equality;
the injected and reference-host `start`, `read`, `signal`, `list`, `inspect`,
and `wait` schemas; exact custom signal authorization; authority-free read,
list, and exact-wait preparation; same-incarnation
output ownership, wrong-owner indistinguishability, hidden-before-release
registration, merged-stream marker exclusion, live and closed reads, prefix
truncation and cursor advance, closed-entry eviction, invalid UTF-8, worst-case
JSON escaping, output-flood draining, four-read/16-live/100-closed admission,
pending-read cancellation and permit recovery; empty, ordered, truncated, and
100-row list results without sensitive detail; immediate exit, bounded
running-state backoff and safety-ceiling outcomes; all supported and rejected
recorded states and exit-code ranges; four-active-list, 128-observation,
four-active-wait, serialized-result, and live-memory bounds; no PID probe,
process, foreground executor, supervisor initialization, or permission effects
for read-only actions; four-signal admission, wrong-owner and completed-process
rejection, identity-revalidated outside-group descendant delivery, root-group
delivery, incomplete-delivery failure, close-before-reap serialization, and
off-poll-thread traversal;
pending listing, observation, and timer cancellation, destructor-triggered
cancellation, outer-future drop, and exact-once list- and wait-slot recovery;
workspace-relative permission,
private absolute background cwd, and fixed environment identity;
exact and over-limit combined workspace/cwd preflight before authorization;
post-construction retained-root rename/replacement behavior; rejection of
interactive and unsupported control fields; zero-effect pre-cancellation; committed
success despite later cancellation; nonzero/redacted display identities; every
fixed supervisor-error mapping; foreground-capacity and executor bypass; and
denial with zero effects; retained-root cwd and symlink/replacement races;
shell quoting, newlines, fixed program/argv, null stdin, and exact environment;
separate streams, invalid UTF-8, pipe pressure, output and serialized caps;
exit codes, signals, timeout, authoritative output overflow and bounded
overshoot, cancellation-first linearized output/deadline closure, each order of
that close, validated noncontradictory outcome arbitration, and stable specific
wait/pipe/other cleanup errors after an output-limit claim on either side of the
deadline; spawn/wait failure; cancellation before and after spawn plus blocked
outer-cancellation callback saturation/recovery and the final prepublication
check; drop and direct-child reaping; process groups,
TERM-ignore/KILL, normal-exit latency, ambiguous cleanup, and leader-identity
retention; concurrency limits; one non-incrementing shared activity and notifier
across the outer call, owned request/executor, cancellation, deadline, built-in
and injected Waker registrations, callbacks, and native threads; retained-
request and retained-Waker saturation; concurrent multi-family notice
coalescing with at most one underlying callback, serialized replay after a poll-
observed later notice, and no lock held across arbitrary Waker clone/drop/wake;
executor-, deadline-, and cancellation-driven self-repoll through the supplied
notifier Waker without replacing the external target; close-time replay
cancellation and post-close delivery suppression on normal return, pending
outer-future drop, and unwind; target destruction outside the notifier lock;
retained supplied-Waker clone busy behavior and recovery after its drop; no-
Waker thread return; activity
retention through bounded rendering, final cancellation, and public return;
exact-once active-slot release; exact-tilde and tilde-prefixed cwd literals;
redaction; public-
construction and private-host unsupported behavior; engine event/output
persistence; and canonical reference-host catalog composition.
