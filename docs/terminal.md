# Native `terminal` command contract

The CLI and explicit helper-bearing reference-host constructors register the
complete twelve-action terminal host described below. Original embedding
constructors without helper options retain the legacy bounded foreground and
noninteractive background adapter, whose narrower contract is documented later
in this file. Both adapters require explicit native authority and have no
top-level CLI command.

## Complete action contract and host

Core defines normalized, effect-free requests and responses for all twelve
actions: `exec`, `start`, `read`, `screen`, `write`, `wait`, `monitor`, `inspect`,
`list`, `resize`, `signal`, and `close`. Validation binds responses to their
request action and session, checks mutation receipts, bounded pages and cursor
ranges, and preserves foreground stdout/stderr bytes, totals, status and duration.
The public JSON decoder maps flat tool arguments to these contracts; it does
not manufacture owner, actor or writer authority. Its cwd extractor validates
the complete request before a host resolves and authorizes a directory.
For `exec`, it preserves both explicit `user` and `clean` profiles and leaves
an omitted or null profile unset for shell resolution; the pinned shell
resolver defaults omission to `user`. The legacy adapter's clean-only policy
does not constrain this complete, effect-free decoder.
The caller must reject duplicate raw JSON fields before constructing a
`serde_json::Value`. Complete-host constructors register this contract; the
legacy constructors retain the older adapter described below.

`TerminalActionTool` exposes the full twelve-action schema with an explicitly
injected `TerminalActionExecutor`. Preparation is effect-free and binds the
normalized invocation, call identity, workspace/default cwd, and captured
environment/shell-selection fingerprints into the capability. Repeated monitor
probes retain their network, filesystem and process authority classes; lease
revocation requires close authority. The original cwd stays separate from the
private command draft until the executor resolves it on its worker, preserving
native symlink/parent semantics without prematurely concatenating two bounded
paths. Only that resolution releases an executable core request. Complete typed
results are validated against the invocation and an encoder-derived ceiling
before conversion to JSON; cancellation after commitment preserves receipts.
Foreground `exec` marks a nonzero exit, signal, timeout or output-limit stop as
a tool error while preserving its complete typed result and archived output.
Successful observation actions remain successful even when the observed session
has a nonzero exit; they do not reclassify that session's past command as a new
tool failure.
The optional `TerminalActionResultPublisher` takes ownership of complete JSON
on `execute_for_turn` and returns its already-durable reference through the core
complete-output extension. It is not invoked by an unpolled future or direct
`execute`, and publication after an executed action is not cancelled in place
of its receipt. The native publisher owns storage/worker effects; the adapter
does not infer durability. Rejected and unpolled argument values use iterative
cleanup, including direct API calls that have not crossed core validation.
This injectable adapter does not by itself compose the production runtime,
provider admission or reference-host registration. The native archive publisher
and `read_tool_result` paging binding are described in
[the result-reader contract](read-tool-result.md#explicit-native-archives).

`TerminalActionInputPublisher` is a separate pre-execution publication boundary.
It may archive complete arguments but may not execute them or grant permission.
The native archive implementation preserves the original arguments losslessly
behind a small historical call reference, honouring cancellation before any
action. `with_input_publisher` explicitly injects this authority; it does not
raise the host's independent per-turn admission budget. Input and result
archives share the bounded owned-worker and paging contract linked above.
When an input publisher is installed, the adapter explicitly advertises its
complete raw/prepared byte and node limits to core. Ordinary tool limits remain
unchanged. Core commits only the small historical call representation, then
prepares and authorizes the original full input before execution; historical
references are never hydrated into executable arguments.

Numeric-string coercion uses deterministic IEEE binary128 rounding, matching
the pinned typed decoder before integer range validation, including halfway,
underflow, overflow and signed-zero behavior. It uses the pinned Rust
`rustc_apfloat` dependency, not a Zig runtime. Numeric spellings remain bounded
to 64 KiB independently of the complete action envelope. The internal decoder
admits 64 KiB commands, all 32 maximum custom-probe definitions and textual byte
arrays. Its schema-derived JSON ceiling is 16,378,880 bytes, including one
stringified-composite escaping layer; separate node and depth bounds apply
during composite decoding, before recursive normalization.

## Conversation handoff and workspace reset

The complete host exposes an inert, non-host-owning
`NativeTerminalLifecycleRequester`. Native conversation owners may hand off
terminal access between exact session IDs and incarnations without replacing
the terminal host. This component does not add transition support to the
legacy foreground/background adapter or itself implement interactive commands.

Handoff preserves each original backend, storage owner, journal namespace and
acknowledgement history. A bounded host-local route gives the destination access
to the exact transferred resources, including their later terminated history;
it does not expose the original owner's whole disk catalog. Repeated A-to-B-to-C
handoffs replace one current principal, without route chains. A conflicting
public terminal ID rejects the handoff before access changes. List `task_id`
filters use current access ownership; persisted historical owners remain intact.
Routes and principal registrations retain the existing profile bounds of 1,024
sessions and 256 owners. Capacity exhaustion fails explicitly.

Old access generations are revoked across ordinary actions, monitor preparation
and staged-start publication. Pending attention waits are cancelled and the old
writer claim is durably settled before the route commit. Already admitted input
bytes and receipts remain transport-owned; handoff does not permanently quiesce
input. New access generations have distinct writer identities so a late old wait
completion cannot clear a newer lease. Old external probe grants are retired;
permissions, sandbox authority and active probe evidence are not transferred.
Explicit `activate_session` permits a later resume of a retired principal with a
fresh generation, but cannot revive old operations or original-owner access to
transferred or forgotten resources. Activation of an active principal is a no-op.

`reset_current_workspace` stops only exact resources currently routed to the
principal under this host's retained workspace authority. It neither looks up
arbitrary persisted PIDs nor shuts down other principals or the shared host.
Its bounded receipt reports stopped-and-forgotten, already-terminated-and-forgotten,
or retained-indeterminate outcomes separately. Cleanup, publication or outstanding
receipt/residency uncertainty prevents forgetting. Validated terminated disk
history with an observed exit/signal may lose its transient route; merely closing
recovered history is not termination evidence. Unknown historical process state remains
indeterminate. Forgetting never deletes or relabels journal files. Reset retires
source admissions; retained resources can then be handed to the fresh conversation.

Unpolled operations have no effects. Cancellation before owner execution leaves
routes unchanged; handoff preparation failure also leaves source/destination
routes unchanged, although already cancelled waits or attempted durable attention
cleanup are not rolled back. Reset completes its bounded pass once started,
including after caller cancellation. After a route commit or reset begins,
dropping the future does not establish rollback. The enclosing native owner must
drive it to its receipt and queue newer transitions behind that settled result.
An unavailable post-enqueue receipt is publication uncertainty, not permission to
repeat effects blindly. Conversation observation and persistence registrations
are retired by the enclosing conversation owner; terminal journaling continues.

## Native shell resolution

`TerminalShell` resolves explicitly injected account data or performs an
explicit current-user account-database lookup on Linux/macOS. Hosts must run
the latter on their bounded blocking executor; it never trusts the `SHELL`
environment variable. Resolution does not execute a command or create a PTY.
User and clean profiles select the pinned bash/zsh startup flags. Unsupported
account login shells fall back to `/bin/bash` on Linux or `/bin/zsh` on macOS;
unsupported explicitly requested shells fail. Relative and oversized paths,
absent account data, and conflicting explicit shell/profile selectors fail
with fixed errors. Interactive argv differs from captured argv: clean capture
removes `-i`, user bash capture removes `-i` and enables `expand_aliases`, and
user zsh capture retains `-i`. The command remains one exact `-c` argv item.
The resolved program and arguments are inputs to permission identity, never
permission grants. This native API is available for terminal-runtime
composition; the legacy noninteractive adapter below retains its stated
fixed-shell behavior independently of the complete interactive host.

### Captured host launch authority

The complete host captures one explicitly supplied workspace descriptor and
canonical spelling, canonical default cwd, validated environment snapshot,
private bootstrap-artifact descriptor/path, and CLI/tmux executable selections.
Constructing this configuration performs no filesystem or account lookup and
does not discover `current_exe` or ambient environment variables. Capture runs
on an owned blocking worker before registering the prepared host identity.
Account-shell data is either supplied explicitly or selected once through
`TerminalShell`'s native current-user lookup. Environment fingerprints include
every exact key/value byte in launch order, including non-UTF-8 bytes; shell
fingerprints bind the captured selection, platform policy, and helper programs
and arguments. `SHELL` never chooses the executable.

Exec/start cwd resolution consumes `TerminalActionInvocation::resolve_cwd`'s
original requested spelling on the owned worker. Native canonicalization runs
before any lexical simplification, preserving symlink/parent semantics. The
canonical result must remain inside the selected retained workspace root. With
workspace contexts attached, the outer tool/executor future captures the exact
live session-incarnation/turn registration before first poll. Both ordinary and
turn execution, including the governed wrapper, retain that acceptance stamp;
late registration, reused IDs or a different bound call context cannot repair
an unavailable or retired scope. This pure capture performs no filesystem work.
The worker routes the native canonical result through that captured snapshot's
primary and active additional roots, excluding state and without consulting the
current manager or falling back to primary. Relative paths retain the captured
default-cwd base; native symlink/parent ordering is not lexically rewritten.
The worker also compares resolved-directory ancestry against the retained state
object, so moving state under an allowed root does not authorize it as a cwd.
An original-path descriptor must match a no-follow descriptor-relative walk from
the selected root by device/inode, and that root descriptor must still match its
captured canonical spelling. The resulting exact cwd descriptor stays on the owned
effect worker through launch; it is never reopened or returned to an async
polling thread as path-based authority. Non-command actions acquire no cwd.
Permission preparation captures the same acceptance-time scope and passes it to
the actual host resolver, preserving canonical capability and shell identity
checks without a second registration lookup. Legacy explicit host constructors
without workspace contexts retain their primary-only behavior.
A supplied `list.workspace_root` uses the separate effect-free
`TerminalActionInvocation::resolve_workspace_filter` callback, invoked on the
bounded, scoped effect worker against the captured default cwd, preserving native
symlink/parent order. ASCII edge whitespace is trimmed and `~` uses captured
`HOME`; missing or invalid paths fail. An explicit existing foreign root is a
no-match predicate, never command-directory or foreign-owner authority. The
retained workspace identity is checked around resolution. Unfiltered lists
perform no filter filesystem resolution and use ordinary owner-loop admission.
Bootstrap artifacts come only from the supplied private directory, duplicated
after start admission, not from an arbitrary command cwd. Native launch retains
its own final descriptor/path checks and rejects replacement rather than
silently rebinding authority. One supplied deadline and cancellation token are
checked between synchronous boundaries; an individual filesystem or account
lookup syscall is not preempted by those checks.

Workspace selection is independent of permission options. When no permission
policy is attached, scoped exec/start still carry a `none` launch snapshot with
the exact live workspace proof through final native release. A committed process
is not revoked when its originating turn finishes. Custom-probe cwd and path
monitor parent resolution use the same selected scope; successful monitor-grant
installation checks the live registration and transfers retained immutable
directory authority to the monitor's independent owner/generation lifetime.
Stale preparations cannot install a new grant. Existing grants retain their
ordinary deadlines and revocation without requiring the source turn to stay open.

### Captured foreground execution

`TerminalCapturedExec` provides the complete foreground service on Linux and
macOS. The caller supplies its exact authorized shell, captured environment and
cwd descriptor; the worker executes the selected bash/zsh user/clean argv and
returns separate binary stdout/stderr, observed totals, duration and exact
exit/signal/timeout/output-limit status. Calls are inert until polled, admit at
most the configured 1–16 executions, and retain owned cleanup after cancellation
or caller drop. The private CLI helper accepts only its exact single flag before
normal configuration. Its bounded, close-on-exec error channel distinguishes a
missing/non-executable shell from a command that legitimately exits with 125;
launch failure is never guessed from an exit status or stderr content.
The additive `with_process_inventory_helper` builder supplies an explicit
one-shot inventory executable and bounded arguments for macOS session cleanup;
`with_process_inventory_service` explicitly selects the reusable framed mode.
Existing constructors remain available without either capability. The complete
reference host supplies service mode from its selected CLI helper path. Service
preparation consumes the same existing execution deadline, before user-process
startup; it observes execution cancellation, host shutdown and dropped-response
stop signals during preparation and readiness waits. It does not occur in a
constructor or unpolled future.
The configured 1 ms–600 s timeout begins at first poll and includes worker
admission, authorized directory/environment preparation, private helper startup
and exec confirmation. Preparation and descriptor consumption run on the same
collected worker. Its host-stop token is checked by native work even if the
response future is not polled again; completed, unconsumed responses retain their
bounded admission slot. The host passes the required absolute
monotonic `MACHINE_GOD_CAPTURED_DEADLINE` stamp; the helper does not start a new
timer or impose a hidden two-second cutoff. PTY standalone helper defaults and
its independent maximum startup interval are unchanged.
Explicit helper-bearing reference-host constructors compose this executor with
staged PTY/tmux startup, resident/history dispatch and authorized monitor effects.
Only real Engine/Session handles retain the runtime resource; operation futures
use non-owning requesters. Last-resource drop cancels foreground/probe work before
closing the owner registry. Start/monitor preparation has a separate bounded
owned-worker admission, and committed failures retain diagnostic receipt metadata.
The CLI supplies its own helper executable, captured account shell/environment
and available PATH-selected tmux on its existing constructor worker. Embedders
that use the original constructors without helper options retain the legacy tool.
The complete host enrolls its runtime, captured commands, startup preparation,
monitor effects, archive publication/paging and outer action workers in one
explicit completion scope.
Last-resource drop closes new worker admission and requests cleanup without
blocking. `NativeReferenceHost::terminal_shutdown_completion` returns a
non-owning observation handle: a blocking host thread can retain it, drop every
real Engine/Session handle, then wait for joined workers and transferred child
cleanup. Completed unconsumed responses do not hold completion tickets, and
unrelated worker scopes are excluded. A dormant host settles without starting
its lazy registry. The CLI uses this boundary before process exit, including
when turn execution returns an error or unwinds.
Shared cleanup workers temporarily restore the originating completion scope
while servicing its obligations, including nested cleanup children. They restore
the previous attribution on return or unwind; unrelated hosts cannot inherit it.

## Explicit macOS OS sandbox launch

`NativeSandboxLaunch` is immutable native launch authority, separate from
execution permission and mutable host preferences. A taken job captures its
configured `NativeSandboxMode` and permission mode once: `ask` and `auto` use
the configured backend; `yolo` uses effective `none` without overwriting the
configured value. Later preference or workspace-selection changes cannot mutate
that retained snapshot. Capturing `os` on Linux is explicitly unsupported;
an unavailable macOS backend never falls back to an unsandboxed command.

The caller supplies `NativeSandboxRoot` values containing open directory files
and their exact canonical UTF-8 paths, plus an explicitly opened
`/usr/bin/sandbox-exec` file. Root construction checks bounded syntax without
I/O; snapshot capture and revalidation run on the caller's owned blocking worker
under its supplied deadline and cancellation token. There are at most seventeen
roots (primary plus sixteen additional directories), each at most 4,096 bytes.
Missing, replaced, unlinked, non-directory, noncanonical or mismatched roots
fail closed. The launcher must match the retained regular, root-owned,
non-group/world-writable executable at that fixed system path. No ambient CWD,
HOME, PATH executable, cache directory or temporary profile file grants authority.

On macOS the actual `sandbox-exec` Seatbelt profile denies by default, allows
file reads, and allows writes to the captured roots plus the pinned upstream
exceptions `/tmp`, `/private/tmp` and `/dev`. It also permits process execution
and fork, outbound networking, signals, sysctl reads, Mach lookup and IOKit open.
An explicitly supplied flag additionally permits localhost binding/inbound
traffic. This is not workspace-only isolation, read confidentiality, network
isolation, resource accounting or protection from writes to those exceptions.
There is no automatic permissive HOME/cache expansion or retry without isolation.

`TerminalShell::with_sandbox` attaches the snapshot without changing the selected
shell or startup profile. Captured foreground execution and native PTY/tmux
startup wrap that exact shell argv as
`/usr/bin/sandbox-exec -p <profile> <shell> <arguments...>` inside their existing
owned helper protocol. The command stays one exact argument; paths are escaped
as SBPL string data, never interpolated into shell source. Private protocol,
inventory and persistence owners remain outside the sandbox. The background
process request's separate `with_sandbox` builder wraps its gated helper at
spawn; the eventual fixed `/bin/sh` and descendants inherit that OS policy.
Without explicit injection, all existing constructors retain their behavior.
Neither builder alone performs discovery, spawns a process or grants permission;
the embedding host must route the taken-job snapshot into every launch family.

The opt-in terminal host composition accepts `NativeTerminalPermissionPolicy`.
Its inert constructor retains only explicitly supplied roots and launcher;
`bind_controller` installs one weak controller link and rejects rebinding even
after that controller expires. The weak link avoids an ownership cycle through
the controller's prepared tools. On the existing effect worker, the real tool
context resolves the exact live session incarnation and turn's taken policy.
Unbound, expired, closed or uncertain routes fail closed, including effective
`none`; a missing OS executable never selects an unsandboxed fallback.

`NativeTerminalPermissionPolicy::with_workspace_contexts` additionally binds
capture to the exact live workspace registration for that same session
incarnation and turn. The builder is inert. On the already-owned launch worker,
effective `os` derives primary and active additional roots from the captured
scope, never the current workspace manager or constructor root list. Routing
deduplicates canonical root identities, including an additional directory
already covered by primary, before applying the original seventeen-root bound.
Inactive and suppressed entries do not become sandbox roots. Descriptor copies,
root/launcher validation and profile construction remain under the original
deadline and cancellation token.

Effective `none` and `yolo` still require the exact live workspace registration,
but do not acquire OS roots or a launcher or require UTF-8 profile representation
of otherwise valid workspace identities. Scoped snapshots retain a live-turn
proof and check it before final native release even in those modes. Missing or
retired registrations never fall back to constructor authority. Once an
ordinary process has successfully committed, its lifetime and cleanup remain
with the existing native process owner; finishing the source turn does not
invalidate an already-running process through this launch-only proof.

Foreground execution attaches that snapshot to its selected shell. Staged PTY
and tmux starts retain the same snapshot through release, and initial or newly
added/updated custom monitor commands retain it in their authorized shell
context. Later mode/sandbox changes and reset cannot widen those retained jobs.
Pausing/resuming a monitor retains its original snapshot; an update prepares
new authority under the updating turn. Scheduled probes need not keep their
source conversation turn alive. Existing probe grants, deadlines and cleanup
ownership remain authoritative, and the snapshot itself is not an execution
grant. This composition does not enable optional localhost listening. Without
the explicit policy option, the existing host constructor remains unchanged.

For scoped custom monitors, successful grant construction checks the exact
source workspace registration and transfers the immutable captured sandbox
roots/policy to that independent owner/session/monitor-generation grant. A stale
registration cannot create a new grant. This pure transfer occurs only on the
existing grant-installation path, not on ordinary process launch or monitor
description preparation. Later probe launches still revalidate retained OS
roots and launcher, but use the monitor grant's revocation and deadlines rather
than requiring the original conversation turn to stay open. Removing a root in
a later turn does not rewrite a previously authorized monitor's captured policy.

The profile ceiling is derived from seventeen worst-case escaped paths plus
fixed policy syntax. The private helper frame adds only that bounded profile,
the inner program and three wrapper arguments. Ordinary command, environment,
shell-argument and startup limits remain unchanged; excess input rejects without
truncating roots or falling back. The profile is transferred as owned data and
never published to disk. Protected system execution can apply the operating
system's ordinary environment restrictions; the host does not synthesize a
replacement environment.

Retained roots and launcher are checked during preparation and again before
the existing final release/commit. Cancellation, caller drop and failure retain
the existing exact child, process-group/session, cleanup and worker-completion
obligations; no detached sandbox worker or second command execution is created.
The original launch, startup, release and cleanup deadlines are not reset.
Path validation and process launch are not one atomic system call. Seatbelt
enforces pathname-based rules, not permanent inode ownership: a concurrent
rename/replacement after the last check or during an already-running command
is outside an inode-stability guarantee. Symlinks do not authorize writes to
resolved targets outside the allowed paths. Failed setup never authorizes an
unsandboxed fallback; ordinary command failure remains an observed result,
not permission to replay a possibly executed effect.

## Native screen projection

`TerminalScreenEngine` projects bounded raw chunks into structured styled
screens, cursor/modes and data-only hyperlink bytes. It performs no I/O.
Live feeds return bounded protocol replies for exactly-once dispatch by the
runtime; replay and restored history never replay past replies. Checkpoints
retain parser state, including split Unicode and escape sequences. The journal
owner must verify checkpoint integrity and feed only contiguous later output;
an explicit raw gap invalidates the projection rather than inventing a screen.
Oversized feeds are rejected without mutation. Resizing this projection alone
does not resize a process: the runtime must also resize the actual PTY.
Each projected cell's complete UTF-8 text, including its base scalar and
combining suffix, fits the screen contract's 64-byte limit. Excess suffix
scalars do not expand that projection; raw journal bytes remain unchanged.
Checkpoint restoration rejects cells that exceed the same complete-text limit.

The Rust checkpoint encoder exposes a dimension-dependent reservation bound:
`9,978,007 + 44 × cells + columns` bytes, excluding the nine-byte history
envelope. It covers active and saved screens, retained suffix/hyperlink pools
and their length prefixes, tabs and all bounded parser/control buffers. Even
maximum valid geometry stays below the 32 MiB encoder cap; the bound is not a
clamped estimate and does not promise to encode corrupt or unavailable state.

## Native PTY lifecycle component

The native PTY transport prepares a retained-descriptor helper and commits the
shell only after ownership is installed. Its separate protocol channel cannot
consume terminal input bytes. Reads/writes are bounded and nonblocking; resize
changes the actual PTY. Close quiesces input, drains bounded output, terminates
with an 800 ms grace period or forces termination, and verifies cleanup before
reaping the shell. Linux retains process identities through pidfds. On macOS,
foreground signaling uses the retained PTY master, while background job-control
cleanup uses separately verified session-member incarnations, not numeric-PID
signals. See [ADR 0003](decisions/0003-macos-terminal-foreground-signal.md).
Explicitly helper-equipped macOS terminal hosts collect session PID hints through
the bounded read-only private helper in
[ADR 0004](decisions/0004-macos-process-inventory-helper.md), avoiding `ps`
task/thread inspection. Legacy constructors without the capability retain their
existing inventory path; a configured helper failure never falls back to `ps`.
The complete host prepares a reusable helper within the original terminal
startup budget, before starting the user shell/server. Routine queries avoid
new process creation and require a complete sequence-bound success frame under
the original query deadline. The helper remains separately killable; malformed,
partial, stale or expired replies poison the channel and preserve owned cleanup.
Native owners release their service leases at cleanup, while configuration and
retained history cannot keep an idle helper alive. Legacy one-shot registrations
still require EOF and positive reaping for each result.
macOS session inventory rejects unrelated session IDs before querying process
incarnations. A candidate still needs matching identity captures around a
second session-membership check; the preliminary check grants no authority.
Collection and identity scanning share a 250 ms budget, and previously captured
members remain tracked even after leaving the original session.
PTY signal flushing and incomplete drains are retained as output gaps, even if
a later read observes EOF. These native components do not by themselves expose
the full interactive action contract through the reference-host tool.

## Journal and monitor components

The descriptor-bound journal uses checksummed metadata, segmented raw bytes,
opaque screen checkpoints and bounded retained events. Reopen verifies the
committed prefix; only proven uncommitted suffixes/orphans are reconciled.
Retention reports cursor/checkpoint gaps. Mutation failure poisons the writer
until reopen, and one exclusive writer lease prevents concurrent publication.
Checkpoint descriptor reads use held committed metadata without allocating or
rehashing payloads. They do not certify payload integrity or an engine schema;
the history projection supplies those checks separately.
Session facts and monitor state have a separate protected, checksummed blob;
output, event and screen-checkpoint eviction cannot silently remove it. Its
33 MiB bound includes up to 32 MiB of monitor state and 512 KiB of framed facts.
The per-session output budget (64 MiB by default) counts only raw output and
screen checkpoints, matching the pinned fx accounting boundary. State is
bounded independently at 33 MiB; the event ring retains at most 256 records of
at most 4 KiB each. Publishing state or events cannot displace raw output or
screen checkpoints, and output pressure cannot evict state or events. Events
leave the ring only through its count limit or explicit acknowledgement.
Committed total-payload accounting still includes all four artifact kinds;
physical accounting additionally includes temporary/orphan files and metadata.
State-less journal records retain their existing checksum representation.

The journal exposes a non-mutating physical inventory through its retained
writer: actual raw/checkpoint, protected state/event, and metadata bytes remain
separate, including uncommitted suffixes, recognized orphan generations and
temporary metadata. Poisoned writers may be inspected, but unknown, oversized,
missing or unsafe artifacts and a replaced lock fail accounting. This inventory
is not screen validation or a profile-wide quota enforcement claim.

A separate stat-only scan counts a session directory without taking its writer
lease. It uses global artifact-size bounds and includes recognized partial
creation state, rather than trusting or reconciling a manifest. An empty or
partially initialized directory can be counted without being declared a valid
journal. The caller holds the exclusive profile transaction throughout.

The profile store retains `terminal-v1` and a permanent private `profile-lock`.
Each short nonblocking transaction acquires a fresh lock-file description, so
independent handles and processes cannot overlap admission. Transaction drop
explicitly unlocks its open file description before closing it, so a forked
child's temporary inherited descriptor cannot prolong the finished transaction.
Closing that older inherited descriptor cannot release a newer transaction's
independent lock. Inventory traverses all owner namespaces, including busy
writers and nonresident histories, with
limits of 256 owners, 256 sessions per owner and 1,024 sessions overall. Exact
spelling, private descriptors and before/after topology checks reject replaced
or unexplained entries. Recognized incomplete namespaces are counted without
granting recovery or process authority.

Catalog preparation and session-directory creation run through the held profile
transaction through an exclusive mutable borrow, preventing concurrent callers
from sharing a count preflight. They bind the catalog to the exact retained
profile descriptors and check global counts before creating a genuinely new
owner or session.
Existing or partially prepared owners can finish preparation at the owner cap;
duplicate session creation remains a conflict. These directory operations do
not reacquire the profile lock or initialize journal metadata.
Initial journal metadata is separately admitted in that count-admitted
directory before creating journal files. Failed initialization remains charged
and uses the journal's existing partial-state rules on retry.

Profile admission separates the default 512 MiB output budget from protected
state/events and metadata. Their ceilings derive from the journal bounds and
the total session-count limit; metadata allows two 128 KiB files per session.
One reservation admits positive retained growth plus the full temporary
allocation, bounded at 64 MiB plus 128 KiB, rather than treating replacement
size differences as temporary headroom. Its mutable transaction borrow permits
only one outstanding reservation. Owner and session additions are admitted
against the profile count limits before creation; incomplete owner namespaces
still consume a slot. The transaction is revalidated immediately
before dispatch; completion rescans physical usage even after a mutation error.
The mutation receipt and accounting result remain separate, so a committed
write cannot become a retryable operation when accounting fails. Dropping or
unwinding a reservation grants no capacity credit: later admission still counts
all remaining artifacts until explicit journal recovery removes them.

Optional checksum-covered journal metadata retains the live checkpoint reserve
across writer and owner lifetimes. Profile admission separately charges physical
output plus each reserve's excess over its referenced committed checkpoint.
Only descriptor-validated committed checkpoint sizes offset the reserve;
orphan checkpoints remain fully charged. Checkpoint replacement consumes that
headroom and a smaller replacement replenishes it without double charging.
Reserve growth passes ordinary metadata admission, while a sealed decrease or
release can reclaim an overquota profile. Physical and charged-output ceilings
are reconciled independently after each mutation. Inventory remains stat-only;
the separate bounded reservation scan validates metadata without acquiring a
writer lease, repairing files or asserting screen validity.

Existing-journal mutation plans borrow the writer and immutable input through
admission and execution. Shared effect-free planning derives positive ledger
growth and charges the whole submitted payload plus bounded metadata. Committed
artifact sizes are validated before replacement credit; plans bind to the exact
profile session directory before dispatch. Append, checkpoint, state, event,
acknowledgement and eviction share this path. Dropped plans perform no writes.
Only typed acknowledgement and eviction plans may reclaim an already-overquota
profile: they allocate bounded metadata but no new payloads or namespaces, and
reconciliation still counts every remaining physical byte. Ordinary zero-growth
replacements cannot use that exception; no-op maintenance allocates nothing.
The lower-level declared-demand reservation remains available to trusted owners.

History writes take an explicit borrowed persistence context; the ordinary
context consumes exact journal plans under its already-held transaction.
An alternative read permit reserves capacity before native output consumption:
one raw append of at most 16 KiB, up to two bounded checkpoint replacements
and two protected-state replacements. History derives checkpoint headroom from
its current geometry plus framing, or only the gap marker for an unavailable
projection. The permit checks commit-counter headroom before the read, binds
all writes to one exact session directory, and admits no event, namespace or
retention operations. It reserves cumulative retained growth separately from
the largest sequential temporary allocation and reconciles after every write.
Any rejection, operation failure or accounting failure seals further use;
dropping the permit releases no optimistic capacity credit. Append receipts
retain their committed cursor even when accounting subsequently fails.
Legacy no-context history and session mutation entrypoints are available only
to component tests. Session mutations and recovered acknowledgements receive
the caller's held persistence context; they never acquire a hidden profile lock.
These APIs do not intercept bare journal calls. Production history creation
registers the geometry-derived checkpoint floor before its first checkpoint and
before native startup. The owner verifies the floor before a read permit; an
unavailable legacy projection cannot invent missing reservation geometry.
Resize admits a larger floor before native effects, shrinking it only after the
new checkpoint succeeds. Failed publication and contextless cleanup retain the
floor. Successful cleanup releases it only after durable completed-session facts.

Profile retention uses explicit metadata-first eviction operations rather than
rewriting session limits. Completed output/checkpoint eviction preserves facts,
events and the latest cursor. Live eviction only removes full, checkpoint-covered
prefix segments, preserving the newest segment. The history layer must decode a
usable screen checkpoint; unavailable markers grant no coverage. A checkpoint
identity binds its generation, source and journal so replacement invalidates a
previous selection. Equivalent full-segment end/next-segment start cursors remain
gap-free when replaying immediately adjacent retained bytes.

Session dispatch binds retention to the exact owner and lifecycle: completed
eviction requires closed/exited sessions without native ownership or unresolved
state publication; recovered lost sessions are not assumed completed. The
profile coordinator holds the transaction across selection and eviction. It
retires completed crash-leftover checkpoint reserves before discarding payloads,
including for resident recovered histories. It then orders candidates by
completed raw output, completed checkpoints, then live
checkpoint-covered raw prefixes; within each class it uses creation time,
owner namespace and session ID. The active namespace/session pair is excluded.
Discovery reads bounded manifest and facts-prefix hints, skips histories with
neither output nor reserves, and does not recover or hash unselected payloads.
Each payload class requires physical bytes in that category, including orphans;
a checkpoint-only history is not a raw-output victim.
Hints are selection data, not mutation authority: selected nonresident candidates
require their normal nonblocking journal writer lease, full recovery, matching
state identity, and validated, current, namespace-bound facts. Recovery-only
orphan cleanup is re-accounted before discarding committed output. Legacy
metadata-less histories remain readable but are ineligible for profile retention;
they do not block other eligible victims. No recovered process authority is used.
Busy foreign writers are skipped without bypassing their leases. Physical and
reserved charges are rescanned after each metadata-first mutation; hypothetical
reclaimed bytes never authorize a read. Eviction errors retain ordinary journal
poison/recovery behavior.
Before selecting victims, the owner checks all non-output admission limits,
directory binding and publication counters without issuing a read permit. It
also rejects an active session whose own charge cannot fit after the next read;
reclaiming other histories cannot remedy either kind of refusal.

The history owner binds the live screen to the journal's committed cursor.
Raw bytes commit before screen processing can return a protocol reply; a screen
failure does not turn committed output into a retryable append. Recovery uses
the validated checkpoint and bounded contiguous replay, never live replies or
process authority. Missing, unsupported, corrupt and retention-disconnected
screen evidence remains explicitly unavailable while intact raw history can
still be read. The native owner must reserve profile-budget publication
headroom before dispatching these synchronous journal operations.

Resize publishes an unavailable-screen barrier before invoking the native
resize, then saves the resized projection at the current committed cursor.
An interrupted resize or failed replacement checkpoint cannot resurrect the
old dimensions on recovery. Operations that may flush unobserved PTY output
must first durably record an output gap; subsequent raw bytes cannot silently
repair that screen. These are runtime composition components, not additional
model-facing actions in the current reference-host subset.

Resize validates dimensions and projection availability before starting those
effects. An unavailable projection, including a previously recorded signal
gap, rejects resize without changing the running session's lifecycle, input,
monitors or persistence; ordinary raw-output pumping can continue.
Quota refusal before the invalidation barrier is likewise effect-free. If
final checkpoint admission fails after native resize, the session instead
becomes lost, quiesces input and monitors, and retains the publication failure
even when its best-effort lost-state publication succeeds.

The single-owner session driver composes PTY transport, history, ordered input
and monitor observation. Raw output does not establish shell readiness: only
the trusted startup-control path may transition from starting to running.
The native startup transport uses a nonce-bound one-shot marker connection and
a pinned bootstrap in an explicitly supplied private directory. It works with
bash/zsh user and clean profiles even when profiles close inherited descriptors.
The owner persists shell readiness before acknowledging it, and an optional
command cannot execute before the separate command-start acknowledgement.
The bootstrap is transport-neutral: validate shell/source and transport inputs,
publish its private artifacts, then attach the committed backend to the same
control protocol. Its generic backend wrapper preserves whole-paste intent,
backend-specific input limits, pending-write settlement and signal/output-loss
semantics. Dropping the control handle closes admission but does not discard
the backend's observational authority over an already submitted write.
The session owns its control handle through the ordinary registry pump. With an
initial command, shell readiness remains `Starting`; only a durably recorded
command-start boundary becomes `Running`. The owner drains profile output in
bounded admitted reads before recording that cursor, releases the read permit,
then publishes the transition and acknowledges it with ordinary persistence.
Partial acknowledgements retry without repeating the durable transition.
Startup stage and command cursor are data-only recovery facts, not native
authority. Deadline failure still stops startup when profile storage is busy,
retaining any unavailable final-state publication for the owner to retry.
The startup request's bounded deadline covers preparation, commit and both
acknowledgements without restarting between phases. Private helpers receive a
validated absolute monotonic deadline in the same clock domain as Rust
`Instant` (`CLOCK_UPTIME_RAW` on macOS, `CLOCK_MONOTONIC` on Linux);
standalone PTY preparation retains its
two-second default. Expiry is distinct from generic process failure, and a stale
owner timestamp cannot authorize an acknowledgement after the actual deadline.
Commandless startup suppresses bootstrap input echo until shell readiness.
The canonical artifact directory and retained directory identity are validated
before publication. Private helpers bind and connect short socket leaves from
that retained directory, without changing the host's working directory; long
artifact, workspace and profile paths need no shortening or temporary aliases.
Commandless startup transfers shell-quoted source in bounded ASCII fragments,
using nonce-tagged terminal acknowledgements only to pace canonical input.
Those acknowledgements neither establish readiness nor release user commands;
only the authenticated private startup-control protocol can do that. All bytes
remain observable, and stalled bootstrap input shares the original deadline.
Polling and acknowledgements do not retire filesystem artifacts. The backend
retains exact-artifact cleanup for owner-side retries, including after the
control handle is dropped; failed cleanup cannot become successful close.
Same-session/different-incarnation callers cannot read or mutate the owned
terminal. A scheduler step performs one bounded input attempt, reads at most
16 KiB, and returns typed probe descriptions for separate authorization.

The admitted-read path never performs a shutdown drain. Exit observed before
or after its single read requests cleanup without consuming the remaining tail.
For native PTYs, physical EOF immediately closes input but permits at most
100 ms of nonblocking exit-status observation before reporting final stream
closure. The deadline never restarts: a fast child exit can converge without a
false backend failure, while a still-running process with a permanently closed
stream still reaches the existing loss/cleanup path. Teardown drains use physical
EOF directly while the close routine holds the process capability.
The owner releases the one-read permit before draining through ordinary
persistence authority under the same profile transaction. This preserves a
multi-chunk exit tail without repeating an already committed read or its replies.

Native text/paste input preserves bytes, including NUL; named keys use the
pinned fixed sequences and control spellings map to control bytes. Exactly one
user payload (up to 64 KiB) may remain pending. A shared ordered queue prevents
partial UTF-8, key sequences and protocol replies from interleaving. Native PTY
transport attempts are at most 8 KiB. Backends may admit one whole bounded
64 KiB payload while advancing their transport in bounded nonblocking steps;
the queue preserves paste intent across retries instead of splitting one paste
into several independently bracketed operations. Text and protocol replies do
not acquire paste intent. Receipts report bytes actually accepted,
retain pending suffixes across attention cancellation, and keep the latest 64
operation results. Protocol replies do not require a writer lease, but have a
separate resident bound of 16 frames/4 KiB. Revoke and close quiesce both input
sources; release cannot strand an in-flight payload under another writer.
Resize and signal return busy while input or replies remain queued, allowing
the host to serialize control actions without overtaking accepted input.
After submission, a failed durable quiescence publication is reported separately
from the retained accepted-byte receipt; it does not authorize retrying bytes.

The prepared-pane tmux adapter consumes an authenticated raw capture stream and
retained process-incarnation authority supplied by its host. An absolute tmux
executable and exact private socket, namespace and pane identities constrain
each command; saved numeric PIDs are comparison data only. Supported tmux
versions start at 3.2. One input operation retains at most 64 KiB, with pipe I/O
attempts bounded to 8 KiB. Explicit paste uses one bracketed paste operation;
ordinary text and protocol replies use ordinary paste-buffer delivery, including
tmux's pinned LF-to-CR conversion. Completed receipts survive later close/exit;
an indeterminate submitted paste fails instead of claiming a reliable zero-byte
receipt. Close retains failed native cleanup for retry. Native platform CI
installs tmux and supplies an explicit executable path, so real private-server
tests cannot silently skip when that dependency is missing.

Private tmux launch retains a foreground server child and checks its supported
version before releasing any shell command. A gated pane supervisor stays on
tmux's original controlling terminal, starts the exact bash/zsh shell in its
foreground process group, and reports the retained child's real exit/signal
status. This distinguishes a signalled child from exit code 128 plus the signal
even on tmux 3.2; the supervisor's lifetime alone never means the job is running.
Its freshly authenticated native incarnation anchors bounded session discovery
and cleanup after a fast shell exit. No saved numeric PID or pane fact restores
process authority, and no nested PTY is introduced.

Before receiving launch frames, the authenticated pane helper queries its own
controlling-terminal session and sends a fixed 52-byte session/device/inode
receipt. The host compares it with the authenticated pane PID and its retained,
no-follow character-device descriptor. This supports Linux, where querying the
session through a foreign slave-terminal descriptor returns `ENOTTY`, without
dropping the terminal-identity check. Truncation, mismatch, cancellation and
expiry reject startup before command release; the existing native-incarnation
challenge and commit gate remain required.

On Linux, a subreaper host may inherit dead descendants from its owned tmux
session. Cleanup reaps only exactly identified adopted zombies; it does not
consume unrelated child exit statuses. Previously captured descendants remain
cleanup obligations across session changes, including exited processes still
waitable by another parent. Each authenticated descendant handle stays owned if a
later inventory, capacity, deadline or anchor check fails; an incomplete scan
cannot discard prior captures. Retained handles are settled before new discovery;
a zombie reaped during discovery still makes that inventory nonempty. A fresh
complete anchored snapshot and no remaining retained obligations are required
before the supervisor can retire. Existing descriptor and scan bounds apply.

If tmux close fails before ordinary process signaling, the native adapter makes
one bounded cleanup-only attempt through its already authenticated handles,
after checking the full retained tmux identity. Discovery failure cannot prevent
this progress or grant authority over an unproved process. Graceful close uses
TERM and force close uses KILL; a prior ordinary signal attempt suppresses the
fallback. The original close error remains authoritative and the backend stays
owned until a later complete quiescence proof succeeds. Custom process adapters
deny this cleanup-only operation by default.

Raw capture and its two-byte completion receipt use separate fresh-nonce
authenticated streams on one private socket path. Success is reported only
after capture reaches EOF with all observed bytes delivered; overload, missing
receipt, truncation or reported failure retains an explicit output gap. The
helper uses a 16 KiB buffer and nonblocking downstream delivery. Stock tmux's
own pipe-pane queue is not a hard byte-bounded RSS guarantee. Close drains
while waiting for actual pane teardown after supervisor retirement, independently
of the earlier child-exit status; deadline/drain exhaustion cannot be called a
complete capture. Failed cleanup retains the exact owned server/backend for
retry and never unlinks a replacement artifact.

Tmux command and foreground-server children reserve the existing bounded
child-reap authority before spawn. Abort and retirement use nonblocking child
observation with one 500 ms cleanup window per child, never an unbounded wait
after kill. Failed explicit cleanup retains the exact child for retry; later
retries can observe exit or retry pending kill delivery without refreshing that
window. Drop does not restart an exhausted cleanup grace; unresolved ownership
transfers to the existing bounded reaper with its original host-completion
obligation. Server artifacts
remain owned until that child is reaped or its wait authority is known lost.
Pending tmux-child kill delivery is retried only after a later observation of
that exact child as still running; failed observation or lost wait authority
cannot authorize signaling. Other direct-child quarantine behavior is unchanged.

The private CLI helper entrypoint bypasses ordinary configuration and accepts
only its four bounded protocol arguments. The same shared startup bootstrap,
owner acknowledgements and absolute deadline cover native PTY and tmux user/
clean profiles, optional commands and commandless sessions.

Close commits a discontinuity barrier, drains final output without protocol
reply effects, and removes monitors after final observations. A positively
complete drain may publish the final coherent screen; an incomplete drain
retains the gap. Persistence failure cannot skip native cleanup, and failed
native cleanup retains owned authority for an explicit retry. If profile
authority is unavailable, explicit teardown performs no journal writes, records
failed publication in memory, and discards drained output with an unavailable
projection. Later state publication must first establish the missing durable
gap barrier; a state-only write cannot erase that obligation. The driver is a
runtime composition component; the persistent catalog/host,
startup-control transport and full tool routing are not
provided by this driver alone.

A failed bounded close may already have quiesced input, closed the PTY master
or signaled jobs. Its error and output-loss evidence remain observable. A later,
separately authorized close of that exact session can finish retained cleanup;
the failure does not authorize replaying a start, command or write.
Captured descendant identities and Linux process handles belong to the retained
process across close attempts, including descendants that subsequently leave
the original session. Each positively authenticated capture enters retained
cleanup before later ancestry, session-inventory or union-merge work can fail;
this includes successful macOS identity/session observations. An unproved PID
lookup or incomplete ancestor chain grants no cleanup authority. Existing
retained proofs remain exact-process cleanup authority; an incomplete inventory
never proves quiescence or authorizes signaling an unproved process.
A failed inventory or quiescence proof does not reap the
shell: it remains the original session/group anchor. Signal-delivery failure may
still be followed by escalation or concurrent natural exit; it is never used as
proof of cleanup. Positive shell exit must precede the final quiescence proof,
so a running shell cannot have cached quiescence or bypass signals on retry.
Exact positive reaping records its exit receipt before releasing authority.
Retries after master closure skip foreground ioctls that require that master.
Dropping an unresolved terminal transfers its existing process, captured set,
reap permit and completion obligation to the bounded shared cleanup queue.
The queue services ordinary reaps and round-robin terminal retries outside its
mutex; new arrivals cannot displace older retries. No new worker or duplicate
reap permit is created by that transfer.

The driver binds durable lifecycle, known termination, creation/last-output
times, exact logical owner/incarnation and monitor snapshots to the committed
output cursor. Output observations, due timers, probe transitions, monitor
mutations and event acknowledgements commit their state before returning their
successful result. Idle scheduler steps without those changes do not rewrite
metadata. A state-publication failure quiesces live input and probes without
discarding the owned backend needed for explicit cleanup.
Acknowledgements advance the in-memory record only after durable publication.
A committed acknowledgement remains advanced if subsequent accounting fails,
while the publication error is retained; a pre-commit failure cannot become a
successful memory-only retry. Committed raw-output cursors likewise survive
accounting failure without replaying the bytes or their protocol replies.

New driver sessions require trusted launch metadata: host and backend identity,
resolved shell, workspace, working directory, optional command, backend kind
and shell profile. Paths and backend identity are bounded to 4 KiB each;
commands are bounded to 64 KiB. The facts envelope includes worst-case JSON
escaping within its 512 KiB bound. Owner-authorized inspection and recovery
retain those facts without executing paths or granting backend authority.
Older records without launch metadata remain explicitly unidentified; recovery
does not guess missing shell or command facts.

Provider-neutral attention facts enforce that a human lease exists exactly
when attention is user takeover. Role-specific cancellation clears only the
caller's attention/lease, never terminal lifecycle. Native recovery clears stale
attention and lease facts before publishing a formerly live record as lost;
inactive records cannot claim an active attention/lease. Native input and
attention mutations bind both the trusted actor role and exact writer identity;
human acquisition takes over the lease, while human waits do not acquire one.
Completing an agent wait preserves its write lease; cancellation clears only
that actor/writer's authority without discarding already accepted input or its
queued suffix. Revocation requires close authorization at host dispatch.
Profile-backed registry mutations authorize the owner before acquiring storage
authority and release the transaction before returning a reply. Recovered wait
observations reject a recorded observation gap rather than inventing matches.
These components do not themselves grant tool or human-host authority.

Profile-backed shutdown retries recovered-history publication failures on the
same owner. It reconciles current facts and committed acknowledgements without
replaying a failed request, acquiring native authority, or clearing an unresolved
storage error. Repaired transient failures can therefore release the retained
history; irrecoverable failures remain explicit cleanup obligations.

Owner-authorized inspect projections cover live and recovered sessions, masking
controls by role, lifecycle and input quiescence. A checkpoint on an older raw
segment is re-anchored through explicit persistence before exposing current
facts, without granting recovered history live backend authority. Missing
launch metadata remains an unavailable full projection, not fabricated facts.

Native recovered-session views expose only owner-authorized facts, raw history,
screens and durable event acknowledgements. A previously starting/running host
record becomes lost, with pending monitors/probes stopped before publication.
The host injects a non-rewinding recovery clock for that transition; known
closed/exited records retain their observed outcome. Saved state never
contains a PID, process capability or input lease capability, and recovery never
replays input. If raw output committed after the last state snapshot, recovery
records an observation gap instead of inventing monitor matches or output
timestamps. That gap is distinct from missing raw bytes: retained output and
an independently valid screen can still be read.
Notification-counter exhaustion also stops pending monitors and records
incomplete monitor notifications, rather than making retained history
unreadable or pretending every terminal notification was delivered.
This incomplete-notification fact also survives failures while consuming live
or close-time output, observing a committed resize, or degrading monitors for a
signal output gap, even if stopping the remaining monitors succeeds. Failed
gap observation prevents signal delivery; failed observations quiesce input.

The monitor state machine implements all thirteen conditions with explicit
clock/cursor/lifecycle observations. It emits bounded typed probe requests,
not network/filesystem/process effects. Evidence must match the exact session,
monitor generation and probe sequence. Snapshots retain matcher/event state;
pause/resume preserves activation baselines and rejects stale probes. Waits
observe started/exit/quiet/literal-match conditions without owning process
lifetime. Runtime probe authorization, profile-wide persistence coordination and model-facing
action routing remain separate composition responsibilities.

### Authorized native monitor probes

The native probe executor requires a live, nonserializable grant bound to the
exact owner incarnation, terminal session, monitor ID, generation and target.
The grant can be shared across checks; each one-shot binding independently
checks its request sequence and clock anchor. A successful monitor mutation
returns its exact generation for installing or retiring that grant in the
same trusted owner callback. Saved descriptions and observations cannot rebuild
native grants. The host explicitly revokes grants before replacement, removal
or shutdown: queued clones are denied, running checks observe revocation even
without another caller poll, and late receipts cannot publish successful
evidence after revocation.

TCP and plain-HTTP probes use at most four explicitly approved socket addresses,
without ambient DNS resolution, proxies or redirects. HTTP sends one HTTP/1.0
GET and retains only its first 1024 response bytes; credentials and other URL
schemes are rejected. Connection attempts retain the pinned 250 ms per-address
bound, and HTTP reads/writes retain 500 ms phase bounds, all within the request's
original two-second absolute deadline. Local readiness endpoints require the
same explicit approval as other addresses. This does not change `web_fetch`
network policy.

Host preparation lexically resolves monitor paths and custom cwd against the
trusted terminal session cwd before workspace canonicalization, matching the
pinned monitor resolver rather than the separate command-cwd rules. Existing
symlinks resolve during preparation. Missing nested paths retain the nearest
existing approved ancestor descriptor and at most 4096 bytes of normalized
relative components. Checks open intermediate directories without following new
symlinks and stat the final leaf without following it; unresolved symlinks cannot
become latent authority. Missing components yield an absent observation, so later
ordinary directory/file creation remains observable.
Custom probes retain the approved PATH-selected `sh -lc` invocation, independently
of the terminal's bash/zsh or user/clean startup choice, matching pinned legacy
custom execution. Shell lookup uses the captured environment on an owned worker;
relative PATH entries use the custom probe's canonical cwd. The environment,
canonical-cwd fingerprint and directory descriptor stay explicitly owned, with
immutable environment storage shared across grants. Each execution
rechecks the named directory's identity before launching. They reuse the owned
captured-process executor with a separate 16 KiB aggregate output boundary and
at most one excess byte for detecting overflow. Normal foreground execution
keeps its independent 1 MiB contract. Effects run on bounded collected workers;
queue time does not reset deadlines, abandoned futures cancel owned work, and
evidence preserves the exact session/monitor/generation/request identity.

The worker-owned host scheduler installs grants only from successful exact live
mutation receipts. Pause revokes in-flight work while retaining the approval
template; a matching newer resume generation renews that template without
reacquiring paths. Update, removal, until-match completion, expiry, terminal loss
and shutdown retire old grants and reject late evidence. At most sixteen session
namespaces and sixty-four monitors per session retain grants; descriptions queue
within that same 1024-entry bound. At most sixteen probe/publication futures and
one housekeeping future are polled per owner tick. Start publication and monitor
mutation reconcile all retained namespaces against current live IDs/generations
in the same owner callback before grant admission. Retired identities therefore
cannot occupy quota until housekeeping or revoke unrelated active grants when
a valid replacement is added; paused approval templates retain their slots.
Housekeeping reads only live
monitor IDs/generations, not saved facts or authority descriptions, and successful
evidence publishes through the registry's exact profile transaction. The shared
host stop and grant revocation reach owned native workers even without another
poll of an abandoned caller future. Scheduler shutdown cancels the shared stop;
non-owning requester futures cannot prevent last-host cleanup.

## Resident registry and disk catalog components

The resident registry owns at most sixteen sessions on the host's blocking
worker. Tool calls borrow live sessions and cannot acquire ownership of their
lifetime. Duplicate, unrecyclably full and closing admissions fail before invoking the
launch/recovery factory. Owner-incarnation and workspace checks bind returned
facts before residency is accepted. Recovered history has no live-control path.

Exited/lost/closed sessions stay resident while empty slots remain. Only a new
admission under pressure recycles an inactive slot, in round-robin order, after
native cleanup and state publication succeed and all operation residency
references are released. An exited resident remains waitable until recycling;
a wait after recycling reports not found, while durable read/screen/inspect/list
and explicitly authorized close remain available without reacquiring native
authority. Recycling never deletes the journal. Cancelled or expired staged
admission cannot evict another resident.

Pure process-local residency leases carry no host-lifetime vote or native
authority. Wait/write registrations retain their lease through pending input
settlement or durable attention completion, even when the caller abandons them.
Unconsumed replies and their final facts projection retain the same resident;
staged start receipts retain it through the enclosing start attention wait.
Last-host shutdown still cleans up native resources with such leases outstanding.

Bounded round-robin pumping advances at most the requested number of active
sessions per step, continues past per-session failures, and returns raw chunks
and unexecuted probe descriptions without adding another retained output queue.
Profile-aware pumping acquires a nonblocking transaction and read reservation
for each running session before consuming native output. Lock contention or
capacity refusal advances fairness but leaves that session's output and input
authority untouched. Known exit cleanup does not require normal-read headroom.
Native status failures are not capacity deferrals: they mark the session lost
and quiesce input and monitors. The owner publishes that observation when
admitted, otherwise retaining an explicit publication failure without journal
writes or abandoning the backend needed for cleanup.
Exit racing an admitted read releases its permit before multi-chunk cleanup;
a separate cleanup error preserves the successful read result and final observed
cursor/lifecycle, without returning stale probes. A transaction may include
bounded retention mutations on other sessions before the active read, but never
spans scheduler turns, observers, request callbacks, reply wakes or idle waits.
The injected clock cannot rewind any resident session. Resident listing is
owner-scoped, lexically paged and optionally filtered by lifecycle/backend.
Inactive residency may be released without deleting history, but a lost session
with unfinished native cleanup remains owned. Failed state publication is
tracked independently of native ownership: shutdown retries publication after
successful process cleanup, and ordinary release rejects unresolved failures.
An explicit failed-history transfer retains the journal lock and in-memory
facts for a host recovery owner; it is not a successful durable release.
Recovered-history publication failures also block ordinary release, remain
visible in shutdown results, and require an explicit failed-history transfer.
Shutdown stops admission before attempting every owned cleanup; it retains
failed native cleanup for retry and keeps completed history readable.
Final registry drop forces a no-persistence cleanup pass on
the blocking owner, not on a tool future's poll thread. Profile-aware shutdown
attempts native cleanup even when it cannot obtain the profile transaction,
while retaining the failed-publication obligation instead of claiming durability.

The disk catalog receives a retained state-root descriptor and derives a framed
SHA-256 namespace from the canonical workspace and exact logical owner and
incarnation. Each namespace retains at most 256 validated session directories,
listed in exact-spelling lexical order. Private directories, a permanent
nonblocking owner lock, no-follow opens and retained inode checks confine access.
The catalog lock explicitly unlocks on owner drop, including failed preparation,
so a forked child's inherited descriptor cannot prolong the finished ownership.
Closing an older inherited descriptor cannot unlock a new catalog owner's
independently acquired lock.
New directory publication syncs the child and parent. Ambiguous post-creation
failure poisons that catalog handle; preparation of validated existing
directories and locks retries the child and parent durability barriers. Opening
a session explicitly also reconciles its directory barriers; listing remains
validation-only and does not sync every retained session. No path deletes or
guesses repairs. Catalog reads never infer process authority.

Only newly created directories have their private mode restored after the
process umask is applied; existing entries are validated, never chmod-repaired.
On Linux, restoration retains a no-follow `O_PATH` descriptor, including for a
mode-000 directory, and changes its mode through a verified procfs descriptor
link. The procfs type, mount identity, current-process link and exact target
inode must agree before the effect. Missing or mismatched procfs or mount-ID
support fails closed; there is no ordinary pathname chmod fallback, helper
execution or process-wide umask mutation. The descriptor remains held through
the effect and the final named-entry check and durability barriers still apply.

The complete list projection merges resident facts with validated nonresident
histories, in exact session-ID order, without consuming resident slots. Its
256-row bound is independent of the sixteen live slots; an oversized union is
an error, not a truncated success. Task, workspace, lifecycle and backend filters
cannot expand owner authority or hide corrupt records. An inode-bound catalog
snapshot avoids repeated sibling scans and directory syncs for each row. Only
one disk history is decoded at a time; the result retains compact facts, not
commands or screen cells. Recovery observations never grant native authority.

The continuous owner-loop component runs on an explicitly owned blocking
worker; its constructor spawns no thread. Its host handle owns admission,
while inert-before-poll tool futures submit bounded authorized commands.
At most 32 submitted requests, including unconsumed results, retain admission
slots. Cancellation before execution prevents effects; cancellation after
execution begins does not discard a committed receipt. Dropping a tool future
does not shut down the registry. Last-host-handle drop requests shutdown without
joining or running native cleanup on the polling thread.
Profile-aware jobs receive the owning worker's exact store and budget, acquire
their short mutation transaction inside dispatch, and release it before reply
wakes. Borrowed profile authority cannot escape through a request result. Such
jobs refuse an unmetered test loop before invoking the mutation callback.
Each dispatched job receives a fresh owner-clock sample validated against the
registry and every resident session before its callback can perform effects.
The admitted sample advances the registry's time floor even for a read-only
job. A probe completed after the previous pump is therefore not rejected merely
because dispatch reused that pump's timestamp. Backward or panicking clocks
still stop dispatch and retain the existing owned cleanup path; probe evidence
timestamps and absolute deadlines are never clamped or extended.

The native host's reusable reserved-worker spawn path shares the existing
bounded worker collector. It rejects foreign/non-single reservations before
spawning and releases a task only after registering the owned thread handle.
Failed registration disconnects the release gate and joins that handle; normal
collection retains the capacity permit through join and thread-local cleanup.
Opaque worker panic payloads are suppressed without destructor execution, so
one failing worker cannot stop collection for unrelated owners.
Lazy background initialization uses the same path while preserving its atomic
whole-cohort reservation. `NativeOwnedWorkerSpawner` is an inert zero-state
binding to this path; its first explicit spawn acquires the collector and one
capacity reservation. The terminal runtime implements its injected spawner
contract with this production binding, without a detached-thread or separate
collector lifecycle. The worker job retains its own cleanup obligations even
when the submitting future or spawner value is dropped.

`NativeOwnedWorkerScope` adds explicit per-host completion enrollment to that
same collector. Closing a scope atomically rejects new admissions; the host
separately cancels its existing jobs. Its observation-only completion handle is
ready only after closure and collector joins, including thread-local cleanup.
Unconsumed response values do not hold completion tickets, and unrelated host
or provider workers are not drained. A dedicated caller worker can wait for
settlement; waiting from a worker enrolled in the same scope is rejected.
Native process adapters retain metadata-only cleanup tokens while inside an
explicitly scoped worker. Existing child-reap permits carry those tokens through
quarantine until the exact obligation settles. These tokens grant no process,
worker-admission or host-lifetime authority, and do not change the existing
cleanup/reaper algorithms. Scope creation and unpolled scoped futures are inert.

The lazy runtime assembly starts its injected owned-worker spawner only after
an accepted request is polled. Registry and profile construction occur inside
that worker. Unpolled/cancelled requests do not initialize it, and tool futures
do not keep the host alive. Last-host drop requests shutdown without joining
on the polling thread. Unresolved cleanup stays on the same worker with capped
backoff; completed cleanup is not held forever by an older request error.
An already-executed operation's stored receipt survives a later runtime failure,
including failure of its reply waker. Unresolved or unadmitted requests still
report the runtime's initiating failure; a committed mutation is not relabeled
as an unexecuted operation.
The runtime can additionally own a typed host state, created by its initializer
on that worker. Context requests borrow the same state with the registry,
profile store/budget, wait/write coordinators, validated time and cancellation;
the stateful observer borrows it between requests. State need not be `Send` or
`Sync`, and no state value lives in an async-caller-owned synchronization cell.
State is retained through cleanup retries and destroyed after registry/backend
destruction, including initialization or callback unwind. Its destructor panic
is contained independently so teardown cannot double-panic through that state.
Unpolled, pre-cancelled and early-closed requests do not construct state, and
pending reply futures do not keep it alive after the last host shuts down.
Owned effect workers can retain a non-owning request handle for staged callbacks.
Creating or cloning that handle neither starts a worker nor counts as a host;
last-host shutdown rejects its later requests and still closes native sessions.
Slow startup can first reserve an exact process-local registry slot. Pending
starts count with resident entries against the sixteen-session limit but never
appear in observations or pumping. Reservations bind registry, owner and session
identity without reusable numeric authority. Commitment consumes admission before
the factory runs, including failure or panic; the owned startup job explicitly
withdraws unused reservations. Shutdown invalidates pending admission before
native cleanup, so a late worker cannot reopen the registry.
One boxed native backend selection forwards PTY and tmux operations through the
same registry contract, including whole-paste intent, input limits, observation-only
write settlement and close receipts. The contained backend remains the sole
cleanup owner; choosing a transport adds no independent cleanup lifecycle.
The shared native launch factory resolves captured shell selection and the
24-row/80-column default before effects, then prepares either transport under
one supplied absolute deadline. Commit rechecks the exact retained cwd and
returns the backend, startup acknowledgement control and descriptive launch
identity together. Neither a transport selection nor saved identity creates
process authority; the existing prepared owner handles failed launch cleanup.
Staged startup reserves a resident slot before acquiring launch authority, then
prepares and commits the native transport on a separately collected worker. The
single absolute startup deadline begins at first poll and includes worker and
owner queue delay. A host that already began owned preparation supplies its
original absolute deadline without converting it back into a remaining timeout;
expired admission neither recycles another session nor acquires launch authority. The
owner continues pumping other sessions throughout preparation. Only short
admission/publication requests cross back to it; closures carrying a backend
are constructed, polled and dropped on the effect worker or owner, never the
tool's polling thread. Initial facts and monitor admission are durable before
the startup controller can release the user command. Failed initial publication
retains a quiesced, lost session and its cleanup/publication obligation rather
than reporting a successful start. Pending reservations cancel preparation on
withdrawal, failed commit or host shutdown. Abandoned callers leave rollback on
the collected worker; a busy owner queue retries without keeping the host alive.
Successful admission transfers startup lifetime to the registry, independently
of the caller's future. Its receipt describes durable initial admission; the
enclosing host applies the requested attention wait separately. At most sixteen
staged operations or unconsumed receipts occupy the starter's admission bound.
An owner-state callback receives committed initial-monitor mutation identities
before the first pump, so the host can install exact process-local probe grants.
The callback changes in-memory owner state inside the publication transaction;
it must not reenter profile transactions. It must install transactionally,
rolling back partial grants on error, and retain
their normal cancellation/shutdown cleanup. Callback failure quiesces and retains
the failed session; it never releases the user command.
Worker-owned catalog state retains bounded, exact-profile owner leases between
requests. Cold saved histories can be opened without admitting a live registry
entry or constructing a backend, so live-session capacity does not cap historical
reads. Recovery validates owner and workspace binding in its single state decode
before publishing lost-session facts; a live writer lock prevents this path from
reinterpreting a currently owned session as abandoned. Profile transactions end
before results leave the worker, and cancellation cannot erase an already
committed recovery or acknowledgement receipt.
The resident dispatcher routes read, screen, write, wait, monitor, inspect,
resize, signal and close through the typed owner context. Actor/writer identities
and lease-revoke authority come from the host, not descriptive controls. Wait
capacity is reserved before attention changes and withdrawn without cancelling
pre-existing attention if admission fails. Pending writes and waits retain their
typed receipt plus an explicitly admission-time facts snapshot if shutdown
prevents a later projection. Committed-effect failures are not retry-safe, and
monitor probes remain available to the ordinary owner observer.
Cold read, screen and inspect share the same result projection without native
admission. Explicitly authorized cold close persists closed history and monitor
quiescence using the exact owner/profile transaction, without reconstructing a
backend or signal target. Resident and cold reads aggregate up to 1 MiB within one retained
segment, reporting the actual normalized raw range and any retention gap.
Successful close replies clear their next-action hints, matching the pinned
close result; subsequent historical observation still projects readable history.
Runtime-facing dispatch combines these routes with the complete disk/resident
list projection on the same owner. Only a missing exact owner/session permits
cold-history fallback; only durable close and inspect acknowledgement mutate
history there, and no native-control action falls back to saved identifiers. The
asynchronous dispatcher retains a non-owning requester, waits for write/wait
receipts outside the owner loop, and then attempts an uncancelled facts refresh.
Its reply explicitly distinguishes refreshed facts from the admission snapshot
retained when shutdown or publication refusal prevents that refresh. Failed
receipt projection preserves the typed native completion instead of implying
that retrying the operation is safe. Construction and unpolled requests perform
no profile initialization or native effects.

The loop pumps immediately while output is available and polls idle timers at
10 ms intervals, with at most one command between pump opportunities. Output
and probe descriptions go directly to a bounded synchronous host observer,
not another retained queue. Clock failure or a panicking command/observer stops
admission and initiates cleanup. Exit reports include every unresolved shutdown
failure; the blocking host retains registry ownership for recovery disposition.
Queued rejection contains each request's waker/destructor panic independently,
so one caller cannot bypass native cleanup or strand the remaining replies.
Suppressed opaque panic payloads are deliberately retained without invoking
their potentially panicking destructors, matching the host's cleanup policy.
External probes remain separately authorized, off-loop effects.

Attention waits share the owner loop rather than occupying one worker per
wait. At most 32 registrations or unconsumed completions remain resident. Each
tick reads at most one 16 KiB durable page per pending wait; catch-up defers
quiet/condition decisions until the observed cursor is reached, but never
extends the absolute safety ceiling. Physical segment rollover is not an
observation gap. Dropped wait futures cancel only their attention, not the
session, monitor set or queued input.
Outcomes freeze before attention completion is persisted and reply wakes occur
after releasing profile authority. Transient publication failures retry on the
owner. Final shutdown still resolves each future, preserving its frozen outcome
and explicitly reporting unavailable attention persistence instead of claiming
successful cleanup or leaving the future pending indefinitely.

Pending user writes also share the owner loop. A separate 32-entry bound includes
unconsumed completions and is reserved before submission. Observation after
pumping or shutdown uses exact owner, session, actor, writer and operation
identity; it never resubmits input. Cancellation before submission prevents the
effect, while later cancellation or dropping the reply cannot discard the queued
suffix. Final replies retain accepted-byte counts and independent publication
errors; unavailable observation reports the last known receipt. Reply wakes run
after releasing profile and reply locks, with individual wake panics contained.
Revoking input admission retains only bounded, already-attempted input identity
for observation. Teardown settles it before and after native close without
sending new bytes; completed counts survive removal of the backend, and an
ambiguous final result is failed rather than a reliable zero-byte close.

Journal writer leases explicitly unlock when their owner is dropped, including
failed construction. A temporary fork-inherited descriptor cannot keep a retired
writer's lease busy, and closing that descriptor cannot release a newer writer's
independent lock.

The sixteen-entry resident bound is not disk-history retention. The profile
transaction and admission components cover nonresident histories and concurrent
owner namespaces, including persistent live checkpoint reserves and bounded
retention selection before running-session reads.
Production worker ownership, disk-catalog composition, trusted
startup control, attention and lease effects, tmux launch/capture/recovery, and model/CLI routing
remain full-runtime integration work.

## Boundary

The reference-host tool implements the `exec`, bounded `start`, bounded
process-local `read`, bounded persisted-record `list`, bounded persisted-record
`inspect`, bounded persisted-record `wait`, and bounded process-local `signal` and `write`
subsets of fx's `terminal` tool.
`exec` captures bounded standard output and error and waits for the direct
child. `start` durably records and releases one noninteractive command through
the native background supervisor, starts capturing its merged output, and
returns its display identity without waiting for command completion. `read`
pages that captured output only for the exact session incarnation that started
it. `list` returns a compact bounded catalog of recorded history.
`signal` delivers exactly one of `hangup`, `interrupt`, `quit`, `terminate`, or
`kill` to the live Linux process tree or macOS original process group owned by
the exact session incarnation, then acknowledges delivery without waiting for
exit or escalating to another signal. It does not derive authority from the
displayed PID.
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
`write` and the optional `start.stdin` field appear only when a trusted
process-local input writer is injected alongside a starter. Stdin defaults to
`"null"`; only an explicit `"pipe"` start retains writable input authority.

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
mistyped values, an empty command, and a command over 64 KiB reject. The
complete canonical argument object is bounded by 417,865 bytes: six times the
command and cwd byte limits plus the maximal fixed canonical field envelope.
The composed host aligns both provider and engine admission to this ceiling.

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
Run a foreground command, start a background command, read bounded same-session background output, signal one live same-session background process scope, write bounded same-session background input, list persisted background records, inspect one persisted background record, or wait for its recorded exit
```

An exec-only construction retains its earlier foreground-only description and
schema. All forms deliberately exclude `screen`, `monitor`, `resize`,
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
    stdin: "null", // "pipe" only for an explicitly opted-in background start
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
two arguments, authorized cwd, stdin mode, profile name, and digest. Successful preparation
returns those same canonical model arguments. Direct `execute` reparses and
revalidates all fields and rejects any canonical-argument, program, argument,
cwd, profile, or digest divergence before filesystem access, worker creation,
or process spawn. The existing engine presents terminal execution as critical
risk. Denial has zero terminal-owned effects. `signal` instead prepares the
exact custom capability
`{"name":"terminal_signal","details":{"background_id":7,"signal":"terminate"}}`;
the requested identity and signal cannot change after authorization, and
denial performs no registry lookup, process-table scan, or signal syscall.
`write` similarly requests critical custom `terminal_write` authority with
exact `background_id`, decoded `byte_length`, lowercase SHA-256 `sha256`, and
`eof` fields. Permission details never contain plaintext or encoded input.
Canonical prepared arguments use padded standard base64, so UTF-8 and base64
spellings of identical bytes have the same authorized payload identity.
`read`, `list`, `inspect`, and `wait` prepare with no authority because they can read only through
explicitly injected owner-scoped output or persisted-history boundaries; none
requests process permission.

The capability authorizes a process, not a sandbox. The retained workspace
descriptor constrains only the child's starting-directory identity. Once
approved, `/bin/sh` and the command can use absolute paths, inherited
credentials in the approved snapshot, child processes, and network or other
host authority available to the machine account unless the host additionally
injects the explicit sandbox launch authority described above. The capability
and cwd descriptor alone provide no isolation; effective `none` retains this
unrestricted process boundary.

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
macOS. The production system executor is therefore Linux-only. Public
`TerminalTool::open` and
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

`start` accepts the common `action`, `command`, `cwd`, and `profile`
fields and, only with an injected writer, optional string `stdin` (`"null"`
or `"pipe"`, default `"null"`). Foreground `exec` never accepts stdin.
Shell, backend, return condition, wait ceiling, dimensions, initial
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
stream. Standard input is `/dev/null` unless `start` explicitly requested
`stdin: "pipe"`. The supervisor continuously drains
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

## Process-local background input

The separate write form is:

```json
{
  "action": "write",
  "background_id": 7,
  "data": "hello\n",
  "encoding": "utf8",
  "eof": false
}
```

`action`, nonzero JSON-u64 `background_id`, and string `data` are required.
`encoding` is exactly `utf8` (default) or `base64`; `eof` is a boolean defaulting
to false. Unknown or mistyped fields reject. UTF-8 is sent exactly, without
newline insertion, Unicode normalization, or NUL filtering. Base64 requires
canonical padded RFC 4648 standard-alphabet spelling: whitespace, URL-safe
characters, missing padding, and nonzero unused bits reject. The decoded
payload is at most 8,192 bytes; the bounded canonical serialized argument cap
also applies. An empty payload is allowed only with `eof: true`.

The writer privately receives the exact caller session ID and incarnation,
not a model-selected owner or display PID. Preparation is effect-free;
permission denial performs no input lookup or write. The execution future is
inert until polled, and cancellation before submission has no effect. After
the first writer poll, committed completion wins over cancellation, including
durable tool-result replacement in core. Four independent write admissions
are available. The trusted writer retains the opaque admission through actual
native completion even when its caller drops the future.

One bounded native attempt returns an exact receipt:

```json
{
  "action": "write",
  "background_id": 7,
  "bytes_written": 6,
  "stdin_closed": false,
  "status": "written"
}
```

`bytes_written` counts bytes accepted by the pipe, not bytes consumed by the
command. `written` means all supplied bytes were accepted and requested EOF
was applied; without EOF the writer remains open. `backpressure` reports a
strictly shorter accepted prefix and leaves input open without applying EOF.
Callers may submit only the unaccepted suffix and repeat their EOF request.
`closed` means input was already closed or its reader disappeared; `failed`
means a transport failure closed input. Both retain any accepted prefix count,
and neither promises the suffix can be retried. A partial side effect is never
converted into a generic retryable error. EOF is a half-close of the sole
input writer, never an implicit newline or process termination; it is applied
only after every supplied byte was accepted. Output remains separately readable.

Unknown and wrong-owner targets share fixed `terminal_write_not_found`;
capacity/contention returns retryable `terminal_write_busy`. Other pre-effect
failures are fixed non-retryable `terminal_write_failed`. Invalid trusted
receipts return fixed non-retryable `terminal_writer_failed`. Every receipt
is validated against the requested identity, byte count, and EOF semantics;
error and Debug values contain no input, command, path, or owner details.
This process-local pipe is not a PTY, restart-safe handle, or interactive
terminal emulation. Process cleanup revokes input authority before reap.

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

On Linux the controller validates the retained root identity, takes one
bounded process ancestry snapshot, delivers the selected signal deepest-first
to every identity-pinned descendant, and then signals the original group. An
inside-group descendant can therefore receive the requested signal twice; that
deliberate duplicate closes the race in which it could call `setsid` after its
group was observed but before the final group signal. Linux uses the retained
procfs mount authority and opens a proc directory plus pidfd for every
descendant while its queued parent remains pinned. A descendant's proc
directory is released immediately after that node's children and identity are
fully scanned; the prepared delivery retains only pidfds and never resolves a
numeric PID again. Before
capturing signal authority and on every later authority validation, Linux also
requires descriptor-relative `self/status` to have the retained mount ID and
exactly one `NSpid` equal to the current process PID. Ancestor procfs namespace
views and absent, duplicated, multi-level, mismatched, or malformed namespace
identity fail closed before numeric PIDs reach `pidfd_open`. One Linux
traversal shares a 250 ms monotonic deadline, 524,288 read attempts, 131,072
entries, and 32 MiB across mountinfo, the 64 KiB namespace-status record, all
64 KiB task-children records, and 4 KiB proc-stat identity records; `EINTR`
consumes the same finite attempt
budget, including mountinfo, and terminal exhaustion suppresses later proc and
pidfd probes.
The retained root-identity capture is independently subject to the same finite
reader limits. A vanished Linux descendant is harmless, but an incomplete
snapshot, identity ambiguity, bound overflow, non-vanished descendant-delivery
failure, or original-group delivery failure rejects the operation; partial
delivery is never reported as success. One descendant failure does not
suppress later descendant attempts or the original-group attempt. Once
descendant delivery starts, later root or group disappearance is a process
failure rather than a pre-effect not-found result.

macOS deliberately does not enumerate or individually signal descendants:
there is no public incarnation-pinned per-process signal handle equivalent to
Linux pidfd. The retained, unreaped direct child pins its original PID and
process-group identity because the child is launched with `PGID == PID`.
While holding the same close-before-reap lifecycle gate, one request therefore
makes exactly one atomic `killpg` delivery to that original group, with no
separate `getpgid` check and no process-table read. A descendant that leaves
the original group is outside the supported macOS signal scope. Success
acknowledges that single group delivery only. One request never waits for exit,
repeats, or escalates its chosen signal.

Native signal dispatch runs on the supervisor's existing fixed worker pool
rather than the engine poll thread; only Linux performs process-table
traversal. Terminal admits at most four signal actions independently of
foreground executions, output reads, record reads, and supervisor process
capacity. Linux additionally limits retained signal-preparation descriptors to
256 per operation and 1,024 process-wide. The process-wide ceiling is reduced
on every admission to at most half the current soft `RLIMIT_NOFILE` and at most
that limit minus 128 descriptors, preserving capacity for unrelated process
work. Reductions tighten the next admission and later increases restore
capacity without reconstructing the host. Every transient proc traversal
descriptor, queued proc directory, and retained pidfd acquires a permit before
its open syscall; descriptors close before their permits return on success,
error, unwind, or drop. On Linux, one per-process reservation rejects
overlapping signals while read-only traversal runs outside the lifecycle lock.
Before the first Linux signal, delivery reacquires that lock and rechecks both
close admission and the exact controller target. A close can therefore finish
and reap while a proc read is stalled, and the stalled preparation performs no
later effect.
macOS performs no preparation and holds the lifecycle lock across its only
group syscall. Registry and per-process lifecycle lock contention fail fast as
retryable `terminal_signal_busy`. An unknown, completed, or wrong-owner ID is
indistinguishable as `terminal_signal_not_found`; a process-table or delivery
failure is the fixed non-retryable `terminal_signal_failed` error, while
descriptor admission exhaustion is the fixed non-retryable
`terminal_signal_resource_limit` error. Signal
future submission is the ordered mutation commit boundary: cancellation before
submission has no signal effect, while cancellation after submission cannot
relabel a completed or partially attempted native delivery. The trusted
prepared-call mode makes core retain a first-polled signal execution through
durable result replacement and `ToolFinished` delivery. Dropping the direct
caller future after native submission likewise cannot release one of the four
signal admissions early: the native blocking closure owns its admission until
the real delivery attempt returns.

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
accepts at most 100 records, retains at most 479,744 bytes per record, and accepts at
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
or observation cap wins. At most one 479,744-byte record, one bounded decoded detail,
and one persistent absolute-ceiling timer are live per admitted wait. During a
backoff, its shorter delay is the only second timer; aggregate decoded input is
at most 128 times the per-record ceiling across all observations and does not
accumulate in memory.

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
rejection, Linux identity-pinned all-descendant delivery, original-group
delivery, Linux incomplete-delivery failure, macOS group-only delivery,
close-before-reap serialization, and off-poll-thread native dispatch; pre-poll
cancellation, same-first-poll and post-submission result precedence,
caller-drop admission retention through native completion, and Linux proc-stat
scratch reuse;
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
