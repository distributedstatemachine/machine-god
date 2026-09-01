# Native background supervisor contract

The background supervisor is the bounded producer for the persisted history
read by the top-level [`background` command](background-cli.md). It is a
host-owned library capability. This slice adds no public CLI grammar and does
not turn a persisted PID into process-control authority.

## Ownership and request boundary

`machine-god-core` owns provider-neutral start ordering over explicitly
injected clock, store, process-spawner, and process-retainer traits. Core owns
no filesystem, environment, clock, thread, task, process, signal, or network
effect. `machine-god-native` owns the concrete Linux and macOS store and
process implementations. A host must retain the native supervisor for at least
as long as work may run.

One start request contains a nonempty command of at most 32 KiB and one
absolute canonical Unicode cwd of at most 4,096 bytes. Both reject NUL. The
fixed program is `/bin/sh` with arguments `[-c, command]`; the command is an
argument and is never interpolated into a machine-god wrapper script.
Standard input, output, and error are null after release. This slice captures
no output, detects no URL, accepts no interactive input, and creates no PTY.
Linux and macOS use the same private helper protocol. The helper consumes only
its bounded release frame and gives the user shell input from `/dev/null`.

The requested environment is never installed in the pre-release process. The
host starts its helper with an emptied, fixed bootstrap environment; loader
controls such as `LD_PRELOAD`, `LD_AUDIT`, and `DYLD_*` therefore cannot affect
helper loading, cwd retention, or readiness. The already-validated command and
environment remain inert bytes in a versioned, length-delimited frame retained
by the parent. That frame is written only by release after the running record
is durable. The helper independently revalidates every bound and applies the
requested environment only to the final post-release `/bin/sh` exec. Frame
bytes cannot be interpreted as readiness, arguments, or shell interpolation.

The public start future is inert until first poll. Dropping it before first
poll has no store, process, clock, allocation-ID, thread, or task effect.
Admission is fail-fast with no queue: the default is four active jobs and the
hard configurable maximum is sixteen. Saturation occurs before ID reservation
or process preparation. Polling performs no potentially blocking process
operation on the caller's async executor. The supervisor owns one fixed-size,
bounded blocking worker set, and offload admission fails promptly rather than
queuing without a bound. Cancellation or drop retains cleanup ownership until
the admitted blocking operation returns and the prepared or released process
has been closed through the applicable lifecycle path.

## Persist-before-release protocol

An admitted start performs one linear ownership protocol:

1. reserve a globally monotonic workspace ID and its exclusive record lease;
2. capture the injected start time;
3. prepare a child that cannot execute the requested command yet;
4. durably publish the complete schema-v1 `running` record;
5. release the prepared child; and
6. transfer the released process, record lease, and active slot to the
   host-owned retainer.

The command cannot execute before step 4 succeeds. Cancellation, persistence
failure, or drop before release closes the private gate, reaps the prepared
child, and never runs the requested command. Release may fail if the private
gate or helper has already failed; that path guarantees the command did not
run, reaps the child, and attempts to replace the already-published `running`
record with `dead` before returning a fixed process failure. A failed
replacement cannot mask that primary process failure. Likewise, cancellation
after initial publication attempts a `stopped` replacement but always returns
the fixed cancellation result. Retention after a successful release is an
infallible ownership transfer: dropping the caller's start future cannot
orphan a process or free its capacity slot.

The successful start result contains only the allocated ID and display-only
PID. Neither value authorizes signaling. The supervisor controls a process
only through the exact live child and process-group handles it already owns.

## Cross-platform retained-cwd launch

Linux and macOS both launch a fixed internal single-threaded exec helper from
the retained directory identity, not by reopening the original cwd spelling
after preparation. Linux starts it through the retained platform descriptor
path. On macOS the parent passes the retained directory as a standard
descriptor and the helper applies safe `fchdir` through `rustix`. In both
cases the helper first emits one fixed readiness byte, waits for the complete
private release frame, and then replaces itself with fixed `/bin/sh`. No
machine-god crate contains unsafe Rust.

The production adapter resolves the current host executable and supplies one
exact private helper argument. Native hosts must dispatch that exact singleton
argument to `run_background_process_helper` before ordinary argument parsing,
without first locking or replacing standard streams. The reference CLI does
so. The argument plus any additional value is ordinary invalid public input;
helper failure returns the fixed private exit status without reflected output.

Renaming the retained directory or replacing its old path with a symlink or a
different directory cannot redirect the child. Helper startup or protocol
failure is a pre-release spawn failure and runs no requested command. The
helper is an internal process primitive, not a documented command or a
detached daemon protocol.

The path-taking production constructor snapshots the canonical workspace's
device and inode before opening it and accepts the retained descriptor only
when its identity matches. A rename or replacement in that boundary therefore
fails closed instead of binding the display spelling to a different directory.
Descriptor-taking constructors require their caller to establish the same
association.

## Store and completion

The writer uses the exact `background-v1` hierarchy and strict record schema
defined by the [reader contract](background-cli.md#native-persisted-schema).
State-root and workspace components are retained and opened
descriptor-relatively without following symlinks. Private owner/mode and macOS
ACL policy match the reader.

IDs are nonzero `u64` values allocated under one private allocator lock and
counter per workspace. Allocator and newly reserved record locks are acquired
nonblocking: contention fails the operation promptly rather than blocking an
async poll or constructor. Allocation is monotonic across cooperating
processes; gaps after a failed start are allowed, IDs are never reused, and
overflow fails closed. Each live record retains its own exclusive lock.
Every successful allocator, record, and maintenance authority explicitly
unlocks on logical release before its descriptor closes, so a forked
pre-`exec` descriptor duplicate cannot extend that authority's lifetime.
Initial publication is no-clobber. Its bounded private temporary file is
synchronized and atomically renamed before the record directory is
synchronized. A failure before rename publishes nothing. If rename succeeds
but the directory synchronization fails, admission fails closed and may leave
exactly one complete valid `running` record at the reserved ID; it never leaves
partial record bytes or overwrites an existing record. Caller cleanup releases
the prepared process and record lease; the next successful bounded startup
reconciliation then replaces that unlocked record with `stale`. The ambiguous
publication is never blindly retried. Later complete records use the same
synchronized temporary-file, atomic-rename, and directory-synchronization
sequence.

The writer keeps at most 100 total record or admitted-unpublished slots per
workspace. On open it validates a lifecycle snapshot of at most 1,024 entries
and 8 MiB; before reservation it compacts to at most 99 occupied slots. Every
running or locked slot is preserved. Among removable terminal records, the
newest `(updated_at_ms, id)` values are retained and older records plus their
unowned per-ID locks are durably removed. Unowned unpublished lock orphans are
also reclaimed. Consequently an old exact ID may later return `not_found`;
active work and the monotonic allocator counter are never evicted or reused.
Corrupt preflight or insufficient removable capacity fails before deleting
valid history.

Normal exit records `exited` with code zero or `failed` with the nonzero exit
code. A signal termination is recorded as the conventional nonzero
`128 + signal` failed code. Supervisor shutdown, explicit stop, or dispatch
failure after release records `stopped` when owned termination and reap
succeed. `dead` is reserved for cleanup whose process outcome remains
ambiguous after the supervisor exhausts its owned cleanup protocol. If
completion publication fails, the last complete record remains readable; no
partial JSON is exposed.

Startup reconciliation takes record locks nonblocking and changes only
unlocked persisted `running` records to `stale`. Its complete lifecycle scan
uses the 1,024-entry and 8 MiB bounds independently of the reader's smaller
100-record presentation bound. It never probes a PID, sends a signal, adopts a
child, or claims that a locked record is alive. A PID may have been reused and
is only historical presentation data.

## Process lifecycle

Every released child is a process-group leader retained by one host-owned
worker. Waiting reaps the direct child and removes remaining original-group
descendants before releasing the record lease and capacity. Stop sends TERM to
the owned group, waits through a fixed grace period, sends KILL if still
present, and reaps. Before reaping the leader, a bounded platform group scan
proves that the unreaped leader is the sole remaining member of its original
group. Linux reads the required numeric process and group identities directly
from a bounded `/proc` traversal and does not depend on a GNU `ps` dialect.
macOS retains its fixed `/bin/ps` adapter. Each adapter accepts at most 64 KiB
of evidence and has a 250 ms timeout. Permission denial is never disappearance
evidence; a surviving credential-changed member therefore produces a fixed
cleanup failure. The numeric group identity is not consulted after it becomes
reusable.

This process-local adapter requires exclusive child-reaping authority for the
entire prepared/owned-handle lifetime: the host must leave `SIGCHLD` waitable
and must not run another `wait`/`waitpid` consumer that can reap these exact
children. Immediately before each helper spawn, the adapter creates and waits
for a fixed no-op probe. On systems where `SIGCHLD = SIG_IGN` or
`SA_NOCLDWAIT` removes wait authority, or when a competing reaper steals that
probe, preparation fails before the requested helper exists. Signal modes that
remain waitable on a supported operating system are compatible.

Every `waitid` errno is classified before public redaction. `ECHILD` means the
exclusive prerequisite was violated after admission and irreversibly loses
authority over the child identity. That path returns the fixed wait failure,
drops the stale handle, and performs no later numeric PID/PGID signal or group
query; it cannot accidentally target a recycled process group. Other
observation failures retain the still-waitable leader through best-effort
cleanup. The adapter never attempts a redundant direct-child signal after the
group KILL.

An active worker observes a new process after 2 ms and exponentially backs off
to a 32 ms maximum, keeping an idle job below 40 leader observations per
second. Cooperative cancellation registers a thread wakeup, so it interrupts
the parked observation instead of waiting for that timeout. Completion may be
observed within the bounded backoff interval; explicit stop still includes its
fixed TERM grace.

Dropping the native supervisor stops, joins, and reaps every process it still
owns. This is process-local supervision: jobs do not promise survival after
host exit, cross-process control, crash adoption, or control by a later
machine-god invocation.

All public errors and debug output use closed fixed categories and do not
reflect commands, paths, environment values, record contents, helper details,
PIDs, IDs, or operating-system diagnostics. Host syscalls and joins have no
unconditional wall-clock guarantee.

## Deferred surface

Future slices may connect this capability to terminal `action: "start"`, add
managed logs and read/wait operations, or design an authenticated durable
worker protocol. Top-level `background` remains inspection-only. Interactive
input, output streaming, URL detection, persisted-PID signaling, detached
survival, cross-process stop, crash adoption, setsid containment, and
fx-equivalence or performance claims remain out of scope.
