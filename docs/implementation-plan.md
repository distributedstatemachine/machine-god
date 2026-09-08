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
- Delivered slices: `59`
- Delivered main: `229cf94a798fa561f3ff7401c07a549b79e50616`
- Main CI: `34163483918` (`GREEN`)
- Main Benchmark evidence: `34163483934` (`GREEN`)
- Active branch: `agent/m60-cli-shell`
- Active phase: `implementing the full combined M03 top-level CLI and slash-command feature`
- Next gate: `finish resume/session lifecycle, interactive commands and remaining policy/workspace scenarios before the full feature gates`
<!-- canonical-live-status:end -->

The complete terminal is delivered as one feature. The exact behavior commit
passes the full Rust 1.94.1 local gate, three fresh independent reviews with zero
actionable findings, and both feature and main CI/Benchmark gates. Native Linux
and macOS pass on x86_64 and aarch64, including the previously failing Intel
inventory path. Both required unexpired exact-main benchmark artifacts are
retained. Main was advanced by fast-forward without force. This establishes
regression acceptance, not an M07 performance claim; documentation-only seals
do not increment the count or replace the canonical behavior evidence.

Detailed candidate, failure, remediation and review history is retained in the
[full terminal review ledger](reviews/m03-terminal-full-review-01.md).
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

### Active product work

Terminal delivery and its exact lightweight documentation-seal gates are complete.
Complete the combined M03 CLI/slash-command boundary below as one feature.
The initial parallel inspection maps all six top-level command families, every
command in the five pinned slash categories, and reusable Rust ownership seams.
Existing partial forms do not define the target; resolve any cross-milestone
dependency explicitly before freezing shared contracts. Keep core effect-free, native
effects explicitly owned, and CLI presentation thin. Freeze shared contracts
before parallel implementation in isolated worktrees with non-overlapping
ownership. Do not resume documentation-tool development during product work.

The combined feature pulls the following M04 prerequisites forward. They are
part of the same feature and gates, not separately counted deliveries:

- Native-owned `ask`/`auto`/`yolo`/`reset`, persistent allow/deny rules,
  confirmation and identity-bound grants, revocation, safe restoration, queued
  policy snapshots and actual enforcement. Late prompts cannot resurrect grants.
- Real macOS `os` sandbox enforcement for foreground, background and PTY
  execution with the active workspace roots; configured preferences remain
  distinct from effective yolo behavior. Unsupported platforms remain explicit.
- Authoritative session metadata, context selection without deleting history,
  and produced/consumed continuation checkpoints that preserve confirmed tool
  evidence and never automatically repeat uncertain effects. `/undo` tracks
  committed file mutations, not conversation deletion; clear/new/reset create
  fresh persisted session IDs with their distinct background-lifecycle behavior.
- Native machine-god schema migration, including supported historical versions,
  already-current and oversized-input outcomes, original-authoritative failure
  semantics and recoverable interruption. Foreign `.fx` import remains later.
- Corrupt-session recovery to a separate resumable copy without modifying the
  source, and bounded doctor cleanup distinguishing active writers, untrusted
  artifacts, report-only candidates, completed cleanup and indeterminate results.
- Real interactive latest/exact/picker resume and command aliases, applicable
  recording, durable title/workspace/origin/time/context preferences, and actual
  additional-root tool authority. Absent historical metadata stays unknown;
  neither current CWD, file mtime nor IDs fabricate historical associations.

The safety of these operations includes their admission, revocation, persistence,
cleanup and reset races now. Broader concurrency hardening, legacy import,
encryption, record authentication, key management, secure erasure and non-Unix
hardening remain M04 work. Required scenarios cannot close with unsupported
stubs. Three independent scope inspections accepted this sequencing; they are
not substitutes for the final exact-candidate product reviews.

Internal component commits are not deliveries. Their implementation inventory is:

- Core/native conversation ownership: exclusive metadata transactions,
  incarnation/revision-guarded load and CAS, atomic initial metadata and title,
  transcript-preserving reconciliation, paused continuation reservation and
  native finalization. Context selection projects provider-only views without
  deleting canonical history or repeating uncertain effects.
- Runtime/model ownership: bounded queued jobs, taken-job snapshots, deferred
  session preferences and independent session/user-default save receipts.
  Model identifiers retain the pinned 1,024-byte UTF-8 limit; rich catalogs
  preserve advertised capabilities, exact-first fuzzy resolution and bounded
  shared fetch/retry behavior. Initial resume restores saved controls with an
  explicit process-model override. Fresh sessions use workspace defaults.
- Secondary model routes: fixed Gemini vision and incarnation-bound search
  snapshots preserve their distinct pinned behavior, without inheriting
  reasoning/fast controls. The one-shot CLI shares one acquired credential
  across catalog and inference before terminal-host acquisition.
- History and undo: all five mutations share bounded file undo. Nine file
  tools record exact per-call typed observations; pending facts survive dropped
  streams and failed saves, merge before continuation/context projection and
  retire only after confirmed publication. Historical read staleness,
  destinations, background facts and missing metadata remain explicit.
- Permissions: bounded strict saved-rule values, ordered configured patterns,
  exact-turn policy snapshots, revocable execution proofs and reset-cleared
  grants. Confirmed edits use core CAS with per-rule generations and preserve
  uncertainty. Schema-v5 config retains mode, sandbox and ordered rules across
  legacy reads and descriptor-bound, conflict-checked user-default publication.
- Actual enforcement: prepared-root host composition shares registered tool
  allocations, live invocation contexts, descriptor/preimage file approvals and
  the fixed-model Gateway reviewer. Recoverable preparation denial permits
  replanning. Ask/resume enforce configured Ask/Auto/Yolo and deny unresolved
  noninteractive human requirements without fabricating confirmation.
- macOS sandboxing: actual foreground/background/PTY/tmux launches retain
  executable/root checks and taken policy. The CLI supplies the system
  executable explicitly; missing Os authority never falls back. None and
  effective Yolo remain distinct. Legacy background attachments are not
  inferred from terminal records or late notices.
- Sessions: bounded rich catalogs rank all reached records, preserve unknown
  facts, paginate with honest incomplete-scan outcomes and require complete
  eligibility for latest. Exact lookup is scan-independent. The CLI presents
  canonical current-workspace scope, all/limit/cursor selection and lossless
  non-UTF-8 path bytes without manufacturing authority.
- Resume: original workspace is distinct from current association; exact
  revision rebinding preserves origin and bounded history. Validated preparation
  checks the full native record (including saved rules), incarnation and
  revision before load; consuming adoption rechecks durable/live state. A
  prepared association publication is not rolled back by dropping adoption.

Remaining work includes CLI composition of native interactive ownership and presentation,
latest/exact/picker startup, long-lived cache and human-prompt composition,
remaining commands, recovery/retry and actual policy/workspace authority.
Clear/new/reset must allocate fresh IDs, not reuse the same-ID incarnation
reset. Clear/new carry terminals forward; reset and live resume stop/forget.
Started transitions and uncertain outcomes must remain owned through settlement.
The complete combined feature and all final gates remain open.

The rich sessions checkpoint `734a11f` passes Rust 1.94.1 formatting,
warnings-denied workspace Clippy, replacement workspace tests and doctests.
Focused coverage includes 67 native catalog/cursor/listing/store tests, all 87
CLI integration tests and 152 CLI unit tests with five existing helper ignores.
The freshly built release CLI passes all 17 sessions integration tests and is
the production terminal helper for the workspace run. All 1,590 native unit
tests pass with seven existing helper ignores. Repository Python checks run
263 tests successfully with 14 skips. Both integrated worker worktrees are
removed; their commits remain recoverable.

The first workspace attempt failed 11 PTY child-reaping admission checks and
two tmux checks. Admission waited for fixed `/bin/sh -c 'exit 0'`, before the
selected release helper or inventory started. No matching cross-test signal or
fault-injection mechanism was found. All 26 focused PTY tests, both exact tmux
tests and the replacement full workspace pass without source, deadline or test
concurrency changes. This preserves the failure evidence; it does not establish
its cause or claim a timing fix. These internal checkpoints are not deliveries
and do not promote compatibility or performance evidence. Final reviews and
exact remote evidence remain gates for the complete combined feature.

The validated-resume/origin integration passes 132 focused native session tests,
153 CLI unit tests with five existing helper ignores, all 89 CLI integration
tests and ten CLI frame tests. Its core checked-revision and guarded-load
prerequisites pass the complete core suite and doctests. The integration's
first workspace Clippy run rejected one CLI boolean expression; after the
equivalent simplification, all 17 sessions renderer tests and warnings-denied
workspace Clippy pass. Exact `0382b0f` then passes a fresh release build and all
19 release sessions integration tests. Its replacement full workspace, doctests,
formatting and documentation checks pass; Python checks pass 263 tests with 14
skips. The first workspace run failed 24 child-reaping admission checks and one
tmux check. The unchanged failed binary passes focused PTY/startup/staged checks,
and the unchanged replacement passes all 1,621 native unit tests with seven
existing ignores. No deadline or test-concurrency relaxation was made; the
replacement is not a claim that the unexplained first-run cause was fixed.

The current integration adds native runtime quiescence and exact route retirement,
including retained-alias and attach-time control races. Resume validation also
rejects malformed saved permission rules before canonical reconciliation. The
host retains its actual injected undo tracker and exposes explicit terminal
lifecycle authority and current-policy candidate attachment. Combined checks
pass 14 resume, 37 runtime, 18 controller, 11 preparer and 47 reference-host tests,
plus 24 internal lifecycle/route tests. These are prerequisites for the native
interactive owner, not a delivered session-switch feature.
Exact `f36444f` also passes 153 CLI unit, 89 command and ten frame tests,
workspace Clippy, formatting and documentation checks. Its three released worker
trees and integration worktree are removed; commits remain recoverable.
The terminal component now supports bounded exact-principal handoff/reset,
immutable journal ownership, generation/writer revocation, transferred history
and retained-indeterminate cleanup. Its 74 focused worker tests and scoped Clippy
pass; the integration passes 104 terminal host/start/catalog/wait tests with one
existing helper ignore. Pinned clear/new carry terminals forward;
reset and live resume use stop/forget. The native owner must preserve that
distinction and retain started operations through their actual receipts.
Owned undo-clear reservation and guard-owned settled selection snapshots pass
focused native and WASI worker checks. Their integration passes 30 undo, 38
runtime and 47 reference-host tests, workspace Clippy, formatting and docs checks.
The host exposes exact workspace/model/observation allocations and cheap runtime
identity. The integrated native interactive owner retains started preparation,
terminal commits, fenced uncertainty, undo and shutdown receipts independently
of presentation. Borrowed retirement keeps admission fenced on failure; empty
output takes no longer self-wake idle polling. Combined checks pass 39 runtime,
eight private and eight composed owner tests, plus 15 native byte-input tests.
The bounded CLI framer passes 11 standalone tests and pinned Clippy. One-shot
host acquisition now accepts explicit prompt adapters through a shared setup
path. These are component checks, not final feature gates. Human-prompt bridging,
flag-preserving ordinary stdin acquisition and the actual CLI driver remain open.
The shared setup integration passes 153 CLI unit tests with five helper ignores,
89 command tests and ten replay/frame tests. Workspace Clippy passes after a
test-only naming correction; no persistence or timing contract was weakened.

The integrated preparer passes all five real file mutations, actual reviewer
Allow/Ask/error and cancellation, read-grant/reset, canonical terminal identity,
and separate execution/review-bound regressions. All reference-host tests pass,
including configured denial, Ask/Auto/Yolo, real automatic review and actual
sandboxed terminal execution. The CLI's real create/resume turn path also passes
configured-mode regressions. These checks do not close the unfinished combined
feature or replace its final reviews and remote evidence.

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
  PTY, exact process-incarnation, fixed uptime-clock and read-only inventory
  bindings are the sole exception under
  [ADR 0003](decisions/0003-macos-terminal-foreground-signal.md) and
  [ADR 0004](decisions/0004-macos-process-inventory-helper.md); they remain subject
  to the full terminal feature's adversarial and platform gates.
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
| M04 | Security, lifecycle, concurrency, and persistence hardening | IN PROGRESS |
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
| 57 | Bounded same-incarnation `terminal` background output read | [terminal](terminal.md), [supervisor](background-supervisor.md) | [review](reviews/m03-terminal-read-review-01.md) | `b1bbb81` |
| 58 | Bounded same-incarnation `terminal` native signal | [terminal](terminal.md), [supervisor](background-supervisor.md) | [review](reviews/m03-terminal-signal-review-01.md) | `8545ea4` |
| 59 | Complete terminal actions, PTY/tmux sessions, durable history/screens, monitors, archives and owned cleanup | [terminal](terminal.md), [host](native-reference-host.md) | [review](reviews/m03-terminal-full-review-01.md) | `229cf94` |

The exact delivered-main record is in the canonical live-status
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
| M04 | Required modes/grants, native migration/recovery and guarded cleanup are pulled into the active combined CLI feature above; remaining work includes explicit legacy import, encryption, record authentication, key management, secure erasure, broader persistence/lifecycle concurrency hardening, and hardened non-Unix workspace/store construction |
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
supported Linux/macOS execution (including Linux's default CI test concurrency),
relevant FreeBSD/WASI compilation or active
unsupported behavior, documentation policy, no-unsafe conformance checks, and a
fresh locked release-binary smoke of user-visible behavior. Evidence is a
regression/delivery claim unless a milestone explicitly promotes it.
For macOS native/workspace runtime checks, build the release CLI first and set
`MACHINE_GOD_TERMINAL_RELEASE_BINARY` to its absolute path, matching the Apple
matrix's existing production-helper fixture selection. Keep explicitly
constructed protocol and failure fixtures; do not relax deadlines or skip tests.
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
