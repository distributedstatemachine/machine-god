# Milestone 05 `skill` review history

This is the compact historical review record for the contract in
[`../skill.md`](../skill.md). Current phase, delivery, workflow, and next-gate
status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh product-review scopes: correctness/API/compatibility,
lifecycle/platform/effects, and performance/resources. Any finding rejected the
whole candidate and required a complete replacement local gate and three fresh
reviews.

## Candidate history

| Cycle | Exact candidate | Exact tree | Track verdicts | Deduplicated findings |
| ---: | --- | --- | --- | --- |
| 1 | `5c2301d289302df700a584368d8dc1fc24382225` | `0100e9e4d04fe6e169b451f4b8c28cc34d52391d` | correctness and performance rejected; lifecycle green | **MEDIUM:** direct execution cloned unbounded JSON strings before enforcing argument limits. **MEDIUM:** a one-byte read eagerly reserved maximum-sized file and UTF-8-boundary buffers. |
| 2 | `73d4bcb1f13cf3c2f06ac506100884e8bc68048c` | `8c61eeaa6a75288909418ba446151f9dba69fdbf` | lifecycle rejected; correctness and performance green | **MEDIUM:** FreeBSD reports `EMLINK` for an `O_NOFOLLOW` symlink, but the mapper recognized only `ELOOP`, producing the wrong error code and retry classification. |
| 3 | `0fae5abe099f24055c57f835cfdf5a78e36497ee` | `3a50ac407d8435030726a726e065a0c5da646cc0` | all three green | None. Three fresh reviewers independently reported zero findings. |

## Accepted candidate evidence

Cycle 3 passed the complete local gate with exact Rust and Cargo 1.94.1 before
review: formatting, warnings-denied all-target/all-feature Clippy, workspace and
documentation tests, 147 repository Python tests with eight expected platform
skips, pinned-upstream compatibility drift, the bounded documentation policy,
dependency policy and vulnerability audit, FreeBSD warnings-denied target
checks, WASI compilation, clean-diff/no-added-unsafe checks, and the freshly
built locked release-binary smoke scenarios.

The accepted reviews covered strict schemas and capability identity, retained
workspace descriptors, traversal and symlink handling, cancellation and I/O
exhaustion, supported and unsupported targets, full-file UTF-8 admission,
pagination and serialization ceilings, allocation complexity, and composed-host
identity. Reviewers confirmed that the direct-execution and one-byte allocation
regression rejects cycle 1, while the synthetic FreeBSD `MLINK` regression
rejects cycle 2.

This review seal makes no remote-workflow, delivery, release,
upstream-equivalence, or comparative-performance claim. The implementation plan
remains the sole live source for those gates.
