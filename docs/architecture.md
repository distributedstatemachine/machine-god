# Architecture

`machine-god-core` contains provider-neutral contracts and orchestration without
ambient operating-system authority. `machine-god-native` provides explicit native
capabilities. `machine-god-cli` composes them. `machine-god-testkit` provides
deterministic test doubles.

The testkit's concrete boundary doubles are documented in
[`testkit.md`](testkit.md). They are executor-neutral like core: finite scripts,
cancel-driven pending provider/tool work, and permanently pending observer or
policy steps support precise manual polling without clocks. Each double owns a
single mutex per related script-and-record state so concurrent call ordering is
linearizable and inspection returns a consistent snapshot. The in-memory store
executes comparison and mutation inside that same critical section, making its
optimistic revision behavior atomic rather than merely convenient for
single-threaded tests.

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

The multi-round turn loop is an executor-neutral future polled inline by the
`Turn` stream. A one-event acknowledgement gate connects it to observer
delivery: orchestration cannot advance past a nonterminal event until the sink
accepts that event and the caller receives it. No task, channel, timer, or
runtime-specific primitive is required. Provider, store, policy, and tool
futures stay owned by that orchestration frame, so cancellation or drop tears
down the in-flight phase without detached work. Immutable tool specifications
are cached in deterministic name order when the engine is built and cloned into
each provider request.

Durability divides each loop into explicit phases:

```text
user+turn reservation -> model
model tool calls -> atomic assistant + N unknown placeholders commit
                 -> permission/tool -> exact in-place result replacement -> model
model final answer -> assistant commit -> terminal events
```

The transcript prefix is the optimistic-merge boundary. Allocator and metadata
changes may advance across a retry while messages remain identical; any message
change is divergence and fails closed. This preserves external allocator work
without guessing how concurrent conversation suffixes should be ordered.
Prompt, inference-option, session-metadata, tool-catalog, and complete-transcript
message/serialized-byte limits are checked before their corresponding build,
store, or model boundary. Every committed call therefore has exactly one result
message even if cancellation, an infrastructure error, or a process interruption
prevents its real result from replacing the conservative placeholder. The next
model round is not requested until all placeholders in the current round have
been replaced.

Canonical session state stores its record behind an immutable `Arc`. Reservation
and transcript mutation capture only that cheap identity and persistence bit
under the session mutex, then serialize and deep-clone outside the critical
section. Immediately before starting a store compare-and-save, core reacquires
the mutex and requires the exact record identity and persistence bit to match;
otherwise it retries from the new snapshot. A state change after that recheck is
still caught by the durable revision CAS. Reconciliation similarly performs
whole-record comparisons outside the mutex and uses pointer identity to recheck
before a constant-time state update.

A live turn owns the session lease and provider cancellation signal as one
lifecycle unit. Destruction signals cancellation before removing waiters and
releasing the lease; terminal completion has already released that unit, so its
later destructor is cleanup-only. Out-of-band observer or delivery-state failure
before a terminal provider outcome follows the same cancel-before-release rule,
preventing dropped streams from orphaning retained provider work.
Terminal establishment is the cancellation precedence boundary. A final model
stop remains preterminal through its assistant-message save so cancellation can
wake and release a turn blocked on persistence. After that save, the stop
retains its pending observer delivery and final reason even if cancellation
races afterward. Provider failures and missing stops establish their terminal
outcome when accepted because they have no final assistant commit.
Provider startup, stream, persistence, policy, tool, and observer delivery polls
include a post-poll cancellation observation before their result is
interpreted. Cancellation observed there establishes the terminal outcome first
while the turn remains preterminal. Only a locally synthesized cancellation
bypasses observer delivery; a provider-originated cancelled stop follows normal
durability and observer ordering.
Cancellation treats wakers as user-controlled callback objects: cloning happens
before locking, registry mutation only moves values, and superseded, removed, or
drained wakers are dropped or invoked after unlocking.

Each `Engine` owns a weak session-state registry keyed by `SessionId`. All
create/load races inside that engine converge on one in-memory record and active
turn flag; a live turn itself keeps the state alive if its originating session
handle is dropped. This is an in-process coordination boundary, not a
distributed lease. Registry access uses one requested-ID `BTreeMap` lookup
rather than scanning all live sessions. The
last owner removes its weak entry during state destruction only when pointer
identity still matches, so dead keys are reclaimed without an old destructor
removing a concurrently installed replacement. Independent engines and
processes coordinate durable turn-number allocation through the session store's
optimistic revision contract. Loaded records reconcile strictly and
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
