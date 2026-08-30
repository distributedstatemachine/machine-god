# Milestone 05 `install_skill` review history

This is the compact historical review record for the contract in
[`../install-skill.md`](../install-skill.md). Current phase, delivery, workflow,
and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh product-review scopes: correctness/API,
lifecycle/platform, and performance/resources. Any finding rejected the whole
candidate and required a complete replacement local gate plus three fresh
reviews.

## Candidate history

| Cycle | Exact candidate | Exact tree | Track verdicts | Deduplicated findings |
| ---: | --- | --- | --- | --- |
| 1 | `988f7ff81f815281f50032f2b05b53cd83ec1df0` | `31a17041d2b4dade0ac7c5a95e1e36158be3a58c` | all three rejected | Inclusive depth was off by one; destination collision had the wrong kind; the catalog count and error contract were incomplete; managed-root relocation could produce false success; stage acquisition and cleanup had replacement and unbounded-work gaps; absent-root durability omitted an outer-directory sync; macOS aliases were admitted; and directory names were buffered before entry admission. |
| 2 | `73e01fce39824df71caa8fa4e0c6abe2d1b6e73d` | `e8e399d7e83afe79d887e5688d1cbde52fc43534` | all three rejected | Cancellation was not rechecked directly before rename; postcommit identity failure skipped parent sync; post-`mkdirat` acquisition failures leaked residue; injected or replaced staged content could produce success; APFS long-s and Kelvin aliases remained; and the exact error table used prose rather than variant names. |
| 3 | `d30b71dffdf2ffbe023fcac702d590a177c1ba4e` | `d6b7d7a24039143071586593e4d14a0c941e136f` | correctness and lifecycle rejected; resources green | An uncertain rename error returned before recursive recovery and parent sync; nested empty directories lacked their own prepublication sync. |
| 4 | `91630b2081ba951e9391ac41650e00db430b09d8` | `23d225b9aa595df52a55c544bb5fb5ba367d62af` | all three green | None. Three fresh reviewers independently reported zero actionable findings. |

## Accepted candidate evidence

Cycle 4 passed the complete local gate with exact Rust and Cargo 1.94.1 before
review: formatting, warnings-denied workspace Clippy, workspace and
documentation tests, 147 repository Python tests with eight expected
platform-specific skips, pinned-upstream compatibility drift, bounded
documentation policy, dependency policy and vulnerability audit, FreeBSD
unsupported-surface compilation, clean-diff/no-added-unsafe checks, and a
fresh locked release-binary smoke. Focused reviewers additionally exercised
the supported install/engine/lifecycle suites and Linux, FreeBSD, and WASI
compilation surfaces.

The accepted reviews covered canonicalization and capability identity,
Unicode managed-root aliases, descriptor-relative traversal, stage acquisition
and cleanup, source and destination races, pre- and postcommit recursive
identity/content validation, cancellation, ambiguous rename recovery,
durability, supported and unsupported platforms, allocation and operation
bounds, hashing, descriptor ceilings, and exact result/error contracts.

This review seal makes no remote-workflow, delivery, release,
upstream-equivalence, or comparative-performance claim. The implementation
plan remains the sole live source for those gates.
