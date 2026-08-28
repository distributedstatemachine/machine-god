# Milestone 03 top-level `ask` review history

This is the compact historical review record for the contract in
[`../ask-cli.md`](../ask-cli.md). Current phase, delivery, workflow, and
next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle uses three fresh scopes: correctness/API/compatibility,
lifecycle/platform/effects, and performance/resources. Any substantive finding
rejects the complete candidate. Repeated agent prose and per-check transcripts
are omitted; the table retains the exact candidate, tree, verdict, and
deduplicated findings.

## Candidate history

| Cycle | Exact candidate | Exact tree | Track verdicts | Deduplicated findings |
| ---: | --- | --- | --- | --- |
| 1 | `1a968500b131a18fe5f126631242bbcea570f897` | `1baef8f1d06aa5d2826fb4b5677ef6a5caffb4fc` | all three rejected | **HIGH:** synchronous standard-output backpressure could block signal observation and turn cancellation. **MEDIUM:** the output-error drain stopped observing a later signal; this was unique to the correctness track. **MEDIUM:** no explicit final flush meant a late output failure could be lost. **MEDIUM:** the Tokio deadline adapter used panic catching as runtime-driver control flow. **LOW:** the documented configuration/current-directory order differed from implementation. |
| 2 | `1cdc59394ff675395b9eab1c012969d8855b271e` | `74abe95fe55051410a8b0a016072483e39c40cbc` | all three rejected | **HIGH:** signal observation was not independently driven through setup, blocking session persistence, diagnostics, and final return; dropped Tokio receivers could leave signals swallowed. **MEDIUM:** fallible scoped-thread creation used a panicking API. **MEDIUM:** a signal during a write could restart the 100 ms output grace for flush. **MEDIUM:** `status` compaction removed durable CLI behavior. **LOW:** the flush promise included early paths that had no output bridge. **LOW:** caught writer panics still ran the uncontrolled panic hook. **LOW:** blocked-child test failures did not always kill and reap the child. |
| 3 | `a3eddcda85ec32dacc7de72362bee6f071449145` | `d47c12d4222501639f01c083d9d83e337ceee355` | correctness and lifecycle rejected; performance green | **HIGH:** a full capacity-one turn-signal channel made a repeated signal exit before terminal cleanup. **MEDIUM:** a same-poll signal could be classified in setup before an already queued turn activation. **MEDIUM:** partial Tokio signal registration could drop one permanently installed handler without retaining a receiver. **MEDIUM:** the CLI path lacked deterministic successful composed-host output and persistence evidence. **LOW:** the sole live plan had not advanced from replacement gate to cycle-three review. |

## Rejected-candidate disposition

The exact Rust and Cargo 1.94.1 local gate was green for cycle 1, but local-gate
success did not override adversarial review. Because all three independent
tracks rejected the candidate, it is not an accepted behavior or delivery
candidate. This record makes no delivery, remote-workflow, measured-performance,
live-provider, or full pinned-fx equivalence claim.

Cycle 1 remediation produced cycle 2, whose exact replacement local gate was
green. All three fresh cycle-2 tracks nevertheless rejected that candidate;
focused reviewers also reproduced the swallowed-signal failure with a freshly
built binary and saturated standard error. Cycle 2 remediation produced cycle
3, whose complete exact local gate was also green. Its performance track found
no issue, but correctness and lifecycle findings still rejected the complete
candidate. No replacement candidate or remediation verdict is recorded here.
Any later candidate must pass the complete replacement gate and three fresh
review tracks; the implementation plan remains the sole live source for that
work.
