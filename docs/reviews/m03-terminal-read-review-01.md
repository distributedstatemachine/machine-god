# Milestone 03 terminal `read` review history

This is the compact historical review record for process-local background
output reads through [`terminal`](../terminal.md). Current delivery, workflow,
and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Rejected and superseded candidates

| Candidate | Finding and resolution |
| --- | --- |
| `fd61448` | Exact Linux CI exposed `ESRCH` while reading a retained proc stat descriptor after task disappearance. The replacement classifies only disappearance errors and empty retained reads as vanished, while other I/O failures remain closed. |
| `d5121b7` | Lifecycle review found the retained read also needed the documented `ENOENT` disappearance case. Performance review found capture was enabled for callers without an output owner and that returned pages were copied twice. The replacement added exact error coverage, explicit null-versus-capture release framing, and moved each page into its result. |
| `321f262` | All required gates and reviews passed, but broader container execution exposed a PID-namespace gap: when machine-god is PID 1, adopted killed descendants can remain zombies. The final candidate identity-rechecks a pidfd and consumes status only for a captured zombie adopted by the current process. Foreign ownership, PID reuse, and unsupported pidfd paths remain fail-closed. |

## Accepted candidate

Three fresh adversarial tracks reviewed exact candidate
`b1bbb81f5dbb5dcb4010cc2b49dbf95561c94811` and reported **zero findings**:

- correctness/API checked the closed schemas, same-incarnation ownership,
  cursor/result invariants, helper framing, fixed errors, and documentation;
- lifecycle/platform checked proc identity races, PID namespaces and PID 1,
  pidfd/wait semantics, cancellation, old-kernel behavior, and platform seams;
- performance/resources checked capture opt-in, registry and page bounds,
  allocation/copy behavior, lock scope, syscall/backoff ceilings, descriptor
  lifetime, output floods, and process cleanup.

Exact Rust 1.94.1 formatting, warnings-denied Clippy, workspace tests, doctests,
FreeBSD/WASI checks, and release smoke passed. The Linux background-process
suite passed 68/68 in five consecutive container runs before review and again
serially and in parallel during review. Feature CI `33904974540` and Benchmark
evidence `33904974534` passed for the exact candidate. Main CI `33905683447`
and Benchmark evidence `33905683324` then passed for the same SHA with both
exact-SHA artifacts retained. Every auxiliary worktree was clean and removed.

This is regression and delivery evidence only. It does not promote a broader
pinned-fx compatibility or performance claim.
