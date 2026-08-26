# Milestone 03 session CLI review ledger

Status: contract frozen; implementation not started. Bounded slice 32 starts
from exact delivered base `6e687b6872e11845a306c6eaff77b1252a66c393`.

## Frozen boundary

The candidate will implement strict top-level `session <id> [--json]`, the
engine-free native by-ID inspection facade, independent native/CLI/release
evidence, and maintained documentation. The normative contracts are
[`docs/session-cli.md`](../session-cli.md) and
[`docs/native-session-inspection.md`](../native-session-inspection.md).

Two independent read-only discovery agents examined the remaining Milestone 03
CLI work and the pinned fx boundary. Both recommended the same summary-only
slice and independently rejected `ask`, `resume`, `replay`, `workspace`, and
parser-only slash commands as larger or semantically incomplete next steps.
Discovery agents are not formal reviewers and will not be reused for the
required post-gate review.

Implementation will use isolated worktrees with non-overlapping ownership:

| Component | Owned files |
| --- | --- |
| Native production and evidence | Native inspection/state-capture modules, exports, and native focused tests. |
| CLI production and evidence | `crates/machine-god-cli/src/main.rs`, CLI integration tests, and release smoke. |
| Documentation/integration | Contracts, maintained summaries, composition, complete gates, and review ledger. |

## Candidate gate

Before review, the exact composed behavior candidate must pass focused native
and CLI tests followed by the complete exact Rust/Cargo 1.94.1 gate:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

The gate also includes the complete Python suite, exact pinned-fx compatibility
regeneration, relevant target checks, documentation integrity, diff/unsafe
checks, and fresh release-binary execution over success, missing/no-create,
invalid-grammar-before-effects, and exact human/JSON output.

## Required adversarial review

After the complete candidate gate, three fresh isolated read-only agents review
the same exact commit and tree:

1. CLI correctness, API contract, output/error behavior, and pinned-fx boundary;
2. native state-root, persistence, race, error, and portability behavior; and
3. performance, resource bounds, concurrency, benchmark classification, and
   evidence completeness.

Every report must end with exact counts for blocker, high, medium, and low
findings. Any finding rejects the entire candidate. Remediation must be
composed, the complete replacement gate rerun, and three new agents must review
the replacement exact SHA. No prior discovery, implementation, review, or
remediation agent may approve its own work or be reused in a later review
cycle. Only a deduplicated `0/0/0/0` candidate is formally green.

## Delivery gate

A documentation-only review seal may record returned zero-finding verdicts
without another product review. The sealed feature SHA must pass CI and
benchmark-evidence workflows before `main` is fast-forwarded without force.
The exact integrated `main` SHA must pass both workflows, and each benchmark
run must retain exactly two unexpired exact-SHA artifacts. No package
publication or GitHub release is authorized.
