# Milestone 03 native `terminal` review ledger

Status: **IN PROGRESS — CONTRACT FROZEN**

Slice 34 begins from exact delivered base
`52b5885f275c9f6f4f16b378f71780c29f2ebab2`. Its normative boundary is
[`../terminal.md`](../terminal.md). It implements only bounded foreground
`exec`; every other pinned-fx terminal action remains deferred.

## Initial composition

The review-exempt frozen-contract checkpoint is exact `79ae1b7`. Exact isolated
host component `ecf3e78c4abc70bb4f3329a6f8dffa9237ff130b`, production precursor
`ea216db80cb38601da268d84dd05b962c802c5df`, independent-evidence component
`785b193e2735401ffd3966ed6ef84db637891d59`, and lifecycle remediation
`64a48504275afc0e6989cee03eeeeb8174267225` are followed by configurable-system
component `13dfd28b9f49424c5272545b7ce6ceb75f445b92` and expanded Linux-evidence
component `ebc631e136bd052a41ff104534b5ecc0d5613d93`. They compose through local
exact `59b069a84e7d4dc4d76ac65520b9045603cae8af`.

Early independent evidence found that an inline reentrant Waker makes the
literal no-thread-tail wording impossible: a worker cannot join itself from its
own callback. The accepted invariant is stronger where it matters. Every child,
original process-group member, pipe, reader, descriptor, and capacity permit is
cleaned before publication; only the resource-free notification callback tail
may self-detach. Non-self paths join. Production also remediated pre-spawn
deadline, reader-join, escaped-writer, exit/signal-range, duration, pending-
executor deadline, cancellation-precedence, and portability gaps before any
formal candidate.

Exact focused evidence is green for four private limit/deadline/outcome tests,
nineteen portable contract/lifecycle tests, two engine permission/durability
tests, one unsupported-platform test, two workspace clone/failure tests, and
eight reference-host tests. Seven real Linux process tests compile and await
Linux execution. The complete exact candidate gate and formal review have not
begun.

## Required composition

Production, independent tests, and maintained documentation are owned in
non-overlapping isolated worktrees. Each component must be committed, verified
clean, integrated, and then removed and pruned. The composed candidate must pass
focused tests, all four exact-1.94.1 required commands, portability and
release-mode evidence before review.

Three fresh read-only adversarial product tracks review one exact immutable
candidate and tree:

1. correctness, public API, schema, capability, and engine integration;
2. native filesystem/process effects, cancellation, lifecycle, and platform
   behavior; and
3. performance, concurrency, memory/output bounds, and resource ownership.

Findings are recorded as blocker/high/medium/low. Confirmed findings are fixed
and the complete gate plus three fresh tracks repeat until each track and the
deduplicated union report `0/0/0/0`. This is ordinary terminal-agent product
review, not a cybersecurity assessment.

## Delivery gate

Only a review-green exact candidate may be pushed as the feature branch. Its
exact feature CI and Benchmark evidence SHA must pass before `main` is
fast-forwarded without force. Exact main CI and Benchmark evidence must then
pass, with the expected exact-SHA artifacts retained. No package or GitHub
release is authorized.

The final record will append exact component commits, candidate/tree, local
evidence, every review report and adjudication, workflow IDs and SHAs,
integration result, and worktree cleanup. Documentation-only result and
delivery seals follow the user's review exemption and do not restart product
review.
