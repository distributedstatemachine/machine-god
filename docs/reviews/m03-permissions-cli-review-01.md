# Milestone 03 `permissions` CLI review 01

Status: **IN PROGRESS — CONTRACT FROZEN; IMPLEMENTATION PENDING**

## Base and boundary

- Exact delivered base:
  `8d8ecc7a37f866251d4047c01acdf1bbd485f4da`.
- Integration branch: `agent/m03-permissions-cli`.
- Normative contract: [`permissions-cli.md`](../permissions-cli.md).
- Pinned comparison reference: `vercel-labs/fx` commit
  `b1774fbf6c7602b503026f96f6e960e946c692ef`.

This ledger covers only top-level `permissions [--json]`. It excludes
interactive slash commands, modes beyond `ask`, persistent rule or grant state,
session/provider/runtime composition, compatibility promotion, benchmark
workloads, and product-performance or fx-equivalence claims.

## Component ownership

Parallel agents use isolated worktrees and non-overlapping files:

1. **Production** owns `crates/machine-god-cli/src/main.rs`.
2. **Independent evidence** owns `crates/machine-god-cli/tests/cli.rs`.
3. **Documentation** owns maintained behavior summaries after production and
   evidence compose.

No component may revert another agent's changes, edit generated compatibility
artifacts manually, add dependencies, or move product state into the CLI.

## Acceptance gate

Focused parser and release-binary tests run first. The exact candidate then
passes:

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

An allowed stable fallback must report exact release 1.94.1 for both Rust and
Cargo and must be recorded. A newer floating stable is not evidence.

## Formal review protocol

After the complete local gate, three fresh agents independently inspect the
same immutable candidate SHA and tree:

1. **Correctness/API** — grammar, exact bytes, configuration projection,
   unchanged commands, and maintained-document agreement.
2. **Native config/error lifecycle** — effect ordering, read-only/no-create
   behavior, schema versions, failure precedence, and redaction.
3. **Performance/CLI portability** — bounded work and allocation, dependency
   delta, non-Unicode/process behavior, platform cfg, and release-binary checks.

Each report states blocker/high/medium/low counts. Any finding rejects the
candidate. Remediation receives a complete replacement gate and three fresh
same-SHA reviews. Documentation-only green seals and delivery records are
exempt from redundant adversarial review under the user's instruction.

## Review cycles

No implementation candidate has been nominated.
