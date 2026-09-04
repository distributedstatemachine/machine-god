# Agent instructions

## Workflow

- Read `docs/implementation-plan.md` before changing code.
- Treat `docs/implementation-plan.md` as the only live source for the current
  phase, delivered-slice count, delivered-main SHA, workflow IDs, and next gate.
- Work on one bounded feature branch at a time.
- Use isolated worktrees for parallel agents and assign non-overlapping files.
- Do not revert another agent's changes; adapt to integrated work.
- Update documentation with behavior in the same commit.
- Update behavior documents only when durable behavior changes. Review ledgers
  are historical evidence, not live status dashboards; do not duplicate live
  phase, delivered count, or workflow IDs outside the implementation plan.
- Do not call work complete until local checks, adversarial review, and remote
  CI pass for the exact commit.

Documentation-only maintenance, review-result seals, and delivery records that
change no product behavior are exempt from a new adversarial product-review
cycle. They still require proportionate local checks and the exact lightweight
remote CI and Benchmark aggregate gates. They do not require Rust, platform,
audit, or benchmark-evidence jobs and produce no new benchmark artifacts. Such
maintenance does not increment the delivered-slice count.

Compact `docs/implementation-plan.md` after every five deliveries or whenever
it exceeds 600 lines. Preserve its canonical live-status block, milestone
state, compact delivered-slice inventory, remaining boundary, and gates; keep
detailed review history under `docs/reviews/`.

## Required checks

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Run focused tests first, then the whole workspace. Exercise user-visible work
through the freshly built `target/release/machine-god` binary.

CI and local checks use the exact Rust 1.94.1 toolchain. A damaged local
installation must be repaired with `rustup`; substituting `+stable` does not
satisfy the pinned-toolchain gate.

## Architecture

- `machine-god-core` owns provider-neutral contracts and orchestration.
- `machine-god-native` owns operating-system, network, and persistence effects.
- `machine-god-cli` is a thin host and must not own product state.
- `machine-god-testkit` owns deterministic test doubles and fixtures.
- Core has no ambient filesystem, process, environment, or network authority.
- Unsafe Rust is forbidden unless a future ADR narrows an audited exception.

## Git and remote operations

- Branch names: `agent/mNN-feature-slug`.
- Use conventional commits.
- Never force-push `main`.
- Push feature branches, wait for their exact CI SHA, then fast-forward `main`.
- Product and evidence-affecting commits require exact CI and Benchmark
  artifact-producing success. Documentation-only commits use the bounded
  classification contract in `docs/ci-change-classification.md`.
- Do not publish packages or GitHub releases without separate authorization.
