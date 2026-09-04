# Native background supervisor contract

The background supervisor is the bounded producer for the persisted history
read by the top-level [`background` command](background-cli.md). It is a
host-owned library capability. The reference host exposes it only through the
noninteractive `terminal` action `start`; it adds no public top-level CLI
grammar and does not turn a persisted PID into process-control authority.

## Ownership and request boundary

`machine-god-core` owns provider-neutral start ordering over explicitly
injected clock, store, process-spawner, and process-retainer traits. Core owns
no filesystem, environment, clock, thread, task, process, signal, or network
effect. `machine-god-native` owns the concrete Linux and macOS store and
process implementations. While a supervisor is live it accepts work; dropping
it closes admission and requests stop without abandoning already-acquired
ownership.

One start request contains a nonempty command of at most 32 KiB and one
absolute canonical Unicode cwd of at most 4,096 bytes. Both reject NUL. The
fixed program is `/bin/sh` with arguments `[-c, command]`; the command is an
argument and is never interpolated into a machine-god wrapper script.
Standard input is null after release. Only a start request carrying an output
owner selects capture in the private release frame: the helper then duplicates
one retained pipe onto the final shell's standard output and error, producing a
single merged byte stream whose relative ordering is only the order observed by
that pipe. The readiness byte is consumed before the pipe becomes captured
output. Without an output owner, the final shell retains the original null
standard-output and standard-error behavior, and no worker drains or counts its
output. The supervisor drains captured output continuously and retains a
bounded process-local prefix. It detects no URL, accepts no interactive input,
and creates no PTY. Linux and macOS use the same private helper protocol. The
helper consumes only its bounded release frame and gives the user shell input
from `/dev/null`.

The requested environment is never installed in the pre-release process. The
host starts its helper with an emptied, fixed bootstrap environment; loader
controls such as `LD_PRELOAD`, `LD_AUDIT`, and `DYLD_*` therefore cannot affect
helper loading, cwd retention, or readiness. The already-validated command and
environment remain inert bytes in a versioned, length-delimited frame retained
by the parent. The validated environment and its encoded frame segment are
immutable shared allocations; starts retain that same representation instead
of cloning or revalidating its entries. Release assembles the frame through one
fixed 16 KiB buffer in at most nineteen payload batches after the running
record is durable. Every physical write is capped at the platform's atomic
pipe-write size: 4 KiB on Linux and 512 bytes on macOS. Including thirty-two
retry or short-write allowances and the distinct commit, the nonblocking gate
therefore caps underlying attempts at 107 on Linux and 618 on macOS, and applies
cancellation-aware 4-to-32 ms backoff after retryable failures or any positive
short write. Its bounded payload remains inert until a distinct final commit
byte is written separately after the last cancellation and deadline checks;
closing the gate after any payload prefix, including the complete payload,
cannot emulate that commit.
The helper independently revalidates every bound and applies the requested
environment only to the final post-release `/bin/sh` exec. Frame bytes cannot
be interpreted as readiness, arguments, or shell interpolation.
Environment entry, per-key, per-value, and aggregate-byte bounds are validated
through borrowed bytes before the caller-owned vector is accepted. Valid
caller vectors and their entry buffers move into the supervisor without a
constructor clone. The production constructor performs no ambient environment
lookup and always supplies exactly `LANG=C`, `LC_ALL=C`, and
`PATH=/usr/bin:/bin`, in that order. Explicitly retained-root composition may
still inject any caller-owned environment that satisfies the same bounds.
Environment-key
uniqueness is validated in one pass through a borrowed-key ordered set; the
512-entry bound does not trigger a quadratic prefix scan. Every invalid path
returns the same fixed environment category.

The supervisor derives one redacted `ProcessEnvironment` identity from the
accepted environment by sorting raw entries and hashing length-prefixed key and
value bytes. Reference-host terminal preparation uses profile
`background_fixed` and that digest, so permission policy sees the environment
the helper will install without receiving any raw entry.

Reference-host composition retains the exact verified workspace and state-root
descriptors, canonical workspace identity, and fixed environment identity in a
lazy starter. Construction read-only validates that the retained state root is
still an owned private directory with the supported macOS ACL shape. It does
not prepare or reconcile `background-v1`, resolve the process helper, or create
supervisor workers. The first permitted `start` future that is actually polled
atomically reserves the full default ten-authority cohort before it starts one
process-wide-owned initialization worker. At most sixteen start futures may
wait for that shared initialization;
further callers fail immediately with `capacity`. All concurrent first callers
observe the same result, and success installs one supervisor reused by every
later start. Initialization is attempted exactly once per composed host. A
state or reconciliation failure is retained as fixed `persistence`; every
other construction failure is retained as fixed `process`. Neither category
contains native details, and a new host must be composed to retry. Cancelling
or dropping one waiting start removes only that bounded waiter: an initializer
already started continues so another caller cannot create a competing worker
cohort or reconciliation pass.

The public start future is inert until first poll. Dropping it before first
poll has no store, process, clock, allocation-ID, thread, or task effect.
Admission is fail-fast with no queue: the default is four active jobs and the
hard configurable maximum is sixteen. Saturation occurs before ID reservation
or process preparation. Polling performs no potentially blocking process
operation on the caller's async executor. In addition to its fixed retainer
workers and one retainer rescue worker, the supervisor owns one fixed-size
blocking-operation worker set; offload admission fails promptly rather than
queuing without a bound. Every worker handle is registered at creation with a
process-wide collector that retains at most 256 handles. One supervisor
atomically reserves `2 * max_active + 1` of those authorities before creating
any worker. Lazy reference-host initialization atomically reserves that whole
default nine-authority supervisor cohort together with one initializer
authority, so it peaks at ten authorities and retains nine after
initialization. At most 25 complete default lazy cohorts fit in the registry;
the remaining six authorities cannot admit a partial cohort. Aggregate
admission failure creates no worker and fixes the lazy result as `process`;
direct supervisor construction instead fails with the fixed worker category.
Each worker publishes one completion flag
while holding the collector's predicate mutex, then sends one condition
notification as it leaves. Shutdown publishes its predicate under that same
mutex. The collector's predicate check and condition wait are therefore one
atomic handoff with respect to every producer; a completion or shutdown wake
cannot be lost between them. The collector sleeps between registration or
completion events and never polls or periodically scans idle handles.
Finished handles return their authorities to the collector; unresolved handles
are never implicitly detached.
Cancellation or drop retains cleanup ownership until the admitted blocking
operation returns and the prepared or released process has been closed through
the applicable lifecycle path. Before enqueueing each cancellable start, the
blocking pool registers that start's private operation token with its assigned
worker. One per-worker admission transition serializes the `Run` enqueue with
shutdown's close and `Shutdown` enqueue. Shutdown first closes global
admission, then closes each worker transition and cancels its registered token.
If `Run` wins, that worker retains the job and result and shutdown cancels it;
if shutdown wins, registration rejects and cancels before enqueue. A worker can
therefore never dequeue `Shutdown` and then accept a successful `Run` send.
Workers unregister only after the submitted operation returns. Pool shutdown
never runs cleanup inline and never waits for user work; the registered worker
continues to own cleanup and result publication within the fixed worker and
collector bounds. Helper readiness observes the same private operation
cancellation token through a registered thread wakeup. Cancellation therefore
interrupts a stalled readiness wait promptly, closes the private gate, signals
and reaps the helper group, and releases blocking capacity without waiting for
the two-second readiness deadline. A preparation failure maps to the fixed
`cancelled` start result only when the native adapter reports its typed
cancellation after proving that cleanup completed. A readiness, protocol, or
spawn failure remains the fixed process result when cancellation merely races
that failure. The exclusive-child-
reaping probe uses the same registered cancellation wakeup and a fixed 500 ms
deadline; a stalled probe is killed and either reaped or transferred to bounded
quarantine ownership before preparation returns.
After readiness, release makes the gate nonblocking and transmits the frame
under both cooperative cancellation and a fixed 500 ms deadline. Cancellation,
timeout, or any uncommitted frame closes the gate and completes owned group
cleanup, so a ready helper that stops reading cannot retain blocking capacity.
Pre-commit cancellation has its distinct native category only after abort and
reap succeed. Proven cancellation remains `cancelled` only when cancellation
itself terminates the bounded write. A terminal frame or commit-write failure
is classified from the cause observed at that failing attempt and is not
relabelled if cancellation races after that observation. Every pre-commit
failure aborts and reaps before return; ambiguous cleanup remains a process
failure rather than being hidden by cancellation.

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
after initial publication explicitly aborts and reaps the prepared child. It
publishes `stopped` only when that fallible cleanup proves success and
publishes `dead` when cleanup fails or remains ambiguous. Only proven cleanup
preserves the fixed cancellation result; ambiguous cleanup returns the fixed
process result. After successful release, the owned process first performs its
bounded synchronous retain-time activation. Activation failure drops and
cleans that sole owner, best-effort replaces the record with `dead`, and
returns a fixed process failure; cancellation can no longer revoke release.
Successful activation makes retention an infallible ownership transfer:
dropping the caller's start future cannot orphan a process or free its capacity
slot. A retention permit reserved before shutdown still
dispatches to its reserved worker. The fixed rescue worker owns the otherwise
unreachable channel-failure path, so dispatch never performs a process wait or
completion publication on the polling caller.

The successful start result contains only the allocated ID and display-only
PID. Neither value authorizes signaling. For an owner-scoped terminal start,
the supervisor separately retains an exact native process controller keyed by
the allocated ID and the private session-incarnation owner. On Linux its scope
is the identity-pinned ancestry tree plus the original process group; on macOS
its scope is only the original process group. The controller
becomes visible through core's post-release retain-time activation hook before
the public handle is returned and never reconstructs authority from either
display number.

## Cross-platform retained-cwd launch

Linux and macOS both launch a fixed internal single-threaded exec helper from
the retained directory identity, not by reopening the original cwd spelling
after preparation. Linux starts it through the retained platform descriptor
path. On macOS the parent passes the retained directory as a standard
descriptor and the helper applies safe `fchdir` through `rustix`. In both
cases the helper first emits one fixed readiness byte, waits for the complete
private release frame, and then replaces itself with fixed `/bin/sh`. Unsafe
Rust is forbidden throughout the production workspace.

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
members before releasing the record lease and capacity. Stop sends TERM to
the owned group, waits through a fixed grace period, sends KILL if still
present, and reaps. Before reaping the leader, a bounded platform group scan
proves that the unreaped leader is the sole remaining member of its original
group. Before creating any Linux helper, the adapter opens and retains the
procfs root and binds it to the exact mount ID reported by both
`statx(STATX_MNT_ID)` and its descriptor-relative `self/mountinfo`. The same
bounded validation opens descriptor-relative `self/status`, requires that file
to have the retained mount ID, and requires exactly one `NSpid` value equal to
the host process's current PID. An absent, malformed, duplicated, mismatched,
or multi-level namespace projection fails closed, so numeric PIDs passed to
`pidfd_open` cannot be interpreted in an ancestor procfs PID namespace. Linux
therefore requires mount-ID reporting support and fails closed when it is
unavailable. Mount metadata is read incrementally without a 1 MiB reserve,
while still accepting at most 1 MiB, a 64 KiB status record, and 4,096 complete
mount entries. The adapter
requires exactly that retained mount ID to identify one procfs mounted at
`/proc` from the procfs root, accepts only absent or explicit `hidepid=0`, and
rejects malformed, truncated, duplicated, unknown-`hidepid`, restricted,
numeric PID-path-overmounted, or authority-file-overmounted configurations.
This makes the required stat identity fields visible even when a descendant
changes UID or GID; a host that cannot prove that cross-credential visibility
fails before the requested helper is spawned or released.

Linux retains one proc-root descriptor for that authority through both prepared
and released process ownership. Every group snapshot opens the proc root,
numeric PID directories, and each `stat` file descriptor-relatively; it does
not reopen ambient `/proc`.
An unrelated task that disappears while its retained `stat` descriptor is
being read is omitted when Linux reports `ENOENT`, `ESRCH`, or an empty
record; every other I/O failure and every nonempty malformed record still
fails cleanup closed.
It revalidates the retained filesystem type, exact mount identity, options,
and topology before and after every scan, so a remount, bind overmount, or
post-admission topology change fails cleanup before the leader is reaped. The
traversal remains bounded to 131,072 entries, 4 KiB per stat record, 32 MiB of
aggregate stat bytes, and one aggregate union of at most 32,768 retained
members across every phase. Proc-stat reads use the explicit bounded reader:
one scan shares the existing 250 ms snapshot deadline and at most 524,288 read
attempts, while one captured-member observation permits at most eight attempts
across its identity and optional zombie recheck. `EINTR` consumes those finite
attempt budgets. An indexed union makes duplicate and reversed
snapshots linear rather than quadratic. It reuses one directory buffer and
one stat-record buffer for a scan, and reuses one stat-record buffer throughout
the captured-member backoff rather than allocating per PID,
and does not depend on a GNU `ps` dialect. Lingering cleanup performs a
constant number of global process-table scans: the TERM and KILL observations,
one complete post-KILL capture, and one final completeness proof. Between the
last two scans it checks an identity-bearing union of every member observed
before TERM, before KILL, or in the post-KILL capture. Linux binds each
captured PID to its procfs start time and therefore distinguishes disappearance
from PID reuse; a captured process must cease to exist even if it leaves the
original group after partial signal delivery. When Linux reports a captured
member as a zombie, the adapter opens a pidfd, rechecks the retained start-time
identity, and consumes its status only when `waitid(P_PIDFD)` proves that the
member was adopted as this process's child. This also discharges descendants
when the host binary is a container's PID 1; a zombie owned by another reaper
remains unresolved and therefore fail-closed. Kernels without pidfd support
retain the original disappearance proof. macOS lacks that retained
start-time identity in this adapter and conservatively treats any still-
existing captured PID as a survivor. The wait advances permanently past each
vanished prefix member and checks one live witness per backoff interval, making
its work linear in captured members plus wait iterations. Reaching the fixed
disappearance deadline with any captured member unresolved fails cleanup even
when the final original-group snapshot contains only the leader. The unreaped
leader reserves the original group identity throughout. A raced or previously
uncaptured survivor makes the final proof fail closed. macOS retains its fixed
`/bin/ps` adapter with a 64 KiB output bound and one 250 ms deadline shared by
nonblocking pipe reads and child observation. It has no snapshot-reader thread
or reader join; timeout kills and then boundedly reaps or quarantines the exact
child. Permission denial is never disappearance evidence. The EPERM path
reuses its phase snapshot and adds no global scan; a surviving
credential-changed member therefore produces a fixed cleanup failure. The
numeric group identity is not consulted after it becomes reusable. Linux
formats repeated descriptor-relative PID components in a fixed ten-byte stack
buffer and allocates no decimal string per observation.

Direct-child reaping uses nonblocking probes under fixed deadlines. Before any
probe or helper spawn, the adapter reserves one of 64 process-wide reap
authorities and fails fast before spawning when none is available. A killed
child that remains unreaped at the deadline transfers, with its authority, to
one fixed-capacity background quarantine reaper; no cleanup caller blocks in an
unbounded child wait, and unresolved ownership cannot grow without bound.

This process-local adapter requires exclusive child-reaping authority for the
entire prepared/owned-handle lifetime: the host must leave `SIGCHLD` waitable
and must not run another `wait`/`waitpid` consumer that can reap these exact
children. Immediately before each helper spawn, the adapter creates and waits
for a fixed no-op probe. On systems where `SIGCHLD = SIG_IGN` or
`SA_NOCLDWAIT` removes wait authority, or when a competing reaper steals that
probe, preparation fails before the requested helper exists. Signal modes that
remain waitable on a supported operating system are compatible.

Every `waitid` and direct-child `try_wait` errno is classified before public
redaction. Bounded direct-child reaping retries `EINTR` only within the current
fixed observation deadline. `ECHILD` means the exclusive prerequisite was
violated after admission and irreversibly loses authority over the child
identity. That path returns the fixed wait failure, drops the stale handle, and
performs no later numeric PID/PGID signal or group query; it cannot accidentally
target a recycled process group. Any other direct-child observation failure
remains a wait failure and transfers both the child handle and its process-wide
reap authority to the bounded quarantine instead of dropping either. The
adapter never attempts a redundant direct-child signal after the group KILL.

An active worker observes a new process after 2 ms and exponentially backs off
to a 32 ms maximum, keeping an idle job below 40 leader observations per
second. Cooperative cancellation registers a thread wakeup, so it interrupts
the parked observation instead of waiting for that timeout. Completion may be
observed within the bounded backoff interval; explicit stop still includes its
fixed TERM grace.

Dropping an unpolled lazy reference-host starter closes its retained root
descriptors without creating a worker or namespace. If initialization has
started, its registered worker owns the initializer and exact descriptors to
completion; a successfully constructed supervisor is then either retained by
the shared starter or immediately dropped under the same rules when no owner
remains. Dropping the native supervisor closes both pools, cancels active
process waits, and returns without joining a worker or running process cleanup
on the caller.
The process-wide fixed-capacity collector already owns every worker handle;
retainer workers continue the bounded stop, reap, and terminal-publication
protocol, while blocking workers finish any acquired prepared-process cleanup.
The collector joins each handle after it reports finished and only then returns
its reserved authority. Thus Drop latency does not inherit a process grace
period or worker stall, while leases, processes, job results, and cleanup
tokens remain explicitly owned until completion. This remains process-local
supervision: cleanup continues only while the host process exists, and jobs do
not promise survival after host exit, cross-process control, crash adoption, or
control by a later machine-god invocation.

Captured output follows that same process-local lifetime. One registry shared
by the lazy starter and reader admits at most 16 live streams, retains at most
100 closed streams, and evicts only the oldest closed entry. Registration is
hidden before release, activation occurs only after the process release commit,
and failed or dropped preparation removes the hidden entry. The retainer worker
drains through fixed nonblocking chunks while waiting and continues discarding
after the 64 KiB retained prefix is full, so pipe pressure cannot turn the
retention bound into a child deadlock. It marks the entry closed on every owned
wait return or active-capture drop. After the leader exit or requested stop is
observed, final draining remains bounded; exhausting that drain budget closes
and truncates the capture pipe without relabeling the already-observed process
outcome as dead. The capture remains marked truncated even when all observed
bytes fit the retained prefix, and its observed-byte count is then a lower
bound. Actual pipe-read failure still fails the wait. Each read is
capped at 7 KiB and requires the exact session and session-incarnation owner
carried by the original start. The registry, bytes, and ownership disappear on
host exit and are deliberately independent of the durable background record.

Process control uses a separate registry capped at 16 live entries. A hidden
entry and identity-pinned native controller are installed during the same
prepared-process transaction as output capture. Release transfers both leases
to the owned process, and core activates control only at its synchronous
retain-time boundary. An activation failure drops the sole released-process
owner and completes its cleanup before the start returns a fixed process
failure. The lease remains held by the retained wait future until its native
controller has synchronously closed under the same lifecycle gate before the
leader is reaped. Unknown IDs, completed entries, and wrong session or
incarnation owners are deliberately indistinguishable.

On Linux, one explicit signal operation revalidates the retained leader,
captures a bounded ancestry tree, signals every identity-pinned descendant
deepest-first, and signals the original process group last. Descendants still
inside that group may receive the requested signal twice; individual delivery
deliberately closes an inside-group-to-escaped-group race before the final
group syscall. Linux traversal uses the retained procfs descriptor and mount
identity, caps inspected entries, metadata bytes, read attempts, and retained
members, and opens each descendant's proc directory and pidfd while its
verified parent is still pinned. Each proc directory is released after that
node's children and identity are fully scanned; prepared delivery retains only
pidfds, so a same-clock-tick numeric PID replacement cannot inherit signal
authority.
Every signal traversal shares one 250 ms
monotonic deadline, 524,288-attempt cap, 131,072-entry cap, and 32 MiB byte
budget across mount-authority and namespace-status records, task-children
records, and every
proc-stat identity read. Each
record remains independently capped at 64 KiB for task children or 4 KiB for
proc stat. Mountinfo uses the same explicit reader; `EINTR` consumes an attempt
rather than extending the operation, and terminal budget exhaustion prevents
later proc opens or pidfd probes;
deadline or budget exhaustion fails with the fixed process category. The
pre-release root-identity capture uses the same bounded reader with a fresh
operation budget, so constructing retained signal authority is bounded too. A
vanished Linux descendant does not fail the operation; incomplete inspection,
capacity exhaustion, identity ambiguity, a non-vanished descendant-delivery
failure, or original-group failure does. One descendant failure does not
suppress later descendant attempts or the original-group attempt, but the
combined operation still fails. After a descendant attempt begins, later
leader or group disappearance is reported as a process failure; not-found
remains limited to absence before any delivery.

Linux signal preparation also has a descriptor budget independent of the
process-count bound: at most 256 descriptors per operation and 1,024 across
all concurrent controllers and tools in the process. Each admission recomputes
the process-wide ceiling as the minimum of that hard cap, half the current soft
`RLIMIT_NOFILE`, and the soft limit minus 128 reserved descriptors. Transient
mount/status/stat/task/children descriptors, queued proc directories, and all
retained pidfds acquire before their syscall and release only after the owned
descriptor closes. Exhaustion therefore suppresses the next syscall, fails
with the fixed resource-limit category, and cannot leak capacity through
errors, unwinds, or abandoned prepared deliveries.

macOS deliberately performs no signal ancestry traversal or per-descendant
numeric PID delivery because it has no public incarnation-pinned process
handle equivalent to Linux pidfd. The retained, unreaped direct child pins its
original PID and process-group identity (`PGID == PID`) through the
close-before-reap lifecycle gate. An admitted signal therefore performs
exactly one atomic `killpg` syscall against that original group while holding
the gate, without a preceding `getpgid` or process-table query. Descendants
that leave the original group are outside the supported macOS signal scope.
Success acknowledges that single group delivery, not descendant coverage or
process exit.

The registry lock is released before target dispatch. On Linux, a separate
per-process reservation excludes concurrent signals while read-only
process-table preparation runs outside the lifecycle lock. Delivery reacquires
that lock and rechecks close plus the exact controller target before its first
effect. Close can therefore revoke a stalled preparation and reap without
waiting for a proc read. macOS has no preparation reservation and holds the
lifecycle lock across its sole group syscall. On both platforms an already
admitted delivery remains serialized against close and reap.

Native signal dispatch runs through the supervisor's existing bounded blocking
pool; only Linux performs process-table traversal. The terminal layer
separately caps active signal operations at four. Cancellation before blocking
admission has no process effect; admission commits the ordered mutation, so
later cancellation cannot abandon or relabel an already dispatched delivery.
The committed blocking closure retains
the terminal admission even if its caller future is dropped and releases it
only when native delivery actually returns. An unexpected unwind from the
registered native target is contained at that delivery boundary, returns the
same fixed process failure, and still publishes completion and releases the
worker and terminal admissions. Linux reuses one preallocated,
bounded proc-stat scratch buffer across snapshot and identity revalidation.
The lazy reference adapter shares the
same registry and blocking handle but does not initialize a supervisor merely
to reject an unknown signal target.

All public errors and debug output use closed fixed categories and do not
reflect commands, paths, environment values, record contents, helper details,
PIDs, IDs, or operating-system diagnostics. Worker-side host syscalls retain
their documented deadlines; supervisor Drop itself performs no worker join or
process syscall.

## Deferred surface

The bounded terminal `start`, same-incarnation process-local `read`, and
identity-checked process-local `signal` entrypoints and separate exact
persisted-record `inspect` and `wait` entrypoints are delivered. `read`,
`inspect`, and `wait` do not
initialize, call, or control this supervisor and make no PID or liveness claim;
`read` observes only the in-memory capture registry, while `wait` observes only
bounded atomic record replacements through its separately injected
persisted-history boundary. Future slices may add durable or restart-safe logs
or design an authenticated durable worker protocol. Top-level
`background` remains inspection-only. Interactive input, output streaming, URL
detection, persisted-PID signaling, detached survival, cross-process stop,
crash adoption, and broader fx-equivalence or performance claims remain out of
scope.
