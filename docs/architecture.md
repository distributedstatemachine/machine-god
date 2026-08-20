# Architecture

`machine-god-core` contains provider-neutral contracts and orchestration without
ambient operating-system authority. `machine-god-native` provides explicit native
capabilities. `machine-god-cli` composes them. `machine-god-testkit` provides
deterministic test doubles.

The milestone-02 public contracts are documented in
[`core-api.md`](core-api.md). Public interfaces keep model access, storage,
tools, permission policy, and event delivery behind object-safe traits. Core
uses standard futures and `futures-core::Stream`; it does not select or require
an async executor.

```text
                        machine-god-core
 host ---------------------------------------------------------------+
  |                                                                  |
  +-> ModelProvider ----+                                             |
  +-> SessionStore -----+-> Engine -> Session -> Turn event stream    |
  +-> PermissionHandler +                  |                          |
  +-> Tool(s) ----------+                  +-> TurnHandle/cancellation|
  +-> EventSink (optional observer)                                   |
                                                                     |
 native filesystem / process / network authority remains outside ----+
```

An engine requires explicit provider, store, and permission components. Event
observation may use the authority-free no-op sink. Validated IDs, structured
component errors, optimistic session revisions, monotonic event sequences,
one-live-turn session leases, and idempotent cancellation form the initial
cross-component invariants.

A live turn owns the session lease and provider cancellation signal as one
lifecycle unit. Destruction signals cancellation before removing waiters and
releasing the lease; terminal completion has already released that unit, so its
later destructor is cleanup-only. Out-of-band observer or delivery-state failure
before a terminal provider outcome follows the same cancel-before-release rule,
preventing dropped streams from orphaning retained provider work.
Provider terminal establishment is the cancellation precedence boundary. A
provider stop or failure retains its pending observer delivery and final reason
even if cancellation races afterward; only cancellation established before that
boundary may bypass observer backpressure.
Cancellation treats wakers as user-controlled callback objects: cloning happens
before locking, registry mutation only moves values, and superseded, removed, or
drained wakers are dropped or invoked after unlocking.

Each `Engine` owns a weak session-state registry keyed by `SessionId`. All
create/load races inside that engine converge on one in-memory record and active
turn flag; a live turn itself keeps the state alive if its originating session
handle is dropped. Weak entries are pruned on later registry access. This is an
in-process coordination boundary, not a distributed lease. Independent engines
and processes coordinate durable turn-number allocation through the session
store's optimistic revision contract. Loaded records reconcile strictly and
monotonically: corrupt sequences, stale revisions, and equal-revision divergence
are protocol errors, and completion of an older in-flight save cannot replace a
newer canonical record. Successful-save reconciliation also rejects divergent
records at the same revision. Intrinsic load validation precedes registry
publication, preventing a concurrent handle from retaining invalid persisted
state even when the originating load returns an error. Revision zero is an
in-memory unsaved sentinel only; persisted loads and conflict reloads require a
positive optimistic-concurrency revision. Missing conflict reloads may change
persistence status only with an exact snapshot comparison under the session-state
lock, while record revisions stay monotonic independently of that status flag.
The durable turn allocator is monotonic independently of both fields: a higher
revision cannot authorize a lower `next_turn_sequence`. Higher revisions may
advance conversation messages and metadata only after passing that allocator
guard, while equal revisions require whole-record identity.
