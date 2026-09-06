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
- Delivered slices: `58`
- Delivered main: `8545ea48623a40d412bce4f707c70a6be991d108`
- Main CI: `33949137350` (`GREEN`)
- Main Benchmark evidence: `33949136988` (`GREEN`)
- Active branch: `agent/m59-terminal-input`
- Active phase: `completing the full terminal feature; input/write is an internal component, not a separate delivery`
- Next gate: `wire the production terminal executor and reference host, including staged startup, foreground execution and authorized monitor effects, before the full terminal feature gate`
<!-- canonical-live-status:end -->

The exact delivered-main CI and Benchmark runs are green, and the Benchmark
run retains both required unexpired exact-SHA artifacts. Slice 58 adds the
closed five-value terminal `signal` action with exact session-incarnation
ownership, separate custom authorization, bounded off-poll-thread Linux
process-tree or macOS original-group delivery, and close-before-reap native
authority. The main-CI fixture race is fixed with an isolated process group,
independent pidfd assertions, bounded readiness, and owned cleanup. The exact
Rust 1.94.1 local gate and three fresh correction-review tracks passed.
Detailed history is retained only in the linked review ledger.

The delivered non-product CI maintenance replaces binary
full-versus-documentation routing with fail-closed dependency-aware concern
selection while preserving stable aggregate gates and exact-SHA Benchmark
evidence for behavior and evidence-affecting changes. It does not increment the
delivered-slice count. Documentation-only descendants now exercise only the
bounded documentation checks and aggregate gates, without Rust, platform,
audit, compatibility, or benchmark-artifact jobs.

The active complete terminal feature includes interactive handles, durable
logs, restart/recovery and the action contract below. ACP, teams and extension
slash commands retain their later milestone ownership.
Documentation-tool maintenance remains a separate non-product task.

The terminal integration now explicitly routes terminal-sys changes through
their package and native/CLI consumers, including Apple-only binding lint,
and verifies Unicode generation against the pinned upstream source. This is
an internal CI integration correction, not a new delivery; full-feature
review and exact remote gates remain pending.

### Active feature: complete terminal behavior

Deliver complete features, not separately counted action-sized slices. The
terminal branch stays open across input/write, interactive PTY lifecycle,
screen/monitor/resize/close, and the composed-host behavior needed to complete
the pinned terminal action contract. Internal component commits may be tested
and integrated independently; they are not deliveries or compatibility claims.
Review and exact local/remote delivery gates apply to the complete feature.

Native session lifetime belongs to the long-lived host registry, not individual
tool futures. Pinned registry shutdown terminates native sessions; durable
history and backend-specific recovery do not require a separate native daemon
or a new IPC protocol. Recovered numeric PIDs never grant process authority.
Registry shutdown tracks final-state publication independently of native
cleanup, and catalog reopening retries interrupted directory durability
barriers. These remain internal integration components, not a delivery.
The continuous loop now composes resident pumping with bounded asynchronous
requests and retained shutdown failures. Profile storage integration must cover
nonresident histories and concurrent owner namespaces, not just resident totals.
Reusable owned-worker spawning now shares the production collector. Retention
integration routes physical inventory and explicit eviction through registry,
session and validated history boundaries. Profile transactions now inventory
nonresident and busy foreign histories; admission accounts separately for
retained growth and whole temporary allocations, preserving committed receipts
across accounting failures. Borrowed journal plans now derive write demand and
bind it to the profile's exact session directory; sealed acknowledgement and
eviction plans permit bounded metadata-first quota recovery. Profile-bound
catalog preparation and directory creation enforce global counts before effects.
Initial journal metadata now passes admission, and history writes take explicit
persistence contexts. Pre-read permits reserve bounded output and follow-on
publications using Rust-encoder-derived checkpoint bounds, keeping committed
receipts separate from accounting errors. Session mutations now require held
persistence authority; cleanup without that authority retains a failed
publication and requires a durable gap barrier before later state publication.
The owner/registry now acquires profile admission before running-session reads
and defers contention or quota refusal without consuming output. Exit races
release the one-read permit before multi-chunk draining under the same held
transaction, preserving committed reads independently of cleanup errors.
Shutdown retains failed publication even when profile authority is unavailable.
Transaction drop explicitly releases the lock despite temporary fork inheritance.
Native status failures take the lost-session/quiescence path even when profile
publication is unavailable; they are not treated as harmless capacity deferrals.
Effect-free resize validation rejects unavailable projections without stopping
an otherwise running session or its raw-output pumping.
Resize admission distinguishes initial barrier refusal from final checkpoint
refusal after native effects; the latter loses/quiesces the session and retains
the unresolved publication failure.
Checkpoint reserves now persist in checksummed journal metadata; profile
admission and reconciliation separately enforce physical usage and reserved
output across busy and nonresident owners. Lifecycle registration precedes
startup and growth resize; release follows native cleanup and durable completed
facts. The profile owner selects bounded, oldest-first retention classes before
reads, using resident authority or acquired nonresident writer leases. Completed
crash-leftover reserves retire before payload eviction, including after resident
recovery. Discovery uses bounded facts-prefix hints without recovering unselected
payloads; classes without physical bytes are skipped, while selected histories
still require full validation before mutation.
Legacy metadata-less records remain readable but do not block eligible victims.
Profile-aware owner requests now pass the worker's exact store and budget into
bounded mutation dispatch; transaction guards end before reply wakes. Production
worker startup and host assembly remain open.
Actor/writer-bound attention and inspect projections are integrated, including
checkpoint re-anchoring for live and recovered facts. Registry wait observations
reject known observation gaps, and profile-backed completion releases storage
authority before reply publication. Asynchronous waits now share the owner loop,
including bounded durable catch-up, absolute ceilings and explicit shutdown
receipts. Lazy owned-runtime assembly and nonce-bound startup are implemented;
real bash/zsh user and clean profiles pass even when profiles close inherited
descriptors. Extracted PTY/marker helpers now have private production CLI entrypoints
and a release-binary test seam. Full host/action assembly and delivery gates remain
open; these internal component commits do not claim a feature delivery.
Startup now carries one bounded absolute deadline across preparation, commit and
acknowledgements, removing the hidden two-second cutoff inside longer requests.
Corrected component `a327580b754e25b8c1beaa5edf744b2b2252c620` passed pinned
Rust 1.94.1 formatting, strict workspace Clippy, workspace tests and doctests,
all 421 terminal component tests, and the freshly built release helper's 16
startup and 16 PTY tests, including delayed startup, deadline expiry and cleanup.
Full-feature reviews and remote delivery remain pending.
Session-owned startup now advances in the ordinary registry pump, with durable
shell/command phases, a drained profile-output boundary and timeout cleanup even
under profile-lock contention. Full disk/resident catalog projection is integrated
without resident admission, and ordered input retains paste intent and committed
byte receipts across later publication failure. Production host assembly remains
open.
The tmux controller and private foreground-server launch are integrated with
bounded whole-paste input, authenticated supervisor/process authority, exact
child outcomes and explicit capture-completion evidence. Native platform jobs
require a real tmux executable. The shared bootstrap and private CLI helper
retain original-terminal user/clean behavior; full host composition and action
routing remain open.
Pending write completion now shares the owner loop with capacity reserved before
effects, exact receipt identity, and final shutdown counts; cancellation does not
resubmit or discard the queued suffix.
Native PTY EOF now closes input immediately while allowing a bounded,
nonblocking exit-status convergence window; permanent capture loss still fails.
Final input settlement survives teardown without sending new bytes, and journal
writer retirement explicitly releases fork-inherited lock descriptions.
The complete authorized twelve-action adapter is integrated as an injectable
native API, with cwd resolution deferred to the worker. Core now separates
explicitly bounded complete results from their already-persisted transcript
references, retaining ordinary inline limits. Full provider argument admission
still requires composition. The nonce-bound startup
bootstrap is shared across transport backends without dropping paste or pending
write semantics. These are internal components, not a completed feature gate.
The runtime now binds its spawner to the production owned-worker collector;
the action adapter has an explicit owned-result publication hook. Native
archive publication and read-tool paging share bounded owned-worker completion
and preserve complete results outside the small durable transcript.
The fresh release helper now passes all 41 tmux component checks, including
original-terminal user/clean startup, exact child outcomes and capture-loss
cleanup. Native input publication and prior-call paging now compose with core
per-tool raw/prepared admission and separately bounded Gateway input framing.
The actual terminal adapter preserves a full byte-array command through archival
publication, authorization and execution while keeping subsequent model history
small. Full reference-host registration and its explicit admission configuration
remain open; these components are not a feature delivery.
Typed worker-owned host state and the common PTY/tmux backend selection are
integrated. State survives cleanup retries and is destroyed after native
resources; transcript admission moves validated input instead of cloning it.
The preceding input/archive integration passed the complete pinned workspace
format, Clippy, test and doctest gate. Production action dispatch and full host
registration remain the next work, not documentation of a compatibility exception.
The shared native launch factory and nine-action resident dispatcher are now
integrated with cold-history read/screen/inspect routing. Catalog leases stay on
the worker; historical access does not consume live slots. Non-owning request
handles support staged effect-worker callbacks without extending host lifetime.
The real launch matrix covers PTY/tmux, bash/zsh, user/clean and full command/paste
payloads. The descriptor-bound foreground service and exact private CLI helper
are registered, including separate exec-failure versus command-exit reporting.
Pending start admission now counts against the same registry limit and is
invalidated at shutdown. Staged startup/monitor effect orchestration and final
reference-host wiring remain open; these are not a complete-feature delivery.
The complete normalized twelve-action core contract and effect-free public
decoder are present, including request/result identity checks and lossless
foreground execution outcomes. Numeric coercion now matches pinned binary128
rounding. Foreground decoding preserves explicit user/clean profiles and leaves
omission unset for the pinned user-profile default at shell resolution.
Semantic 64 KiB commands survive native transport, background
persistence, host/provider admission and CLI rendering with encoding-derived
ceilings. Background storage preserves its independent aggregate quota through
pending and live-replacement reservations, including old orphaned histories.
These remain internal components; the complete action contracts do not yet
replace the reference host's legacy terminal adapter. Compatibility component
`de480453718169adf5e458a34c727833dbf32cd1` passed the exact pinned local gate,
fresh release command round-trip and three fresh local review tracks after
correcting the decoder's legacy-only exec profile restriction. Full-feature
host assembly and exact remote delivery gates remain open; this is not a delivery.
Per-session retention separates raw/checkpoint output from independently bounded
state and events, matching the pinned output-accounting boundary. Non-output
publications cannot evict output; recovery retains all ledger categories.

Completion is measured against the pinned upstream terminal schema and native
session contracts, not the existing numeric background-record subset:

- All twelve actions: `exec`, `start`, `read`, `screen`, `write`, `wait`,
  `monitor`, `inspect`, `list`, `resize`, `signal`, and `close`.
- Interactive start without a command, optional command startup, native PTY
  and tmux backends, user/clean profiles and supported bash/zsh resolution,
  actual terminal dimensions, and sessions that outlive individual tool calls.
- Durable segmented raw output with explicit cursor gaps, retained events and
  acknowledgements, validated screen checkpoints/recovery, and owner-authorized
  session facts/catalog filters. The pinned defaults are 1 MiB segments,
  64 MiB per session, 512 MiB per profile, and 256 retained events.
- Styled structured screens, cursor/modes, Unicode/wide cells, terminal query
  replies in live execution only, and real PTY resize verified by `stty size`.
- Text, paste, named keys and control-character input; acquire/use/release/revoke
  write leases; bounded accepted-byte receipts and input quiescence.
- Started/exit/quiet/match waits with safety ceilings; all thirteen monitor
  conditions, operations, schedules, notifications and lifetimes. Repeated
  network, filesystem and custom-probe effects require explicit authorization.
- Graceful close (terminate, up to 800 ms, then kill) and force close, monitor
  shutdown, retained readable history, restart/recovery and backend-failure
  behavior, without granting control from a persisted numeric PID.

Reference:
[pinned terminal schema](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/tools/terminal/terminal.zig),
[native sessions](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/core/terminal/native_session.zig),
[monitor contracts](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/core/terminal/monitor.zig).

As an internal component, implement `terminal write` and explicit piped stdin for
background starts; null stdin remains the default. Freeze shared signatures,
then parallelize core/permission contracts, native input ownership/transport,
and terminal parsing/receipts in isolated worktrees with one integrator.
Bound decoded writes to 8 KiB, report accepted byte counts and backpressure,
and apply explicit EOF only after all supplied bytes are accepted. Bind writes
to the exact session incarnation and separately authorized payload identity;
close authority before process cleanup and preserve committed-write receipts
across cancellation. Replace buffered helper protocol reads with exact reads
before inheriting the pipe, so immediate post-commit input cannot be lost.
Exercise binary/Unicode round trips, partial writes, EOF, ownership, concurrent
cleanup, and cancellation. PTY/interactive terminal and live output behavior
remain part of this feature; piped input does not complete it.

## Architecture ownership

- `machine-god-core` owns provider-neutral contracts and orchestration. It has
  no ambient filesystem, process, environment, terminal, or network authority.
- `machine-god-native` owns explicitly injected operating-system, network,
  terminal, configuration, and persistence effects.
- `machine-god-cli` is a thin host and owns no product state.
- `machine-god-testkit` owns deterministic test doubles and fixtures.
- Unsafe Rust is forbidden in the product crates. The isolated macOS
  PTY and exact process-incarnation bindings are the sole exception under
  [ADR 0003](decisions/0003-macos-terminal-foreground-signal.md); it remains subject
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
| 57 | Bounded same-incarnation `terminal` background output read | [terminal](terminal.md), [supervisor](background-supervisor.md) | [review](reviews/m03-terminal-read-review-01.md) | `b1bbb81` |
| 58 | Bounded same-incarnation `terminal` native signal | [terminal](terminal.md), [supervisor](background-supervisor.md) | [review](reviews/m03-terminal-signal-review-01.md) | `8545ea4` |

The exact delivered-main record after slice 58 is in the canonical live-status
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
unsupported behavior, documentation policy, no-unsafe conformance checks, and a
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
