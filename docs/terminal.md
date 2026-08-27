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
paths, `~`, `.` or `..` components, NUL, C0/C1 controls, and bidi-control
characters reject rather than normalize. Unicode is not normalized or
case-folded. Preparation performs no filesystem, environment, process, thread,
or network effect.

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
contain no NUL. Entries are sorted by raw platform spelling before length-
prefixed SHA-256 hashing, so insertion order cannot change permission identity.
The system executor clears its environment and installs exactly that snapshot.
The model cannot add, remove, or replace an entry.

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
executor is therefore Linux-only in this slice. Its construction on macOS,
FreeBSD, WASI, and other targets fails with the fixed unsupported category
before filesystem lookup, environment inspection, thread creation, or spawn.
The public contract remains portable, and a trusted injected `TerminalExecutor`
may implement the same ownership contract on Unix for deterministic tests or a
future separately reviewed helper.

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

The absolute deadline begins on first poll before capacity admission or cwd
validation. After admission, one tool-owned condition-variable guardian wakes
the outer future at the deadline independently of the executor. Therefore even
a permanently pending injected executor is synchronously dropped at expiry.
Failure to create that bounded guardian fails before executor construction or
process spawn. Cancellation is checked again when the guardian becomes ready.

The child starts in a new process group. One system worker owns it and two
bounded readers drain stdout and stderr concurrently to prevent pipe deadlock.
Across both streams, execution:

- retains at most 64 KiB of raw output using deterministic head-and-tail
  retention;
- continues draining and counting through 1 MiB of produced bytes; and
- terminates the process group with status `output_limit` on the first byte
  beyond 1 MiB.

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
creation, in the serialized final-spawn gate, after spawn failure, and while
waiting. Cancellation wins any same-poll race and returns the fixed cancelled
tool error without partial command output. The final spawn attempt and abort
transition share one state gate: abort recorded first guarantees zero child;
successful spawn recorded first is the command-effect commit point.

After commit, cancellation, timeout, output overflow, or future drop sends
`SIGTERM` to the owned process group, waits at most 250 milliseconds, sends
`SIGKILL` if necessary, reaps the direct child, closes pipes, joins readers and
the worker, and releases the permit. After the direct child exits normally, the
executor also terminates any remaining members of the original group before
returning. Once stop is requested, each nonblocking reader performs at most 64
additional 16 KiB reads, so a deliberately escaped descendant that retains and
continuously writes a pipe cannot make join unbounded.

No command resource, descriptor, child, original process-group member, reader,
or permit survives output publication or future drop. An inline Waker may
re-enter and consume the completed result on the worker or deadline-guardian
thread itself; that thread cannot join itself, so only its resource-free
notification callback tail may briefly outlive the consuming poll. Non-self
paths join the thread. A descendant that deliberately escapes the group with
`setsid`, and a process stuck in an uninterruptible kernel wait, remain outside
the claimed containment boundary.

## Acceptance evidence

Before delivery, focused and workspace evidence must cover strict schema and
canonical arguments; exact capability serde and policy/execution equality;
denial with zero effects; retained-root cwd and symlink/replacement races;
shell quoting, newlines, fixed program/argv, null stdin, and exact environment;
separate streams, invalid UTF-8, pipe pressure, output and serialized caps;
exit codes, signals, timeout, output overflow, spawn/wait failure; cancellation
before and after spawn; drop and reaping; process groups and TERM-ignore/KILL;
concurrency limits and permit release; redaction; unsupported targets; engine
event/output persistence; and the fifteen-tool alphabetical reference catalog.
The required exact Rust checks, release-mode focused tests, fresh release-binary
smokes, portability checks, three fresh adversarial product reviews, exact
feature workflows, fast-forward integration, exact main workflows, and clean
worktree removal must all be green for the same reviewed behavior.
