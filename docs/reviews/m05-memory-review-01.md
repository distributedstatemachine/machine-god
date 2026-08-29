# Milestone 05 `memory` review history

This is the compact historical review record for the contract in
[`../memory.md`](../memory.md). Current phase, delivery, workflow, and next-gate
status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh product-review scopes: correctness/API/compatibility,
lifecycle/platform/effects, and performance/resources. Any finding rejected the
whole candidate and required a complete replacement local gate and three fresh
reviews.

## Candidate history

| Cycle | Exact candidate | Exact tree | Track verdicts | Deduplicated findings |
| ---: | --- | --- | --- | --- |
| 1 | `f74333b65b512ae13eee65e2e947637ca077bf1a` | `f611fc92f40e80ddb708befbdaf6e7244a7a3075` | correctness and lifecycle rejected; performance green | **MEDIUM:** cancellation after permanent-lock creation could precede `fchmod(0600)` and leave an owner-inaccessible lock under a restrictive umask. **MEDIUM:** an exact duplicate save returned before validating an unexpected fixed temporary child. **LOW:** the I/O-exhaustion prose omitted the post-commit `memory_commit_ambiguous` exception. |
| 2 | `102538f8bda66da512c5c66ffe1f67623240a2d7` | `0b75e58dc5af8ce03443140bdad57a6103b95e6e` | lifecycle rejected; correctness and performance green | **MEDIUM:** cancellation injection and restrictive-umask coverage were split across two tests, so neither alone proved the rejected ordering regressed. |
| 3 | `8c405432913a082fba8f2445651883e61023aad6` | `7cb619a1512fce24373ae0d605f0f46779a370e8` | all three green | None. Three fresh reviewers independently reported zero findings. |

## Accepted candidate evidence

Cycle 3 passed the complete local gate with exact Rust and Cargo 1.94.1 before
review: formatting, warnings-denied all-target/all-feature Clippy, workspace and
documentation tests, 147 repository Python tests with eight expected platform
skips, pinned-upstream compatibility drift, the bounded documentation policy,
dependency policy and vulnerability audit, Linux and FreeBSD warnings-denied
target checks, and WASI compilation. The fresh locked release binary passed
bounded version, doctor, and sessions smoke scenarios.

The accepted reviews covered strict action schemas and capability identity,
host and engine integration, descriptor-root identity, cancellation and commit
boundaries, fixed-child lifecycle, concurrent locking, supported and
unsupported targets, serialization and I/O ceilings, allocation complexity,
catalog overhead, and output sizing. Reviewers also confirmed that the
duplicate-save malformed-temp test and the combined cancellation-after-create
test under umask `0777` fail against their respective rejected orderings.

This review seal makes no remote-workflow, delivery, release, live-provider,
upstream-equivalence, or comparative-performance claim. The implementation
plan remains the sole live source for those gates.
