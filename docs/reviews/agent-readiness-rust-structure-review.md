# Agent readiness and Rust structure review

Reviewed revision: `3a0df99098f62e88aa6d2f29c826797a7320abab`.

This is a historical advisory snapshot, not a feature acceptance review or a
second live task ledger. The [implementation plan](../implementation-plan.md)
remains the only live source for implementation and delivery state. Findings
below describe the reviewed revision; they do not assert that later revisions
retain these issues. Remediation has not been performed as part of this review.

## Scope and method

The review covered agent instructions, documentation navigation, repository and
test structure, idiomatic Rust, separation of concerns, SOLID, and DRY. The
request's spelling “SOKOL” was interpreted as SOLID.

Three parallel read-only reviewers inspected independent areas. Their findings
were checked against source by the coordinator. Inspection was weighted toward
high-churn code and important composition boundaries, not every source line.
Unintegrated implementation worktrees were excluded and preserved.

The initial eight findings were disproportionately concentrated on CLI and
workflow concerns. The expanded pass below assesses every workspace crate,
native subsystem groups, the excluded test-support crate, and repository
tooling. Additional findings retain separate IDs so that broadening this review
does not silently change the implementation scope already selected from the
initial findings. Both passes use the same reviewed source revision.

The review did not rerun full workspace tests, remote CI, dependency audits,
benchmarks, or a comprehensive platform/security audit. It does not establish a
performance improvement or release readiness. Source paths and line numbers
below refer to the reviewed revision.

## Codebase coverage

“Implementation-focused” means selected implementations and their consumers
were traced, not that every line in that area was audited. “Boundary-focused”
means contracts, dependencies, composition, or selected tests were inspected;
it is not a correctness endorsement of every implementation behind them.

| Area | Inspection and assessment | Depth / findings |
| --- | --- | --- |
| `machine-god-core` | Engine dependency injection, cancellation, validated identifiers, model and permission contracts, tool preparation/execution, session storage/orchestration, and subagent JSON-bound consumers. Provider-neutral contracts and explicit authority are strengths; generic JSON machinery is misplaced inside session orchestration. | Implementation-focused on these seams; findings 6 and 9. Not an exhaustive session state-machine review. |
| `machine-god-native` | Filesystem tools, retained-root checks, history/approval composition, persistence/configuration, runtime composition, network adapters, and extensibility boundaries. Effects remain in the appropriate crate; shared low-level mechanisms and the root configuration façade need better cohesion. | Mixed depth, detailed below; findings 6, 7, 10, and 12. |
| `machine-god-cli` | Command dispatch/injection, bounded rendering, extracted command modules, signal/helper fixtures, and platform test prerequisites. The host boundary is generally sound; local rendering and command wiring repeat mechanisms. | Implementation-focused on reviewed command seams; findings 1, 4, and 8. Not a fresh end-to-end acceptance run. |
| `machine-god-testkit` | Exports and scripted provider, tool, permission, event sink, session-store, and subagent fixtures. Bounded recording and strict scripts are useful agent-facing test seams; contextual preparation cannot be recorded by the shared prepared-tool double. | Implementation-focused on preparation and selected script/storage paths; boundary-focused elsewhere. Finding 11. |
| `machine-god-terminal-sys` | Manifest/lints, public wrapper surface, process identity bindings, bounded process-inventory decoding, and C ABI assertions. Narrow unsafe exceptions are documented rather than spread through product crates. | Selected binding/decoder implementation and ABI-test inspection; no additional finding established. No independent Darwin ABI or kernel-behavior verification. |
| `test-support/reentrant-waker` | The excluded test-only crate and its `RawWaker`/`Arc` ownership implementation were read in the context of ADR 0002. Keeping this fixture outside product unsafe exceptions is intentional. | Implementation-focused static read; no additional finding established. No Miri run. |
| Build, CI, and test layout | Workspace manifests, pinned toolchain/lints, change classification, workflow prerequisites, source-including integration harnesses, and formatter discovery. The platform/package selection contract is useful; its prerequisite and test-topology gaps are actionable. | Implementation-focused on the reported selection/layout issues; findings 1, 2, 5, and 7. |
| Compatibility and benchmarks | Compatibility test inventory, benchmark provenance/shape validation, and selected fixture tests. These are evidence tooling, not substitutes for product acceptance or a measured Rust-versus-upstream performance result. | Boundary-focused, with selected validator code read; no additional finding established. Generators and all benchmark paths were not exhaustively audited or executed. |
| Agent instructions and documentation | Root instructions, README entry points, the live ledger, review navigation, architecture decisions, and the bounded documentation checker. The principal problem is duplicated/stale operational guidance, not a need for more dashboards or parser features. | Implementation-focused on navigation and gate recipes; findings 2 and 3. |

### Native subsystem coverage

The native crate is too broad for a single undifferentiated “reviewed” label.

| Subsystem | What was inspected | Assessment / remaining depth limit |
| --- | --- | --- |
| Filesystem and workspace | Read/search/mutation root-identity paths, workspace authority composition, file-history wrapping, and approval binding. | Findings 6 and 10. Caller-specific cancellation and commit-phase checks are meaningful differences, not duplication to erase indiscriminately. Every filesystem race/interleaving was not replayed. |
| Persistence and configuration | Session-store publication/revision handling, user-config store structure, bounded memory contracts, and root environment/status implementation. | Finding 12 concerns module ownership. Native ownership of persistence is appropriate; no new storage-correctness failure was established. Crash/durability behavior was not tested. |
| Provider and network adapters | Shared HTTP credential/header machinery, adapter interfaces, cached model-loading ownership, and feature composition. | A shared HTTP primitive is a positive DRY example. Boundary-focused coverage does not establish full codec, streaming, retry, or web-tool protocol correctness. |
| Conversation and process runtime | Conversation limits/lifecycle contracts, owned-worker observation, reference-host composition, terminal/background test graphs, and helper entry points. | Findings 6 and 7 affect these seams. No comprehensive scheduler, cancellation-race, or process-lifetime acceptance run was performed by this audit. |
| MCP, skills, and other extensibility | MCP selection/features contracts and authority injection, memory bounds, and skill-install staging/checkpoint structure. | Boundary-focused; no additional confirmed finding. Full MCP transport and skill-install rollback behavior remain outside the deep inspection performed here. |

No additional finding means none was established by this inspection. It does
not mean that an area is proven bug-free. Dependency freshness/advisories,
cross-platform execution, measured performance, and exhaustive concurrency
verification remain explicitly outside this advisory review.

## Initial prioritized findings

Order reflects expected benefit relative to effort, confidence, and change risk.
Effort includes tests: S means hours, M roughly a day, L multiple days. Risk is
the risk of implementing the proposed change, not vulnerability severity.

| ID | Finding | Effort | Fix risk | Confidence |
| --- | --- | --- | --- | --- |
| 1 | CLI-only macOS CI skips a mandatory helper | S | Low | High, static control-flow evidence |
| 2 | Prominent local-check recipes omit prerequisites | S | Low | High |
| 3 | The live ledger retains superseded remaining-work statements | S | Low | High |
| 4 | Seven CLI bounded-output writers duplicate one mechanism | S | Low | High |
| 5 | Standard formatting checks miss included test fragments | S | Low | High, locally reproduced |
| 6 | History wrapping changes invocation-capture timing | M | Medium | High for the mismatch; production reachability unverified |
| 7 | Integration targets reconstruct private source-module graphs | L | Medium | High |
| 8 | CLI command responsibilities remain concentrated in the entry point | M | Medium | High |

### 1. Provision the Apple helper for CLI-only test selections

Evidence:

- [.github/workflows/ci.yml](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/.github/workflows/ci.yml), lines 428–452,
  selects CLI independently of native and enables the platform matrix.
- The same workflow, line 1425, builds and exports the production terminal
  helper only for native or full-workspace selections. Lines 1467–1469 still
  execute selected Apple CLI tests.
- [terminal_lifetime_tests.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-cli/src/ask/production/interactive/terminal_lifetime_tests.rs),
  lines 23–29, requires `MACHINE_GOD_TERMINAL_RELEASE_BINARY` on macOS.
- [test_ci_change_classification.py](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/tests/test_ci_change_classification.py),
  lines 815–820, asserts the incomplete prerequisite condition.

Impact: a CLI-only change reaches macOS tests without their mandatory fixture,
causing an infrastructure failure after compilation. This follows from the
selection and assertion paths; a remote failure was not reproduced in this audit.

Recommendation: include selected CLI tests in the Apple helper condition and
add a CLI-only prerequisite regression. Preserve negative cases for unrelated
selections; this does not require running more packages' tests.

### 2. Keep one complete local-check recipe

Evidence: [AGENTS.md](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/AGENTS.md), lines 30–40, and
[README.md](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/README.md), line 55 onward, present the four-command gate
without helper setup. The [implementation plan](../implementation-plan.md),
lines 492–508, repeats the commands before explaining that macOS runtime checks
need a freshly built release CLI and its absolute helper path.

Impact: agents copying the prominent commands encounter predictable fixture
failures and then spend time distinguishing setup problems from regressions.

Recommendation: retain one canonical recipe in the implementation plan, with
platform prerequisites before test commands, and link to it from AGENTS and
README. Preserve the exact toolchain, explicit protocol fixtures, and deadlines.
No additional gate document is needed.

### 3. Compact historical checkpoints out of the live ledger

Evidence: the [implementation plan](../implementation-plan.md) had 577 lines.
Line 240 says actual tool/approval/sandbox integration and CLI handlers remain
required; line 259 records integrated top-level workspace management; line 269
still describes production tool routing as required; lines 279–290 record later
host integration and the newer remaining boundary.

Impact: agents instructed to read the ledger must reconcile successive
historical “remaining” statements before selecting work. This consumes context
and increases the risk of redundant implementation.

Recommendation: replace the checkpoint narrative with one current
integrated/remaining summary. Preserve superseded SHAs, test counts, and cleanup
evidence in the existing [combined CLI history](m03-cli-full-review-01.md).
Keep the canonical live fields and do not expand the Markdown scanner.

### 4. Share the CLI bounded-output mechanism

Evidence: equivalent bounded `fmt::Write` implementations appear in
[main.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-cli/src/main.rs), lines 67, 97, and 127;
[status.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-cli/src/status.rs), line 53;
[sessions.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-cli/src/sessions.rs), line 277;
[workspace.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-cli/src/workspace.rs), line 291; and
[background.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-cli/src/background.rs), line 584.

Impact: changes to byte-limit enforcement require repeated edits. Implementations
already use different arithmetic and initialization patterns for the same job.
No bounds violation is claimed from those differences alone.

Recommendation: introduce a small CLI-local `BoundedOutput` with a configurable
byte ceiling. Preserve command-specific rendering, capacities, error categories,
and output-before-write guarantees. Test exact limits and multibyte text; avoid
turning this into a general rendering framework.

### 5. Make included test fragments visible to formatting checks

Evidence: [terminal_host.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/terminal_host.rs),
line 1035, uses `include!` for
[terminal_permission_policy/host_tests.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/terminal_permission_policy/host_tests.rs).
The workflow's formatting steps use Cargo formatting.

The coordinator reproduced the gap at the reviewed revision: the first command
passed, while the second failed with ordinary formatting differences, not
edition or parser errors. Both commands were check-only:

```sh
cargo +1.94.1 fmt --all -- --check
rustup run 1.94.1 rustfmt --check --edition 2024 --config-path rustfmt.toml crates/machine-god-native/src/terminal_permission_policy/host_tests.rs
```

Impact: agents can satisfy the advertised formatter gate while included Rust
test files remain unformatted.

Recommendation: prefer ordinary discoverable module declarations where practical,
preserving helper test paths. Explicitly cover any remaining included fragments.
Treat verification-tool maintenance separately from product feature work.

### 6. Preserve invocation capture across history wrappers

Evidence:

- [file_history_tool.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/file_history_tool.rs),
  lines 198 and 218, constructs the wrapped execution future only when polled.
  Its test at line 417 deliberately requires no inner construction before polling.
- [write_file.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/write_file.rs), line 319,
  captures an approval ticket when its execution future is constructed.
- [file_approval.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/file_approval.rs),
  lines 389–408, explains that this construction stamp prevents an old unpolled
  execution from consuming a later replacement grant.
- [reference_host/permissions.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/reference_host/permissions.rs),
  lines 208, 221, and 365–371, retains outer history wrapping for the supported
  non-workspace mutation composition. Scoped mutations use a different order.

Impact: wrapper composition changes when an invocation acquires its identity
stamp. An apparently observational adapter therefore changes a lifecycle
property relied on by another component. A delayed history-wrapped execution
may capture a later grant rather than its original construction outcome;
production-engine reachability was not reproduced and is not asserted as a
demonstrated policy bypass.

Recommendation: first add a regression through the actual non-workspace
history/permission wrapper chain, including replaced grants and unpolled calls.
Define an explicit effect-free binding contract that preserves identity capture
while delaying effects and history reservation until polling. Do not blindly
make every inner constructor eager; that would contradict existing admission
and unpolled-construction expectations.

### 7. Consolidate redundant private-source test graphs

Evidence: [terminal_pty_component.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/tests/terminal_pty_component.rs),
line 47; [terminal_state_components.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/tests/terminal_state_components.rs),
line 10; and [native lib.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/lib.rs), line 399,
each compile `terminal_display_width.rs`, including its identical pure unit tests
at [line 248](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/terminal_display_width.rs).

Static inspection found 54 direct `../src` inclusions across five integration
targets: 38 PTY, eight state-component, six background, one parser, and one
permission-preparer inclusion.

Impact: duplicated compilation and execution add verification work. Mirrored
crate-root imports also make private-module refactors affect multiple harnesses.
No whole-suite speedup has been measured.

Recommendation: inventory duplicated unit coverage and retain one authoritative
location for pure tests. Preserve distinct public integration assertions,
private fault-injection coverage, and explicit helper-process entrypoints.
[ADR 0004](../decisions/0004-macos-process-inventory-helper.md) justifies the
helpers, not triplicating unrelated pure display tests. Consolidate incrementally.

### 8. Finish separating CLI command responsibilities

Evidence: [main.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-cli/src/main.rs) contains model
composition at line 454, model signal driving at line 741, command-specific
rendering from line 1708, and dispatch through a positional seven-host tuple
at line 1346. Test injection helpers repeat that tuple from line 1135 onward.

Impact: independent command changes share the entry-point file and manually
repeated dependency wiring. This makes non-overlapping agent ownership harder.
The finding concerns mixed responsibilities and coupling, not file length alone
or a demonstrated violation of core/native authority boundaries.

Recommendation: follow the existing `sessions`, `status`, and `workspace`
module pattern for models, doctor, and session inspection. Replace positional
host tuples with named dependencies. Preserve parsing-before-effects behavior,
signal/output lifetimes, diagnostics, and test injection seams.

## Additional whole-codebase findings

These extend the original review beyond its CLI-heavy emphasis. They are
structural or testability findings, not newly reproduced runtime failures.
Their implementation must be selected and recorded in the single live plan;
this section is not a second implementation checklist.

| ID | Finding | Effort | Fix risk | Confidence |
| --- | --- | --- | --- | --- |
| 9 | Generic core JSON machinery is coupled to session orchestration | M | Medium | High, dependency/call-site evidence |
| 10 | Native retained-root identity observation is repeated across seven consumers | M | Medium | High, implementation comparison |
| 11 | Testkit cannot record contextual tool preparation | M | Low–Medium | High, contract and fixture comparison |
| 12 | Native root façade also owns environment/status implementation | M | Medium | High, module-responsibility evidence |

### 9. Separate core JSON mechanics from session orchestration

Evidence:

- [core/session.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-core/src/session.rs),
  lines 2520–2820, defines JSON byte counting, validation budgets, root
  validation, iterative value destruction, and bounded serialized sizing.
  The same module owns session-store contracts, session state, and turn
  orchestration, including `run_turn_inner` at line 1686.
- [core/tool.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-core/src/tool.rs),
  lines 198–212 and 370–372, calls `crate::session::drop_json_value_iterative`.
- [core/engine.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-core/src/engine.rs),
  lines 118, 205–254, and 405, also uses session-owned JSON helpers;
  [core/subagent.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-core/src/subagent.rs),
  lines 3–6, imports the generic helpers and limit type from `session`.

Impact: changing shared JSON bounds or cleanup requires navigating and owning
the session orchestration module. Tool values, engine construction, and
subagents depend on an unrelated high-level module for a lower-level
mechanism, complicating independent agent assignments. This is a cohesion
finding, not evidence that current limits or destruction are incorrect.

Recommendation: move these helpers and focused tests into a private core
JSON/bounds module. Preserve depth, node and byte limits, iterative destruction,
and exact error mapping. Avoid a new crate, public API, ambient effects, or a
generic validation framework. Validate existing deeply nested/oversized values
and destruction behavior through all four consumers before and after the move.

### 10. Share the narrow retained-root identity observation primitive

Evidence: materially repeated macOS `fstat` / retained-path / parent-open /
non-following `statat` / device-and-inode comparison sequences occur in:

- [grep_files.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/grep_files.rs),
  lines 2291–2324, and
  [glob_files.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/glob_files.rs),
  lines 998–1031.
- [write_file.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/write_file.rs),
  lines 1265–1287;
  [edit_file.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/edit_file.rs),
  lines 926–948; and
  [copy_file.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/copy_file.rs),
  lines 1015–1049.
- [file_info.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/file_info.rs),
  lines 406–435, and
  [session_store.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/session_store.rs),
  lines 853–882.

Impact: a correction to the same identity-observation mechanism needs several
coordinated edits across independently owned tools and storage. This raises
maintenance drift risk; the audit did not establish an existing confinement
failure from that duplication.

Recommendation: characterize replaced, renamed, unlinked, and filesystem-root
cases, then extract only the shared identity observation. Preserve caller-owned
cancellation/checkpoint placement, error types, retryability, and precommit or
walk-phase semantics. Grep's scan checks, glob's cancellation checks, copy's
precommit wrappers, and edit's phase mapping are deliberately different. A
shared primitive must retain those distinctions, not merge whole traversal
implementations or reduce validation around syscalls.

### 11. Let shared test doubles observe contextual preparation

Evidence:

- [core/tool.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-core/src/tool.rs),
  lines 496–515, defines `prepare_for_turn` with a `ToolContext`; the default
  delegates to `prepare`, while wrappers are required to forward the context.
- [testkit/tool.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-testkit/src/tool.rs),
  lines 44–48, records only `ToolCall` in `RecordedToolPreparation`. Its
  `ScriptedPreparedTool` implementation at lines 313–363 overrides `prepare`
  but not `prepare_for_turn`, so contextual preparation falls through without
  recording that context.
- [native/file_history_tool.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/file_history_tool.rs),
  lines 360–366, instead implements a custom fixture hook that embeds context
  into prepared arguments to exercise forwarding.

Impact: agents cannot use the shared prepared-tool fixture to assert exact
session-incarnation, turn, and call context forwarding. Tests of this core
contract need custom doubles, increasing fixture duplication and making a
missing forwarding override easier to overlook. Execution-context recording
already exists; this finding concerns preparation specifically.

Recommendation: add bounded contextual-preparation recording and a
`prepare_for_turn` override while preserving strict script consumption and
existing `prepare` behavior. Account for callers constructing the public record
type before adding a required field; an additive recording accessor or separate
record type can preserve that source compatibility. Test exact context
forwarding, direct non-contextual preparation, exhaustion, and recorder limits.
Do not change invocation-construction timing as an incidental fixture cleanup.

### 12. Separate native environment/status implementation from the façade

Evidence: [native/lib.rs](https://github.com/distributedstatemachine/machine-god/blob/3a0df99098f62e88aa6d2f29c826797a7320abab/crates/machine-god-native/src/lib.rs)
combines module declarations and extensive feature/platform-gated re-exports
with `PermissionMode` at line 973, `NativeSandboxMode` at line 997,
`NativeEnvironment` and process capture from line 1022, `NativeStatus` at line
1131, status inspection from line 1189, path/root/configuration helpers from
line 1231, and their tests.

Impact: wiring an unrelated native module and changing configuration/status
behavior both require ownership of the crate root. The concern is mixed
responsibilities and a shared edit surface, not the number of exports or file
length by itself. Keeping these effects in native is correct.

Recommendation: move environment/status implementation and its focused tests
into a cohesive private module, retaining compatible root re-exports. Preserve
all feature/platform conditions, process-environment snapshot behavior, path
validation, and diagnostics. Check default/all-feature configurations and
supported platform compilation; do not widen this into a mass namespace or
crate split.

## Architecture assessment and intentional choices

Dependency inversion and explicit ownership are strengths: core remains
independent of native effects, adapters accept injected capabilities, and
platform-specific unsafe bindings have narrow ADR-defined boundaries.

The principal weaknesses are module cohesion, adapter substitutability, and
duplicated mechanisms rather than basic Rust syntax. Executor-independent boxed
futures, explicit capability checks, and platform-specific implementations were
not treated as defects merely because they add code. File length alone was
also rejected as sufficient evidence of poor architecture.

Core's validated identifier types, explicit injected dependencies, structured
errors, and cancellation ownership are idiomatic Rust strengths. Native
effects, CLI hosting, and deterministic testkit fixtures have sensible crate
boundaries. The additional recommendations strengthen cohesion inside those
boundaries instead of replacing them with a generic framework.

The terminal binding and test-only waker exceptions are deliberate decisions
in [ADR 0002](../decisions/0002-reentrant-raw-waker-test-fixture.md),
[ADR 0003](../decisions/0003-macos-terminal-foreground-signal.md), and
[ADR 0004](../decisions/0004-macos-process-inventory-helper.md), not reasons to
introduce unsafe code elsewhere. Likewise, upstream Zig build inputs belong
to compatibility/evidence work; they do not make the Rust product a Zig
implementation or warrant removal in this maintainability review.

Keep the concise root AGENTS instructions and the single live ledger. More
agent-specific dashboards or a larger documentation parser would not address
the identified problems.

## Suggested follow-up order

Prioritize findings 1, 2, 3, and the regression investigation for 6. Establish
wrapper-chain characterization before changing lifecycle abstractions. Findings
4 and 5 are bounded maintenance opportunities; undertake test-graph and command
module refactors separately after preserving their existing coverage.

For the additional findings, the contextual-preparation fixture (11) provides a
bounded testing improvement. Keep the core helper extraction (9), native
configuration extraction (12), and retained-root primitive (10) as independent
slices; establish platform/caller characterization before the last of these.

The original eight findings formed the initial selected maintenance scope.
Findings 9–12 broaden the advisory review and are not silently added to that
implementation batch. Accepted work and its delivery status belong in the
existing implementation plan, not this historical document.

## Historical maintenance acceptance

The selected scope ultimately included all twelve findings. Candidate
`0193fab9926cd0aa9b6e348fffabd11e8e3d4e46` passed its complete pinned-toolchain
local gate, three independent local reviews with no actionable findings,
feature CI `34363000919` and artifact-producing Benchmark `34363001004`.
Both exact-candidate benchmark artifacts were retained through 2026-12-08.
The test-graph change preserved all 2,089 unique native test identities while
removing 965 duplicate executions. Additional fixture corrections preserved
production behavior: deterministic model-save obstruction, schema-v7 current
and future credential expectations, and direct hexadecimal-byte construction.
Quality and Linux/Apple jobs now provision the explicit production helper;
benchmark collectors and evidence validation agree on explicit `--help`.
These corrections followed rejected remote candidate `53378c8c`; that
candidate's earlier local/review success did not override its CI failure.

Documentation seal `e14cab5d` passed lightweight CI `34365966890` and Benchmark
`34365966876`, with heavy jobs skipped and no new artifacts. It was
fast-forwarded into parent `agent/m60-cli-shell`. Maintenance implementation
and review worktrees were removed; the three then-unfinished product trees were
preserved. This accepted the maintenance only, not the unfinished combined CLI
or an M07 performance claim. Superseded checkpoint detail is retained in the
implementation plan at that seal; current delivery state belongs only in the
live implementation plan.

## Test execution and parallelism follow-up

Assessment date: 2026-09-09. This follow-up examines maintenance revision
`0193fab9926cd0aa9b6e348fffabd11e8e3d4e46`, not the original reviewed revision.
It combines the coordinator's recorded local gate timings with read-only
inspection of test orchestration and official nextest documentation. No nextest
comparison was run; the recommendations below are not measured speedup claims
or an expansion of the original twelve-finding acceptance boundary. Any selected
implementation belongs in the single live plan as separate test-infrastructure
maintenance, not in product-tool iterations.

### Observed bottlenecks

| Area | Recorded local elapsed time | Interpretation |
| --- | ---: | --- |
| macOS native library tests | 434.31 seconds; 2,064 passed, 12 existing helper tests ignored | The serial native suite dominates Rust test execution. |
| CLI unit tests | 6.56 seconds; 302 passed, 5 existing helper tests ignored | Not the principal runtime bottleneck. |
| CLI integration tests | 10.45 seconds; 99 passed | Smaller optimization opportunity. |
| Repository Python suite | 317.524 seconds; 107 tests | Includes a fresh release compilation, not just Python execution. |

These are individual gate observations, not controlled before/after benchmarks.
Compilation, test execution, host load, and warm versus cold caches must be
measured separately before choosing concurrency or claiming a gain.

Evidence at the follow-up revision:

- `.github/workflows/ci.yml`, lines 1465–1480, serializes every selected Apple
  package's tests to avoid shared process-table contention. Linux retains its
  default test concurrency; platform jobs already run in parallel.
- `tests/test_native_manifest.py`, lines 111–184, creates a fresh temporary
  target directory and builds the release panic-cleanup example with locked,
  offline Rust 1.94.1 before running it. This rebuild is part of the Python gate.
- The CI workflow has no Rust build-cache step and installs pinned cargo-deny
  and cargo-audit from source. Its existing dependency-aware path selection
  already avoids unrelated checks; nextest is complementary to that selection.
- `crates/machine-god-native/src/background_process.rs` defines
  `GROUP_SNAPSHOT_TEST_LOCK`; process, PTY, tmux and host fixtures use it.
  Locks protecting process-local fault hooks must be distinguished from locks
  protecting shared host resources before changing runner semantics.
- The implementation plan's review ordering places three independent review
  tracks after the complete local gate and remote verification after review.
  The review tracks themselves already run concurrently.

### Safe parallelism boundaries

| Work | Proposed scheduling | Constraint |
| --- | --- | --- |
| Pure core, testkit, JSON, codec and parsing tests | Concurrent execution | Tiny tests may be faster under Cargo because process startup has a cost; measure both runners. |
| Independent native filesystem/configuration tests | Bounded concurrency | Verify isolated temporary roots, environment assumptions and absence of shared host state first. |
| Terminal, process inventory, PTY and tmux lifecycle tests | Explicit limited/serial groups or a dedicated runner | Preserve original deadlines, ownership assertions and child cleanup; do not overlap host-sensitive fixtures with unrelated heavy builds. |
| Lightweight repository checks | Overlap independent Rust work | Separate the release-compiling probe from genuinely lightweight Python checks. |
| Frozen-candidate reviews and CI | Overlap independent verification | Requires an explicit workflow amendment; all required results must still pass for the exact accepted SHA. |

Nextest runs each test in a separate process. Existing static mutexes cannot
provide inter-test mutual exclusion across those processes. Process-local
fault-injection state may become naturally isolated, while host-wide resources
still need runner-level coordination. See the official
[execution model](https://nexte.st/docs/design/why-process-per-test/).

Use nextest [test groups](https://nexte.st/docs/configuration/test-groups/) to
limit related tests. A group with `max-threads = 1` serializes only its members,
not all other tests. Tests requiring exclusive access within one runner can use
`threads-required = "num-test-threads"`; this does not coordinate separate runner
or Cargo processes. Truly host-sensitive work may need a dedicated machine.
See [heavy-test scheduling](https://nexte.st/docs/configuration/threads-required/).

Verify ignored subprocess-helper entrypoints, exact test names, custom harnesses,
environment fixtures, failure propagation and leak/timeout handling before
substituting runners. Keep doctests under `cargo test --doc --workspace`.
Preserve existing shared-process concurrency coverage during the trial; process
isolation can change which interference bugs tests expose. Do not relax product
deadlines, silently drop tests, or use retries to hide new flakes.

### Build reuse and bounded rollout

First investigate building the exact release panic probe once and supplying its
verified path to the harness, preserving release-mode cleanup coverage and its
build configuration. Cache pinned tooling and compatible Rust dependencies with
explicit OS, architecture, toolchain, lockfile and build-configuration inputs.
Do not trust a stale helper merely because its path exists. Avoid multiple
agents independently compiling the same frozen candidate or contending on one
mutable Cargo target directory.

If execution still warrants CI sharding, build once per compatible target and
configuration, then distribute test binaries and required helpers/fixtures to
matching runners at the same source SHA. Account for archive transfer and setup
costs. Nextest supports [build archives](https://nexte.st/docs/ci-features/archiving/);
it does not make binaries portable across incompatible operating systems or
architectures. Keep benchmark provenance requirements separate from test-cache
reuse.

Suggested implementation ownership, using isolated worktrees:

- Runner agent: test inventory, nextest configuration and lifecycle grouping.
- Build-reuse agent: release-probe harness and its focused regression tests.
- CI agent: pinned tooling/build caching and workflow scheduling.
- Coordinator: shared configuration agreements, integration, exact-SHA gates
  and the existing live ledger. Remove each integrated clean worktree afterward;
  preserve unrelated paused product worktrees.

Start with a pinned runner version and a small measured concurrency trial,
compare complete selected-test inventories and warm/cold elapsed times against
Cargo, and repeat lifecycle checks on Linux and macOS. Review the integrated
change adversarially before adoption. Prefer build reuse and selective native
concurrency first; add remote sharding only if measured savings justify its
complexity. No new gate dashboard or Markdown-parser work is needed.
