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
