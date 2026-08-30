# Milestone 03 workspace CLI review history

This is the compact historical review record for the contract in
[`../workspace-cli.md`](../workspace-cli.md). Current phase, delivery,
workflow, and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh product-review scopes: correctness/API,
lifecycle/platform, and performance/resources. Any finding rejected the whole
candidate and required a complete replacement local gate plus three fresh
reviews.

## Candidate history

| Cycle | Exact candidate | Exact tree | Track verdicts | Deduplicated findings |
| ---: | --- | --- | --- | --- |
| 1 | `dc2fdb7d3fd983289710a274654de93c1fe84791` | `9aff1ad818687406c33a107072ce6e64b7827ac1` | correctness and lifecycle green; resources rejected | Low: the output-bound regression used ordinary `x` bytes and did not lock the worst-case six-byte JSON escaping expansion at the exact 4,096-byte path boundary. |
| 2 | `1c72f947a55b1cb753dd398f3a5f25dac81f19cd` | `0769c81be99326d113e5741dfb596a196371ee65` | all three green | None. Three fresh reviewers independently reported zero actionable findings. |

## Finding disposition

Cycle 1 was rejected despite two green tracks. The replacement adds a portable
absolute 4,096-byte path dominated by DEL scalars and proves that both human
and JSON rendering escape every control, retain exact LF framing, succeed
without truncation, and remain within the 32,768-byte output ceiling. The
cycle-2 performance/resources reviewer explicitly confirmed that the prior
escaping-expansion gap is closed.

## Accepted candidate evidence

Cycle 2 passed the complete local gate under exact Rust and Cargo 1.94.1 before
review: formatting, warnings-denied workspace Clippy, workspace and
documentation tests, 147 repository Python tests with eight expected
platform-specific skips, the bounded documentation policy, dependency policy
and cached vulnerability audit, pinned-upstream compatibility drift, FreeBSD
unsupported-surface Clippy, WASI no-default compilation, clean-diff and
no-added-unsafe checks, and freshly built release-binary smoke for all four
accepted workspace forms. The pre-existing WASI `read_file` dead-code warning
was unchanged.

The accepted reviews covered strict parse-before-authority grammar, exact
human/JSON/error bytes, one lexical `current_dir` observation, read-only and
provider-neutral boundaries, Windows/Unix/FreeBSD/WASI behavior, path and
output ceilings, worst-case JSON escaping, allocation and syscall bounds,
tests, documentation, and CI smoke agreement.

This review seal makes no remote-workflow, delivery, release,
upstream-equivalence, or comparative-performance claim. The implementation
plan remains the sole live source for those gates.
