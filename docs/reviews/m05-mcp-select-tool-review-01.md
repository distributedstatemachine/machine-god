# Milestone 05 `mcp_select_tool` review history

This is the compact historical review record for the contract in
[`../mcp-select-tool.md`](../mcp-select-tool.md). Current phase, delivery,
workflow, and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`c5d86c91f2a39f808ebd63039884344d6c130eac`, tree
`5285265a8728ad07561a20b27292a6c9911e9566`. The candidate had passed the
complete exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after exact selection, next-round advertisement, dispatch, ordering, isolation, persistence, schema-boundary, and compatibility review. |
| Lifecycle, platform, and effects | Green | Zero findings after authority, authorization, cancellation, persistence-before-activation, turn/session isolation, executor-identity, and target-surface review. |
| Performance and resources | Green | Zero findings after schema depth/node/byte bounds, snapshot ownership, insertion and lookup complexity, retained state, output, and cancellation-bound review. |

## Rejected candidates and remediation

| Candidate | Decisive findings | Replacement |
| --- | --- | --- |
| `eea0e0c` | Reselection compared metadata without binding executable identity; wide-schema validation retained excess queued work; durable documentation was incomplete. | `8fd5c79` required exact executor identity, bounded iterative schema validation, and aligned the contract. |
| `8fd5c79` | The depth-64 acceptance and depth-65 rejection boundary lacked frozen evidence; authorization obligations and reference-host ownership were under-documented. | `57ce8c1` added the boundary tests and clarified the public contracts. |
| `57ce8c1` | Dynamic tools were advertised lexically instead of in selection order, and deferred-scope wording contradicted the implemented overlay. | `5de7713` preserved activation order while retaining exact lookup and corrected the durable wording. |
| `5de7713` | Evergreen host wording still described only metadata and intent; `execute_for_turn` did not state the authorization obligations imposed on direct callers. | `54f87c0` aligned the host overview and documented the authority boundary. |
| `54f87c0` | Core API prose still claimed every tool call received the authorization decision, contradicting the explicit direct-call boundary. | `c5d86c9` aligned the authorization dispositions without changing runtime behavior. |

Every accepted finding rejected its candidate. The complete replacement local
gate and all three fresh review tracks were rerun for the final candidate.
Review worktrees were verified clean and removed after each round.

## Remote delivery evidence

The exact candidate passed feature CI `33406742121` and Benchmark evidence
`33406742128`, then exact-main CI `33407513449` and Benchmark evidence
`33407513315`. The main benchmark run retained unexpired upstream and bootstrap
artifacts whose names and recorded head SHA both identify the accepted
candidate.

This review seal makes no release or comparative-performance claim. The
implementation plan remains the sole live source for delivery and workflow
gates.
