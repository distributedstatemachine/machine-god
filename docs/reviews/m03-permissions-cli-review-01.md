# Milestone 03 `permissions` CLI review 01

Status: **IN PROGRESS — IMPLEMENTED; LOCAL GATE AND FORMAL REVIEW PENDING**

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

The isolated components are:

- production `c75d44a26f401b6151563b11416d78aaf0ca8d03`, tree
  `a6ee3296d1b183529238d29a7663dcd28df74d66`;
- independent evidence `54832ba8ec10743b49d0154499fdf66c30b90dd1`,
  tree `c030e2ce1290c926cb0229a443979c97067cd4f8`; and
- maintained documentation `2379d90e2226fcbe20d86a5e19eb276cd9a63b5c`,
  tree `a470d603ab5b325ac031a04c1d40964b46c1c2db`.

They are composed in one feature change before candidate validation. None was
pushed independently.

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

No formal-review candidate has been nominated. The composed implementation
must first pass the complete local gate above.
