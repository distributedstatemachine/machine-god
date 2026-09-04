# Milestone 03 terminal `wait` review history

This is the compact historical review record for exact persisted-background
waiting through [`terminal`](../terminal.md). Current delivery, workflow, and
next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

The slice begins from exact delivered base
`2186e2f0b487c8ccf4f7638b400f7593cbefd946`. It adds only the closed
`{"action":"wait","background_id":N,"return_when":{"kind":"exit"},"wait_ceiling_ms":M}`
form. The action observes one exact persisted record, never infers process
liveness, and owns no process-control authority.

## Rejected candidates

Every product finding rejected the whole candidate. Each replacement passed a
complete exact Rust 1.94.1 local gate before three fresh review tracks began.

| Exact candidate | Finding and remediation |
| --- | --- |
| `803dbfc` | Initial reviews found publication/cancellation races, detached deadline ownership, and outcome-contract gaps. The replacement made deadline waits directly owned, checked cancellation at publication boundaries, and aligned the durable outcomes. |
| `58caceb` | Reviews found that the direct timer still lacked a lifecycle-paired runtime and that the observation-cap resource claim lacked end-to-end proof. The replacement paired timer futures with runtime identity/lifecycle state and added retained-memory and exact-cap evidence. |
| `e157f47` | Lifecycle review found close/admission races, incomplete runtime-identity validation, and unsafe blocking runtime destruction from async contexts. The replacement closed admission atomically, validated every nonterminal poll, captured readiness before teardown, and extracted the runtime exactly once for nonblocking shutdown. |
| `1cb4117` | Final review found delay readiness sampled after blocking teardown, a public-description mismatch, and missing deadline checks at observation/delay poll admission. The replacement captured readiness inside the decisive poll, aligned the exact description, and enforced cancellation and ceiling checks at every stage boundary. |
| `2b2e5df` | The replacement local gate exposed Cargo ancestor discovery attaching the excluded reentrant-Waker fixture in a nested worktree to the primary checkout. The fixture now declares and locks its own workspace, and a fresh nested-worktree formatting and strict-Clippy regression passes. |
| `4af4adf` | Correctness and resource reviewers found that a legal non-self-waking pending inspector had no registered deadline wake and could retain all four wait slots indefinitely. The accepted replacement races both pending inspections and pending backoffs against one persistent absolute-ceiling timer and proves Waker replacement, wake delivery, synchronous teardown, and full permit recovery. |

## Accepted candidate

Three fresh adversarial tracks reviewed exact candidate
`f16f099b73c2f1ab2513f9bc7d9c41630e2cb439`, tree
`f2ca17677f2103f0a1e64dbee906eca61ffa809a`, and reported **zero findings**:

- correctness/API checked the closed schema, preparation, record validation,
  all outcome and error precedences, construction/poll/readiness/teardown
  boundaries, exact descriptions, and nested-worktree fixture behavior;
- lifecycle/platform checked timer and runtime close/admission, runtime
  identity, reentrant Waker destruction, async-context shutdown, cancellation,
  outer-future drop, permit recovery, adapter ownership, and supported and
  unsupported platform surfaces; and
- performance/resources checked four fail-fast slots, the 128-observation cap,
  one persistent ceiling timer plus at most one backoff timer, at most 128 timer
  constructions, compact snapshot retention, wake cleanup, and the absence of
  supervisor, process, worker, task, or thread tails.

Focused regressions cover real asynchronous Waker replacement and delivery,
four-slot pending-inspection expiry, pending-backoff expiry, early/erroring
timers, deadline crossings at every controllable boundary, destructor-triggered
cancellation, observation and allocation caps, reference-host composition, and
Tokio runtime closure. All detached gate and review worktrees were clean and
removed before delivery.

## Acceptance evidence

The exact candidate passed formatting; warnings-denied all-target/all-feature
Clippy; workspace tests and doctests; repository Python tests; documentation
and pinned compatibility checks; dependency policy and vulnerability audit;
FreeBSD and WASI compilation; no-added-unsafe and clean-diff checks; and a fresh
locked release-binary smoke under exact Rust and Cargo 1.94.1. Feature and
fast-forward main CI passed for the same SHA, and both artifact-producing
Benchmark gates retained their required exact-SHA artifacts.

This is regression and delivery evidence only. It does not promote a new
performance comparison or broader pinned-fx compatibility claim.
