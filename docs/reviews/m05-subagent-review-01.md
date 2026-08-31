# Milestone 05 `subagent` review history

This is the compact historical review record for the contract in
[`../subagent.md`](../subagent.md). Current phase, delivery, workflow, and
next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`ba52dbfeb7d05b67ee314934905abee7e0c35ffc`, tree
`b31b79e92acebef4c18cff921038f529a3c88c68`. The candidate passed the complete
exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after strict create-shape decoding, NUL and byte bounds, fixed error taxonomy, structural context identities, canonical preparation, typed execution, pinned behavior, host composition, testkit, and durable-contract review. |
| Lifecycle, platform, and effects | Green | Zero findings after inert-before-poll authority and fixture review, effect-free preparation, same-poll cancellation precedence, wake/drop and permit release, fail-fast admission, inert native defaults, injected constructors, macOS smoke paths, FreeBSD, and WASI review. |
| Performance and resources | Green | Zero findings after raw and serialized JSON bounds, escaping amplification, iterative rejected-value destruction, output bounds, allocation passes, global/per-parent contention, mutex/static state, cancellation/wakers, and testkit capacity review. |

## Rejected candidates and remediation

| Candidate | Decisive findings | Replacement |
| --- | --- | --- |
| `0ffa79f` | The scripted authority recorded and consumed before first poll; input resource failures used the execution error class; security omitted structural context identities; and preparation documentation claimed a typed request too early. | `5a03a16` moved authority work into the future, separated input and execution resource failures, and corrected both durable contracts. |
| `5a03a16` | The core API omitted structural context identities, while core and native review fixtures still mutated state before their returned futures were polled. | `c6806e1` documented the identities as non-authoritative and made every fixture future inert with direct unpolled regressions. |
| `c6806e1` | The testkit overview overpromised string error codes for the kind-only subagent boundary, and the input contract omitted its NUL exclusion. | `ba52dbf` documented the exact `Failed`/`ResourceLimit` mappings and the no-NUL input rule. |

Every accepted finding rejected its candidate. The complete replacement local
gate and three entirely fresh review tracks were rerun for each replacement.
All isolated gate and review worktrees were verified clean, removed, and
pruned after their iteration.

## Remote delivery evidence

The exact accepted candidate passed feature CI `33440115994` and Benchmark
evidence `33440115963`, then exact-main CI `33440894118` and Benchmark evidence
`33440894154`. Both benchmark runs retained unexpired upstream and bootstrap
artifacts whose names identify the exact accepted behavior SHA.

This review seal makes no package, release, or comparative-performance claim.
The implementation plan remains the sole live source for delivery and workflow
gates.
