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

## Turn lifecycle

Awaiting [`Session::prompt`](crate::Session::prompt) reserves a durable turn ID
and returns a [`Turn`](crate::Turn), an asynchronous stream of ordered
[`EngineEvent`](crate::EngineEvent) values. Every event carries a session ID,
turn ID, and monotonic sequence number.

```text
                  provider stream
created -> started ---------------> model event(s) -> completed
   |          |                            |              |
   |          +---- provider failure ------+----------> failed
   |                                       |
   +---- handle.cancel() ------------------+---------> cancelled

live turn -- drop / terminal event --> session lease released
```

Exactly one turn may be live for a session within an [`Engine`](crate::Engine),
including across separately created, separately loaded, and cloned session
handles. A second prompt returns
[`EngineError::SessionBusy`](crate::EngineError::SessionBusy). Engine instances
keep a weak registry by session ID; handles and live turns share the canonical
state, while dead entries are pruned without keeping abandoned sessions alive.
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

The next turn sequence is part of [`SessionRecord`](crate::SessionRecord).
Prompt creation reserves it through the configured store's optimistic revision
before exposing the `Turn`; stale handles reload and retry within a fixed bound.
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
Before a provider terminal outcome is established, cancellation has priority
over observer backpressure: core drops a pending observer future and yields the
terminal cancellation directly before releasing the session lease. Once a
provider `Stop`, provider failure, or missing-stop failure is established, later
cancellation cannot relabel or bypass its pending delivery or terminal result.
An already-staged `Completed(Cancelled)` still bypasses observer delivery so an
optional observer cannot make cancellation or shutdown wait forever.

The milestone-02 runtime layers tool execution, permission caching, durable
message commits, and multi-round model/tool orchestration onto this lifecycle.
The contracts intentionally expose those boundaries now without granting core
ambient native authority.
