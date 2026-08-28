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

## Cycle 1 disposition

The exact Rust and Cargo 1.94.1 local gate was green for cycle 1, but local-gate
success did not override adversarial review. Because all three independent
tracks rejected the candidate, it is not an accepted behavior or delivery
candidate. This record makes no delivery, remote-workflow, measured-performance,
live-provider, or full pinned-fx equivalence claim.

No replacement candidate or remediation verdict is recorded here. Any later
candidate must pass the complete replacement gate and three fresh review tracks;
the implementation plan remains the sole live source for that work.
