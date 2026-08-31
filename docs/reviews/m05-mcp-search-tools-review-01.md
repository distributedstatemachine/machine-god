# Milestone 05 `mcp_search_tools` review history

This is the compact historical review record for the contract in
[`../mcp-search-tools.md`](../mcp-search-tools.md). Current phase, delivery,
workflow, and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`a8a94f9595c297e6c1342ef6ad8c6a05ad07a347`, tree
`8024711e9336bf2e429084729357f78ac4912ff2`. The candidate had passed the
complete exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after contract, strict-schema, ordering, matching, projection, host-composition, and compatibility review. |
| Lifecycle, platform, and effects | Green | Zero findings after cancellation precedence, wake/drop behavior, catalog authority, inert construction, redaction, and target-surface review. |
| Performance and resources | Green | Zero findings after snapshot ownership, allocation, scan/work, serialization, output, and cancellation-bound review. |

## Rejected candidates and remediation

| Candidate | Decisive findings | Replacement |
| --- | --- | --- |
| `2f3e300` | Cancellation and same-poll precedence were incomplete; a non-cooperative catalog could remain unwoken; retained capacities and snapshot clones exceeded the intended bounds; debug output exposed metadata; the schema limit and server-name validation contradicted the runtime contract. | `89831d7` added the explicit cancellation race and wake/drop ownership, exact-size shared metadata, redacted debug shapes, schema/runtime agreement, and the documented bounded server syntax. |
| `89831d7` | Search scanned every match before truncation and could report a false work-limit failure; JSON Schema counted characters where the runtime counted UTF-8 bytes; model-visible metadata bypassed the pinned scalar encoding boundary. | `a8a94f9` stopped after `limit + 1` matches, removed the false character maximum, and bounded encoded model projection without cutting entities. |

Every accepted finding rejected its candidate. The complete replacement local
gate and all three fresh review tracks were rerun for the final candidate.
Review worktrees were verified clean and removed after each round.

This review seal makes no release or comparative-performance claim. The
implementation plan remains the sole live source for delivery and workflow
gates.
