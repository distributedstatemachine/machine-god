# Provider-neutral core API

`machine-god-core` is an embeddable, executor-independent streaming engine. It
contains no filesystem, process, environment, credential, clock, randomness, or
network access. A host supplies every authority-bearing component explicitly
through [`EngineBuilder`](crate::EngineBuilder).

The traits use boxed standard futures and `futures-core::Stream`, so core and
custom injected implementations may use Tokio, async-std, smol, a custom
executor, or direct polling. A concrete implementation may document a narrower
host requirement; the optional native AI Gateway HTTP transport requires a
host-owned Tokio runtime. All public extension traits are object-safe, `Send`,
and `Sync`.

## `terminal` process capability

The native [`terminal` tool](terminal.md) extends the existing
provider-neutral `Capability::Process` with exact `working_directory` and a
`ProcessEnvironment { profile, sha256 }` identity. Core receives strings and a
digest only; it gains no process, filesystem, environment, timer, or executor
authority. Native effect-free preparation fixes `/bin/sh`, `[-c, command]`, the
authorized cwd, and a bounded environment digest before the existing
critical-risk authorization. Foreground `exec` uses the canonical
workspace-relative cwd and construction-snapshot digest. Noninteractive
`start` uses the absolute canonical workspace/cwd and the native supervisor's
fixed-environment digest. Allowed execution reparses the canonical arguments
and derives the same immutable identity from tool state. The environment
values, retained descriptors, background persistence, process group, pipes,
deadline guardian, threads, and cleanup remain wholly native.

Native rechecks cancellation after executor/guardian destruction and directly
before returning `ToolOutput`. Its timeout starts at first poll and is
independent of the executor around controllable userspace phases, but core makes
no wall-clock guarantee for a native thread blocked in filesystem, spawn,
kernel-wait, other uninterruptible host work, synchronous executor poll/drop,
or a Waker callback. Native checks cancellation first, then uses one linearized
close to order output-limit observation against timeout while preserving
validated status/counter invariants. One admitted execution owns one active
slot shared without another increment by the outer call, request/executor,
TerminalTool-wrapped task-Waker family, callbacks, and native worker/deadline
threads through actual return. The wrapper is supplied even to public injected
executors, which need no private counter authority. Retained requests or Wakers
and no-Waker native thread tails keep the same slot, so later admissions fail
fast at capacity. Public system construction is Linux-only; private non-Linux
reference-host composition retains the advertised tool and returns fixed
unsupported only after strict preparation, permission, and execution argument
validation, before cwd lookup or spawn.

## `web_search` boundary

The native [`web_search` tool](web-search.md) requires no new core
provider event, provider-tool advertisement, permission mode, or ambient
authority. The outer model receives an ordinary `ToolSpec`; its `ToolCall`
passes through the existing registered-tool, strict-round, effect-free
`prepare`, critical-risk permission, local `execute`, durable `ToolResult`, and
next-round lifecycle. Preparation supplies `Capability::Network` for the exact
configured Gateway target and canonical arguments that execution reparses.

The provider-executed Perplexity call occurs only inside the approved native
tool's injected transport adapter. A dedicated native one-shot codec decodes
that inner call/result and converts it to a bounded local `ToolOutput`. It does
not enter `ModelEvent`, `ContentBlock`, `ToolCall`, or `ModelRequest`, and the
ordinary AI Gateway `ModelProvider` continues to reject provider-executed
records. This keeps provider-specific wire identity and network authority in
`machine-god-native` while preserving core's provider-neutral local-tool
invariants.

```rust,no_run
use machine_god_core::{Engine, SessionId, SessionIncarnationId};

# fn configured_engine() -> Engine { unimplemented!() }
let engine = configured_engine();
let session = engine.create_session(
    SessionId::new("example").unwrap(),
    SessionIncarnationId::new("0198d2f9-ef9a-7d72-9c1d-6f6db8f3dd50").unwrap(),
).unwrap();
assert_eq!(session.id().as_str(), "example");
```

## Composition and authority

An engine cannot be built without a [`ModelProvider`](crate::ModelProvider),
[`SessionStore`](crate::SessionStore), and
[`PermissionHandler`](crate::PermissionHandler). There are no permissive hidden
defaults. [`EventSink`](crate::EventSink) is observational and defaults to
[`NoopEventSink`](crate::NoopEventSink). Tools are registered explicitly and are
looked up by validated [`ToolName`](crate::ToolName).

`Engine` debugging is structural: it reports a fixed `has_provider: true` flag
and tool count without calling [`ModelProvider::name`](crate::ModelProvider::name)
or formatting any provider-controlled value. Logging an engine therefore cannot
trigger provider code or expose a hostile provider name.

The permission decision is distinct from tool execution. A handler error never
means approval. [`Tool::prepare`](crate::Tool::prepare) is a synchronous,
effect-free preflight boundary where native implementations can normalize paths,
process arguments, and network destinations before presenting a
[`Capability`](crate::Capability) to policy. It is trusted host code and must do
only bounded, nonblocking work. An allowed execution receives the exact
arguments returned by preflight.

Preparation has an explicit provider-neutral authorization disposition.
[`PreparedToolCall::new`](crate::PreparedToolCall::new) and the default
[`Tool::prepare`](crate::Tool::prepare) mark an invocation as permission-required
and require policy authorization for its exact capability. A narrowly trusted
tool whose durable contract requires no policy-governed authority may instead use
[`PreparedToolCall::without_authority`](crate::PreparedToolCall::without_authority)
for that invocation. That explicit form skips `PermissionRequested`,
`PermissionResolved`, permission-ID construction, and
[`PermissionHandler::authorize`](crate::PermissionHandler::authorize);
it does not skip prepared-argument validation, cancellation, `ToolStarted` and
`ToolFinished`, bounded output validation, durable result replacement, or the
next provider round. Core never derives the disposition from model-controlled
arguments. Hosts must treat the no-authority constructor as a trust-boundary
assertion rather than a permission optimization.

[`EngineLimits`](crate::EngineLimits) supplies nonzero resource bounds. Defaults
allow 8 model rounds, 16 tool calls per turn, 4 calls per round, 1 MiB each of
assistant text and observer-visible reasoning, a JSON container depth of 64,
65,536 JSON nodes, 4,096 model events, 1 KiB of provider stop detail, 256 KiB
per user prompt,
256 KiB of serialized session metadata, 64 KiB of serialized inference options,
4,096 transcript messages, 8 MiB of serialized transcript, 1 MiB for the
aggregate cached tool catalog, 64 KiB of serialized arguments per call, 64 KiB
per serialized tool result, 256 KiB of cumulative tool results, and 4 KiB for a
host-facing permission denial reason. Hosts may replace the complete limits
value through [`EngineBuilder::limits`](crate::EngineBuilder::limits). Counters
use checked arithmetic and a limit failure occurs before another tool is
authorized or executed. JSON byte sizes are counted through a serializer
without allocating a second copy of the value. Engine construction rejects a
tool catalog whose aggregate descriptions and recursive JSON Schemas exceed its
byte, depth, or node bound before the catalog is cloned into the engine.

JSON depth counts containers rather than scalar nodes: a scalar root has depth
zero, a root array or object has depth one, and every array or object nested
inside another container adds one. Validation is iterative and retains one
child-iterator frame per active container, so auxiliary traversal memory is
O(depth), not O(total nodes). It runs before core-controlled recursive
serialization, deep cloning, provider/store calls, permission checks, or tool
execution at each relevant boundary. Every scalar and container root counts as
one node. Node budgets are aggregate across all inference-metadata values, all
stored metadata and message values, and the complete tool-schema catalog;
provider arguments and tool outputs each receive their own complete budget.
Traversal stops after visiting the configured limit plus one and never queues
unvisited siblings.

[`MAX_SAFE_JSON_DEPTH`](crate::MAX_SAFE_JSON_DEPTH) is an independent hard
ceiling of 64 containers. Hosts may lower `max_json_depth`, but
[`EngineBuilder::build`](crate::EngineBuilder::build) rejects a higher value
with [`BuildError::JsonDepthLimitExceedsSafeMaximum`](crate::BuildError::JsonDepthLimitExceedsSafeMaximum)
before catalog validation, serialization, caching, or runtime component calls;
the value is never silently clamped. This ceiling protects the recursive
serialization, clone, retained-value destruction, and downstream extension
paths that follow iterative validation. Builder-owned Schemas are still drained
iteratively if this configuration check fails.

## Foreground child-agent boundary

The provider-neutral [`subagent` tool](subagent.md) exposes one bounded
`one_off` create operation through `SubagentTool` and an explicitly injected
`SubagentAuthority`. Core owns its closed input schema, admission, cancellation,
result projection, durable tool-result lifecycle, and next model round. It
receives no child provider, executor, filesystem, process, network, permission,
clock, task, thread, queue, or persistence authority implicitly.

The authority receives only an owned validated name and prompt, the bounded
structural `ToolContext` identifiers for the parent session, incarnation,
turn, and call, plus a cancellation token. Those identifiers are identities,
not handles or authority. The child context is fresh. Core does not pass the
parent transcript, grants, dynamic tools, executable registrations, tool
catalog, `subagent` capability, or model/effort/permission/notification
overrides.
Preparation uses the explicit no-authority disposition because all child work
is behind the separately injected seam; it does not call the permission
handler or turn the child's returned text into authority.

The execution future is inert until first poll and remains foreground. Four
global and two per-parent-turn active executions are admitted fail-fast with no
wait queue. A successful admission owns both counters until every call-local
authority future and value is dropped. Cancellation wins over authority success
or failure observed in the same poll. The implementation starts no detached
task, thread, timer, watcher, queue, or child session.

Only a completed final-text authority result is accepted. Core stamps the
model-visible projection with `status: "completed"`,
`trust: "untrusted_child"`, and `authority: "none"`. Names are bounded to 128
bytes; prompts and final text to 32 KiB each; complete compact input and output
to 48 KiB each; and tool-local JSON to 8 container levels and 64 nodes. See the
complete [subagent contract](subagent.md) for failure, drop, reference-host, and
intentional pinned-manager divergence semantics.

## Background admission boundary

The provider-neutral [`BackgroundSupervisor`](crate::BackgroundSupervisor)
orders one bounded process-local start across explicitly injected
[`BackgroundClock`](crate::BackgroundClock),
[`BackgroundStore`](crate::BackgroundStore),
[`BackgroundProcessSpawner`](crate::BackgroundProcessSpawner), and
[`BackgroundProcessRetainer`](crate::BackgroundProcessRetainer) authorities.
Core receives no ambient clock, filesystem, environment, task, thread,
process, signal, or persistence authority.

[`BackgroundStartRequest`](crate::BackgroundStartRequest) owns one bounded
command and absolute canonical persisted cwd. Its start future is inert until
poll. On poll, core admits capacity fail-fast, reserves one durable nonzero ID,
reads the injected clock, prepares a barrier-held process, durably publishes
the complete running record, asks native release to open the execution barrier,
and transfers the lease and exact owned process to the retainer. Release may do
bounded pre-open work, during which cancellation still drops and cleans the
prepared process under its no-command-executed guarantee. Opening the barrier
is the irreversible commit point; cancellation cannot revoke the process after
that point.

The returned [`BackgroundHandle`](crate::BackgroundHandle) exposes only the
durable ID and optional PID for presentation. Those numbers provide no
liveness, lookup, or signaling authority. Retained completion waits receive a
host-owned stop token; cancellation of that token requests cleanup and yields
a stopped outcome only after the implementation's bounded process ownership
set is discharged. The Linux/macOS native set contains the retained direct
child, its original process group, and members captured by the bounded cleanup
snapshots; a descendant that changes group or session before any snapshot
observes it is outside that set and is not covered by a successful cleanup
result. The full native persistence and lifecycle contract is in the
[background supervisor contract](background-supervisor.md).

## Available-model catalog boundary

The available-model catalog adds provider-neutral
catalog types without changing `Engine`, `ModelProvider`, generation model
selection, or turn orchestration. `AvailableModel::new` accepts one 1–128-byte
ID whose bytes are all visible ASCII `0x21..=0x7e`; failures expose only
`InvalidModelIdReason::{Empty, TooLong, NotVisibleAscii}`. `AvailableModel::id`
returns the validated ID.

`ModelCatalog::new` stores a provider-supplied ordered `Vec<AvailableModel>` and
`ModelCatalogAccess`; `models`, `access`, and `into_models` expose that result
without sorting or projection. Access is `Authenticated` or `PublicOnly` with
`PublicCatalogReason::{NoCredential, AuthenticatedCredentialRejected}`. The
object-safe `ModelCatalogProvider` exposes `name` and one
`list_models(CancellationToken) -> BoxFuture<Result<ModelCatalog,
ProviderError>>` operation. Its future is inert until polled.

Core owns no catalog URL, Gateway fields, credential, access attempt,
authentication fallback, deadline, clock, runtime, HTTP, parser, sort, vector
cap, or output representation. The bounded native implementation validates and
orders before constructing the core result; the thin CLI renders it. See
[`models-cli.md`](models-cli.md).

## Native ask handler

The native ask handler implements the existing
provider-neutral `PermissionHandler` boundary as
`machine_god_native::AskPermissionHandler`. It does not change core's trait or
give core terminal, UI, environment, filesystem, process, network, clock, or
executor authority. A host explicitly injects a `PermissionPrompter`, either as
an owned concrete value through `AskPermissionHandler::new` or as an
`Arc<dyn PermissionPrompter>` through
`AskPermissionHandler::shared_prompter`.

On the engine path, core validates the prepared arguments and capability under
the configured byte/depth/node limits, constructs the complete
`PermissionRequest`, and emits `PermissionRequested` before calling the
handler. The adapter forwards the owned request to the prompter exactly once
without cloning, mutation, serialization, truncation, revalidation, or
traversal. Structured allow-once, allow-turn, allow-session, and deny prompt
results map to the corresponding existing `PermissionDecision`. The scopes are
auditable decisions; neither core nor this adapter caches them for a later
request.

A denied prompt returns the fixed reason `permission denied`. A prompt failure
cannot carry source diagnostics in its zero-data error and maps fail-closed to
the core error `permission_prompt_failed` / `permission prompt failed`. The
authorization future is inert until polled and creates no detached work.
Dropping it before first poll does not call the prompter; dropping it while
pending drops the underlying prompt future. Core cancellation relies on that
drop behavior and supplies no permission-specific cancellation token. The
complete contract and host obligations are in
[`ask-permission.md`](ask-permission.md). Existing CLI
behavior is unchanged.

## Native file store

The native file store implements this unchanged
provider-neutral boundary as `machine_god_native::FileSessionStore` on Linux
and macOS Unix targets. A host explicitly supplies one existing absolute root;
the native constructor opens and retains its directory descriptor without
environment discovery or root creation. Fixed v1 names are lowercase SHA-256 of
the domain-separated session ID. The digest is a stable filename and
privacy-reduction device, not encryption or confinement. Loads verify the exact
decoded record ID.

The store persists an exact schema-v1 compact JSON envelope bounded by
`MAX_FILE_SESSION_BYTES` (`8_651_165`), which accommodates every record obeying
the default `EngineLimits`. Loads use a bounded cap-plus-one open-then-`fstat`
regular-file read. Saves implement new/update compare-and-swap, immutable
incarnation identity, and checked revision assignment beneath one permanent
per-session regular no-follow advisory lock. Publication uses an exclusively
created no-follow `0600` temporary regular file, file sync, same-directory
atomic rename, and directory sync. Corrupt or nonregular artifacts fail closed
and are not repaired. The store iteratively enforces core's default aggregate
JSON bounds of 64 container levels and 65,536 nodes for direct trait callers as
a separate check from its byte cap.

The load/save futures perform no effect before first poll and detach no work.
Their first poll performs bounded synchronous serialization, filesystem I/O,
advisory-lock acquisition, and synchronization inline, so it can block the
executor thread. Advisory coordination applies only to cooperating processes
on filesystems honoring the assumed Unix semantics. A directory-sync error
after rename has an ambiguous outcome and requires load-and-reconcile. This is
not an NFS, multi-record transaction, hostile-writer, or full sudden-power-loss
guarantee. The exact layout, failure taxonomy, trust boundary, and deferred
scope are normative in [`session-store.md`](session-store.md).

## Turn lifecycle

Awaiting [`Session::prompt`](crate::Session::prompt) atomically reserves a
durable turn ID and its user message, then returns a [`Turn`](crate::Turn), an
asynchronous stream of ordered
[`EngineEvent`](crate::EngineEvent) values. Every event carries a session ID,
session incarnation ID, turn ID, and monotonic sequence number. Event sinks can
therefore deduplicate or audit otherwise identical sequences from reset session
lifetimes without merging them.

```text
created -> started -> provider round -> final assistant commit -> completed
                         |
                         +-> tool-call stop -> atomic assistant +
                                  unknown-result placeholders commit
                                  -> prepare -> validate -> required permission?
                                  -> tool
                                  -> in-place result replacement -----+
                         ^                                           |
                         +------------- next provider round <--------+

live turn -- drop / terminal event --> session lease released
```

Exactly one turn may be live for a session within an [`Engine`](crate::Engine),
including across separately created, separately loaded, and cloned session
handles. A second prompt returns
[`EngineError::SessionBusy`](crate::EngineError::SessionBusy). Engine instances
keep a weak ordered registry by session ID; handles and live turns share the
canonical state only when their durable incarnation IDs also match. Creating or
loading the same live session ID with another incarnation fails with
[`EngineError::SessionIncarnationConflict`](crate::EngineError::SessionIncarnationConflict)
instead of merging distinct logical lifetimes. Create and load perform only a
targeted logarithmic lookup,
while the last state owner reclaims its own key with an identity check so a
delayed destructor cannot remove a concurrent replacement.
Registry lookup upgrades an existing weak state while holding the entries
mutex, then releases that mutex before validating session/incarnation identity.
The upgraded `Arc` keeps the state alive for validation. If validation fails and
that reference is the last owner, `SessionRegistration::drop` can therefore
reenter registry cleanup without attempting to relock a mutex still held by the
same thread.
This lease is deliberately process-local and scoped to one `Engine`. Separate
engine instances or processes rely on optimistic store revisions for unique turn
IDs, but this milestone does not claim a cross-engine or distributed live-turn
lease.
Cancellation is cooperative, wakes the turn stream without depending on an
executor, and is idempotent: only the first `TurnHandle::cancel` returns `true`.
Dropping a live turn signals its shared cancellation token before releasing the
session lease, so provider work that retained the token is not orphaned. A stale
handle therefore observes cancellation and cannot request it a second time.
Dropping an already completed turn only repeats waiter and lease cleanup; it
does not synthesize a cancellation or wake completed work.
Cancellation wait registrations are keyed per live future and removed when that
future or turn is dropped. A turn also removes its registration before yielding
each nonterminal event, because no poll is outstanding while its consumer holds
that event. While observer delivery is pending, each poll refreshes the keyed
registration with that poll's waker, so cancellation wakes only the current
poller. Repeated polls, idle streams, and abandoned waiters therefore do not
retain stale wakers. Waker clone, replacement drop, deregistration drop, and wake
callbacks all execute outside the waiter-registry mutex, so a custom waker may
reenter cancellation APIs without self-deadlocking.
Once a terminal outcome is established, its pending observer delivery does not
retain or refresh a cancellation waiter. Later cancellation cannot change that
outcome and therefore cannot create a self-waking hot loop while the terminal
observer remains backpressured. A final provider `Stop` is not established as
the turn outcome until its assistant message has been saved. Its save therefore
remains cancellable while pending, and a pending store cannot prevent shutdown.
Immediately before constructing that final save, core checks cancellation. It
then always polls the newly returned future once. If save construction or that
first poll durably succeeds while requesting cancellation, the success is
reconciled and terminal precedence is established synchronously; the
already-persisted final answer is not relabeled as cancelled. If the first poll
is pending, later polls restore the ordinary cancellation precheck, so a
previously pending store cannot gain another success-winning poll after
cancellation.

The next turn sequence is part of [`SessionRecord`](crate::SessionRecord).
So is the validated [`SessionIncarnationId`](crate::SessionIncarnationId) that
identifies one logical lifetime of a reusable session ID. A host must supply a
globally unique incarnation when calling [`Engine::create_session`](crate::Engine::create_session)
and persist it unchanged for every later load and save. Core deliberately has no
clock or randomness from which to synthesize one. A host that deletes, resets,
rewinds, or otherwise starts a fresh logical session under an old session ID
must allocate a new incarnation first. Stores must reject a save that changes
the incarnation of an existing record; assigning or migrating identities for
legacy records is an explicit host operation. Deserialization does not invent a
fallback incarnation for records that omit it.
Prompt creation reserves it and appends the user message through the configured
store's optimistic revision before exposing the `Turn` or calling the provider;
stale handles reload and retry within a fixed bound.
Successful reservations therefore remain consumed across reloads and process
restarts, while core remains deterministic and does not acquire clock or random
authority. Reconciliation fails closed if a conflict reload has a zero next-turn
sequence, is older than the engine-canonical revision, or differs from the
canonical record at the same revision. If a delayed successful reservation
finishes after a newer load has already reconciled, its saved snapshot still
drives that turn's model request but cannot rewind the canonical session record.
An equal-revision save result is accepted only when it is identical to the
canonical record; divergence is a protocol error rather than an ambiguous
overwrite. Intrinsic stored-record validation happens before a loaded record can
enter the engine's shared registry, so a rejected record cannot be retained by a
concurrent create or load handle. Existing canonical state is validated again
during reconciliation. Revision zero is reserved for an unsaved
`SessionRecord::empty`; every record returned by a store must have a positive
revision, including conflict reloads. The first successful save replaces the
zero sentinel with a positive revision.
If a conflict reload reports that the record is absent, core clears its
persisted flag only when the canonical record and persistence status still match
the exact snapshot used by the failed save. A concurrent newer load is therefore
preserved, and the retry uses its positive revision. Revision comparisons remain
monotonic even after a legitimate missing-record result; the persistence flag
cannot make an older load or save eligible to replace newer canonical state.
The next-turn allocator is independently monotonic: no load, conflict reload, or
successful-save result may reduce `next_turn_sequence`, even when it carries a
higher revision. Such a result is a protocol error and leaves canonical state
unchanged. A valid higher revision may otherwise replace messages and metadata;
an equal revision continues to require equality of the entire record.

Providers emit at most one terminal `ModelEvent::Stop`. A stream that ends
without it becomes a structured `failed` event. Observer backpressure is honored:
an event is yielded to the caller only after the configured event sink accepts
the same event. Observer failure terminates the turn with `EngineError::EventSink`.
Untrusted sink codes and messages are dropped and replaced by the stable
`event_sink_failed` / `event sink failed` diagnostic before that error crosses
the public boundary.
If that failure occurs before the provider reaches a terminal outcome, core
cancels the shared provider token before dropping the stream and releasing the
lease; stale cancellation handles then observe that cleanup signal. Observer
failure after an already-terminal provider outcome does not relabel completion
as cancellation.
Before a terminal outcome is established, cancellation has priority over
observer backpressure: core drops a pending observer future and yields the local
terminal cancellation directly before releasing the session lease. Core
rechecks cancellation immediately after provider startup, provider-stream,
store, policy, tool, and observer-delivery polls and before interpreting their
results. Cancellation observed at one of those boundaries wins while the turn
is still preterminal. The narrow exception is a ready successful final-assistant
save: durable success wins that poll, reconciliation completes, and the final
`Stop` is established before control returns to the outer turn. Cancellation
still wins if that save is pending or returns an error in the cancelling poll.
A provider failure or missing-stop failure establishes precedence when accepted;
later cancellation cannot relabel or bypass an established pending delivery or
terminal result.

Cancellation provenance is explicit. Only `Completed(Cancelled)` synthesized by
the local cancellation token bypasses an optional observer, so observer
backpressure cannot prevent shutdown. A provider-originated
`StopReason::Cancelled` is an ordinary provider result: it must complete its
durable save and observer delivery, and an external cancellation request cannot
misclassify it as locally synthesized cancellation.

## Provider grammar and durable tool rounds

A `Stop` ends a provider round immediately and core drops that stream rather
than polling it for EOF. This keeps a valid `Stop` followed by a permanently
pending stream live. Providers are contractually forbidden to emit after
`Stop`; items produced lazily after that boundary cannot be observed by core.
A round ending without `Stop` fails. A `ToolCalls` stop requires one or more
calls, while any call paired with another stop reason fails. Calls are validated
as they arrive for count, unique turn-wide ID, registered name, and serialized
argument size. The complete round is valid before any permission request or tool
execution begins. Core advertises JSON Schema in `ToolSpec` but deliberately
leaves schema enforcement to the tool implementation; it does not claim core
JSON-Schema validation.

Assistant text deltas are concatenated into one durable text block. Reasoning
deltas remain observable model events but are never persisted. After a valid
tool-call `Stop` is delivered, core atomically commits the assistant message and
exactly one conservative unknown-result placeholder for every call before any
permission request or tool execution. Placeholder sizes count against both the
per-result and cumulative budgets; a budget that cannot hold all placeholders
fails before that commit and before external work. Calls then run serially in
provider order.

Before authorization, core passes each validated provider call by value to
[`Tool::prepare`](crate::Tool::prepare). Its source-compatible default returns
a [`PreparedToolCall`](crate::PreparedToolCall) containing the original
arguments and the same raw `Capability::Tool` used before preflight existed. A
tool may instead use [`PreparedToolCall::new`](crate::PreparedToolCall::new) to
return a normalized filesystem, process, network, composite vision, custom, or
tool capability together with replacement JSON arguments. Preparation is
required to be deterministic, synchronous, bounded, nonblocking, and effect-
free: it may validate and normalize values but must not open files, start
processes, contact networks, mutate state, or otherwise exercise the capability
that policy has not yet allowed. Core checks cancellation immediately before
calling preparation and immediately after it returns. Because preparation is
synchronous, core cannot interrupt it in flight; a blocking implementation
would delay cancellation and violates the contract.

Core validates prepared execution arguments against the configured JSON depth
and node bounds and the exact `max_tool_argument_bytes` serialized-byte limit.
Within a prepared capability, the same depth and node traversal applies only to
the embedded JSON `serde_json::Value` in `Capability::Tool` or
`Capability::Custom`; typed filesystem, process, network, and vision variants
contain no embedded JSON value to traverse. Every capability variant is
additionally validated as a whole against one serialized-byte cap of
`max_tool_argument_bytes + 1024`. The fixed 1 KiB is headroom within that total
cap, not a separately metered envelope field or second payload budget. It keeps
the source-compatible default from rejecting raw arguments that were valid at
the existing exact boundary. Rejection occurs before authorization or tool
execution. A preparation error also consults no permission handler and starts
no tool. It becomes the same fixed generic, durable tool-error result as an
execution error, replacing that call's unknown
placeholder so the next model round can recover without receiving the tool's
diagnostic.

`Capability::Vision { paths, target }` presents one indivisible policy choice:
disclose the exact ordered normalized workspace paths to the exact normalized
provider destination. The path set and destination share the ordinary whole-
capability byte envelope; neither is authorized separately. A prepared call
that contains only attachment IDs may use
[`PreparedToolCall::without_authority`](crate::PreparedToolCall::without_authority)
only while resolving those IDs performs no policy-governed effect. The current
native path deterministically reports unavailable attachments without reading
the filesystem or contacting a provider. The exact behavior is defined by the
[`vision` contract](vision.md).

Every successfully prepared permission-required invocation receives a fresh
critical-risk authorization request for its prepared capability. Its fixed
reason remains `model requested this registered tool`, and its deterministic ID
remains a domain-separated SHA-256 v2 digest of length-delimited session ID,
session incarnation ID, turn ID, and ordinal. Both
[`ModelRequest`](crate::ModelRequest) and
[`PermissionRequest`](crate::PermissionRequest) carry the incarnation as audit
input. The fixed lowercase hex encoding is portable ASCII and remains below the
128-byte public ID limit. Core does not cache positive grant scopes. A host
policy may implement its own identity-safe caching without reusing an allow
across sessions, turns, or reset session lifetimes. For permission-required
calls, denial becomes a fixed generic error `ToolResult` without starting the
tool. The detailed policy reason remains available only in the host-facing
`PermissionResolved` event and is truncated on a UTF-8 boundary to its
configured limit before it is cloned or staged. A tool implementation error
likewise becomes a fixed generic model-visible result, allowing the next model
round to recover without copying tool-specific diagnostics into the
transcript. A policy infrastructure error fails the turn.

An allowed tool receives a [`ToolContext`](crate::ToolContext) containing the
session ID, session incarnation ID, turn ID, and call ID, plus exactly the JSON
arguments returned by its successful preflight. A tool that implements
idempotency, replay protection, or an audit key must include the incarnation;
the other three values can repeat after a durable reset.

Ordinary implementations return only `ToolOutput`; the source-compatible
default `Tool::execute_for_turn` wraps that value in `ToolExecution`. A bounded
extension tool may instead attach one opaque `TurnToolRegistration` containing
a captured `ToolSpec` and the exact executable `Tool`. Core keeps registrations
in a registry local to the current `run_turn` invocation. A candidate is
checked for static/dynamic name collisions and against the complete catalog's
configured JSON depth, node, and serialized-byte limits before its visible
result is stored. Reselection is idempotent only for the exact same captured
registration allocation; another capture under the same name fails closed even
when its visible specification is equal. Core activates a registration only
after durable placeholder replacement succeeds. It is therefore absent from
the provider response that selected it, present in later provider requests of
the same turn, appended in successful activation order, and resolved through
the same captured executable for call admission and dispatch. Exact idempotent
reselection does not change that order. Error outputs cannot register tools.
Completion, failure, and cancellation drop the registry; it is never stored in
`Engine`, `SessionState`, or `SessionRecord`, so later turns and other sessions
cannot inherit it. Registration is not an authorization grant: the dynamic
tool follows the ordinary preparation and permission pipeline.

Prepared arguments may drive only effects contained by the exact prepared
capability that policy allowed. This is a normative obligation of the trusted
tool implementation, not a semantic relation core can infer from arbitrary
JSON. In particular, native filesystem, process, and network tools must execute
the normalized path, command, or destination represented by that capability and
must not reinterpret their prepared arguments into broader authority.

The native [`web_fetch` tool](web-fetch.md) applies this
existing rule to one rootless network tool. Effect-free preparation must turn
the sole bounded URL into both one canonical HTTPS execution URL and the exact
`Capability::Network { target: NetworkTarget { .. } }` presented to policy.
Allowed execution may contact only that scheme, host, and effective port; DNS
admission and connection pinning cannot broaden it. Existing core behavior
continues to classify network authority as `Critical`, so the default path is
`Ask`. The tool adds no new capability variant or core ambient network
authority.

The concrete consumers are the native
[`read_file` tool](read-file.md), [`list_files` tool](list-files.md),
[`file_info` tool](file-info.md), [`glob_files` tool](glob-files.md),
[`grep_files` tool](grep-files.md), [`write_file` tool](write-file.md),
[`edit_file` tool](edit-file.md), [`delete_file` tool](delete-file.md),
[`rename_file` tool](rename-file.md), [`copy_file` tool](copy-file.md),
[`create_folder` behavior](create-folder.md), and the [`open_file`
contract](open-file.md). `create_folder` is another
single-path consumer.
`read_file` effect-free preflight turns the strict
provider `{path:string}` object into both a prepared
`Capability::Filesystem { access: Read, path }` and prepared execution
arguments containing the same normalized workspace-relative path. The native
tool, not core, owns the injected workspace directory authority, Unix
descriptor-relative no-follow traversal, 4,096-byte path bound, 8 KiB content
bound, UTF-8 requirement, redacted error taxonomy, and syscall-granularity
cancellation limitations. Core's policy ordering, prepared-value limits,
generic durable tool-error mapping, and result limits remain unchanged.

`list_files` effect-free preflight accepts only `{}` or a sole string `path`,
defaults omission to `.`, and produces both
`Capability::Filesystem { access: Enumerate, path }` and exact prepared
`{"path":"<normalized>"}` execution arguments. The native tool owns the
workspace descriptor opened from an explicit absolute host path, Unix
descriptor-relative directory and no-follow traversal, 4,096-byte lexical path
bound, safe UTF-8 entry-name validation, and fixed redacted errors. It
enumerates one level without opening children, retains at most 100 entries and
16 KiB of aggregate raw name bytes, reads one extra visible entry to establish
truncation, and sorts only the retained subset. Its exact `{path, entries:
[{name, kind}], truncated}` structured content plus the fixed `ToolOutput`
envelope is at most 44,130 serialized bytes under the independent tool bounds,
so it remains within the default 64 KiB result limit. A configured lower result
limit still applies after execution. Core does not add recursion, ordering,
snapshot, or filesystem semantics to this native result.

`FilesystemAccess::Metadata` is a distinct serialized
filesystem operation. It authorizes inspection of metadata for exactly one
normalized path; it does not imply `Read`, `Enumerate`, mutation, symlink-target,
or external-path authority. `file_info` effect-free preflight accepts only a
required sole string `path` and produces both
`Capability::Filesystem { access: Metadata, path }` and exact prepared
`{"path":"<normalized>"}` execution arguments. Its Linux/macOS native
implementation owns the retained workspace descriptor, 4,096-byte lexical path
bound and explicit `.` root normalization, fresh acquired-root liveness
validation, descriptor-relative no-follow ancestor traversal, final no-follow
metadata lookup, checked fixed-width metadata conversion, lexical regular-file
extension, redacted errors, and syscall-granularity cancellation limits. Its
exact `{path, kind, size_bytes, modified: {unix_seconds, nanoseconds},
extension}` content remains below 17 KiB at its independent worst case. Core
does not infer any relationship among `Metadata`, `Read`, or `Enumerate`, and
adds no filesystem or snapshot semantics to this result.

`FilesystemAccess::EnumerateRecursive` is another distinct serialized
filesystem operation. It authorizes recursive enumeration beneath exactly one
normalized selected subtree. It neither implies nor is implied by one-level
`Enumerate`, and it does not imply `Read`, `Metadata`, mutation, symlink-target,
or external-path authority. `glob_files` effect-free preflight accepts exactly
`{pattern:string,path?:string,mode?:"matches"|"count"}`, defaults `path` to `.`
and `mode` to `matches`, and produces both
`Capability::Filesystem { access: EnumerateRecursive, path }` and exact
prepared arguments containing normalized `pattern` and `path` plus the explicit
mode. Pattern and mode attenuate the output of the complete recursive scan; the
capability continues to name the entire selected subtree whose entries may be
observed.

The Linux/macOS implementation owns the retained workspace
descriptor, independent 4,096-byte requested and normalized path/pattern
bounds, strict bytewise matcher, fresh acquired-root liveness validation,
iterative descriptor-relative no-follow traversal, safe entry-name validation,
fixed scan budgets, globally sorted bounded match-prefix selection, exact count
mode, fixed redacted errors, synchronous first-poll execution, and syscall-
granularity cancellation limits. Both modes complete the bounded scan or fail
without partial output. Its exact matches content is `{path, pattern, mode:
"matches", matches, truncated}`; count content is `{path, pattern, mode:
"count", count}`. Core adds no glob grammar, traversal, ordering, truncation,
snapshot, or filesystem semantics to these results. The complete contract is
in [`glob-files.md`](glob-files.md).

`FilesystemAccess::SearchContent` is a distinct
serialized filesystem operation. It authorizes bounded recursive entry-name
observation and bounded regular-file content inspection at exactly one
normalized selected path: that object if it is a regular file, or eligible
regular files beneath it if it is a directory. It neither implies nor is
implied by `Read`, `Metadata`, `Enumerate`, or `EnumerateRecursive`, and it does
not imply mutation, external-path, or symlink-target authority. Core treats it
as an auditable value and infers none of those relationships.

`grep_files` effect-free preflight accepts exactly required `pattern` and
optional `path`, `include`, `case_insensitive`, `mode`, `head_limit`, `offset`,
and `context_lines`. It prepares all eight canonical values with explicit
defaults `.`, `null`, `false`, `matches`, `100`, `0`, and `0`, plus the
`SearchContent` capability at the exact normalized selected path. Pattern,
include, case, mode, pagination, and context attenuate one invocation's output;
the capability remains conservative authority to search content at or beneath
the selected path. Preparation opens no filesystem object.

The Linux/macOS native implementation, not core, owns retained workspace
identity and liveness, selected-file/directory classification, iterative
descriptor-relative no-follow sorted traversal, pre-allocation full-path bounds,
selected-file include filtering before content open, stable-special skipping and
raced-special opened-type rejection, fixed literal pattern-table work before
root resolution, once-per-call fully metered include compilation, regular-file
eligibility, a worst-case-linear literal matcher with ASCII-only folding,
one scan-local content buffer using an 8 KiB read window and a logical-reset
204,801-byte high-water ceiling, same-buffer context, complete scan/work/output budgets,
reusable 64 MiB-bounded offsets, fixed redacted errors, and fixed cancellation
checks through line indexing and serialization trimming. Slashful selected-file
rejection is charged and cancellation-checked; slashful candidate splitting and
both dynamic-programming branches route through injectable fixed cancellation
checks. Its
`matches`, `files_with_matches`, and `count` shapes echo the canonical request,
return exact eligible-text totals plus candidate/search/skip statistics, and
remain under an independent 48 KiB complete serialized `ToolOutput` cap. Core
adds no content-search grammar, traversal, eligibility, pagination, excerpt,
context, ordering, race, or snapshot semantics. The complete contract is in
[`grep-files.md`](grep-files.md).

The provider-neutral API includes the typed capability
`Capability::FilesystemRename { old_path, new_path }`, serialized with the
exact tag `filesystem_rename`. Unlike single-path `Capability::Filesystem`, it
places both canonical endpoints in the permission request so policy can decide
the complete move. The variant contains no embedded JSON value: the existing
whole-capability serialized-byte cap still applies, while JSON depth and node
walking remain limited to `Tool` and `Custom` values. Native `rename_file`
strictly prepares the same canonical `old_path` and `new_path` pair for policy
and execution. Core assigns no overwrite, parent-creation, traversal, or
durability semantics to the variant; the Linux/macOS implementation owns the
confined no-follow, regular-file-only, absent-destination, exactly-once
`NOREPLACE`, postcommit identity-check, and bounded parent-sync contract in
[`rename-file.md`](rename-file.md).

The provider-neutral API includes the typed capability
`Capability::FilesystemCopy { source, destination }`, serialized with the exact
tag `filesystem_copy`. Both canonical endpoints are therefore visible to policy
without embedding a JSON value; whole-capability serialized-byte limits apply,
while JSON depth and node walking remain limited to `Tool` and `Custom` values.
Native `copy_file` strictly prepares the same canonical pair for policy and
execution. Core assigns no traversal, streaming, staging, overwrite, metadata,
or durability semantics to this variant. The Linux/macOS
implementation owns the confined regular-file-only, absent-destination,
16 MiB source and 64 KiB chunk bounds, SHA-256 verification, one no-replace
commit, postcommit verification, and destination-parent synchronization
contract in [`copy-file.md`](copy-file.md). A successful tool result contains
exactly canonical `source`, canonical `destination`, and `bytes_copied`; core
continues to apply the ordinary prepared-value and result limits.

`create_folder` requires no new provider-neutral core variant.
It uses the existing
`Capability::Filesystem { access: FilesystemAccess::Create, path }`, whose
stable policy JSON is exactly:

```json
{"type":"filesystem","access":"create","path":"canonical/path"}
```

Native `create_folder` preparation turns its strict sole `path` field into that
canonical capability and identical prepared execution arguments. Core assigns
no recursive creation, mode, umask, ACL, traversal, commit, rollback, or
durability semantics to `Create`; the Linux/macOS native tool owns the no-
follow recursive protocol and bounds in [`create-folder.md`](create-folder.md).

`open_file` uses one dedicated provider-neutral capability rather than reusing
filesystem read, metadata, or arbitrary process authority. Its stable policy
JSON is:

```json
{"type":"open_file","path":"canonical/path"}
```

`Capability::OpenFile { path }` means approval to present exactly one canonical
workspace-confined existing regular file to the host's default application. It
does not authorize model-selected programs, arguments, external paths,
directories, URLs, or content returned to the model. Native preflight prepares
that exact path for policy and execution. Path length/shape rejection precedes
complete-value serialization, so hostile over-bound paths cannot create
unbounded pre-path serialization work. Linux native execution owns
descriptor-relative no-follow validation, retained file identity, and the
fixed `/usr/bin/xdg-open` lifecycle over a parent-owned proc descriptor path.
The exported trusted `OpenFileLauncher` seam receives the approved path, proc
path, and owned file descriptor; its returned future must remain inert until
polled and clean up every owned helper on cancellation or drop. Exactly 32
global system-launch permits bound active workers; saturation is precommit
unavailable with zero new worker/helper, and each permit remains held through
arbitrary Waker completion and worker return. Spawn and
cancellation/drop linearize through one serialized gate: abort-first guarantees
zero launch and successful spawn commits. Before publication, cleanup suppresses
waking, reaps the helper, drops request/descriptor ownership, and synchronously
joins. Normal published completion joins. Inline or blocking arbitrary Waker
permit-bounded callback/final bookkeeping may outlive future drop after helper/
request cleanup. The system launcher uses fixed `/` working directory and null
stdio, makes its timeout decision at 30 seconds, and maps postspawn uncertainty
to a fixed redacted result. `open_file`'s reference-host placement and retained-workspace
allocation are recorded in the canonical
[tool catalog](native-reference-host.md#tool-catalog). On macOS the catalog
entry is present but execution returns unsupported before filesystem lookup or
spawn.

Each completed result replaces its matching placeholder in place with an exact
transcript-prefix compare-and-save before `ToolFinished`, the next call, or the
next model round. Cancellation or a policy, tool, observer, or store failure
therefore leaves one result for every committed call: completed prefixes are
known and the untouched suffix remains explicitly unknown. Resume never
automatically replays those calls. If an executed tool returns an oversized
result, core drops that value and terminates while retaining the precommitted
unknown-result placeholder. An over-depth or over-node result is handled the
same way after execution: the side effect is not replayed and the placeholder
remains. The
final assistant message is committed before its model `Stop` and `Completed`
events are delivered. Token usage is the latest report within each round and is
added across rounds with checked counters.

Message commits retry optimistic conflicts at most 32 times. A retry is allowed
only while the durable messages exactly match the captured transcript; newer
turn-allocation state and metadata are preserved. A missing, stale, corrupt, or
divergent record fails closed, so core never blindly merges or duplicates
messages. Durable saving is authoritative: observer success events follow their
related commit, and observer failure never replays a committed effect.

A final non-tool `Stop` remains preterminal during its required assistant
commit. Cancellation can interrupt a pending save and release the live-turn
lease; store failure becomes a terminal durability failure. A ready successful
save is authoritative even if its poll also requests cancellation: core
reconciles it and synchronously establishes the provider result before the outer
turn observes cancellation. Cancellation then cannot relabel its model `Stop`,
observer delivery, or `Completed` event. Intermediate `ToolCalls` stops are not
turn-terminal, so cancellation may interrupt their atomic placeholder commit,
permission request, tool work, result replacement, or next-provider startup.
Synchronous preparation is the bounded exception described above: cancellation
is observed at its immediate before/after checks, not inside the call.
All such futures are owned and polled inline by `Turn`; dropping the turn drops
them rather than detaching work.

Prompt and serialized inference-option bytes are checked before persistence.
Transcript message count, transcript bytes, and recursive session-metadata bytes
are checked before loading a record into the engine registry, before every
provider request, and before every commit or replacement. Model events,
including `Stop`, are counted across the whole turn; provider-specific
`StopReason::Other` details are bounded before they are cloned or delivered.
The JSON depth and node bounds cover inference metadata, stored metadata, JSON
message blocks, stored and provider tool arguments, tool-result content, and
tool input Schemas. Provider arguments fail before authorization or execution.
Tool output is checked immediately after execution and before serialization or
replacement. Core iteratively drains every owned rejected JSON tree at these
ingresses, including abandoned/replaced builders, unpolled prompt futures,
failed direct or conflict loads, rejected mutation candidates, provider events,
and post-effect tool results. Reclamation visits every owned node and holds one
iterator per active container, avoiding recursive `Value::drop`.
The same guard is armed within cancellation-aware polling when a provider event,
conflict-loaded record, or tool output becomes ready in the poll that first
observes cancellation.
Every normally yielded provider event is also guarded immediately after the
stream poll, before event-count accounting or any other early-return gate.
Structured provider, policy, and store failures expose fixed component codes
(`provider_failed`, `permission_failed`, and `store_failed`) plus the trusted
retryability/category fields where applicable; hostile source codes and
messages are not forwarded. Tool failures likewise become a fixed generic
model-visible result. Permission decisions are host-facing and may contain a
bounded sensitive policy reason; event sinks and event consumers must therefore
be treated as trusted components.

Resource limits apply once an owned value crosses into core. They cannot undo
allocations already performed by a caller constructing prompt options, a store
loading a record, a tool publishing its specification or result, a provider
creating model values, or a policy creating its decision and reason. Hosts must
apply complementary limits while decoding or constructing those inputs.
Once a provider event is yielded from `poll_next`, core owns and safely drains
it. Values still queued inside a provider stream that core never receives
remain the provider's responsibility; its stream destructor must be stack-safe
or its decoder/construction limits must keep those values safe to drop.

The live-turn lease remains process-local to one `Engine`. Core does not claim
cross-engine fencing. A crash after a tool side effect but before placeholder
replacement leaves an explicit unknown result, which prevents automatic replay
but cannot establish whether the external side effect completed; stronger
exactly-once recovery requires the M04 lifecycle and persistence design.
