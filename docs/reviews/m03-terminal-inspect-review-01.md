# Milestone 03 terminal `inspect` review history

This is the compact historical review record for exact persisted-background
inspection through [`terminal`](../terminal.md). Current delivery, workflow,
and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

The slice begins from exact delivered base
`9792b38b23f5e924581e75559ddc8a85b5231563`. It adds only
`{"action":"inspect","background_id":N}` through an explicitly injected
inspector. The action performs one bounded exact-ID persisted-record read and
makes no process-liveness or control claim.

## Rejected candidates

Each finding rejected the whole candidate. Its replacement passed the complete
exact Rust 1.94.1 local gate before three fresh review tracks restarted.

| Exact candidate | Finding and remediation |
| --- | --- |
| `af552921` | Correctness found that adding inspect widened the legacy exec-only schema, while lifecycle found that dropping a ready inspector future could publish success after destructor-triggered cancellation. The replacement restores the exact legacy schemas unless an inspector is injected and checks cancellation after destroying the inspector future. |
| `105325fd` | Lifecycle found that destroying the cancellation waiter's stored Waker could itself reentrantly cancel after the decisive check. The replacement destroys both the inspector and waiter futures before the final cancellation decision and checks again before publishing success. |

## Accepted candidate

Three fresh adversarial tracks reviewed exact candidate
`bd4e97d5dd1db3a02bad9938adf8c086ee1a5d8f`, tree
`c2497fcdf511a955601be7fb0f013f1f9e88a69d`, and reported **zero findings**:

- correctness/API checked legacy and injected schemas, strict parsing,
  canonicalization, projection, error mapping, reference-host wiring, tests,
  and durable contracts;
- lifecycle/platform checked provider-future and Waker teardown ordering,
  caller and final cancellation precedence, descriptor confinement, and the
  absence of supervisor, process, thread, timer, or liveness effects; and
- performance/resources checked constant-record-count exact lookup, the single
  64 KiB-bounded read, bounded JSON/output, cancellation cleanup, and the
  absence of listing, worker, timer, process, or capacity work.

Focused regressions cover pending inspector wake/drop, ready-future destructor
cancellation, raw-Waker destructor cancellation, exact lookup, schema
preservation, no-authority preparation, and reference-host composition. All
three detached review worktrees were clean and were removed and pruned before
delivery.

## Acceptance evidence

The exact candidate passed formatting; warnings-denied all-target/all-feature
Clippy; workspace tests and doctests; repository Python tests; documentation
and compatibility checks; dependency policy and vulnerability audit; FreeBSD
and WASI compilation; no-added-unsafe and clean-diff checks; and a fresh locked
release-binary smoke under exact Rust and Cargo 1.94.1. Feature and fast-forward
main CI passed for the same SHA, and both artifact-producing Benchmark gates
retained their required exact-SHA artifacts.

This is regression and delivery evidence only. It does not promote a new
performance comparison or broader pinned-fx compatibility claim.
