# Native `terminal` command contract

The native tool executes one bounded foreground shell command, starts one
noninteractive background shell command after explicit process authorization,
or inspects or boundedly waits on one exact persisted background record without
process authority. It is registered by the reference host and has no top-level
CLI command.

## Boundary

The reference-host tool implements the `exec`, bounded `start`, bounded
persisted-record `inspect`, and bounded persisted-record `wait` subsets of
fx's `terminal` tool. `exec` captures bounded standard output and error and
waits for the direct child. `start` durably records and releases one
noninteractive command through the native background supervisor and returns
its display identity without waiting for command completion. `inspect` reads
the validated record for that display identity without claiming current
liveness. `wait` observes bounded atomic replacements of that exact record
until it contains a supported recorded exit or reaches the requested safety
ceiling. Standalone public terminal constructors remain exec-only; injecting
only a trusted background starter adds only `start`, `inspect` appears only
when a trusted inspector is explicitly injected, and `wait` appears only when
its separately bounded waiter is also injected.

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
The reference host also accepts the separate closed form
`{"action":"inspect","background_id":7}`, where `background_id` is a
nonzero JSON `u64`, and this exact wait form:

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
Run a foreground command, start a background command, inspect one persisted background record, or wait for its recorded exit
```

An exec-only construction retains its earlier foreground-only description and
schema. All forms deliberately exclude `read`, `screen`, `write`, `monitor`,
`list`, `resize`, `signal`, and `close`; PTYs; interactive
stdin; managed background output; artifacts; custom or login shells; user shell
profiles; retries; external working directories; and benchmark workloads. They
make no fx-equivalence or product-performance claim.

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

The stable serialized capability therefore contains the fixed program, exact
two arguments, authorized cwd, profile name, and digest. Successful preparation
returns those same canonical model arguments. Direct `execute` reparses and
revalidates all fields and rejects any canonical-argument, program, argument,
cwd, profile, or digest divergence before filesystem access, worker creation,
or process spawn. The existing engine presents terminal execution as critical
risk. Denial has zero terminal-owned effects. `inspect` and `wait` instead
prepare with no authority because they can read only through explicitly
injected persisted-history boundaries; neither requests process permission.

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
macOS, and unsafe Rust is forbidden by repository policy. The production system
executor is therefore Linux-only. Public `TerminalTool::open` and
`TerminalTool::open_with_limits` construction on macOS, FreeBSD, WASI, and other
non-Linux targets fails with the fixed unsupported category before filesystem
lookup, environment inspection, thread creation, or spawn.

Reference-host composition retains the workspace authority and advertises
`terminal` as defined by the canonical
[tool catalog](native-reference-host.md#tool-catalog). On macOS, `exec` returns
its fixed unsupported error after strict preparation and permission. `start`,
descriptor-confined `inspect`, and persisted-record `wait` are supported on
Linux and macOS. On other platforms the complete reference host is unavailable.
The exported exec
contract remains portable through a trusted injected `TerminalExecutor`, and a
trusted injected `TerminalBackgroundStarter` may implement the documented
background ownership contract. A trusted injected
`TerminalBackgroundInspector` may implement the exact persisted-record read
contract, while a separately injected waiter may implement the bounded
persisted-record wait contract.

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
the injected and reference-host `start`, `inspect`, and `wait` schemas;
authority-free exact wait preparation; immediate exit, bounded running-state
backoff and safety-ceiling outcomes; all supported and rejected recorded states
and exit-code ranges; 128-observation, four-active-wait, serialized-result, and
live-memory bounds; no listing, PID probe, process, foreground executor,
supervisor initialization, or permission effects; pending observation and
timer cancellation, destructor-triggered cancellation, outer-future drop, and
exact-once wait-slot recovery; workspace-relative permission,
private absolute background cwd, and fixed environment identity;
exact and over-limit combined workspace/cwd preflight before authorization;
post-construction retained-root rename/replacement behavior; rejection of
interactive and control fields; zero-effect pre-cancellation; committed
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
