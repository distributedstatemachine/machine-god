# Milestone 02 core contracts review 01

Reviewed candidate: `2bb8db7` (rebased form of `c4177f2`).

Three adversarial lifecycle findings were confirmed and resolved:

- A permanently pending event observer prevented the turn from polling its
  cancellation signal. Delivery now registers the turn's keyed cancellation
  waiter, gives cancellation priority, drops the pending observer future, yields
  the terminal cancellation directly, and releases the session lease.
- Session-local atomic turn counters restarted at one after reload and could
  collide across independently loaded handles. `SessionRecord` now persists the
  next sequence. `Session::prompt` asynchronously reserves it with optimistic
  store concurrency before exposing a `Turn`, reloads and retries stale handles,
  requires increasing revisions, and has a fixed conflict retry bound.
- `Cancelled` futures left cloned wakers in an append-only vector after drop.
  Waiters now use one keyed registry entry per live waiting future or turn.
  Re-poll updates that entry, cancellation drains it, and `Drop` deregisters it.

Regression coverage includes a permanently pending sink, reload continuity,
two stale session handles racing the same durable sequence, repeated waiter
polling, dropped waiters, and selective wake delivery.

A fresh review of `af90f8d` found that the active-turn flag was still owned by
each independently created `SessionState`. Durable CAS prevented turn-ID reuse
but did not prevent two loaded handles from streaming concurrently. The engine
now owns a weak registry keyed by `SessionId`; create/create, load/load, and
load/create races converge on one state and live-turn lease within that engine.
The turn lease holds the state strongly, dead weak entries are pruned, and
persisted records reconcile monotonically by revision. Tests cover every handle
combination and registry cleanup. Documentation explicitly limits this live-turn
invariant to one engine instance; no cross-process lease is claimed.

A fresh review of `261a535` found three remaining interleaving and retention
issues. A delayed successful save could overwrite state that a concurrent load
had already advanced, conflict reloads bypassed the strict load reconciliation
path, and a turn retained its last poller's waker after yielding a nonterminal
event. Successful saves now update canonical state only when it is not newer,
while the reserved snapshot continues to drive the already-created request.
Conflict reloads reject zero sequences, stale revisions, and same-revision
divergence through `SessionState::reconcile_loaded`. Nonterminal ready events
deregister the turn waiter before returning. Controlled save/load interleaving,
all three hostile conflict records, and turn-level waker retention have dedicated
regression tests.

A fresh review of `66d14d3` found that a pending observer future skipped the
cancellation-registration refresh on later polls, so cancellation could wake W1
after W2 had taken over polling. It also found that successful-save
reconciliation treated an equal revision as installable even when a concurrent
load had installed different contents. Pending delivery now refreshes its keyed
waiter before every observer poll and rechecks cancellation before continuing.
Save reconciliation accepts equal revisions only for identical records and
propagates divergence as `EngineError::Protocol`; newer canonical state remains
untouched and a valid saved record still replaces an older canonical revision.
Regression tests control both the W1/W2 cancellation sequence and the
same-revision save/load interleaving.

A fresh review of `1408119` found that dropping a live `Turn` released its
in-process lease and waiter but did not signal the cancellation token already
given to the provider. Providers that retained the token could therefore leave
background work running without an owning turn. `Turn::drop` now cancels first
when the live lease is still held, then uses the idempotent finish path to
deregister and release. A retained-provider-token regression verifies that the
token is cancelled and a stale `TurnHandle::cancel` returns false after drop. A
separate completed-turn regression registers an external waiter and verifies
that cleanup-only drop neither changes the completed token nor wakes it.

A fresh review of `b35d203` found that `Engine::load_session` published a newly
loaded state into the weak registry before `reconcile_loaded` rejected an
intrinsically invalid zero turn sequence. A concurrent create in that narrow
window could retain the corrupt state after the load returned its error.
Intrinsic persisted-record validation now runs before any registry access, and
the same validation remains in reconciliation for existing state. Regression
coverage checks the registry boundary directly and coordinates a pending corrupt
load with a create, then proves the surviving handle remains revision zero with
sequence one and reserves `turn-1` rather than poisoned `turn-0` state.

A security and robustness review of `c9815f1` found that persisted revision zero
was accepted even though core uses it as the unsaved sentinel. That converted a
loaded record into `Some(SessionRevision(0))` CAS semantics and blurred the
new-record boundary. Persisted-record validation now rejects revision zero before
publication and on conflict reload, while `SessionRecord::empty` retains zero
until its first successful save returns a positive revision. Direct-load,
conflict-reload, and normal initial create/save regressions cover all three
paths.

A fresh correctness review of `bb4ebc6` found that nonterminal `EventSink`
failure dropped the provider stream and released its lease without cancelling
the shared token. A provider retaining that token could continue background work
after the turn returned `EngineError::EventSink`. All premature observer and
delivery-state failure exits now cancel before the common finish path and mark
the out-of-band failure terminal, while already-terminal provider behavior is
unchanged. A retained-provider regression accepts `Started`, rejects the first
model event, and verifies the sink error, cancelled token, false stale-handle
cancel result, exhausted turn stream, and released lease.

A fresh review of `4789dc6` found that a conflict reload returning `None`
unconditionally cleared the canonical persistence flag after an async gap. A
concurrent `Engine::load_session` could install revision N+1 during that gap, only
for the stale reservation to retry with `expected_revision=None` and make later
reconciliation replaceable through the cleared flag. Reservation now retains its
entire attempted snapshot and clears persistence only if both record and status
still match under lock. Load and successful-save reconciliation compare revisions
monotonically regardless of persistence status. Controlled tests cover a blocked
missing reload followed by a concurrent N+1 load and `Some(N+1)` retry, plus a
legitimate missing-record clear followed by a rejected stale reload.

A security review of `c565046` found that waiter registration cloned and
replaced wakers, and deregistration removed and dropped them, while holding the
registry mutex. Custom `RawWaker` clone/drop callbacks can reenter cancellation
and deadlock that non-reentrant mutex. Registration now clones before locking,
performs only map moves inside a tight guard scope, and drops unused or
superseded wakers afterward. Deregistration likewise drops the removed value
after unlocking; cancellation already drains before waking. Bounded reentrant
tests cover clone, replacement drop, deregistration drop, and wake callbacks.
The audited unsafe fixture required to exercise raw clone/drop callbacks is an
excluded dev-only helper described by ADR 0002; production and workspace crates
remain under the original `unsafe_code = "forbid"` policy.

A fresh correctness review of `dc192a1` found that reconciliation treated a
higher revision as sufficient to replace canonical state even when its
`next_turn_sequence` moved backward, allowing later `TurnId` reuse. Loaded and
successful-save reconciliation now reject every allocator regression before
revision or persistence handling and leave canonical state unchanged. Higher
revisions may still update messages and metadata when their sequence is
nondecreasing; equal revisions still require full record equality. Direct load,
conflict reload, and delayed successful-save interleaving regressions cover the
three paths, and the direct-load case also proves valid higher-revision
message/metadata changes remain accepted.

A fresh correctness review of `45a8fd9` found that cancellation always replaced
a pending observer delivery, including a provider `Stop` or structured failure
that had already established the turn's terminal outcome. Final reasons could
therefore change with poll timing. Pending-delivery cancellation now respects
the provider terminal boundary: an established stop or failure and its
subsequent terminal event cannot be superseded. The existing direct path for an
already-staged `Completed(Cancelled)` remains non-blocking. Regressions cancel
after a delivered stop, while stop delivery is pending, and while provider
failure delivery is pending, and separately preserve direct staged cancellation.
