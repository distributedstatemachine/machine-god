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

The rejected candidate was replaced by bounded changes through `e35e486`:

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

## Rejected product candidate: cycle 02

Three fresh review tracks inspected exact candidate
`ee67e9ae187b2d16dd01d4ab49779c3bf24b396c` after its complete exact local
gate passed. Every track rejected it.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | A post-rename directory-sync failure could leave a complete running record despite a contract promising publication or nothing; completion outcome and record debug output exposed payloads. |
| Lifecycle, platform, and effects | Observation or group-snapshot failures could release child ownership before kill and reap; compaction deleted some valid history before proving enough victims; reconciliation could misclassify a concurrently compacted record and leave an orphan lock. |
| Performance and resources | The same cleanup and partial-compaction defects could orphan work or mutate rejected reservations; crash-left canonical record temporaries were never reclaimed and could exhaust the bounded lifecycle scan. |

The unique findings require payload-free completion debug, a precise complete-
record publication-ambiguity contract, cleanup that retains ownership through
best-effort group termination and direct-child reap, two-phase compaction,
bounded temporary reclamation, and race-safe reconciliation. Cycle 02 remains
historical rejection evidence and cannot contribute to acceptance.

## Cycle 02 remediation

The rejected candidate was replaced by these bounded changes:

- completion outcomes and completion records use fixed categorical debug
  output, and publication failure explicitly permits only one complete valid
  running record after a successful rename followed by directory-sync failure;
- process cleanup accumulates deterministic failures while still attempting
  group TERM, group KILL, direct-child kill, quiescence, and direct-child reap
  with the observed leader retained until descendant cleanup is exhausted;
- compaction retains enough nonblocking, revalidated victim authorities before
  deleting anything, so insufficient capacity leaves the namespace unchanged;
- the lifecycle scan reclaims bounded canonical orphan temporaries while
  preserving contended owners; and
- reconciliation holds allocator authority, uses existing record locks, and
  cannot race compaction into false corruption or an orphan lock.

Fault, race, survivor, namespace-equivalence, and bounded-scan regressions cover
each change. Acceptance still requires the complete replacement gate and three
entirely fresh zero-finding reviewers on one later exact SHA.
