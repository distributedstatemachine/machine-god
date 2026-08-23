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

## Native ask handler

The integrated ninth bounded Milestone 03 slice implements the existing
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
[`ask-permission.md`](ask-permission.md), with delivery evidence in the
[`ask handler review`](reviews/m03-ask-permission-review-01.md). Existing CLI
behavior is unchanged.

## Native file store

The eighth bounded Milestone 03 slice implements this unchanged
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
scope are normative in [`session-store.md`](session-store.md). Its exact feature,
documentation-seal, and `main` checks are green, with evidence retained in the
[`native file session store review`](reviews/m03-session-store-review-01.md).
It is integrated on `main` at
`8f7b47db9580b14570bf9fb55763858f71a81271`; exact main CI run `32541315998`
and benchmark run `32541315997` are green.

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
                                  -> prepare -> validate -> permission -> tool
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
execution begins. M02 advertises JSON Schema in `ToolSpec` but deliberately
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
return a normalized filesystem, process, network, custom, or tool capability
together with replacement JSON arguments. Preparation is required to be
deterministic, synchronous, bounded, nonblocking, and effect-free: it may
validate and normalize values but must not open files, start processes, contact
networks, mutate state, or otherwise exercise the capability that policy has
not yet allowed. Core checks cancellation immediately before calling
preparation and immediately after it returns. Because preparation is
synchronous, core cannot interrupt it in flight; a blocking implementation
would delay cancellation and violates the contract.

Core validates prepared execution arguments against the configured JSON depth
and node bounds and the exact `max_tool_argument_bytes` serialized-byte limit.
Within a prepared capability, the same depth and node traversal applies only to
the embedded JSON `serde_json::Value` in `Capability::Tool` or
`Capability::Custom`; the filesystem, process, and network variants contain no
embedded JSON value to traverse. Every capability variant is additionally
validated as a whole against one serialized-byte cap of
`max_tool_argument_bytes + 1024`. The fixed 1 KiB is headroom within that total
cap, not a separately metered envelope field or second payload budget. It keeps
the source-compatible default from rejecting raw arguments that were valid at
the existing exact boundary. Rejection occurs before authorization or tool
execution. A preparation error also consults no permission handler and starts
no tool. It becomes the same fixed generic, durable tool-error result as an
execution error, replacing that call's unknown
placeholder so the next model round can recover without receiving the tool's
diagnostic.

Every successfully prepared invocation receives a fresh critical-risk
authorization request for its prepared capability. Its fixed reason remains
`model requested this registered tool`, and its deterministic ID remains a
domain-separated SHA-256 v2 digest of length-delimited session ID, session
incarnation ID, turn ID, and ordinal. Both
[`ModelRequest`](crate::ModelRequest) and
[`PermissionRequest`](crate::PermissionRequest) carry the incarnation as audit
input. The fixed lowercase hex encoding is portable ASCII and remains below the
128-byte public ID limit. Core does not cache positive grant scopes. A host
policy may implement its own identity-safe caching without reusing an allow
across sessions, turns, or reset session lifetimes. Denial becomes a fixed
generic error `ToolResult` without starting the tool. The detailed policy
reason remains available only in the host-facing `PermissionResolved` event and
is truncated on a UTF-8 boundary to its configured limit before it is cloned or
staged. A tool implementation error likewise becomes a fixed generic
model-visible result, allowing the next model round to recover without copying
tool-specific diagnostics into the transcript. A policy infrastructure error
fails the turn.

An allowed tool receives a [`ToolContext`](crate::ToolContext) containing the
session ID, session incarnation ID, turn ID, and call ID, plus exactly the JSON
arguments returned by its successful preflight. A tool that implements
idempotency, replay protection, or an audit key must include the incarnation;
the other three values can repeat after a durable reset.

Prepared arguments may drive only effects contained by the exact prepared
capability that policy allowed. This is a normative obligation of the trusted
tool implementation, not a semantic relation core can infer from arbitrary
JSON. In particular, native filesystem, process, and network tools must execute
the normalized path, command, or destination represented by that capability and
must not reinterpret their prepared arguments into broader authority.

The concrete consumers are the native
[`read_file` tool](read-file.md), [`list_files` tool](list-files.md), and the
delivered seventeenth-slice [`file_info` tool](file-info.md).
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

The delivered slice adds `FilesystemAccess::Metadata` as a distinct serialized
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

Production and independent-test lineage is green through replacement behavior
candidate `4193ecc`. Documentation seal and integrated `main` SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` passed exact feature CI run
`32605071080` on successful retry attempt 2, feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four workflows report that exact seal SHA. Benchmark success
is evidence only, not a product-performance claim. This documentation-only
commit is the final delivery record, is explicitly exempt from another
adversarial review after behavior was green, and reports its own exact workflows
at handoff.

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

The live-turn lease remains process-local to one `Engine`. M02 does not claim
cross-engine fencing. A crash after a tool side effect but before placeholder
replacement leaves an explicit unknown result, which prevents automatic replay
but cannot establish whether the external side effect completed; stronger
exactly-once recovery requires the M04 lifecycle and persistence design.
