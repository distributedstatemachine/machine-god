# Provider-neutral core API

`machine-god-core` is an embeddable, executor-independent streaming engine. It
contains no filesystem, process, environment, credential, clock, randomness, or
network access. A host supplies every authority-bearing component explicitly
through [`EngineBuilder`](crate::EngineBuilder).

The traits use boxed standard futures and `futures-core::Stream`, so an embedding
application may use Tokio, async-std, smol, a custom executor, or direct polling.
All public extension traits are object-safe, `Send`, and `Sync`.

```rust,no_run
use machine_god_core::{Engine, SessionId};

# fn configured_engine() -> Engine { unimplemented!() }
let engine = configured_engine();
let session = engine.create_session(SessionId::new("example").unwrap());
assert_eq!(session.id().as_str(), "example");
```

## Composition and authority

An engine cannot be built without a [`ModelProvider`](crate::ModelProvider),
[`SessionStore`](crate::SessionStore), and
[`PermissionHandler`](crate::PermissionHandler). There are no permissive hidden
defaults. [`EventSink`](crate::EventSink) is observational and defaults to
[`NoopEventSink`](crate::NoopEventSink). Tools are registered explicitly and are
looked up by validated [`ToolName`](crate::ToolName).

The permission decision is distinct from tool execution. A handler error never
means approval. Native implementations must normalize paths, process arguments,
and network destinations before presenting a [`Capability`](crate::Capability)
to policy.

[`EngineLimits`](crate::EngineLimits) supplies nonzero resource bounds. Defaults
allow 8 model rounds, 16 tool calls per turn, 4 calls per round, 1 MiB each of
assistant text and observer-visible reasoning, 4,096 model events, 1 KiB of
provider stop detail, 256 KiB per user prompt, 256 KiB of serialized session
metadata, 64 KiB of serialized inference options, 4,096 transcript messages,
8 MiB of serialized transcript, 1 MiB for the aggregate cached tool catalog,
64 KiB of serialized arguments per call, 64 KiB per serialized tool result,
256 KiB of cumulative tool results, and 4 KiB for a host-facing permission
denial reason. Hosts may replace the complete limits value through
[`EngineBuilder::limits`](crate::EngineBuilder::limits). Counters use checked
arithmetic and a limit failure occurs before another tool is authorized or
executed. JSON byte sizes are counted through a serializer without allocating a
second copy of the value. Engine construction rejects a tool catalog whose
aggregate descriptions and recursive JSON Schemas exceed its bound before the
catalog is cloned into the engine.

## Turn lifecycle

Awaiting [`Session::prompt`](crate::Session::prompt) atomically reserves a
durable turn ID and its user message, then returns a [`Turn`](crate::Turn), an
asynchronous stream of ordered
[`EngineEvent`](crate::EngineEvent) values. Every event carries a session ID,
turn ID, and monotonic sequence number.

```text
created -> started -> provider round -> final assistant commit -> completed
                         |
                         +-> tool-call stop -> atomic assistant +
                                  unknown-result placeholders commit
                                  -> permission -> tool
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
canonical state. Create and load perform only a targeted logarithmic lookup,
while the last state owner reclaims its own key with an identity check so a
delayed destructor cannot remove a concurrent replacement.
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
remains cancellable and a pending store cannot prevent shutdown.

The next turn sequence is part of [`SessionRecord`](crate::SessionRecord).
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
is still preterminal. A provider failure or missing-stop failure establishes
precedence when accepted; a final `Stop` establishes precedence only after its
required durable commit succeeds. Later cancellation cannot relabel or bypass
their pending delivery or terminal result.

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

Every invocation receives a fresh critical-risk `Capability::Tool`
authorization request whose deterministic ID is a domain-separated SHA-256
digest of length-delimited session ID, turn ID, and ordinal. The fixed lowercase
hex encoding is portable ASCII and remains below the 128-byte public ID limit.
Core does not cache positive grant scopes. A host policy may implement its own
identity-safe caching without reusing an allow across sessions. Denial becomes a
fixed generic error `ToolResult` without starting the tool. The detailed policy
reason remains available only in the host-facing `PermissionResolved` event and
is truncated on a UTF-8 boundary to its configured limit before it is cloned or
staged. A tool implementation error likewise becomes a fixed generic
model-visible result, allowing the next model round to recover without copying
tool-specific diagnostics into the transcript. A policy infrastructure error
fails the turn.

Each completed result replaces its matching placeholder in place with an exact
transcript-prefix compare-and-save before `ToolFinished`, the next call, or the
next model round. Cancellation or a policy, tool, observer, or store failure
therefore leaves one result for every committed call: completed prefixes are
known and the untouched suffix remains explicitly unknown. Resume never
automatically replays those calls. If an executed tool returns an oversized
result, core drops that value and terminates while retaining the precommitted
unknown-result placeholder. The final assistant message is committed before its
model `Stop` and `Completed` events are delivered. Token usage is the latest
report within each round and is added across rounds with checked counters.

Message commits retry optimistic conflicts at most 32 times. A retry is allowed
only while the durable messages exactly match the captured transcript; newer
turn-allocation state and metadata are preserved. A missing, stale, corrupt, or
divergent record fails closed, so core never blindly merges or duplicates
messages. Durable saving is authoritative: observer success events follow their
related commit, and observer failure never replays a committed effect.

A final non-tool `Stop` remains preterminal during its required assistant
commit. Cancellation can interrupt that save and release the live-turn lease;
store failure becomes a terminal durability failure. After a successful commit,
the provider result is established and cancellation cannot relabel its model
`Stop`, observer delivery, or `Completed` event. Intermediate `ToolCalls` stops
are not turn-terminal, so cancellation may interrupt their atomic placeholder
commit, permission request, tool work, result replacement, or next-provider
startup. All such futures are owned and polled inline by `Turn`; dropping the
turn drops them rather than detaching work.

Prompt and serialized inference-option bytes are checked before persistence.
Transcript message count, transcript bytes, and recursive session-metadata bytes
are checked before loading a record into the engine registry, before every
provider request, and before every commit or replacement. Model events,
including `Stop`, are counted across the whole turn; provider-specific
`StopReason::Other` details are bounded before they are cloned or delivered.
Structured provider, policy, and store failures expose fixed component codes
(`provider_failed`, `permission_failed`, and `store_failed`) plus the trusted
retryability/category fields where applicable; hostile source codes and
messages are not forwarded. Tool failures likewise become a fixed generic
model-visible result. Permission decisions are host-facing and may contain a
bounded sensitive policy reason; event sinks and event consumers must therefore
be treated as trusted components.

The live-turn lease remains process-local to one `Engine`. M02 does not claim
cross-engine fencing. A crash after a tool side effect but before placeholder
replacement leaves an explicit unknown result, which prevents automatic replay
but cannot establish whether the external side effect completed; stronger
exactly-once recovery requires the M04 lifecycle and persistence design.
