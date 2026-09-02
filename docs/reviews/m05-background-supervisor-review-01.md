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

## Rejected product candidate: cycle 10

Three fresh review tracks inspected exact candidate
`3d07a9905364a3ae7807f082db695602de94b779` after its complete exact Rust and
Cargo 1.94.1 local gate passed. All three tracks rejected the candidate.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Cancellation-first repolls could mask a simultaneously ready preparation or fallible abort; release write failures could be mislabeled by later cancellation; bounded pool shutdown detached unresolved workers without an explicit retained ownership contract. |
| Lifecycle, platform, and effects | The cancellation ordering affected all three core selectors, and native child probing treated every `try_wait` error as irrevocably lost wait authority instead of distinguishing interruption and `ECHILD`. |
| Performance and resources | Escaped captured members could survive the disappearance deadline; macOS snapshot reader join, retainer joins, and shutdown cleanup could block; detached worker ownership was implicit; Linux polling allocated a decimal PID string per probe. |

Cycle 10 remains rejection evidence. Replacement requires started-operation
failure precedence with cancellation-before-first-poll inertness, failure-site
release classification, errno-specific reap ownership, complete captured-member
resolution, independently bounded macOS snapshot collection, asynchronous
fixed-capacity shutdown cleanup with explicit retained ownership, bounded
retainer teardown, and allocation-free repeated PID observation. The complete
replacement gate and all three fresh review tracks restart after every finding
is remediated.

## Rejected product candidate: cycle 11

Three fresh review tracks inspected exact candidate
`89ba68d7d74fe6f37f32b5f8cc4905eefba0c739` after its complete exact Rust and
Cargo 1.94.1 local gate passed. All three tracks rejected the candidate.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Proven cancellation during helper preparation was reported as `Process`, while a ready persistence failure could still be replaced by `Cancelled`. |
| Lifecycle, platform, and effects | Dropping the supervisor did not cancel a start already submitted to the blocking pool, leaving a release window after supervisor drop. |
| Performance and resources | Interrupted helper-readiness reads retried without a deadline check or park, and long cleanup waits used repeated two-millisecond sleeps. |

Cycle 11 remains rejection evidence. Replacement requires failure-site
preparation cancellation classification, persistence-error precedence after a
started publication, race-safe cancellation of every submitted start during
shutdown, deadline-aware interrupted readiness, and bounded coarse sleep work.
The complete replacement gate and all three fresh review tracks restart after
every finding is remediated.

## Rejected product candidate: cycle 08

Three fresh review tracks inspected exact candidate
`4048ae9d47a1b88e87e8208c4d6d49137a68d9d7` after the complete exact local gate
passed. All three tracks rejected the candidate.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | The complete pre-KILL membership snapshot was discarded; a credential-changed survivor could escape its group before post-KILL capture and then be treated as gone, allowing false successful cleanup. |
| Lifecycle, platform, and effects | Same-poll cancellation could mask a preparation cleanup failure, and the post-readiness cancellation path dropped its prepared process while ignoring fallible abort. |
| Performance and resources | Child reap probing and release-frame writes remained synchronously unbounded after helper readiness; captured-member backoff could repeatedly traverse a 32,768-member vanished prefix. |

Cycle 08 remains rejection evidence. Replacement requires stronger cleanup
errors to survive cancellation arbitration, bounded cancellation-aware probe
and frame I/O, retained identity-bound pre-KILL membership across group escape,
and amortized captured-member observation. The complete replacement gate and
all three fresh review tracks restart after every finding is remediated.

## Rejected product candidate: cycle 09

Three fresh review tracks inspected exact candidate
`0ee41506d1ad7dffb8a53147b125cf3ef1ffd8b3` after the complete exact local gate
passed. All three tracks rejected the candidate.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | EOF served as both abort and release commit, so cancellation after a complete frame could still execute the command; proven release cancellation was misclassified as `Process`/`dead` instead of `Cancelled`/`stopped`. |
| Lifecycle, platform, and effects | Same-poll and subsequent prepared-process cancellation boundaries still dropped the prepared handle without invoking its fallible abort, allowing cleanup failure to be masked as cancellation. |
| Performance and resources | Cleanup retained unconditional reap and worker joins; the three-snapshot member union was quadratic and could retain 98,304 entries; EPERM could exceed the four-scan contract; Linux member polling allocated a new stat buffer on every backoff. |

Cycle 09 remains rejection evidence. Replacement requires an explicit release
commit that abort EOF cannot emulate, fallible abort at every prepared-
cancellation boundary, cancellation/stopped classification after proven
cleanup, bounded unresolved-reap and worker ownership, one aggregate linear
member cap, no EPERM rescan, and reusable polling storage. The complete
replacement gate and all three fresh review tracks restart after every finding
is remediated.

## Rejected product candidate: cycle 06

Three fresh review tracks inspected exact candidate
`faf112bac133087456e1052d6582e8289d7477a6` after the complete exact local gate
passed. Correctness reported zero findings; lifecycle and performance rejected
the candidate.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Zero findings. |
| Lifecycle, platform, and effects | Procfs admission proved one path snapshot but retained no descriptor-bound mount authority, so a mountinfo overmount or later procfs topology change could invalidate descendant visibility during cleanup. |
| Performance and resources | Caller cancellation depended on the same blocking worker that could be occupied by synchronous start work; mountinfo parsing amplified a maximum line into a large field vector; group scans allocated a fresh stat buffer per PID. |

Cycle 06 remains rejection evidence. Replacement requires independent caller-
side cancellation forwarding, a retained descriptor-relative procfs authority
with mount identity and topology revalidation, one-pass bounded mountinfo
parsing, and one reusable stat buffer per group scan. All three review tracks
restart after the complete replacement gate.

## Rejected product candidate: cycle 07

Three fresh review tracks inspected exact candidate
`6325c3b136ec232a97a567441b9ed04b8c4062ff` after the complete exact local gate
passed. Correctness reported zero findings; lifecycle and performance rejected
the candidate.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Zero findings. |
| Lifecycle, platform, and effects | Caller cancellation observed before blocking-pool submission still attempted admission, so a saturated or contended pool could return `Capacity` instead of `Cancelled` and an available pool could execute known-cancelled work. |
| Performance and resources | Helper readiness was not cancellation-aware and could retain a blocking slot for its two-second timeout; lingering cleanup could perform roughly 54 whole-system process snapshots per job; maximum environment uniqueness validation was quadratic. |

Cycle 07 remains rejection evidence. Replacement requires cancellation to win
before unsubmitted blocking admission, interruptible helper preparation with
owned abort and reap, a constant bounded number of global process snapshots
during cleanup, and one-pass or linearithmic bounded environment validation.
The complete replacement gate and all three fresh review tracks restart after
every finding is remediated.

## Rejected product candidate: cycle 12

Three fresh review tracks inspected exact candidate
`37d60b41362c5a4272f3a760764f926b9ca47cd8` after its complete exact Rust and
Cargo 1.94.1 local gate passed. All three tracks rejected the candidate.

| Track | Decisive findings |
| --- | --- |
| Correctness, API, and compatibility | Native preparation rustdoc still described proven cancellation as `Spawn`, and core API wording placed release irrevocability before the actual commit barrier. |
| Lifecycle, platform, and effects | A worker could dequeue `Shutdown` before a racing registrar enqueued `Run`; submission returned success, the queued job was discarded, and its future remained pending forever. |
| Performance and resources | Rejected environments were cloned before bounds validation; several release, reap, and macOS snapshot loops retained tight or two-millisecond polling; the global worker collector permanently scanned all retained handles every ten milliseconds. |

Cycle 12 remains rejection evidence. Replacement requires one serialized
per-worker admission/shutdown transition, borrowed bounded environment
validation before ownership transfer or ambient collection, deadline-aware
coarse backoff across every process polling loop, event-driven completed-worker
collection, and exact typed-cancellation documentation. The complete
replacement gate and all three fresh review tracks restart after every finding
is remediated.
