# Milestone 03 terminal `list` review history

This is the compact historical review record for persisted-background listing
through [`terminal`](../terminal.md). Current delivery, workflow, and next-gate
status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Rejected candidates

Each finding rejected the complete candidate. Its replacement passed the full
exact Rust 1.94.1 local gate before three fresh review tracks began.

| Candidate | Finding and resolution |
| --- | --- |
| `568578c` | Correctness review found that a constructible list-plus-inspect terminal advertised inspect while omitting it from the model description. The replacement added the exact action-combination description and regression coverage. |
| `1906d67` | Resource review found that native persisted-record validation admitted ID zero, leaving the terminal layer to misclassify storage corruption as an injected-lister defect. The replacement rejects zero before projection and adds native-taxonomy and composed-host redaction evidence. |

## Accepted candidate

Three fresh adversarial tracks reviewed exact candidate
`a8f5af3987ec4a4c79af79d0428830d93d48f9df` and reported **zero findings**:

- correctness/API checked every constructible schema/description combination,
  strict preparation, fixed error mapping, output invariants, documentation,
  engine persistence, and reference-host composition;
- lifecycle/platform checked cancellation and same-poll teardown, list-permit
  lifetime and recovery, retained-descriptor confinement, supported and
  unsupported platform boundaries, and absence of process-control authority;
- performance/resources checked directory, candidate, per-record, aggregate,
  JSON-shape, row-count, allocation, and serialization bounds, deterministic
  ordering and uniqueness, hostile persistence, and private-field exclusion.

Focused evidence included seven terminal-list tests, eighteen native background
inspection tests, three composed-host list tests, and one engine integration
test. The complete formatting, warnings-denied Clippy, workspace test, doctest,
Python, documentation, unsupported-platform, dependency-policy, audit, release,
and benchmark gates passed under exact Rust 1.94.1. Every review worktree was
clean and removed before delivery.

This is regression and delivery evidence only. It does not promote a broader
pinned-fx compatibility or performance claim.
