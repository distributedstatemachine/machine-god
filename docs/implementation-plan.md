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

Backward compatibility is not a product requirement. Target the selected modern
protocols and native formats; do not add legacy negotiation, deprecated
transports, compatibility shims or foreign-session import. The upstream pin
remains a behavioral reference and benchmark baseline, not an obligation to
reproduce its historical compatibility layers. Existing delivery records remain
historical evidence, not requirements to preserve legacy code. Remove obsolete
paths with their callers and tests in the owning feature; do not remove modern
functionality merely because it shares an implementation with an older mode.

## Current delivery state

<!-- canonical-live-status:start -->
- Delivered slices: `64`
- Delivered main: `1ca9eae2c74745d812242876dea1720815873476`
- Main CI: `34954002810` (`GREEN`)
- Main Benchmark evidence: `34954002729` (`GREEN`)
- Active branch: `agent/m65-managed-subagents`
- Active phase: `M05 complete managed-agent tool and CLI implementation`
- Next gate: `verify open-form retirement after external archive with fresh binaries, retaining the now-passing receipt and pre-turn failure regressions; run complete feature gates before three fresh reviews`
<!-- canonical-live-status:end -->

The complete terminal, combined CLI, background, skills, MCP and ACP are delivered.
The canonical behavior passed the exact local gate, three fresh reviews and
feature/main CI and Benchmark gates on supported Linux/macOS architectures.
Both unexpired exact-main artifacts are retained; main advanced by fast-forward.
This is regression acceptance, not an M07 performance claim. Documentation-only
seals neither increment the count nor replace canonical behavior evidence.

Detailed candidate, failure, remediation and review history is retained in the
[terminal review](reviews/m03-terminal-full-review-01.md),
[combined CLI review](reviews/m03-cli-full-review-01.md),
[background CLI review](reviews/m05-background-cli-review-01.md),
[skills CLI review](reviews/m05-skills-cli-review-01.md),
[MCP CLI review](reviews/m05-mcp-cli-review-01.md), and
[ACP review](reviews/m05-acp-cli-review-01.md).
This compact plan does not repeat that history.

### Delivered terminal acceptance boundary

The terminal is one complete feature; its internal input/write and cleanup
commits are not separate deliveries. The native executor, explicit
helper-bearing reference host and CLI compose:

- All twelve actions: `exec`, `start`, `read`, `screen`, `write`, `wait`,
  `monitor`, `inspect`, `list`, `resize`, `signal`, and `close`.
- Interactive PTY/tmux startup, commandless and command-bearing sessions,
  user/clean bash/zsh profiles, actual dimensions and deadline-bound ownership.
- Durable segmented raw output, styled Unicode screens and checkpoints, cursor
  gaps, retained events, restart/recovery and owner-bound catalog/history.
- Ordered text/binary/paste input, named/control keys, writer leases,
  committed-byte receipts, native resize and authenticated process signaling.
- Started/exit/quiet/match waits and all thirteen monitor conditions, with
  separately authorized bounded network/filesystem/custom probes.
- Profile-wide storage accounting and retention across resident/nonresident
  histories, pressure-only safe residency reuse, and lossless argument/result
  archives shared with `read_tool_result` and the Gateway codec.
- Graceful/force close, retained readable history, cancellation and host-owned
  worker completion, including CLI signal and blocked-output finalization.
- Shared owned macOS inventory prepared during startup, with each query still
  bounded by the original 250 ms deadline and independent kill/reap ownership.
  Tmux command/server cleanup retains exact child and artifact ownership under
  one bounded cleanup window, including deferred reap and interrupted observation.

Durable behavior belongs in [terminal](terminal.md),
[reference-host composition](native-reference-host.md), [core API](core-api.md),
[Gateway codec](ai-gateway.md), and [archive paging](read-tool-result.md).
The pinned schema, session and monitor references are maintained there.
ACP, teams and extension slash commands retain their later milestone ownership.

### Combined CLI acceptance boundary

The delivered combined CLI completes native conversation and interactive
ownership, bounded input/output queues, model selection, persistent permission
rules and approval enforcement, file history and undo, macOS sandboxing,
validated session catalogs/resume, prompts/editing/resize, piped input, picker
transitions, recording/replay, session maintenance and workspace management.

Schema-v7 workspace persistence and retained multi-root authority compose
exact-turn contexts with reads, enumeration, grep, metadata, mutations,
permissions, history, undo, vision, sandboxing and Linux/macOS semantic search.
Settings-free workspace launch/listing remains supported; persistence mutations
require settings authority and invalid selected settings remain errors.
Historical tool cards project validated saved evidence without executing tools
or inventing current state.

The frozen M03 boundary is closed without an M07 performance claim. Detailed
candidates, regressions, gates and review provenance remain in the
[combined CLI history](reviews/m03-cli-full-review-01.md).

All twelve agent-readiness maintenance findings were integrated into this
feature's parent before final CLI verification. Their historical acceptance and
separately scoped nextest/build-reuse advice belong in the
[agent-readiness review](reviews/agent-readiness-rust-structure-review.md).
Test-infrastructure maintenance is not part of subsequent product-tool work.

Required permission modes/grants, native migration/recovery and guarded cleanup
are included in the combined CLI boundary. Encryption,
record authentication, key management, secure erasure, broader persistence and
lifecycle concurrency hardening, and hardened non-Unix construction retain
their later milestone ownership below.

### CI and documentation maintenance

Delivered CI maintenance uses fail-closed dependency-aware concern selection
and stable aggregate gates. Documentation-only descendants run bounded
documentation checks and lightweight CI/Benchmark aggregates, without Rust,
platform, audit, compatibility or benchmark-artifact jobs. They do not increment
the delivered-slice count. Product/evidence changes retain artifact-producing
gates. Terminal-sys routes through its package and native/CLI consumers,
including Apple binding lint and pinned-upstream Unicode generation checks.
The classification contract remains in
[CI change classification](ci-change-classification.md).

## Architecture ownership

- `machine-god-core` owns provider-neutral contracts and orchestration. It has
  no ambient filesystem, process, environment, terminal, or network authority.
- `machine-god-native` owns explicitly injected operating-system, network,
  terminal, configuration, and persistence effects.
- `machine-god-cli` is a thin host and owns no product state.
- `machine-god-testkit` owns deterministic test doubles and fixtures.
- Unsafe Rust is forbidden in the product crates. The isolated macOS
  PTY, exact process-incarnation, fixed uptime-clock, read-only inventory and metered directory-refill
  bindings are the sole exception under
  [ADR 0003](decisions/0003-macos-terminal-foreground-signal.md) and
  [ADR 0004](decisions/0004-macos-process-inventory-helper.md), with directory
  refills under [ADR 0005](decisions/0005-macos-directory-reader.md). New bindings
  remain subject to the complete feature's adversarial and platform gates.
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
| M03 | Providers, native tools, permissions, sessions, configuration, and CLI | COMPLETE |
| M04 | Security, lifecycle, concurrency, and persistence hardening | IN PROGRESS |
| M05 | Skills, MCP, ACP, and subagent extensibility | IN PROGRESS |
| M06 | SDKs and advanced product surfaces | NOT STARTED |
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
| 57 | Bounded same-incarnation `terminal` background output read | [terminal](terminal.md), [supervisor](background-supervisor.md) | [review](reviews/m03-terminal-read-review-01.md) | `b1bbb81` |
| 58 | Bounded same-incarnation `terminal` native signal | [terminal](terminal.md), [supervisor](background-supervisor.md) | [review](reviews/m03-terminal-signal-review-01.md) | `8545ea4` |
| 59 | Complete terminal actions, PTY/tmux sessions, durable history/screens, monitors, archives and owned cleanup | [terminal](terminal.md), [host](native-reference-host.md) | [review](reviews/m03-terminal-full-review-01.md) | `229cf94` |
| 60 | Complete combined CLI, interactive conversation, permissions, workspace authority, session lifecycle, history/undo and recording | [CLI](cli.md), [host](native-reference-host.md) | [review](reviews/m03-cli-full-review-01.md) | `1a142174` |
| 61 | Complete interactive background list/stop/open/logs, owned terminal control and unified read-only histories | [background CLI](background-cli.md), [host](native-reference-host.md) | [review](reviews/m05-background-cli-review-01.md) | `fe7a793c` |
| 62 | Complete skills commands, local/Git management, inline picking and prompt-bound invocation/context | [skills CLI](skills-cli.md) | [review](reviews/m05-skills-cli-review-01.md) | `431030b9` |
| 63 | Complete production MCP, modern stdio/HTTP, tool and human continuations, OAuth, catalog activation and CLI composition | [MCP CLI](mcp-cli.md) | [review](reviews/m05-mcp-cli-review-01.md) | `8dfe9255` |
| 64 | Complete modern ACP, native sessions, human continuations, ephemeral MCP, bounded wire/stdio and owned settlement | [wire](acp-protocol.md), [sessions](acp-sessions.md) | [review](reviews/m05-acp-cli-review-01.md) | `1ca9eae2` |

The exact delivered-main record is in the canonical live-status
block. Historical review ledgers may name intermediate candidates, trees,
finding counts, component commits, and older workflow runs; those records are
not current status.

## Milestone 03 completion boundary

M03 is complete at its frozen ownership boundary below. This does not close
the explicitly assigned M04–M07 work or assert literal upstream UI parity.

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
- Combined top-level ownership for `permissions`, `models`, `doctor`,
  `session`, `sessions`, and `resume`, beyond their earlier partial slices.
- Pinned slash-command categories `general`, `session`, `model`, `security`,
  and `workspace`, with scenario-based compatibility and documented intentional
  command-name differences.
- Deterministic composed-host evidence through fake provider, prompt and
  network boundaries, fresh release-binary scenarios, three independent
  product-review tracks and exact local/feature/main gates. The existing
  compatibility inventory records this native category ownership while its
  larger aggregates remain planned for M05/M06; no performance claim follows.

### Later-milestone ownership

| Owner | Explicitly assigned work |
| --- | --- |
| M04 | Required modes/grants, native migration/recovery and guarded cleanup were delivered with the combined CLI; remaining work includes encryption, record authentication, key management, secure erasure, broader persistence/lifecycle concurrency hardening, and hardened non-Unix workspace/store construction |
| M05 | Skills, MCP, ACP, subagents, top-level `acp`/`background`/`teams`, extension/agent slash commands, and built-in memory/search/skill/subagent/MCP tools |
| M06 | SDKs and advanced CLI surfaces including `pr`, `issue`, account, setup, credit, usage, upgrade, media, product, and appearance categories |
| M07 | Claim-eligible performance comparison, thresholds, optimization, packaging evidence, and final hardening |

## Delivered background CLI boundary

`/background`, `/background stop`, `/background open` and `/background logs`
compose native identities, generation-bound controls, retained output and owned
cancellation. Read-only listing unifies numeric supervisor records and opaque
terminal histories without restoring process authority. Bounded URL detection
and explicit launching preserve truncation evidence. The
[contract](background-cli.md) owns behavior; the
[review history](reviews/m05-background-cli-review-01.md) retains acceptance and
earlier intermittent failures without an unsupported cause or source-fix claim.

## Delivered skills CLI boundary

The complete [skills CLI](skills-cli.md) includes list/show/path/create/add/
install/remove, filtered local/Git installs, replacement consent and owned
publication/rollback receipts. Explicit bounded discovery, revision-bound `$`
picking, FIFO prompt invocation and provider-only context preserve canonical
user text, permission provenance, cancellation and handoff. Metadata grants no
authority; pasted installer forms are parsed, not executed; fx roots stay read-only.
The [review history](reviews/m05-skills-cli-review-01.md) retains accepted R5
evidence and the initial intermittent macOS reaping timeout. An unchanged passing
retry is not a cause or source fix. Evidence and caches remain; worktrees do not.
Import components are retained on `agent/m61-fx-session-import` at `5e77b1b4`
for history only; foreign-session import is no longer required. Do not merge it.

## Delivered production MCP and CLI boundary

Production MCP is one complete feature: `/mcp` configuration, reload, resource
and prompt actions, OAuth/browser consent and distinct logout receipts, plus
model search/select/call and modern form/URL continuations. The
[CLI contract](mcp-cli.md) and linked MCP contracts own the detailed boundaries.
Modern stdio and Streamable HTTP use explicit profile/environment/header/auth
authority, bounded framing and atomic paginated executable generations. Exact
JSON, descriptors, schemas and contextual one-shot permission proofs survive
through final submission; raw catalog pages and session IDs grant no authority.
Failed reload preserves the usable runtime, and retired generations retain
bounded cleanup custody. Saving configuration does not activate it; refresh
preserves expiry and never replays ambiguous consequential requests. Continuations
retain original session/request/round, consent and cancellation through settlement.
Deprecated negotiation and HTTP+SSE transport are absent; modern HTTP response
SSE remains. Sampling and roots are not added beyond the pinned feature boundary.

The [review history](reviews/m05-mcp-cli-review-01.md) retains full local, fresh R4
and exact feature/main acceptance, including four platform architectures,
Ask/Resume/interactive composition, production transports, OAuth and human
continuations, archives, blocked output, EOF and PTY transitions. Both exact-main
artifacts were verified. These delivered features are regression acceptance, not
M07 claims; their completed worktrees are removed. Runtime/resource state remains
native and core effect-free. Pinned `teams` means account/team selection, not agents.

## Delivered modern ACP boundary

The complete native implementation passed the full Linux/macOS local gate,
three fresh whole-feature reviews and exact feature/main CI and Benchmark gates.
Main advanced by fast-forward, and both unexpired exact-main artifacts were
verified. This is regression acceptance, not an M07 performance claim.

[Wire](acp-protocol.md), [sessions](acp-sessions.md),
[client interaction](acp-interactions.md) and [ephemeral MCP](acp-mcp.md) own
the complete feature contract; CLI remains a thin stdio host:

- Version-1 newline JSON-RPC, 8 MiB frames, 32 pending outbound requests, one
  active prompt, exact correlation, backpressure, malformed input and EOF cleanup.
- Initialize/capabilities, new/load/resume/close/list, prompt/cancel and
  set_config_option; native origins, scoped listing, model/permission selection,
  owned transitions and current persistence. Load projects saved history without
  execution; resume never replays it. Text/resource prompts and command/tool
  updates settle history/checkpoints before the prompt response.
- Exact permission and modern form/URL continuation custody, an injected client
  URL endpoint and elicitation/complete through success, failure, cancellation
  and stale/drop invalidation. Session grants are not persistent rules.
- Authoritative ephemeral mcpServers: absent/empty means none. Modern stdio/HTTP,
  literal environment and resolved Authorization, no profile/credential fallback
  or injected-secret persistence; readiness, atomic replacement, failed-selection
  preservation and owned retirement.
- Native filesystem/terminal execution remains; client capability flags grant no
  editor delegation. No deprecated SSE transport, negotiation, foreign import or
  WASM-only removal; modern HTTP response SSE remains.

Composed wire/CLI, human round trips, history, cancellation, blocked output,
stale replacement and cross-session isolation tests cover this boundary.
Failure, remediation, local/remote evidence and independent review provenance
remain in the [review history](reviews/m05-acp-cli-review-01.md).

## Next complete feature: managed subagents

After ACP delivery and its documentation seal, use
`agent/m65-managed-subagents` for one complete managed-agent tool and CLI feature.
Replace the foreground one-off implementation, not retain a parallel legacy path.
The [subagent contract](subagent.md) tracks managed commands and native ownership
boundaries; update it with implementation, using the pinned source as evidence.

- Complete create, inspect, message, milestone, attach/detach, reparent,
  configure, cancel, resume, close and reopen, for one-off and persistent agents.
  Include names, standalone prompts, model/effort selection, notifications,
  relationships and bounded status/message/tool/event/history paging and waits.
- Persist create/message acceptance before execution and cancellation before
  signalling. Run one FIFO item per child; interrupted or approval-blocked heads
  require explicit resolution. Restart pending/running/approval work as interrupted
  without automatic model/tool execution. Host teardown is not durable user cancel.
  Persistent cancellation returns idle; one-off cancellation is terminal. Close
  archives and settles rather than deletes; reopen does not implicitly retry.
- Children outlive the creating turn under an outer native manager. Share host
  services/engine per explicitly admitted authority domain, not a host per child.
  Keep distinct conversation runtimes, sessions, workspace selections, policies,
  grants, turn routes and ephemeral MCP custody. Weak requesters/UI observations
  must not retain runtimes or form host-resource ownership cycles.
- Bind model admission to an opaque actual-turn/call execution witness, not
  publicly constructible ToolContext IDs. Core supplies identity/orchestration
  proof only; native privately admits principal/run authority. Repeated provider
  call IDs, stale generations and direct structural tool calls cannot forge it.
- Use explicit aggregate execution/residency/queue/byte/waiter budgets and a fair
  shared scheduler, with no idle thread or timer per child. Page settled durable
  history rather than imposing a lifetime creation limit. A completion ring is
  not a child-count cap. Dependency waits release execution quota while retaining
  bounded waiters; reject cycles and unschedulable waits. Ordinary provider
  pending states still count as active execution.
- Preserve standalone prompt and same-or-stricter permission policy; never inherit
  parent transcripts or grants. Explicit model/effort wins, otherwise use selected
  preferences. Freeze each accepted work item's notification policy: completion,
  failure and cancellation on by default; start/interval optional. Coalesce late
  interval observations without inference. Parent notices enter next-turn context,
  not active-turn injection or an automatic idle turn; closing stops notices.
- Preserve pinned observable bounds, including 32 milestones and eight stop
  conditions, checked positive durations and gap-aware bounded communications,
  consumers, approvals and policy snapshots. Separate history retention from live
  execution admission, and reserve aggregate capacity before accepting work.
- Complete Ctrl-X “Agents & processes” navigation through native projections and
  exact receipts, with a thin CLI renderer. Keep input owner/chunk/flush ACK,
  presentation route/editor/page epoch, and native authority separate. Preserve
  partial UTF-8/paste provenance, reject stale frames, retain the parent runtime,
  and progress hidden children/cancellation even under blocked stdout. Use one
  bounded multi-owner approval/elicitation inbox with targeted retirement.
- Isolate undo and mutable workspace/MCP state per principal under shared budgets.
  Reserve retained and in-flight undo bytes/entries/descriptors before capture;
  bind reservations to the original principal/generation through commit or
  ambiguity. Clear/evict/undo only that owner's history. Keep registry locks short
  and never hold a global lock across filesystem effects or asynchronous waits.
- Parent new/resume/reset retires only its own routes and controls. Manager
  navigation grants no process authority. Shutdown drops owning runtime leases
  and settles children, controls, services and actual worker/TLS/reap obligations
  before terminal/input teardown; reuse async completion without observer admission.

Freeze interfaces before parallel worktrees for manager/tool persistence,
principal isolation/shared service integration, and native UI/thin CLI lanes.
Assign shared files to one owner and integrate complete end-to-end behavior.
Test restart/approval/FIFO and dependency races, aggregate pressure, owner-local
undo, stale/blocked UI, cancellation and actual cleanup on Linux/macOS with fresh
release CLI scenarios. Run the complete gate and three fresh reviews for this
whole feature, then exact feature/main artifact-producing gates and worktree cleanup.

### Remaining M05 integration after managed agents

Complete model-facing skill discovery/location and filtered local/Git installation
through the delivered native catalog/installer. Human /skills acceptance does not
widen the still workspace-local skill and local-only install_skill tools; replace
those narrower paths with model-bound permission, cancellation and receipt tests.
Pinned teams requires account OAuth acquisition/refresh and source selection;
assign its minimal account prerequisites explicitly across M05/M06 before coding.
It is not agent management. The pinned /alias is only an unavailable notice,
not evidence for a configurable alias engine or another agent slash-command family.

## Required gates

### Local feature gate

Use Rust and Cargo 1.94.1 exactly. Repair an unavailable or damaged pinned
toolchain with `rustup`; no floating-channel substitution satisfies the gate.

Run affected tests first, with the same prerequisites. From the repository root,
use this canonical full-gate recipe in one shell. Both Linux and macOS runtime
tests require `/bin/bash`, `/bin/zsh`, tmux and Git supporting `--no-lazy-fetch`.
Containerized runs need an unprivileged test UID without DAC-bypass capabilities,
a passwd entry, private home and valid login shell before their first test.
They also need a reaping init (for example Docker `--init`) so adopted child
processes are reaped; a bare `sleep` as PID 1 does not satisfy the cleanup gate.
Start with clean fixtures; running permission tests as root or reusing root-owned
fixture directories does not establish the supported Linux gate.
Keep their normal profile behavior. Concurrent builds can contend with
process-lifecycle fixtures, so finish worker builds before the full runtime gate.
When Linux and macOS share one physical host, also separate their process-heavy
runtime runs. This does not change Linux's internal default test concurrency or
prevent independent CI runners from running in parallel.

```sh
set -eu
test -x /bin/bash
test -x /bin/zsh
MACHINE_GOD_TERMINAL_TMUX_BINARY="$(command -v tmux)"
export MACHINE_GOD_TERMINAL_TMUX_BINARY
"$MACHINE_GOD_TERMINAL_TMUX_BINARY" -V

cargo +1.94.1 fmt --all -- --check
cargo +1.94.1 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.94.1 build --release --locked -p machine-god-cli --bin machine-god --target-dir target
MACHINE_GOD_CLI_TEST_BINARY="$(pwd -P)/target/release/machine-god"
export MACHINE_GOD_CLI_TEST_BINARY
test -x "$MACHINE_GOD_CLI_TEST_BINARY"
MACHINE_GOD_TERMINAL_RELEASE_BINARY="$MACHINE_GOD_CLI_TEST_BINARY"
export MACHINE_GOD_TERMINAL_RELEASE_BINARY
if [ "$(uname -s)" = Darwin ]; then
  cargo +1.94.1 test --workspace -- --test-threads=1
else
  cargo +1.94.1 test --workspace
fi
cargo +1.94.1 test --doc --workspace
```

The complete gate also includes repository Python
tests, pinned-fx drift checks, dependency policy and vulnerability audit,
supported Linux/macOS execution (including Linux's default CI test concurrency),
relevant FreeBSD/WASI compilation or active
unsupported behavior, documentation policy, no-unsafe conformance checks, and a
fresh locked release-binary smoke of user-visible behavior. Evidence is a
regression/delivery claim unless a milestone explicitly promotes it.
Both platforms select the explicit production helper. The Apple branch matches
the native matrix's serial process-table scheduling; Linux retains default test
concurrency.
Keep explicitly constructed protocol and failure fixtures; do not relax
deadlines or skip tests. A changed source tree requires a fresh release helper.
The CLI integration harness accepts `MACHINE_GOD_CLI_TEST_BINARY` as an explicit
binary path for release-binary checks; absent that override it uses Cargo's
freshly built test binary. Session integration fixtures exercise public commands
through this boundary, not a substituted library-only smoke.

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
