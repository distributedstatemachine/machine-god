# Managed `subagent`

The provider-neutral tool validates complete managed-agent commands and results.
An explicitly injected `ManagedSubagentAuthority` owns native admission,
durable acceptance, scheduling, persistence and lifetime. Core performs no
ambient filesystem, process, environment, network, clock or thread operations.
The foreground-only API and its global counters are removed.

Factory preparation explicitly selects `Create` or `Restore`; the presence of
captured caller authority never chooses between them. Create cannot reuse an
existing transcript; restore cannot publish an empty replacement. Explicit
resume, reopen and nonresident message preparation carry the exact admitted
actor's captured workspace, policy and preferences independently of this kind.
Prepared production children also return their actual session-bound notice
context; the outer manager registers it weakly for source acknowledgement and
outbox cleanup. It creates neither an additional runtime owner nor an idle turn.

## Commands

The control journal retains immutable controller transcript identity and the
exact optional parent transcript identity alongside its public parent label,
including empty persistent children. Reparenting updates the paired relationship
fields, not the original controller. These durable labels do not replace actual
native-call admission. Idle cancellation still publishes intent before its
separate durable settlement; it cannot leave an empty FIFO permanently blocked.
Exact typed notice envelopes are immutable pageable journal records, preserving
source/work generations, target relationship, source sequence and interval gaps.
Their journal confirmation precedes visibility in the parent notice inbox.

Source journals also retain exact acknowledgement identity, original target and
parent prompt-checkpoint evidence. This maintenance can append to an archived
source without reopening it or executing work. The manager must first establish
the original immutable notice and confirmed parent checkpoint; an arbitrary
label, ordinary record readback or current child status cannot establish delivery.
One notice publication occupies its own source sequence and advances the durable
publication cursor; acknowledgements do not synthesize notices or timers.
Suppressed start/milestone observations consume a control-only source checkpoint
before a later occurrence can reuse the journal's next sequence; they create no
visible notice. Confirmed manager writes invalidate replay's exact-head read
cursor, without treating ordinary cursor staleness as ambiguous publication.
Replay and source acknowledgements validate the parent transcript incarnation
against the historical control record at the original relationship revision,
not the current parent or a reused display ID/generation.

The root is `{"command":{...}}`, selecting exactly one of these branches.
Every object rejects unknown fields. Optional fields are omitted, not null.

| Branch | Fields |
| --- | --- |
| `create` | Required `name`, `mode` (`one_off` or `persistent`); `prompt` required for one-off. Optional `model`, `effort`, `permission_mode`, `notifications`. |
| `inspect` | Required `id`, nonempty distinct `sections`; optional `cursor`, `limit`, `wait`. |
| `message` | Exactly `send:{id,content}` or `milestone:{name}`. A milestone belongs to the actual calling child/work, not a supplied target ID. |
| `relationship` | Required `action`, `id`; optional `parent_id`. Attach defaults to actual actor; detach forbids parent; reparent requires parent. |
| `configure` | Required `id` plus at least one of `name`, `model`, `effort`, `permission_mode`, `notifications`. |
| `lifecycle` | Required `id`, `action`: cancel, resume, close or reopen. |

Names/milestones are 1–128 UTF-8 bytes; model 1–256; prompt/message 1–65,536.
NUL is rejected. IDs contain 1–255 ASCII alphanumeric, dot, underscore or hyphen
characters, excluding dot and double-dot. Effort is `auto` or a 1–64-byte name
using ASCII alphanumeric, dot, underscore or hyphen. Explicit model/effort wins;
otherwise native uses admitted selected preferences.

Permission modes are ask/auto/yolo. Omission remains explicit `None` in the DTO:
native inherits the admitted parent policy and enforces same-or-stricter policy.
There is no decoder-default yolo authority. Children receive standalone prompts,
not inherited parent transcripts or grants.

Notifications default terminal completed/failed/cancelled on, started off,
milestones empty and stop conditions `[terminal]`. At most 32 distinct
milestones and eight distinct stop conditions are accepted. Conditions are
terminal/duration_elapsed. Positive signed-millisecond interval/duration values
are checked; duration requires interval and adds duration_elapsed if absent.
Native must also check deadline addition against its actual clock.
The `started` boolean is a deliberate modern extension to the pinned tool.
Each accepted work item freezes its policy; native notices feed the parent's
next-turn context, never an automatic idle turn or active-turn injection.

## Inspection and results

Sections are status, messages, tool_activity, events, configuration and
relationship. There is no additional history or list command. Messages includes
conversation history. Limits default to 50 and range from 1 to 100.
The complete selected page contains at most 100 message/history/event/tool items.
`v1:generation:offset` cursors use canonical checked unsigned integers; native
validates the exact generation and projection and reports gaps/restart-required.
Retained history and tool activity are pageable, not pinned first-page-only
projections. Retention is not a lifetime child-creation limit.

Wait requires `until:"settled"`, timeout_ms from 1 to 60,000, the status section
and no cursor; optional after_generation requires a strictly later generation.
Idle/interrupted/completed/failed/cancelled/archived are settled;
queued/running/awaiting_approval are not. Timeout returns an inspection with
wait_timed_out status, not durable cancellation. Native owns dependency-wait
admission, cycle detection, released execution quota and bounded waiters.

`ManagedSubagentResult` preserves the envelope: ok, operation_id, child_id,
status, error_code, retryable, requested and cursor. Status/error codes are closed
typed tags, never raw host diagnostics. Requested data is a receipt, inspection
or relationship approval. Receipt outcomes are created, message_queued,
relationship_changed, configured, lifecycle_changed and milestone_emitted;
they are not completed child answers. Inspection includes state/configuration/
relationships, messages/history/events/tool activity, generation/cursor,
truncation/gap and bounded source-error projections. Private policy and inherited
root-user evidence are not public message fields. Child text remains untrusted.

Tool arguments are bounded to 448 KiB serialized, 12 container levels and
512 JSON nodes, including worst-case escaping of the full prompt. Output is
bounded to 512 KiB; history fields are at most 16 KiB each and 32 KiB together. Native composition
must provide per-tool complete input/output publication when ordinary engine
transcript bounds are smaller; unrelated tool limits must not be enlarged.
Core checks JSON bounds before recursive decode and destroys rejected deep JSON
iteratively.

## Actual invocation and native authority

Public `ToolContext` IDs grant no authority. Core supplies an
`AdmittedToolInvocation` only through `Tool::execute_admitted`, after the exact
prepared capability and optional final permission admission succeed.
Its immutable arguments, registered name and private call allocation bind the
actual invocation. `SubagentTool` constructs `ManagedSubagentInvocation` only
from this envelope; direct structural execution fails closed.

`Session::witness()` and `Turn::witness()` expose weak opaque allocation
identities, without constructors or deserialization. Native registers an actual
principal against the session witness before execution and owns a turn-scoped
registration. It may route using IDs but must verify actual session/turn identity,
liveness, principal generation and same-or-stricter policy. Calling
`ManagedSubagentInvocation::claim(&TurnWitness)` consumes a unique call claim
once. Foreign/stale witnesses and repeat claims fail. Provider call IDs are not
claim identity. Proofs retain neither session nor runtime ownership.

Wrappers must forward the admitted envelope unchanged. The ordinary default
adapter discards its proof when invoking structural tools; it cannot be used
around a managed tool. Native privately constructs its admitted principal/run
lease only after checking the witness and authority. Core identity alone is
never a native permission or resource grant.

## Principal isolation

Native registers each principal from an actual session's weak allocation witness,
with a nonzero private generation. The registry admits at most 64 resident
routes (configurable downward), not 64 lifetime creations. Dropped, retired and
dead-session routes can be reclaimed; retiring an old owner never removes its
replacement or a sibling. Duplicate live registration of the same actual session
fails closed, even when a caller supplies a different generation.

Each registration forks an independent workspace selection and creates a fresh
owner/generation-bound undo history under the host domain's shared undo budget.
It copies neither transcripts nor grants. The host supplies the selected immutable
permission policy and model/effort preferences at turn registration; mutable permission/grant and ephemeral
MCP lifetimes remain with the separate principal runtime, not in the registry.
The actual turn is registered before its provider is polled. Its workspace
snapshot, policy and model defaults remain pinned independently of later selection
changes. Omitted child model/effort defaults never query the displayed runtime.

Tool and UI reverse routes are weak. Candidate IDs are lookup hints only: call
admission verifies the live actual session owns the live actual turn, checks the
original generation, and consumes the core invocation's one-shot claim. Repeated
provider call IDs cannot replay a claim; different actual sessions with identical
public IDs cannot claim one another's calls. Managed child turns additionally bind
the exact weak scheduler run and its captured FIFO work generation, which is
distinct from the principal generation. Calls require current execution quota;
registration itself may precede scheduler acquisition.

The private nonclone admitted-call lease retains original workspace/undo/policy
resource custody, never a session, engine or conversation runtime. Dropping the
turn registration, cancelling/completing the actual turn, retiring the principal
or dropping the registry ends new authority without stealing already-owned
settlement resources. Host composition must retain the turn registration beside
the real turn and revalidate its admitted lease before new effects. Registry and
principal locks cover only short routing/admission updates, not filesystem effects,
callbacks or asynchronous waits.

## Cancellation and durable ownership

Create/message acceptance is durable before execution; user cancellation is
durable before signalling. Native serializes each child FIFO and explicitly
resolves interrupted/approval-blocked heads. Restart never implicitly executes
interrupted work. Accepted children are owned by the outer native manager and
outlive the creating tool future/turn.

Mutating tool submissions use completion-wins-after-first-poll: core observes
their real result and persistence before a pending turn cancellation. Native
still owns irreversible settlement if that future is abandoned. Inspect/wait
remain normally cancellable. Submission/wait cancellation and host teardown
are not durable user cancellation; only an admitted lifecycle command requests
that mutation. Persistent cancellation returns idle; one-off cancellation is
terminal. Close archives/settles, not deletes; reopen does not implicitly retry.
The manager owns aggregate scheduling, residency, queue, byte and waiter budgets;
core has no foreground admission counters or detached worker loop.

## Native scheduling and actual settlement

The shared native scheduler validates independent execution, resident and waiter
limits. Defaults are 4 executing runs, 64 resident principals and 64 queued or
dependency-waiting runs. All limits are positive, execution cannot exceed
residency, and hard ceilings are 256 executions and 4,096 residents/waiters.
These are live-resource budgets, not lifetime creation or durable-history caps.
Ordinary provider `Pending` retains execution capacity. Acquisition and
reacquisition use one FIFO queue; a grant reserves capacity before its future is
observed, and new work cannot bypass existing queued work.

Only the native manager registers an actual core turn against a non-clone
resident lease and a strictly increasing work generation. Opaque weak run
references cannot be constructed from public IDs and retain no manager/runtime.
Private managed-call admission must bind the actual turn and generation before
using a run reference. Register before polling the provider, acquire before
execution, and never use a principal residency epoch as a work generation.

A dependency wait targets an exact registered run. Self-dependencies, cycles,
foreign or unregistered targets and exhausted waiter capacity fail before quota
release. Under one short lock, the scheduler reserves a waiter and records the
dependency before releasing caller execution capacity. Observation success,
error and timeout all queue for fair reacquisition before the tool returns;
their outputs remain private until then. Cancellation or abandonment unlinks
the exact waiter/grant and cancels the actual caller turn instead of permitting
quota-free continuation. An unpolled wait is inert. No generic pending future
is interpreted as a dependency, and no thread/task/timer is created per child.
Native observation futures must likewise use weak manager/runtime access;
wrapping an owning future does not erase its ownership.

Execution completion and actual worker settlement have separate non-clone
owners. Finishing, cancellation or run-owner drop releases execution capacity
and enters a bounded settlement lane, which progresses even when all execution
slots are occupied. Only the actual finalizer may complete its settlement owner
after turn/worker/TLS/process-reap obligations finish. Dropping a response
observer proves nothing about settlement. Dropping the settlement owner without
completion quarantines its resident slot; dropping a resident with unsettled
work does not make that capacity reusable. A settled resident can admit its next
FIFO generation; a retired settled resident releases its capacity.

Registry locks never cover asynchronous waits, cancellation callbacks or caller
waker clone/wake/drop operations. Cancellation and removed-value destruction
run after unlocking. This scheduler supplies admission and lifecycle primitives;
native manager composition owns durable acceptance, deadlines, scheduling polls
and actual finalizer custody.

The native conversation binding registers its actual core session with a
separately retained manager owner. It forks that principal's workspace selection
and selects its owner-local undo tracker before runtime construction. Each
checkpoint's increasing actual turn sequence supplies the scheduler work
generation, independently of the principal lifetime generation. Admission
requires the taken immutable permission and model snapshots; a missing snapshot,
expired manager owner or unsettled previous run rejects before a new checkpoint
publication. Turn registration precedes provider polling, and initial execution
quota is acquired exactly once. Dependency waits retain their own fair
reacquisition without a second admission from the outer conversation poll loop.

Normal turn execution enters settlement before forwarding native finalization,
but does not complete the separate worker/TLS/reap obligation. The manager can
transfer that settlement owner without freeing the resident or admitting another
turn. Manager-owner retirement cancels the original actual turn even after that
transfer; a weak conversation binding cannot keep management authority alive.

### Per-run native cleanup

Each actual run receives a non-clone cleanup cohort under the shared host worker
scope. A host admits at most 64 open or unsettled ordinary cohorts and separately
reserves 64 settlement-only cohorts. Ordinary work cannot borrow this cleanup
reserve: shutdown must be able to drive retained MCP startup settlement even
when all ordinary slots are occupied. Both classes use the same worker and
journal-custody machinery, without another thread or pool. Closed, settled cohorts
release capacity even when old completion observers remain. Poll attribution is
weak, allocation-bound and restored on return or unwind. Worker futures capture
their original attribution before polling; later callers cannot retarget them.
An explicit run poll also takes precedence over an unrelated worker's ambient
attribution when an embedded driver queues effects from inside that worker.
Implicit attribution to a foreign host fails closed. Closing or dropping a cohort
rejects new work but does not discharge already-owned cleanup.
An inert cleanup-capacity observer uses the existing panic-safe wake primitive;
it wakes after actual capacity refund or host closure, which returns a fixed
error. Observation grants no slot: callers recheck admission under the original
unchanged deadline. Completion observers and abandoned waits consume no cohort
capacity and cannot prevent reuse.
Trusted nested service construction may explicitly verify the original source
scope and enroll a target scope in that same run cohort; it cannot infer this
relationship from public identities. A weak, non-clone service-handoff token
addresses one original worker enrollment only. Promotion requires a confirmed
successful service receipt; dropping or losing the token has no effect.

The existing collector retains both host and run tickets through actual dedicated
worker join, including thread-local destruction. Queued terminal callbacks and
fixed background-pool operations carry their original cleanup obligation across
thread boundaries. Cleanup snapshots follow failed startup and deferred positive
reaping independently of returned values, cancellation and abandoned observers.
Completion observation uses the existing wake-driven primitive, without a new
thread, pool, timer or worker admission per child. The manager binds the cohort
to the exact scheduler run and checks runtime settlement separately.

The actual native turn restores this attribution for every stream poll and for
destruction. Core/provider/tool and native finalizer destruction happens before
the cohort closes, so cleanup admitted while a stream is being dropped remains
charged to the original run. Cleanup lookup requires that exact opaque run
allocation; a sibling or superseded run cannot borrow another run's receipt.
An empty unpolled turn closes its cohort without starting provider work.

Attribution begins at actual native admission, before runtime skill/resource
materialization, MCP readiness/authentication and checkpoint publication. The
same cohort transfers to the resulting core turn. Failed or dropped admission
closes only after its future and checkpoint finalizers are destroyed under that
attribution. Its independent completion remains observable even when no core
turn or run reference was minted, and blocks another admission until settled.
Post-publication registration failure retains its scheduler settlement instead
of treating an error response as proof that checkpoint cleanup has finished.

The manager may bind its journal-owner lease as one bounded cohort keepalive,
never a runtime, session or manager reference. Every admitted host worker ticket
retains that lease independently, and transferred cleanup retains the exact host
ticket. Successful service promotion refunds run capacity without releasing the
journal owner before the service's actual collector join/TLS/reap settlement.
The open cohort also retains the lease; closure and actual run settlement release
that reference outside locks, so old completion observers cannot keep it alive.

A successful handoff to a retained service removes only that owner's run ticket,
never its host ticket or another cleanup snapshot. Terminal processes cross this
boundary after the final startup acknowledgement and successful artifact cleanup;
background processes cross after successful retention activation immediately
before transfer to the preadmitted retainer. The lazy terminal owner crosses after
successful initialization, before its long-lived loop. MCP stdio crosses only
after the exact native executable catalog publication,
not raw process launch, protocol initialization or private candidate preparation.
The original owner/child worker enrollments and process-reap cleanup transfer
together on that child's I/O worker; the existing bounded poll loop observes the
success receipt without another worker. Rejected, stale, abandoned or failed
publication leaves startup cleanup attributed to its original run. Launch futures
capture weak construction-time attribution, including an unbound caller, and
cannot adopt a later polling run.
On macOS, handoff includes the exact inventory child prepared for that process,
without changing the shared inventory's host-scope identity or dropping its reap
custody. A delayed receipt cannot promote a replacement inventory child. Once a
lease is retained, a successful later query can transfer the exact child selected
for that query; failed queries keep their original run cleanup. These transfers
and completion callbacks occur outside inventory locks.
Healthy retained processes therefore do not block a child's next FIFO item.
Failed initialization, startup,
acknowledgement or cleanup keeps the original run obligation. Reusable pool
threads remain service-lifetime resources; their individual operations settle
after execution and result publication, while dedicated worker tickets still
require actual join/TLS completion. macOS inventory identity remains host-bound.

## Managed command mailbox

The shared engine retains only a weak native mailbox requester and weak principal
registry route. Constructing its request future is inert. First poll checks
submission cancellation and claims the actual invocation exactly once through
the registered principal/turn; public IDs cannot supply this admission.
The outer manager polls one FIFO and owns each dequeued command, admitted lease,
original context, cancellation token and reply. A queue entry is not durable
acceptance or authorization to execute after its original admission retires.
The manager retains a Busy/Limit head job without bypassing it; only confirmed
journal acceptance may schedule effects.

One aggregate mailbox budget covers queued commands, dequeued in-flight jobs and
completed replies with slow observers. Defaults are 64 requests and 256 MiB of
reserved capacity; configurable bounds allow 1–256 requests and 4 MiB–1 GiB.
Each request reserves 4 MiB before publication, covering the admitted JSON,
typed command, bounded response, normalization scratch and bookkeeping without
eagerly allocating that amount. Completion validates the full core result bounds
without truncation and normalizes caller-controlled spare allocation capacity.
Command and response custody overlap under the same reservation. Charges survive
observer loss until actual mailbox-owned payloads drop; on successful reply poll,
ownership transfers to core's separately bounded result codec and archive path.

Dropping a reply or cancelling its submission token does not discard an admitted
mutation job or imply durable child cancellation. The manager decides whether a
read-only request may be withdrawn. Closing/dropping the mailbox rejects queued
and unfulfilled observers; already completed replies preserve their exact result.
Already dequeued mutations retain manager-owned
settlement custody and resource charges. Requesters, budget progress handles and
reply registries use weak reverse links, with no queue/engine ownership cycle.
Callbacks and payload destruction run outside queue, reply and budget locks.

## Native manager and runtime factory

The outer manager, not a creating tool future or selected UI page, owns child
conversation runtimes and their separate native principal/scheduler owners.
Its injected runtime factory shares the existing host execution domain and
receives an exact transcript candidate, captured configuration and explicit
timestamp. Fresh preparation receives the original admitted principal custody,
workspace snapshot, policy and model preferences; asynchronous preparation never
re-reads the parent's mutable workspace selection. Identity allocation is inert
before poll and its chosen candidate survives preparation/publication failure.
Preparing an empty transcript reserves runtime/residency but never polls a
provider or executes a model-facing tool before durable create acceptance.

Indeterminate factory preparation retains its exact receipt. Reconciliation
errors preserve custody and cannot trigger another identity allocation or blind
retry. Prepared resources receive and retain the journal owner lease through
actual attributed worker, TLS and process-reap cleanup, including abandonment
of the manager observer. Runtime event completion is not actual cleanup;
independent per-run settlement and final principal closure must be confirmed.
Failed readiness/auth/checkpoint admission also has an independent cleanup
receipt, even before an actual run exists. The manager never treats a missing
or previous run reference as proof that this newer admission has settled.
Attach/reparent authorization uses an injected human endpoint carrying the exact
actor, target generation/revision and old/new parent proposal. Supplied labels
or permission mode alone never authorize that relationship change.

The manager polls its mailbox, child streams, actual cleanup and notice delivery
independently of terminal output. A captured control head pauses only its target's
new stream observations while existing writes drain; siblings continue. Human
relationship approval and inspect waits retain bounded jobs outside the global
journal operation slot. Configuration changes affect future accepted work only.
Each queued work retains its own model, effort, permission mode and normalized
notification policy. Runtime preparation for messages/resume/reopen uses the
current authenticated caller's captured origin and rejects policy escalation.

Journal publication, receipt reconciliation and execution admission have separate
states. Busy/limit pressure retains the original FIFO operation. Ambiguity parks
on explicit retry with its original proposal and resources; capacity becoming
available does not automatically retry an uncertain publication. Cancellation
intent precedes the signal, and ordinary lifecycle observers wait for settlement.
A child cancelling/closing itself receives the confirmed intent receipt before
its own completion-wins tool returns, avoiding a dependency on its own terminal
event; the manager retains the remaining settlement independently.

Native navigation uses weak allocation-bound selections, not control authority
derived from public labels. Idle, actually settled runtimes may be evicted to
admit another child; their immutable journal history remains pageable. Inspection
cursors bind exact source, revision and selected sections, with bounded raw scans
and one shared encoded result budget. A changed or retired cursor asks the caller
to restart rather than silently changing the requested projection.

Restore explicitly repairs a saved original notice outbox before child execution;
factory preparation itself never performs delivery recovery. Close waits for the
resident's exact source acknowledgements and outbox clear. Reopen first prepares
the old generation, repairs and acknowledges its original delivery, clears the
outbox and confirms actual old-resource closure; only then may it prepare and
publish the new generation. No envelope or checkpoint is retargeted.

History replay holds one catalog/source frontier and one candidate, pages through
nonresident histories, and admits only typed confirmed originals for an exact
registered target. It checks source acknowledgement records, excludes archived
or replaced source generations, and neither starts timers nor resumes execution.
Confirmed source acknowledgements remove exact replayed originals before an
outbox clear permits another snapshot. Parent registration restarts this bounded
scan so an absent or retired target never requires an unbounded in-memory queue.

## Durable journal

The native managed control journal is separate from `FileSessionStore`, which
continues to own conversation transcripts. A journal receives an already-open
private directory descriptor and an explicit `NativeOwnedWorkerScope`; it does
not discover paths, reopen directory authority by pathname, or perform model,
process, browser or conversation effects. Construction and unpolled operations
are inert. All journal filesystem work runs on the injected owned workers.

An exclusive descriptor-relative owner lock is separate from head compare-and-swap
revisions. Another manager receives Busy, even in the same process. The outer
manager retains the journal's owner lease through actual child/control/worker
settlement, not merely result observation. Each exclusive open durably advances
a checked ownership epoch. Old-owner heads require explicit recovery before
mutation; recovery persists pending/running/approval work as interrupted without
execution or signalling. Current-owner live work cannot be reset through recovery.

Schema-v1 heads retain child identity, owner epoch, lifecycle generation, checked
revision, mode, selected configuration, transcript binding, parent relationship,
FIFO work references, failure head, cancellation/archive intent and notice cursor.
Full accepted work contents and frozen configuration/notification policy live in
immutable pages, not in a 64 KiB head or a copied child transcript. Pages bind
the exact session/incarnation owner, child/generation/sequence, encoded length
and SHA-256 digest. Their back-links strictly decrease sequence. Accepted work
retains its original source principal and frozen policy without deriving fresh
authority from those labels. The digest detects inconsistent content,
not authenticity, encryption or protection against a writer with equivalent
filesystem authority; those M04 concerns remain separate.

Create and enqueue return confirmation only after immutable pages are written,
file-synced, renamed, directory-synced and validated before the referencing head
is published with the same durability sequence. Head CAS retains the original
opened file revision and rejects equal-byte inode replacement, stale/foreign
observations and modified snapshot contents. Public child/work IDs remain labels,
not execution or process authority. Native run admission remains separate.

Every head publication appends typed immutable control evidence. Accepted work,
state transitions/resolutions, configuration, history, events and tool activity
are pageable. Milestone operation labels retain core's bounded non-whitespace,
non-control syntax rather than being narrowed to child-ID syntax.
FIFO state changes name the exact first work item; interrupted,
failed or approval-blocked heads require explicit resolution. Cancellation intent
is durable before the manager may signal. Persistent cancellation returns idle
and leaves later accepted work interrupted; one-off cancellation is terminal.
Close records archive intent, settles active work and archives without deleting
history. Reopen advances generation and never implicitly retries queued work.

Before publication, one shared reservation covers old/new/temp files, encoding,
decoding, recovery and retained reconciliation receipts. Defaults are 256 MiB
aggregate accounting, 128 KiB encoded heads, 1 MiB encoded pages, 64 queued items
per child and 65,536 directory entries. Explicit limits allow heads up to 1 MiB,
pages from 512 KiB through 8 MiB, up to 256 queued items, up to 1,048,576 directory
entries and aggregate accounting up to 4 GiB; aggregate capacity must also cover
the configured operation reservation. Counts, identity/string bounds and bounded
list deserialization cover structural overhead. Full 65,536-byte messages and
32 milestones remain supported, including JSON escaping. Serialized size limits
reject without truncation.

The operation slot returns Busy rather than blocking the native event loop;
the manager retains an unaccepted FIFO submission for retry and never reports
Busy as accepted work. After any uncertain publication, the journal retains the
exact candidate and reservation even if its result observer disappears. Further
ordinary operations reject while that receipt awaits reconciliation. Reconciliation
repairs directory/file durability and validates the exact referenced state;
readback alone is not confirmation. It reports Confirmed or NotApplied, or retains
ambiguity on failure. A page-only orphan cannot authorize execution of an
unpublished head, and reconciliation never automatically republishes a candidate.
Unreferenced immutable pages and stale temporary files remain charged. Reusing a
known temporary name removes only that validated private temporary descriptor;
heads and immutable histories are not deleted by close or reopen.

Exclusive reopen reconstructs accounting from the bounded directory inventory,
including old, orphan and temporary files, before new acceptance. Payload-free
catalog pages discover nonresident and archived children without pinning all
heads. History pages touch a bounded number of immutable pages and return at most
100 records and 512 KiB of encoded projection data. Opaque continuation cursors
bind the exact journal/snapshot; stale cursors require a fresh projection.
Returned observations/pages are caller-owned bounded values, not additional
execution leases. Storage and residency pressure are explicit bounds, not a
lifetime count of children created.

## Native notifications and deadlines

Stopped tracking and queued payload custody have separate lifetimes. Reclaiming
a stopped tracker does not remove immutable queued originals or refund their
count/byte reservations, including slow snapshots. Exact source retirement can
invalidate those records after the tracker is gone; private durable-ACK repair
removes only the original envelopes, without manufacturing batch ACK tokens.

The native notice component freezes the normalized notification policy for each
accepted work item. Actual start begins its checked monotonic interval/duration
schedule; acceptance alone starts no timer. Started notices are optional,
declared milestones are emitted once per work/name, and completed/failed/cancelled
notices default on and are emitted at most once per work. Disabled terminal
delivery still applies the terminal stop condition. Duration expiry stops
periodic reporting before a report at that same observation, without inventing
a duration event; independently enabled terminal delivery remains available.
An explicit stop prevents new emissions. Close also invalidates pending context
snapshots; durable history custody must already exist before removing those
pending projections.

Preparation reserves one charged, immutable staged notice and proposed cursor
transition per work item. Staged records are absent from snapshots and cannot
be acknowledged. The manager appends that exact original as a journal notice
record; only confirmed page/head durability exposes it to parent context.
Dropping the staged observer neither publishes nor discards it. Ambiguity keeps
the original envelope, relationship, history reference, tick range and proposed
deadline transition; recovery retrieves that same candidate, not a regenerated
observation. Conflicting preparation is Busy until the original resolves.
Only an explicit journal NotApplied result may discard the candidate without
advancing its cursor. Confirmation and discard reject foreign or stale tokens.

Staged payloads and transition storage count against aggregate retained notice
bounds, including after token loss or source close. Work release remains Busy
while staging custody is unresolved. A pending staged transition suspends that
work's deadline eligibility, avoiding a repeated due wake during journal
ambiguity; resolution restores the original schedule without inventing elapsed
observations. Reparent changes only future targets. Stop, close or target
retirement prevents a later confirmation from exposing that staged candidate
and never revives a stopped timer; its confirmed journal history remains intact.
Already-visible notices keep the existing stop/close behavior. Exact restored
durable originals use a distinct replay path that starts no timers or execution.

Every interval is one actual observed state with an exact first/last tick range,
coalesced interval count and explicit gap flag. Late observation never synthesizes
missed state transitions or individual reports. All duration conversion,
monotonic addition, tick arithmetic and capacity checks precede advancement of
the corresponding emission cursor. State-observation cursors are separate from
actual source-event cursors, so observing a terminal snapshot cannot consume its
still-required terminal event notice.

Notice identity combines the original source principal ID/generation, work
ID/generation, supplied durable source-event sequence and typed notice kind;
interval identity includes its exact tick range. A source sequence means the
manager's exact persisted event sequence, not a page number that can contain
multiple records. Bounded opaque history references identify journal records,
not filesystem read authority. Reparent/detach changes the relationship used by
future emissions. Each already-pending notice keeps its original parent and
relationship revision; parent retirement invalidates only that target's pending
context. Weak work references retain no principal, runtime or manager ownership.

Default bounds are 64 trackers, 256 retained notices, 1 MiB retained notice
bytes and 8 KiB per encoded notice. Configurable hard ceilings are 4,096
trackers/notices, 16 MiB retained bytes and 16 KiB per notice. Retained accounting
includes fixed record storage and payloads pinned by old batches, including
already-acknowledged records. A rejected insertion does not silently advance
the source event or timer cursor. Stopped/terminal trackers are reclaimable only
after pending notice custody has been retained or explicitly closed; history is
not a lifetime child-count cap.

Snapshots are non-consuming, bounded to 64 records/64 KiB and round-robin across
source works while preserving each work's order. They report remaining data when
the caller's remaining context budget cannot fit it. An opaque exact-token subset
acknowledgement removes only those original records; new arrivals, sibling
principals and unrelated records survive stale, repeated or foreign receipts.
The manager validates a batch immediately before serialized context/checkpoint
admission. An in-memory acknowledgement is not durable delivery: root composition
must first retain the exact confirmed checkpoint/cursor and only then acknowledge
its accepted subset. Skill, MCP and notice context share the root's 64 KiB
checkpoint bound. Notices enter a future explicitly started parent turn, never
an active-turn injection or an automatic idle turn.

Explicit replay accepts only bounded original persisted identities and payloads,
without retargeting, inferring events or starting timers/execution. Root restores
only unacknowledged journal records against their original work/source high-water
and keeps recovered work stopped until explicit resolution. Private batch tokens
are process-local allocations, not another durable sequence or persisted authority.

One manager-level deadline future uses the explicitly injected paired
`NativeMcpRuntimeClock::now`/`sleep_until` boundary. It owns at most one sleep,
retargets when the earliest interval/duration changes, and releases the timer on
cancellation, drop or removal of every deadline. There is no child thread, task
or timer. Clock calls, timer destruction and caller waker operations occur outside
the notice registry lock; the deadline observer is weak and performs no notice
emission itself. Root composition supplies actual state observations and owns
journal publication, wake-driven polling and lifecycle integration.

## Per-principal MCP routing

The actual native conversation owner binds its principal to one weak MCP owner
before starting work. Each actual turn receives its own MCP guard before the
provider is polled; finish and drop release that exact guard before its principal
turn guard. Missing, retired or mismatched bound owners reject admission rather
than selecting another runtime. The outer manager retains the owning MCP bundle
and cleanup obligations separately from these weak conversation bindings.

The shared engine installs authenticated wrappers for MCP search, selection and
native features. Before provider polling, the host registers each actual
principal turn and its independently composed MCP runtime/context selection.
Bounded resident routes contain weak principal, runtime and turn references;
public context IDs only find a candidate. Non-consuming metadata stamps bind
the exact live turn allocation and principal generation, but do not authorize
execution. Execution consumes the core invocation once and retains the native
principal lease through the original operation. Structural execution fails closed.

A runtime allocation cannot belong to two live principal owners. The factory
constructs each concrete native feature tool from the exact registered runtime
and shared result archive, preserving full output, durable projection,
next-round executable and finish-turn receipts without a portable-payload
conversion. An optional portable feature authority is a separately explicit
trusted injection, not the production feature path. Children inherit only
explicit factory configuration, never another principal's live connections,
grants, elicitation or ephemeral runtime owner.

Snapshots and executable registrations keep the existing MCP generation and
submission custody; reload never retargets an old selection. Dropping a turn
route prevents new use. Retiring a principal MCP owner, or dropping the registry,
synchronously invalidates its exact runtime before releasing the resident
route, without closing siblings. This is not cleanup completion: the outer
manager still retains and drains that runtime's controller, ephemeral owners
and actual worker/reap obligations. In-flight operations may retain original
resource custody after retirement, but cannot obtain new authority. Reverse
requesters and unpolled operations do not retain a session or runtime. Future
construction captures only weak turn-route and publication identity; first poll
rejects a replaced route/publication rather than resolving public IDs again.

Shared host composition retains the original helper-bearing builtin preparer and
reviewer as immutable factory inputs; descendants do not rebuild those authorities.
A child MCP construction receives fresh contexts, runtime, controller lifetime
cancellation and permission bundle. The configuration seed retains no parent's
runtime or request-scoped ephemeral server selection; closing a child cannot
cancel a sibling's controller token.

Permission preparation uses a factory-owned typed bundle. Construction validates
the exact runtime/context allocation pair and builds the existing concrete MCP
preparer with the original builtin/helper-bearing preparer, review contexts,
reviewer and workspace selection. Registering that bundle against any other
runtime fails closed. The outer manager retains the bundle/controller; principal
routes hold only weak preparer references. Missing bundles do not fall back to
another principal or an engine-global preparer.

Shared permission dispatch captures the original weak turn/publication before
polling and forwards the original request and prepared arguments without claiming
the tool invocation. The returned action and final execution admission recheck
the exact route while preserving the underlying native proof. Retirement before
final admission prevents execution; completed effects are not reinterpreted.
Turn guards retain frozen lookup keys solely for targeted cleanup, so cancellation
or principal-guard destruction cannot prevent release of original unclaimed
permission proofs. Shared ID-only `close_turn` callbacks are non-authorizing
no-ops: even one current candidate could be a replacement allocation targeted
by a delayed old callback. The manager drops the exact MCP turn guard on
finish/cancel before finalization; that guard and owner retirement close only
their original preparer, not a replacement. This does not prove worker or
runtime settlement. Cancellation,
preparer callbacks and proof destruction run outside routing locks.

## Parent prompt checkpoints

Native conversation admission binds a weak notice owner to the actual session
allocation. New prompts snapshot notices under the existing admission lease;
ordinary continuation reuses only the saved inert context, leaving later notices
queued. A retained uncertain publication uses the explicit guarded continuation
path. Core checkpoint confirmation acknowledges the selected originals before
subsequent native route registration; normal completion removes the saved notice
context alongside skill/resource context, while interrupted work retains it.
History and paused-turn validation apply the combined context bound as well.

The native prompt-context guard reserves one bounded preparation slot for an
actual parent session allocation, not just its public IDs. It snapshots notices
without consuming them and combines skill, resource and notice text within
65,536 UTF-8 bytes, including every separator and the notice JSON framing.
Only a fitting original subset is selected; remaining notices stay pending.
The prepared prompt metadata retains those original envelopes, their exact
source identities and the session/incarnation/revision/turn/user-message
checkpoint binding. Notice checkpoint metadata and its delivery outbox together
have a 192 KiB encoded ceiling; the configured core limits still apply to the
complete session metadata.

Publication wraps the actual core `prompt_prepared` or explicit
`continue_turn_prepared` operation. It checks the actual session witness, exact
prepared metadata/text and live notice batch before polling the core operation.
No parent-state mutex spans persistence or caller callbacks. Only the core's
confirmed successful publication acknowledges the selected original tokens,
before later native turn registration can fail. Arrivals after the snapshot are
not acknowledged. If a source in the retained snapshot closes or the parent retires during
publication, a confirmed checkpoint still receives its exact acknowledgement,
but the newly created turn is cancelled before it can be supplied to the caller.

Dropping an unpolled preparation leaves notices pending. An error or drop after
publication starts retains one bounded uncertain slot and the original notices;
stored-record readback is not confirmation and cannot clear that fence. An
explicit continuation may reuse exactly matching saved originals, and only its
newly confirmed core publication can settle the fence. Dropping that retry
preserves uncertainty. Ordinary saved continuation context is inert text and
original identity data, never a new snapshot or a receipt. Root composition owns
paused-checkpoint eligibility, actual parent lifecycle admission across the
operation, and durable journal/checkpoint reconciliation. The guard neither
starts idle parent turns nor injects context into an active turn.

### Durable parent delivery outbox

The same prepared core prompt checkpoint also saves one separate delivery outbox:
at most 64 original envelopes and 64 KiB encoded, with the original parent,
source identities and checkpoint evidence. Normal finalization removes only the
prompt context, not this outbox. Until the batch's source-journal acknowledgements
are confirmed and its outbox is explicitly cleared, further notice ingestion is
blocked; ordinary prompts and inert continuations remain available. No lifetime
history or child-creation counter is added.

Only a confirmed original core publication yields a live delivery receipt.
Restart decoding is inert: explicit recovery, under actual parent lifecycle and
session admission, must confirm a revision-pinned unchanged-metadata save before
yielding a recovered-original receipt. That save repairs durability without
creating a new prompt, turn, notice, replay, or timer; the original checkpoint
evidence remains unchanged. It cannot settle a still-live uncertain prompt slot.

The manager verifies each exact original source notice before durably recording
its source acknowledgement. Private subset confirmation updates only that
receipt's bounded progress bits; stale, foreign or invalid subsets cannot clear
another batch. After every original is confirmed, an explicit admitted metadata
save removes only the outbox. Recovery/clear errors and dropped in-flight futures
retain an exact bounded fence; neither readback nor an unrelated ordinary prompt
silently settles it. A committed-but-unconfirmed clear may be repaired explicitly
with another confirmed save even when readback already shows the key absent.
Receipts hold weak parent/session authority, not a runtime ownership edge.

## Source evidence

The pinned FX decoder is `src/tools/agent/subagent.zig`; typed validation and
bounds are in `src/core/subagent/domain.zig`. Result envelopes are in
`tool_result.zig`, encoding in `tool_host.zig` and inspection projections in
`manager.zig` under that subagent directory. The Rust contract intentionally
adds started notifications, complete durable history/tool paging, allocation-bound
identity and checked clock-compatible durations without retaining historical
operation-identity or foreground compatibility paths.
