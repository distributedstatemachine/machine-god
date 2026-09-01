# Milestone 05 `background` CLI review history

This is the compact historical review record for the contract in
[`../background-cli.md`](../background-cli.md). Current phase, delivery,
workflow, and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`a665289da640d69ae88d0a4a336ad90ece889086`. The candidate passed the complete
exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after strict command forms, stable rendering, exact record schema, duplicate and unknown field rejection, canonical stored paths, normalized configured bases, fixed error taxonomy, and pinned compatibility review. |
| Lifecycle, platform, and effects | Green | Zero findings after descriptor-relative no-follow traversal, Linux `openat2` and `O_NOATIME`, macOS `O_NOFOLLOW_ANY` and ACL policy, bounded environment reads, read-only operation, unsupported FreeBSD/WASI behavior, and native macOS tests. |
| Performance and resources | Green | Zero findings after streaming preflight, fixed depth/node/record/file/identifier/path/argument bounds, early oversized-input rejection, bounded rendering, and no-write regression review. |

## Rejected candidates and remediation

| Candidate range | Decisive findings | Replacement |
| --- | --- | --- |
| `7600463` through `2080351` | Persisted records and stored paths were too permissive, and the unsupported-platform terminal surface failed warnings-denied WASI compilation. | `09e49ed` through `2733b0c` made record decoding strict and bounded, rejected non-canonical stored paths, streamed JSON preflight, added no-follow state-base traversal, fixed the compile incompatibility, and enforced FreeBSD/WASI CI. |
| `2733b0c` through `548eff5` | Large identifier tokens were scanned before a byte bound, macOS ACL handling admitted unsafe record policy, and configured state bases did not preserve documented lexical normalization. | `6325d9a` restored bounded normalized environment bases and retained early identifier and descriptor ACL checks. |
| `6325d9a` through `8b45806` | Raw environment values lacked a pre-decode byte limit, and macOS deny-read ACL classification needed identity-stable preflight. | `a665289` bounded raw bases before decoding, added identity-checked macOS policy handling, and aligned the durable failure contract. |

Every accepted finding rejected its candidate. The complete replacement local
gate and three entirely fresh review tracks were rerun for the accepted
candidate. All isolated review worktrees were verified clean, removed, and
pruned after their iteration.

## Remote delivery evidence

The exact accepted candidate passed feature CI `33454785472` and Benchmark
evidence `33454785491`, then exact-main CI `33455374424` and Benchmark evidence
`33455374426`. Both benchmark runs retained unexpired upstream and bootstrap
artifacts whose names identify the exact accepted behavior SHA.

This review seal makes no package, release, or comparative-performance claim.
The implementation plan remains the sole live source for delivery and workflow
gates.
