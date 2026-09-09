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

The review did not rerun full workspace tests, remote CI, dependency audits,
benchmarks, or a comprehensive platform/security audit. It does not establish a
performance improvement or release readiness. Source paths and line numbers
below refer to the reviewed revision.

## Prioritized findings

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

## Architecture assessment and intentional choices

Dependency inversion and explicit ownership are strengths: core remains
independent of native effects, adapters accept injected capabilities, and
platform-specific unsafe bindings have narrow ADR-defined boundaries.

The principal weaknesses are module cohesion, adapter substitutability, and
duplicated mechanisms rather than basic Rust syntax. Executor-independent boxed
futures, explicit capability checks, and platform-specific implementations were
not treated as defects merely because they add code. File length alone was
also rejected as sufficient evidence of poor architecture.

Keep the concise root AGENTS instructions and the single live ledger. More
agent-specific dashboards or a larger documentation parser would not address
the identified problems.

## Suggested follow-up order

Prioritize findings 1, 2, 3, and the regression investigation for 6. Establish
wrapper-chain characterization before changing lifecycle abstractions. Findings
4 and 5 are bounded maintenance opportunities; undertake test-graph and command
module refactors separately after preserving their existing coverage.

These are recommendations, not newly authorized implementation tasks. Any
accepted work should be tracked in the existing implementation plan rather than
turning this historical review into another mutable status document.
