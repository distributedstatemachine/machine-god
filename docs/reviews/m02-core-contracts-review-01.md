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
