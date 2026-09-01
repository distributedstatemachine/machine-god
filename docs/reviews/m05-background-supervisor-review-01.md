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

## Rejected product candidate: cycle 03

Three fresh review tracks inspected exact candidate
`1297cb36ed3e0b8cf68e7c651c707e7de046e451` after the complete exact local gate
passed. Every track rejected it.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Publication between the record and control snapshots could make compaction unlink a newly required lock; cancellation after an installed but pending or ambiguous initial publication could bypass stopped replacement. |
| Lifecycle, platform, and effects | Requested loader-control environment variables could execute code before the persistence gate; losing child wait authority could still signal a reusable numeric process group. |
| Performance and resources | Retaining an authority for every in-bound orphan or victim could exhaust a valid reduced process file-descriptor limit and prevent maintenance progress. |

Cycle 03 remains rejection evidence. Replacement requires snapshot-safe lock
classification, a fixed-FD maintenance protocol, cancellation closure after
ambiguous publication, inert framed environment transfer to a safe bootstrap,
and explicit lost-wait-authority handling before another fresh review cycle.

## Cycle 03 remediation

The rejected candidate was replaced by these bounded changes:

- once initial publication has been polled, cancellation aborts the prepared
  process and attempts stopped replacement even after pending or ambiguous
  persistence outcomes;
- unexpected acquired locks are revalidated against concurrent publication
  before mutation, and store maintenance retains at most eight authorities
  while making progress at a tested 24-descriptor process limit;
- Linux and macOS launch the private bootstrap with a fixed safe environment;
  the bounded command and requested environment are framed only at release and
  applied solely to the final `/bin/sh` exec; and
- `ECHILD` is preserved as lost wait authority, which immediately disables all
  later numeric process and group queries or signals; incompatible Linux child-
  reaping modes are rejected before spawn.

Deterministic cancellation, publication-race, reduced-descriptor, loader-
variable, signal-mode, and external-reap regressions cover the replacement.
Acceptance still requires the complete gate and three new zero-finding reviews.

## Rejected product candidate: cycle 04

Three fresh review tracks inspected exact candidate
`f06b2d508a4dd64e2b6192ca661c27e153a00be1` after its complete exact Rust and
Cargo 1.94.1 local gate passed. Every track rejected it.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Successful supervisor shutdown or post-release dispatch cleanup could persist `dead`; successful owned termination and reap must persist `stopped`, with `dead` reserved for ambiguous cleanup. |
| Lifecycle, platform, and effects | Linux group membership depended on GNU-specific `/bin/ps` behavior; Linux requires a bounded distro-independent `/proc` scan while macOS retains its bounded platform adapter. |
| Performance and resources | Polling could execute blocking process preparation and release on the caller's async executor; the supervisor requires fixed-size, fail-fast blocking offload with cancellation and drop retaining cleanup ownership. |

Cycle 04 remains rejection evidence and cannot contribute to acceptance.
Replacement requires the bounded supervisor-owned blocking worker boundary,
platform-specific bounded group discovery, and outcome-accurate stopped/dead
persistence described by the durable contract. The complete exact-1.94.1
replacement gate must pass before three entirely fresh review tracks begin.

## Rejected product candidate: cycle 05

Three fresh review tracks inspected exact candidate
`8b78bae20c491cd1eafc8df2e3e34b2db8120c0d` after its complete exact Rust and
Cargo 1.94.1 local gate passed. Every track rejected it.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Post-publication cancellation could publish `stopped` despite ambiguous prepared-process cleanup; dropping one submitted start cancelled a caller token shared with unrelated work; a Linux-enabled helper test had a macOS-only import. |
| Lifecycle, platform, and effects | A restrictive Linux procfs `hidepid` mount could hide a credential-changed group member, allowing false quiescence and a leaked descendant. |
| Performance and resources | Replacing and dropping a task waker while holding the result mutex could deadlock publication, exhaust blocking capacity, and hang shutdown. |

Cycle 05 remains rejection evidence. Replacement requires fallible prepared
abort classification, private drop cancellation, visibility-complete procfs
admission, waker destruction outside shared locks, and Linux test compilation.
The complete replacement gate and all three fresh review tracks restart after
every finding is remediated.
