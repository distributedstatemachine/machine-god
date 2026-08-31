# Milestone 05 `mcp_features` review history

This is the compact historical review record for the contract in
[`../mcp-features.md`](../mcp-features.md). Current phase, delivery, workflow,
and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`3ba687b12281e04c55b6306cc6e21e9e3a2d9bdc`, tree
`9491e9cbfe8be93f15c5d357a2baf34e00634993`. The candidate had passed the
complete exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after all seven actions, canonical requests and schemas, stable identities, prompt/resource content, completion, base64, authority, host, engine, and pinned-upstream review. |
| Lifecycle, platform, and effects | Green | Zero findings after inertness, cancellation and wake precedence, deep/wide iterative ownership, redaction, authority confinement, optional-feature, WASI, FreeBSD, and no-default review. |
| Performance and resources | Green | Zero findings after raw and serialized admission, depth/node/aggregate limits, request ownership, allocation passes, canonical base64, zero-scratch flat teardown, amortized depth scratch, and 50,000-level rejection review. |

## Rejected candidates and remediation

| Candidate | Decisive findings | Replacement |
| --- | --- | --- |
| `6439d3e` | Deep rejected JSON could recurse during serialization or destruction; cancellation, nested content, prompt argument, duplicate, and redundant-work boundaries were incomplete. | `4e3d2d0` added iterative ownership, cancellation rechecks, strict nested validation, prompt obligations, and an allocation-free standard-base64 validator while preserving optional `vision` dependency ownership. |
| `4e3d2d0` | Raw strings and keys could be scanned beyond the byte cap; request canonicalization and ownership were duplicated; completion context and schema entry limits were incomplete. | `ec5624e` added raw-byte preflight, removed duplicate canonicalization and full-request retention, and froze per-entry context and schema bounds. |
| `ec5624e` | Empty completion values were rejected contrary to pinned behavior, and wide JSON teardown allocated width-sized scratch. | `b716e07` admitted empty completions and introduced owning-iterator teardown with width-independent scratch. |
| `b716e07` | Flat teardown still reserved scratch eagerly, and its recoverable reserve-failure fallback could leak the remaining subtree. | `3ba687b` keeps the active iterator in a stack slot, allocates only nested ancestor storage under ordinary Rust allocation semantics, and has no recoverable leak path. |

Every accepted finding rejected its candidate. The complete replacement local
gate and all three fresh review tracks were rerun for the final candidate.
Review worktrees were verified clean and removed after every round.

## Remote delivery evidence

The exact candidate passed feature CI `33425035590` and Benchmark evidence
`33425035564`, then exact-main CI `33426575748` and Benchmark evidence
`33426575586`. Both benchmark runs retained unexpired upstream and bootstrap
artifacts whose names identify the exact accepted behavior SHA.

This review seal makes no package, release, or comparative-performance claim.
The implementation plan remains the sole live source for delivery and workflow
gates.
