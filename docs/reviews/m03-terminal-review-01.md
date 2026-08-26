# Milestone 03 native `terminal` review ledger

Status: **IN PROGRESS — CONTRACT FROZEN**

Slice 34 begins from exact delivered base
`52b5885f275c9f6f4f16b378f71780c29f2ebab2`. Its normative boundary is
[`../terminal.md`](../terminal.md). It implements only bounded foreground
`exec`; every other pinned-fx terminal action remains deferred.

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
