# Native `terminal` foreground-exec contract

Status: **IN PROGRESS** as bounded Milestone 03 slice 34. The contract is
frozen from exact delivered base
`52b5885f275c9f6f4f16b378f71780c29f2ebab2` and pinned fx observation
`b1774fbf6c7602b503026f96f6e960e946c692ef`.

## Boundary

This slice implements only the foreground `exec` subset of fx's `terminal`
tool. One approved call starts one fixed shell in one workspace-relative
starting directory, captures bounded standard output and error separately, and
waits for the direct child to terminate. It is a native library tool registered
by the reference host; it adds no top-level CLI command.

The model-facing input is:

```json
{
  "action": "exec",
  "command": "cargo test --workspace",
  "cwd": ".",
  "profile": "clean"
}
```

`action` and `command` are required. `action` accepts only `"exec"`.
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
performs no filesystem, environment, process, thread, or network effect.

The exact tool description is:

```text
Run one foreground shell command from a workspace-relative directory
```

This slice deliberately excludes `start`, `read`, `screen`, `write`, `wait`,
`monitor`, `inspect`, `list`, `resize`, `signal`, and `close`; PTYs; interactive
stdin; background or durable sessions; streaming output; artifacts; custom or
login shells; user shell profiles; retries; external working directories; and
benchmark workloads. It makes no fx-equivalence or product-performance claim.

## Permission and exact execution agreement

Core extends `Capability::Process` so permission policy receives the complete
immutable execution identity:

```text
Capability::Process {
    program: "/bin/sh",
    arguments: ["-c", canonical_command],
    working_directory: canonical_cwd,
    environment: {
        profile: "construction_snapshot",
        sha256: lower_hex_digest,
    },
}
```

The environment digest identifies the exact bounded snapshot retained when the
tool is constructed; it never exposes raw keys or values. Construction accepts
at most 512 entries, 1,024 bytes per key, 16 KiB per value, and 256 KiB in
aggregate. Keys must be nonempty, contain neither `=` nor NUL, and values must
contain no NUL. Entry validity plus individual and aggregate sizes are checked
before sorting, so a rejectable snapshot cannot trigger sort work outside the
stated construction bounds. Valid entries are then sorted by raw platform
spelling before length-prefixed SHA-256 hashing, so insertion order cannot
change permission identity. The system executor clears its environment and
installs exactly that snapshot. The model cannot add, remove, or replace an
entry.

The stable serialized capability therefore contains the fixed program, exact
two arguments, canonical cwd, profile name, and digest. Successful preparation
returns those same canonical model arguments. Direct `execute` reparses and
revalidates all fields and rejects any canonical-argument, program, argument,
cwd, profile, or digest divergence before filesystem access, worker creation,
or process spawn. The existing engine presents terminal execution as critical
risk. Denial has zero terminal-owned effects.

The capability authorizes a process, not a sandbox. The retained workspace
descriptor constrains only the child's starting-directory identity. Once
approved, `/bin/sh` and the command can use absolute paths, inherited
credentials in the approved snapshot, child processes, and network or other
host authority available to the machine account. Hosts requiring isolation
must add an external sandbox; this slice claims none.

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
executor is therefore Linux-only in this slice. Public `TerminalTool::open` and
`TerminalTool::open_with_limits` construction on macOS, FreeBSD, WASI, and other
non-Linux targets fails with the fixed unsupported category before filesystem
lookup, environment inspection, thread creation, or spawn.

Reference-host composition is a deliberate private exception: it retains and
clones the workspace descriptor and advertises `terminal` in the same fifteen-
tool catalog on supported host-composition targets. On a non-Linux host, strict
preparation and permission still occur through the engine; an allowed execution
strictly reparses the canonical arguments and then returns the fixed unsupported
error before cwd lookup, worker or guardian creation, or spawn. The exported
contracts remain portable, and a trusted injected `TerminalExecutor` may
implement the same ownership contract for deterministic tests or a future
separately reviewed helper.

## Fixed execution protocol

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
final cancellation check, and public function return. On outer completion the
notifier closes, cancels any queued replay, and removes and destroys its external
target outside the notifier lock. A notice through an independently retained
supplied Waker after close does not deliver to the completed task, but that
retained clone still owns the activity and capacity until it is dropped. A
callback already in flight likewise owns the activity until it returns. The slot
is released exactly once, only after the last outer, request, executor,
notifier, Waker, callback, or native-thread activity owner returns or is dropped.

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

## Cancellation, timeout, and ownership

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
retains the activity through callback return. Completion closes delivery,
clears the external target outside the notifier lock, cancels replay, and
suppresses subsequent notices to the completed task without releasing activity
owned by an independently retained supplied-Waker clone.

Joining a native notification thread from the consuming path could self-join or
cross-thread deadlock, so its handle may be released only while the same
activity continues to own the slot. Notification tails, OS threads, and stacks
are consequently bounded by configured capacity; further calls fail fast as
busy rather than accumulating work outside admission accounting. This ownership
rule makes no wall-clock bound for executor destruction, native thread return,
or callback completion. The outer activity remains retained through bounded
rendering, the final cancellation check, and public return.

## Acceptance evidence

Before delivery, focused and workspace evidence must cover strict schema and
canonical arguments; exact capability serde and policy/execution equality;
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
cancellation and post-close delivery suppression; retained supplied-Waker clone
busy behavior and recovery after its drop; no-Waker thread return; activity
retention through bounded rendering, final cancellation, and public return;
exact-once active-slot release; exact-tilde and tilde-prefixed cwd literals;
redaction; public-
construction and private-host unsupported behavior; engine event/output
persistence; and the fifteen-tool alphabetical reference catalog.
The required exact Rust checks, release-mode focused tests, fresh release-binary
smokes, portability checks, three fresh adversarial product reviews, exact
feature workflows, fast-forward integration, exact main workflows, and clean
worktree removal must all be green for the same reviewed behavior.
