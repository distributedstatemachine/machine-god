# Milestone 05 background supervisor review history

This is the compact historical review record for the contract in
[`../background-supervisor.md`](../background-supervisor.md). Current phase,
delivery, workflow, and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Rejected product candidate

Three fresh review tracks inspected exact candidate
`a3e9f5470aa90d6cfc66010dcb99962beecb8c8e`. The candidate passed the complete
exact Rust and Cargo 1.94.1 local gate before review. Every track rejected it.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Reconciliation made normal history unusable after 100 records; the path constructor did not bind canonical identity to its descriptor; completion persistence could mask primary cancellation/process errors; public debug output exposed IDs and PIDs. |
| Lifecycle, platform, and effects | `EPERM` could be mistaken for group disappearance despite a credential-changed survivor; Linux left user stdin on the consumed gate pipe instead of `/dev/null`. |
| Performance and resources | Terminal JSON and per-ID locks grew without retention; idle jobs polled every 2 ms; the allocator used a blocking cross-process lock inside the first async poll. |

## Cycle 01 remediation

The rejected candidate was replaced by bounded changes through `4dc62e4`:

- lifecycle scanning and compaction cap total record or pending occupancy at
  100, preserve active work, retain the newest terminal history, and reclaim
  unowned records and locks;
- allocator, constructor, and new-record lock contention fail promptly;
- canonical workspace device/inode identity is checked against the retained
  descriptor;
- primary start outcomes survive replacement-persistence failure and public
  debug output is redacted;
- Linux user stdin is `/dev/null`, permission denial fails closed, and bounded
  pre-reap membership snapshots prevent PGID-reuse ambiguity; and
- idle observation backs off from 2 to 32 ms and cancellation wakes the parked
  worker directly.

Every finding rejected the entire candidate. Acceptance requires the complete
replacement local gate and three entirely fresh reviewers on one later exact
SHA; no result from this rejected cycle is reused as acceptance evidence.

