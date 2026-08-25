# Milestone 03 `permissions` CLI review 01

Status: **IN PROGRESS — CYCLE 2 REMEDIATED; REPLACEMENT GATE AND REVIEW PENDING**

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

### Exact cycle-1 local gate

Exact candidate `fe2475329b50e89bc069eded3eb2f398e8e1a167`, tree
`757f5169f03bf4018b4d85830cf37a2b716cd0cb`, passed its complete local gate
under Rust and Cargo 1.94.1 without fallback:

- the focused CLI suites passed six unit tests and 19 integration tests;
- formatting, warnings-denied workspace all-target/all-feature Clippy,
  workspace tests, and workspace doctests all passed;
- the pinned compatibility generator check and all 31 generator tests passed;
- a fresh 368,944-byte release binary had SHA-256
  `3900c435bb108056f7916764a2b9542e479368a6b9166a499eae06d8f9b0dba3`;
- release-binary human, JSON, valid-config no-rewrite, invalid-config
  redaction, parse-precedence, help, and no-create smokes passed; and
- the candidate added no dependency and no unsafe Rust.

This gate is evidence for the rejected candidate only. It makes no formal-
review, remediation, workflow, integration, delivery, compatibility,
performance, or fx-equivalence claim.

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

### Cycle 1 — NOT GREEN

All three tracks reviewed exact candidate
`fe2475329b50e89bc069eded3eb2f398e8e1a167`, tree
`757f5169f03bf4018b4d85830cf37a2b716cd0cb`:

| Track | Blocker | High | Medium | Low | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| Correctness/API | 0 | 0 | 0 | 0 | **GREEN** |
| Native config/error lifecycle | 0 | 0 | 0 | 0 | **GREEN** |
| Performance/CLI portability | 0 | 0 | 1 | 2 | **NOT GREEN** |
| Deduplicated union | 0 | 0 | 1 | 2 | **NOT GREEN** |

Any finding rejects a candidate, so exact cycle 1 is rejected. The findings
are:

1. **MEDIUM — unbounded interrupted reads.** The configuration reader retries
   every `std::io::ErrorKind::Interrupted` result without a cumulative attempt
   bound. The 64 KiB plus one-byte storage limit remains intact, but a hostile
   or pathological reader can keep one synchronous CLI invocation doing work
   indefinitely.
2. **LOW — unnecessary state-environment observation.** The permissions path
   constructs the general process environment snapshot, including
   `XDG_STATE_HOME`, even though it resolves and loads only configuration and
   must not inspect state-root selection.
3. **LOW — stale maintained process evidence.** The exact candidate's
   summaries still described the complete local gate and formal candidate as
   pending instead of identifying the exact precursor evidence and rejected
   cycle-1 SHA/tree above. This is a process-evidence finding, not a production
   defect, and is corrected as part of cycle-1 remediation.

### Cycle-1 remediation composed

Exact isolated components are:

- production `4cf50b6f7e7ddec00e1e251902e5b9983036dd7b`, tree
  `fb4d0dc7c7501ccd3547cdf1fcc83b84623f4a08`, changing only
  `crates/machine-god-native/src/config.rs`; and
- documentation `278dd45b9e38e8912d803832c37962b84faf9fe5`, tree
  `536ef22cff93d5c478600e7554b65c8442b896ab`, changing only the maintained
  cycle record and behavior summaries.

They are composed in one remediation change. The replacement:

- applies one cumulative 16-`Interrupted` limit to configuration reads, allowing
  the first 15 interrupted results to retry and returning the existing fixed
  `Unreadable` failure on the 16th;
- adds deterministic injected-reader evidence for success after up to 15
  interruptions and fixed failure on the 16th;
- requests `XDG_CONFIG_HOME` and then `HOME` only for configuration loading and
  permissions, never requesting `XDG_STATE_HOME`; status retains its separate
  `XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`HOME` snapshot; and
- reconciles the maintained current-candidate and local-gate record.

The composed replacement therefore required a complete local gate and three
fresh reviews on one new immutable SHA and tree. Its exact gate and cycle-2
result follow. Feature and `main` workflows, integration, and delivery remain
pending.

### Exact cycle-2 replacement gate

Exact candidate `e0d590608640d7fe95f307163c99efd3e90fd2b3`, tree
`cd8919b1ff86af1b1bfbd0421a8280fc57473444`, passed the complete replacement
local gate under Rust and Cargo 1.94.1 without fallback:

- formatting, warnings-denied workspace all-target/all-feature Clippy,
  workspace tests, and workspace doctests all passed;
- focused native-configuration and CLI suites passed;
- the pinned compatibility generator check and all 31 generator tests passed;
- a fresh 368,944-byte release binary had SHA-256
  `9ff974588808823a0419b150bd9b30a016cc377f8bf84f0c5aac2a14035784fe` and
  passed the release-binary smoke matrix; and
- the candidate added no dependency and no unsafe Rust.

This replacement gate is evidence for the rejected cycle-2 candidate only. It
makes no cycle-2 remediation, workflow, integration, delivery, compatibility,
performance, or fx-equivalence claim.

### Cycle 2 — NOT GREEN

All three tracks reviewed exact candidate
`e0d590608640d7fe95f307163c99efd3e90fd2b3`, tree
`cd8919b1ff86af1b1bfbd0421a8280fc57473444`:

| Track | Blocker | High | Medium | Low | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| Correctness/API | 0 | 0 | 0 | 0 | **GREEN** |
| Native config/error lifecycle | 0 | 0 | 0 | 0 | **GREEN** |
| Performance/CLI portability | 0 | 0 | 0 | 2 | **NOT GREEN** |
| Deduplicated union | 0 | 0 | 0 | 2 | **NOT GREEN** |

Any finding rejects a candidate, so exact cycle 2 is rejected. The findings
are:

1. **LOW — eager `HOME` observation and allocation.** The config-only process
   snapshot reads and stores `HOME` even when a nonempty `XDG_CONFIG_HOME`
   already decides selection. A nonempty selected XDG value, whether valid,
   relative, or non-Unicode, must decide without reading or falling back to
   `HOME`.
2. **LOW — overbroad no-follow contract.** The permissions contract describes
   the selected final configuration path as no-follow without its supported-
   Unix qualifier, even though hardened non-Unix opening remains explicitly
   deferred. The production guarantee is final-path `O_NOFOLLOW` and
   nonblocking open on supported Unix targets only.

### Cycle-2 remediation composed

Exact isolated components are:

- native `fa83c6c6427028c18e1c36ba6603eb44e4102eac`, tree
  `a9ac7a1c147cb2ea61c61bcbf8cb58ac407bb14f`, changing only
  `crates/machine-god-native/src/config.rs`; and
- documentation `1f8968d7592de544be3c5549c275c6bc876e62c0`, tree
  `058aa738487bfce08be2d964f8a577fdc12fea09`, changing only the eight
  maintained behavior and review documents named by its component scope.

They are composed in one remediation change. Integration additionally adds the
Windows cfg mirror of the non-Unicode process-snapshot regression. The
replacement:

- read `XDG_CONFIG_HOME` first for config loading and permissions, read `HOME`
  only when XDG is missing or empty, and never read `XDG_STATE_HOME`;
- prove that a nonempty valid, invalid-relative, or non-Unicode selected XDG
  value neither reads nor falls back to `HOME`, while missing and empty XDG
  values take the existing `HOME` fallback;
- qualify final-path `O_NOFOLLOW`, nonblocking, no-follow, and descriptor-
  regularity guarantees as supported-Unix behavior, retaining hardened non-
  Unix opening as deferred; and
- reconcile the maintained current-candidate and gate record.

The composed replacement must pass another complete local gate and receive
three fresh reviews on one new immutable SHA and tree. Feature
and `main` workflows, integration, and delivery remain pending.
