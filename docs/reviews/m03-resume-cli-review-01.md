# Milestone 03 top-level `resume` review history

This is the compact historical review record for the contract in
[`../resume-cli.md`](../resume-cli.md). Current phase, delivery, workflow, and
next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

Each cycle used three fresh scopes: correctness/API/compatibility,
lifecycle/platform/effects, and performance/resources. Any finding rejected the
complete candidate. Repeated agent prose and per-check transcripts are omitted;
the table retains the exact candidate, tree, verdict, and deduplicated findings.

## Candidate history

| Cycle | Exact candidate | Exact tree | Track verdicts | Deduplicated findings |
| ---: | --- | --- | --- | --- |
| 1 | `4b2c4e0f44e59e795ad2bec088f698f5d465b58d` | `2aa1c49b84d3878ba0e936eb1d6e5de3de1b0994` | all three rejected | **HIGH:** the shared ID parser regressed the existing `session -alpha` literal-ID grammar. **MEDIUM:** concurrent prompt reservation and later transcript divergence were described as a single immediate failure. **MEDIUM:** the resource contract claimed only one record read despite compare-and-swap rereads and bounded retries. **LOW:** literal `last` was rejected without being explicitly reserved. |
| 2 | `c29d3156add3da6055935b36c3fdfb2c2cdbdc54` | `413ffcf3ddb3be0a6bb983712cab6a170fb5a04b` | correctness rejected; lifecycle and performance green | **LOW:** the existing session contract still described top-level `resume` as future work. |
| 3 | `f07b965259761b13f7a0ee419952043cb2e73ac6` | `6734e3030e581b0e6b6370c1eb1029111fcd14f8` | performance rejected; correctness and lifecycle green | **MEDIUM:** the resume contract called retries bounded without distinguishing inherited unbounded advisory-lock waits, filesystem latency, and `EINTR` retries. |
| 4 | `3da4aba91dd57fb60b25753bef40d5d91498e80d` | `8a07d80642ff3ff190a99356365844e1bcbc6175` | all three green | None. Three fresh reviewers independently reported zero findings. |

## Accepted candidate evidence

Cycle 4 passed the complete local gate with Rust and Cargo 1.94.1 before
review: formatting, warnings-denied all-target/all-feature Clippy, workspace
tests, doc tests, 147 repository Python tests with eight expected skips, pinned
upstream compatibility drift, the bounded documentation policy check,
dependency policy, vulnerability audit, Linux and FreeBSD feature checks, and
the WASI workspace check. The freshly built release binary passed the bounded
`resume` help, invalid-grammar, and missing-session smoke scenarios; the local
feature-worktree artifact was 5,938,640 bytes.

The accepted reviews independently verified strict explicit-ID grammar without
regressing session inspection, exact transcript continuation, persistence and
conflict semantics, cancellation and output ownership, unsupported-platform
behavior, startup/resource bounds, and agreement among the durable contracts.

This review seal makes no remote-workflow, delivery, live-provider,
upstream-equivalence, release, or comparative-performance claim. The
implementation plan remains the sole live source for those gates.
