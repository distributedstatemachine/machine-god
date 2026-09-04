# Implementation plan

Status values are `NOT STARTED`, `IN PROGRESS`, `BLOCKED`, and `COMPLETE`.
This file is the repository's only live implementation, delivery, and gate
ledger. Durable behavior belongs in the linked contract documents; detailed
review history belongs in `docs/reviews/`.

## Objective

Build a high-performance Rust 1.94.1 coding-agent engine inspired by the pinned
`vercel-labs/fx` revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`. The embeddable asynchronous
engine is the primary product and the CLI is its native reference host.

Core stays provider-neutral and effect-free. Native adapters receive explicit
authority for operating-system, persistence, process, and network effects.
Observable compatibility and performance claims require retained evidence
against the pinned upstream revision. Zig is only an upstream benchmark build
input; it is not a machine-god product language or runtime dependency.

## Current delivery state

<!-- canonical-live-status:start -->
- Delivered slices: `56`
- Delivered main: `a8f5af3987ec4a4c79af79d0428830d93d48f9df`
- Main CI: `33872064499` (`GREEN`)
- Main Benchmark evidence: `33872064372` (`GREEN`)
- Active branch: `agent/m01-affected-ci`
- Active phase: `dependency-aware CI routing maintenance without product behavior changes`
- Next gate: `pass focused workflow tests and adversarial CI review, then exact lightweight remote aggregate gates`
<!-- canonical-live-status:end -->

The exact delivered-main CI and Benchmark runs are green, and the Benchmark
run retains both required unexpired exact-SHA artifacts. Slice 56 adds the
authority-free `terminal` list action over retained persisted background
records. It returns at most 100 rows ordered by `(updated_at_ms, id)` descending
under fixed scan, record-byte, serialization, and four-slot concurrency bounds,
without exposing commands, paths, process identity, exit data, URLs, or native
diagnostics. Missing history is an empty complete result; corrupt storage,
including a zero durable ID, fails closed. Three fresh final review tracks
reported zero findings after two rejected candidates closed a description/schema
mismatch and the zero-ID persisted-record gap. Detailed history is retained
only in the linked review ledger.

The active non-product iteration replaces binary full-versus-documentation CI
routing with fail-closed dependency-aware concern selection while preserving
the stable aggregate gates and exact-SHA Benchmark evidence policy. It does not
increment the delivered-slice count or require a product-review cycle.

After that maintenance, the next product slice returns to unfinished tool or CLI behavior. Managed
background output and read, persistent interactive handles, write/control
actions, interactive terminal behavior, restart-safe control, ACP, teams, and
extension slash commands remain separate until explicitly bounded.
Documentation-tool maintenance remains a separate non-product task.

## Architecture ownership

- `machine-god-core` owns provider-neutral contracts and orchestration. It has
  no ambient filesystem, process, environment, terminal, or network authority.
- `machine-god-native` owns explicitly injected operating-system, network,
  terminal, configuration, and persistence effects.
- `machine-god-cli` is a thin host and owns no product state.
- `machine-god-testkit` owns deterministic test doubles and fixtures.
- Unsafe Rust is forbidden unless a future ADR narrows an audited exception.
- Constructors and futures must preserve the documented inert-before-poll,
  cancellation, resource-bound, redaction, and authority invariants.

## Delivery workflow

1. Read this plan and the relevant durable contract before changing code.
2. Use one bounded `agent/mNN-feature-slug` branch. Parallel subagents use
   isolated worktrees with non-overlapping file ownership.
3. Implement behavior, focused tests, and durable documentation in the same
   commit series. Do not revert another agent's changes.
4. Run focused checks, then the complete exact-1.94.1 local gate.
5. Freeze one exact behavior SHA and run three fresh adversarial product reviews:
   correctness/API, lifecycle/platform, and performance/resources. Any finding
   rejects the candidate; fix it, rerun the complete replacement gate, and use
   three fresh reviewers until all tracks report zero findings.
6. Push the feature branch and require CI and Benchmark evidence to succeed for
   that exact behavior SHA with both benchmark artifacts retained.
   Fast-forward `main` without force, then require the exact `main` CI and
   artifact-producing Benchmark evidence runs to succeed.
7. Verify every worktree is committed, integrated where required, and clean,
   then safely remove and prune it. Never remove active or uncommitted work.

Documentation-only maintenance, review-result seals, and delivery records that
change no product behavior are exempt from another adversarial product-review
cycle. They still require proportionate local checks and exact lightweight CI
and Benchmark aggregate gates before being called complete, but their Rust,
platform, audit, compatibility, and benchmark-evidence jobs are intentionally
skipped and they produce no new benchmark artifacts. A commit cannot record
its own future workflow IDs; retain the last artifact-producing behavior runs
in the canonical live block and report later documentation-only gates at
handoff.

## Milestones

| Milestone | Deliverable | Status |
| --- | --- | --- |
| M01 | Repository, documentation, CI, workspace, pinned upstream benchmark harness, and non-product bootstrap evidence | COMPLETE |
| M02 | Provider-neutral streaming engine and deterministic testkit | COMPLETE |
| M03 | Providers, native tools, permissions, sessions, configuration, and CLI | IN PROGRESS |
| M04 | Security, lifecycle, concurrency, and persistence hardening | NOT STARTED |
| M05 | Skills, MCP, ACP, and subagent extensibility | IN PROGRESS |
| M06 | SDK surfaces and advanced compatibility | NOT STARTED |
| M07 | Optimization, packaging evidence, and final hardening | NOT STARTED |

## Delivered-slice inventory

This table is an index, not a second live ledger. Contract documents define
durable behavior and review ledgers retain accepted/rejected findings and exact
historical evidence. A dash means the compact plan does not assert an early
delivery identifier; the linked review ledger remains authoritative history.

| Slice | Bounded deliverable | Durable contract | Historical review | Delivery identifier |
| ---: | --- | --- | --- | --- |
| 1 | Native config/state discovery and help/version/status | [CLI](cli.md), [configuration](configuration.md) | [config/status CLI](reviews/m03-config-status-cli-review-01.md) | — |
| 2 | Strict schema-v1 native config load | [configuration](configuration.md) | [native config load](reviews/m03-native-config-load-review-01.md) | — |
| 3 | Capability-aware tool preflight | [core API](core-api.md) | [tool preflight](reviews/m03-tool-preflight-review-01.md) | — |
| 4 | `read_file` | [contract](read-file.md) | [review](reviews/m03-read-file-review-01.md) | — |
| 5 | `list_files` | [contract](list-files.md) | [review](reviews/m03-list-files-review-01.md) | — |
| 6 | AI Gateway provider codec | [contract](ai-gateway.md) | [review](reviews/m03-ai-gateway-review-01.md) | — |
| 7 | Native AI Gateway HTTP transport | [contract](ai-gateway-http.md) | [review](reviews/m03-ai-gateway-http-review-01.md) | `508b0ad` |
| 8 | Native file session store | [contract](session-store.md) | [review](reviews/m03-session-store-review-01.md) | `8f7b47d` |
| 9 | `AskPermissionHandler` | [contract](ask-permission.md) | [review](reviews/m03-ask-permission-review-01.md) | `27e3f2b` |
| 10 | AI Gateway credential discovery | [contract](ai-gateway-credentials.md) | [review](reviews/m03-ai-gateway-credential-review-01.md) | `ef6901d` |
| 11 | Native configuration schema v2 | [contract](configuration.md) | [review](reviews/m03-native-host-config-review-01.md) | `a10f24e` |
| 12 | Native reference-host composition | [contract](native-reference-host.md) | [review](reviews/m03-native-reference-host-review-01.md) | `ac3984f` |
| 13 | Configured credential source / schema v3 | [contract](configuration.md) | [review](reviews/m03-configured-credential-source-review-01.md) | `f840576` |
| 14 | Native root selection and preparation | [contract](native-root-selection.md) | [review](reviews/m03-native-root-selection-review-01.md) | `6f66b6e` |
| 15 | Native session lifecycle | [contract](native-session-lifecycle.md) | [review](reviews/m03-native-session-lifecycle-review-01.md) | `dbba2c7` |
| 16 | Native session listing | [contract](native-session-listing.md) | [review](reviews/m03-native-session-listing-review-01.md) | `d3312d7` |
| 17 | `file_info` | [contract](file-info.md) | [review](reviews/m03-file-info-review-01.md) | `60dd54f` |
| 18 | `glob_files` | [contract](glob-files.md) | [review](reviews/m03-glob-files-review-01.md) | `f6ab594` |
| 19 | `grep_files` | [contract](grep-files.md) | [review](reviews/m03-grep-files-review-01.md) | `0f48806` |
| 20 | `write_file` | [contract](write-file.md) | [review](reviews/m03-write-file-review-01.md) | `bdd27ec` |
| 21 | `edit_file` | [contract](edit-file.md) | [review](reviews/m03-edit-file-review-01.md) | `719a9bd` |
| 22 | `delete_file` | [contract](delete-file.md) | [review](reviews/m03-delete-file-review-01.md) | `fe56f4c` |
| 23 | `rename_file` | [contract](rename-file.md) | [review](reviews/m03-rename-file-review-01.md) | `7cb5ef9` |
| 24 | `copy_file` | [contract](copy-file.md) | [review](reviews/m03-copy-file-review-01.md) | `3bdd7cb` |
| 25 | `create_folder` | [contract](create-folder.md) | [review](reviews/m03-create-folder-review-01.md) | `e75578b` |
| 26 | `open_file` | [contract](open-file.md) | [review](reviews/m03-open-file-review-01.md) | `a02c28a` |
| 27 | `web_fetch` | [contract](web-fetch.md) | [review](reviews/m03-web-fetch-review-01.md) | `aac9e5f` |
| 28 | Top-level `permissions` | [contract](permissions-cli.md) | [review](reviews/m03-permissions-cli-review-01.md) | `3e41cc6` |
| 29 | Top-level `models` | [contract](models-cli.md) | [review](reviews/m03-models-cli-review-01.md) | `bacc5c3` |
| 30 | Top-level `doctor` | [contract](doctor-cli.md) | [review](reviews/m03-doctor-cli-review-01.md) | `345f812` |
| 31 | Top-level `sessions` | [contract](sessions-cli.md) | [review](reviews/m03-sessions-cli-review-01.md) | `b5b9116` |
| 32 | Top-level `session <id>` summary | [CLI contract](session-cli.md), [native inspection](native-session-inspection.md) | [review](reviews/m03-session-cli-review-01.md) | `b6db9a6` |
| 33 | `web_search` | [contract](web-search.md) | [review](reviews/m03-web-search-review-01.md) | `52b5885` |
| 34 | Bounded foreground `terminal` exec | [contract](terminal.md) | [review](reviews/m03-terminal-review-01.md) | `ddd6a89` |
| 35 | Ordinary `ask_user_question` | [contract](ask-user-question.md) | [review](reviews/m03-ask-user-question-review-01.md) | `490d122` |
| 36 | Range-only `read_tool_result` with conditional Gateway projection | [contract](read-tool-result.md) | [review](reviews/m03-read-tool-result-review-01.md) | `7371260` |
| 37 | Native `vision` with bounded Gateway evidence | [contract](vision.md) | [review](reviews/m03-vision-review-01.md) | `0a32e2f` |
| 38 | Bounded top-level `ask` CLI | [contract](ask-cli.md) | [review](reviews/m03-ask-cli-review-01.md) | `8e7d317` |
| 39 | Bounded lexical `semantic_search` | [contract](semantic-search.md) | [review](reviews/m05-semantic-search-review-01.md) | `6a63127` |
| 40 | Bounded explicit-ID top-level `resume` CLI | [contract](resume-cli.md) | [review](reviews/m03-resume-cli-review-01.md) | `136d44e` |
| 41 | Bounded native `memory` | [contract](memory.md) | [review](reviews/m05-memory-review-01.md) | `33bdd76` |
| 42 | Bounded workspace-local `skill` | [contract](skill.md) | [review](reviews/m05-skill-review-01.md) | `ef5ab40` |
| 43 | Bounded local-only `install_skill` | [contract](install-skill.md) | [review](reviews/m05-install-skill-review-01.md) | `25b62a0` |
| 44 | Bounded read-only top-level `workspace` CLI | [contract](workspace-cli.md) | [review](reviews/m03-workspace-cli-review-01.md) | `f36a834` |
| 45 | Pinned-fx-compatible offline FXTP `replay` CLI | [contract](replay-cli.md) | [review](reviews/m03-replay-cli-review-01.md) | `e685dc4` |
| 46 | Bounded top-level help and runtime status ownership | [CLI](cli.md), [configuration](configuration.md), [performance](performance.md) | [review](reviews/m03-help-status-cli-review-01.md) | `9019770` |
| 47 | Bounded injected-catalog `mcp_search_tools` | [contract](mcp-search-tools.md) | [review](reviews/m05-mcp-search-tools-review-01.md) | `a8a94f9` |
| 48 | Turn-local executable `mcp_select_tool` | [contract](mcp-select-tool.md) | [review](reviews/m05-mcp-select-tool-review-01.md) | `c5d86c9` |
| 49 | Bounded injected-catalog `mcp_features` | [contract](mcp-features.md) | [review](reviews/m05-mcp-features-review-01.md) | `3ba687b` |
| 50 | Bounded foreground one-off `subagent` | [contract](subagent.md) | [review](reviews/m05-subagent-review-01.md) | `ba52dbf` |
| 51 | Bounded read-only persisted `background` CLI | [contract](background-cli.md) | [review](reviews/m05-background-cli-review-01.md) | `a665289` |
| 52 | Bounded production background supervisor and process lifecycle | [contract](background-supervisor.md) | [review](reviews/m05-background-supervisor-review-01.md) | `1d8ef7b` |
| 53 | Bounded noninteractive `terminal` background start | [terminal](terminal.md), [supervisor](background-supervisor.md) | [review](reviews/m03-terminal-start-review-01.md) | `ceed855` |
| 54 | Bounded exact persisted-record `terminal` inspect | [terminal](terminal.md), [background CLI](background-cli.md) | [review](reviews/m03-terminal-inspect-review-01.md) | `bd4e97d` |
| 55 | Bounded exact persisted-record `terminal` wait | [terminal](terminal.md), [background CLI](background-cli.md) | [review](reviews/m03-terminal-wait-review-01.md) | `f16f099` |
| 56 | Bounded persisted-record `terminal` list | [terminal](terminal.md), [background CLI](background-cli.md) | [review](reviews/m03-terminal-list-review-01.md) | `a8f5af3` |

The exact delivered-main record after slice 56 is in the canonical live-status
block. Historical review ledgers may name intermediate candidates, trees,
finding counts, component commits, and older workflow runs; those records are
not current status.

## Milestone 03 completion boundary

M03 is not complete. Its ownership boundary is frozen as follows; changing it
requires an explicit reviewed plan change.

### Complete

- Provider-neutral engine integration with the native provider, transport,
  permission, session, configuration, and reference-host seams represented by
  delivered slices 1-16.
- Twenty-one native tools: `list_files`, `glob_files`, `grep_files`, `read_file`,
  `write_file`, `edit_file`, `delete_file`, `rename_file`, `copy_file`,
  `create_folder`, `file_info`, `open_file`, `web_fetch`, `web_search`,
  `terminal`, `ask_user_question`, `read_tool_result`, `vision`, `memory`,
  `skill`, and `install_skill`.
- Delivered CLI slices for `help`, `status`, `ask`, `resume`, `permissions`,
  `models`, `doctor`, `sessions`, `workspace`, `replay`, and strict
  summary-only `session <id>`.

### Remaining

- Complete combined top-level CLI ownership for `permissions`, `models`,
  `doctor`, `session`, `sessions`, and `resume`.
  Existing partial/delivered commands do not close the combined boundary.
- Complete the pinned slash-command categories `general`, `session`, `model`,
  `security`, and `workspace`. Compatibility is scenario-based; documented
  command-name differences may remain intentional.
- Retain deterministic composed-host evidence through fake provider, prompt,
  and network boundaries; exercise user-visible behavior through a freshly
  built release binary; close three fresh product-review tracks; pass every
  exact local and remote gate; and update compatibility status without making
  an unsupported performance claim.

### Later-milestone ownership

| Owner | Explicitly assigned work |
| --- | --- |
| M04 | Permission modes and identity-safe grants beyond `ask`; session migration and explicit legacy import; encryption, record authentication, key management, secure erasure, persistence/lifecycle concurrency hardening, and hardened non-Unix workspace/store construction |
| M05 | Skills, MCP, ACP, subagents, top-level `acp`/`background`/`teams`, extension/agent slash commands, and built-in memory/search/skill/subagent/MCP tools |
| M06 | SDKs and advanced CLI/compatibility surfaces including `pr`, `issue`, account, setup, credit, usage, upgrade, media, product, and appearance categories |
| M07 | Claim-eligible performance comparison, thresholds, optimization, packaging evidence, and final hardening |

## Required gates

### Local feature gate

Use Rust and Cargo 1.94.1 exactly. Repair an unavailable or damaged pinned
toolchain with `rustup`; no floating-channel substitution satisfies the gate.

```sh
cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 test --workspace
cargo +1.94.1 test --doc --workspace
```

Run affected tests first. The complete gate also includes repository Python
tests, pinned-fx drift checks, dependency policy and vulnerability audit,
supported Linux/macOS execution, relevant FreeBSD/WASI compilation or active
unsupported behavior, documentation policy, diff/no-added-unsafe checks, and a
fresh locked release-binary smoke of user-visible behavior. Evidence is a
regression/delivery claim unless a milestone explicitly promotes it.

### Review and remote gate

- Three fresh reviewers inspect the same exact behavior SHA after the complete
  local gate. All findings, including documentation findings on a behavior
  candidate, must be fixed and all three tracks restarted.
- Feature CI and Benchmark evidence must report success for the exact pushed
  behavior SHA. Then fast-forward `main` without force and require exact-main
  success.
- Behavior and evidence-affecting Benchmark workflows must retain the expected
  unexpired exact-SHA artifacts. Documentation-only descendants require green
  lightweight aggregate gates and deliberately produce no new artifacts.
- Never publish packages or GitHub releases without separate authorization.

### M07 release thresholds

- Formatting, warnings-denied Clippy, workspace and doc tests, repository
  Python tests, dependency policy, vulnerability audit, and deterministic
  end-to-end tests pass on Linux/macOS x86_64 and aarch64.
- Three equivalent local workloads beat pinned fx by at least 20%; no other
  equivalent workload regresses more than 5%.
- Linux local command startup is at most 2 ms and the stripped Linux x86_64
  binary is at most 7.8 MiB.
- Safety, permission, correctness, and resource bounds cannot be weakened to
  meet performance targets.

## Documentation ownership and compaction

- This plan is the only live source for current phase, delivered-slice count,
  delivered-main SHA, current workflow IDs, and next gate.
- Behavior documents state durable contracts, limits, platform scope, and
  intentional deferrals. Change them only when durable behavior changes; do not
  use them as live delivery dashboards.
- `docs/reviews/` ledgers are historical evidence. Preserve exact candidate,
  finding, remediation, and review provenance there, but do not treat their
  opening summaries as current project status.
- `README.md`, `docs/README.md`, `docs/reviews/README.md`, architecture,
  security, performance, native-reference-host, and tool contract overviews
  stay evergreen. They must not duplicate live phase, delivered count, or
  Actions run IDs.
- `scripts/check_documentation.py` enforces canonical markers and fields, the
  600-line plan ceiling, evergreen overview restrictions, obvious relative
  Markdown link targets, and balanced fences. It is intentionally a small
  repository-policy check, not a CommonMark implementation, and prints
  inventory counts for the current run without persisting them.
- Documentation-checker parser changes are separate, non-product maintenance.
  Product feature iterations use the existing checker and do not expand its
  Markdown grammar in response to product-review edge cases; a future richer
  validator should use an established parser behind explicit repository bounds.
- Compact this plan after every five delivered slices or whenever it exceeds
  600 lines. Keep the live block, milestone state, compact slice inventory,
  remaining boundary, and gates; move cycle detail to review ledgers.
- Docs-only maintenance and compaction do not increment the slice count and are
  exempt from a new adversarial product review when they change no behavior.

## Authorization and stop conditions

The coordinator may commit and push branches and `main` to
`distributedstatemachine/machine-god`. Never force-push `main`. Do not publish
packages or GitHub releases without separate authorization. Continue fixing
ordinary implementation, review, benchmark, and CI failures until green. Stop
only for missing external authority, unavailable required credentials/runners,
irreproducible upstream behavior, or a conflict between a performance goal and
a safety invariant.
