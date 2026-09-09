# Combined CLI feature history

This historical ledger preserves component evidence through
`9273ac426e813a3c78e0516efd07eccbef3eef60`, compacted from the implementation
plan. These internal checkpoints are not deliveries or final adversarial review
acceptance. Current phase, delivery state and gates belong only to the
[implementation plan](../implementation-plan.md).

## Pre-review component evidence

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
the actual CLI driver and complete command surface were still open at that checkpoint.
The shared setup integration passes 153 CLI unit tests with five helper ignores,
89 command tests and ten replay/frame tests. Workspace Clippy passes after a
test-only naming correction; no persistence or timing contract was weakened.

The following integration adds the contextual human-prompt bridge, flag-preserving
shared stdin and owned native controls. Combined native checks pass 17 bridge,
32 input (three private helper ignores) and 22 owner/control tests. Cancellation
waits for an accepted rule save before cancelling the real turn; queued input
and session identity survive. A freshly built release CLI implements the private
input helper and passes all four real-helper integration tests with all features
and all four without default features, with no skips. The two original prompt/input
trees, control tree and production-helper test tree are removed after integration;
commits remain recoverable. Bare/latest/exact interactive CLI composition and its
native-free final-output phase pass 33 focused assembled CLI tests. The driver
and command trees are also integrated and removed; these component results do
not close the full feature or its final local, review and remote gates.
The assembled CLI passes all 186 unit tests with five private-helper ignores,
90 command integration tests and ten replay tests. Workspace all-target,
all-feature warnings-denied Clippy, formatting and bounded documentation checks
pass. Exact `26b3151` then passes a fresh locked release build, all 90 release
CLI integration tests, and all four real-input-helper tests with all features
and all four without default features, with no skips.

The raw frontend now composes explicit native termios ownership, pinned Unicode
cursor editing, atomic paste, bounded row rendering and actual TTY column reads
with resize observation. Ctrl-D exits only when empty/idle, deletes forward in
a draft and does not exit during active work. Physical EOF is a separate abnormal
closure and never submits a draft. Received input retains its original modal
identity. Input readers join before restoration; native joins and the verified
restoration receipt precede final signal exit and native-free output settlement.
Focused integration passes 77 CLI tests, including three real-PTY cleanup tests,
16 native raw/dimensions tests and four shared display-width tests. The first
new exit regression incorrectly injected a signal through its tail helper;
normal acknowledged-tail coverage now checks the intended distinct exit results.
The assembled source passes all 231 CLI unit tests with five private-helper
ignores, 90 command integration tests and ten replay tests. Replacement workspace
all-target/all-feature warnings-denied Clippy, formatting and bounded docs checks
pass. All six component trees are integrated and removed; commits remain
recoverable. Exact `d218f47` then passes a fresh locked release build, all 90
release CLI tests, all four production input-helper tests and all 77 assembled
interactive tests using that helper. These are internal component checks, not
final feature gates.

The next integration streams canonical resumed text and recorded tool summaries
without inference or repeated effects. Snapshot traversal and output are bounded;
shutdown discards unsent history without delaying native cleanup. Headless `ask`
now reads a whole pipe or retained regular file through EOF, validates one bounded
prompt before setup, and preserves the first signal through exact input settlement.
Shared descriptor flags remain unchanged; a distinct helper handshake admits
regular files without widening interactive input. The sessions renderer's platform
import is scoped to its actual consumers, fixing WASI warnings-denied checking.
Combined checks pass 42 native input tests with three private-helper ignores,
254 CLI unit tests with five private-helper ignores, 96 command tests and ten
replay tests. Workspace warnings-denied Clippy, WASI CLI Clippy, formatting and
bounded docs checks pass. The initial platform-import correction also required
an explicit trait import in the sessions tests; the replacement CLI suite passes.
All three worker trees are integrated and removed; commits remain recoverable.
Exact `9273ac4` also passes a fresh locked release build, all 96 release command
tests and all seven production-input tests. All 254 CLI unit tests pass using
that exact release helper, with the same five private-helper ignores.
Typed historical cards and remaining CLI scenarios still need implementation;
the complete feature, final reviews and exact remote evidence gates remain open.

The integrated preparer passes all five real file mutations, actual reviewer
Allow/Ask/error and cancellation, read-grant/reset, canonical terminal identity,
and separate execution/review-bound regressions. All reference-host tests pass,
including configured denial, Ask/Auto/Yolo, real automatic review and actual
sandboxed terminal execution. The CLI's real create/resume turn path also passes
configured-mode regressions. These checks do not close the unfinished combined
feature or replace its final reviews and remote evidence.

## Interactive file undo integration

Native component `d1c6e9c` and CLI component `5a2bf2e` compose the actual host
tracker and owned worker scope with argumentless `/undo`. Admission retains
the exact runtime, and started inverses remain owned through response drops,
blocked output, cancellation, transitions and full host completion. Typed
uncertain outcomes retain their distinct manual-recovery meaning; no model
turn, transcript mutation or automatic retry is introduced.

The full write/edit/copy/rename/delete reversal exposed a real inverse bug:
undoing a later delete reconstructs the destination under a new file identity,
but rename's final verification compared the moved file to its historical
preimage identity. Both the composed chain and a direct regression reproduced
`Ambiguous` after the legitimate move. Final verification now checks the exact
admitted postimage identity while retaining original mode/content checks.
Tracked rewrite/delete reconstruction succeeds; an external same-content
replacement remains rejected. No identity check or resource bound was removed.

Worker checks pass 48 undo and 34 owner/control tests. Combined integration
passes 48 undo tests, 257 CLI unit tests with five private-helper ignores,
96 command tests, ten replay tests and workspace warnings-denied Clippy.
Private process dispatch uses the exact `9273ac4` release helper; this is not
fresh-release acceptance of the new `/undo` handler. A subsequent direct owner
test invocation initially supplied the CLI-test variable instead of the native
terminal-helper variable, so all 34 tests rejected setup before product work.
The replacement invocation selects the same release through
`MACHINE_GOD_TERMINAL_RELEASE_BINARY` and all 34 owner/control tests pass without
source changes; no fixture or production behavior is weakened.

Both original worker trees are integrated and removed, with commits retained.
These are internal component checks, not final adversarial or remote gates.

Exact integration `65886bf` subsequently passes a fresh locked release build,
all 96 production command tests and all seven production-input tests. All
257 CLI unit tests also pass using that exact release helper, with the same
five private-helper ignores. Its integration worktree is removed.

## Interactive clipboard integration

Components `0a801a1`, `28489e0` and `68c35e8` provide the explicit process
capability, incremental canonical reply selection and independent native owner
lane. Their focused checks pass 16 backend tests with one private helper ignore,
eight selector tests and 44 owner/control tests including ten clipboard scenarios.
Each component passes scoped Rust 1.94.1 warnings-denied Clippy, formatting and
bounded documentation checks. Their clean worker trees are integrated and removed.

The selector regression saves a final assistant reply, fails native metadata
finalization, reloads and recovers interrupted history without discarding that
valid saved answer. Owner tests retain the acceptance snapshot after a newer
reply is saved and exercise real fixture child cancellation/full-host joining.
No tests invoke the user's clipboard. Initial fixture corrections supplied the
required permission handler and monotonic native timestamps; production checks
were not relaxed. A backend invalid-image fixture also established macOS
`std::process` executable-format behavior; a missing interpreter now exercises
the distinct spawn-failure category without guessing from executable headers.

CLI composition captures startup authority, dispatches `/copy`, and preserves
typed receipts through blocked output and final flush acknowledgement. An
optional clipboard failure does not fail the conversation. The replacement
combined CLI suite passes 261 tests with five private-helper ignores after its
FIFO fixture was changed to the repository's existing POSIX `mkfifo` convention;
the attempted Rust fixture functions were unavailable on this target. The
complete feature's final local, fresh-review and exact remote gates remain open.

Combined native checks pass 34 clipboard tests with one private-helper ignore
and all 44 owner/control tests. Initial workspace Clippy rejected the startup
function at 104 lines; optional clipboard configuration was extracted into its
own presentation module without suppressing the lint. The replacement passes
all 261 CLI unit tests with five private-helper ignores, all 96 command tests,
ten replay tests, workspace all-target/all-feature warnings-denied Clippy and
WASI CLI Clippy. Formatting, bounded documentation and diff checks pass. These
checks use the preserved exact `9273ac4` release for private helper dispatch;
fresh release acceptance of this clipboard checkpoint remains separate.

Exact clipboard checkpoint `b2b0291` subsequently passes a fresh locked release
build, all 96 command tests, all 261 CLI unit tests with five private-helper
ignores, and all seven production-input tests using that exact release binary.

## Observed session picker composition

Components `8fa9349`, `7d02396`, `cab34f3`, `efd42d2` and `557660c` add observed-row
resume fencing, actual terminal rows/columns, owned cancellable catalog reads,
picker key/chunk identities and pinned resume aliases. Each clean worker tree
was integrated and removed with its commit retained. Component checks pass
21 resume tests, 12 terminal-size tests, seven reader tests, 19 catalog tests,
five store-filter tests, 107 interactive tests and 13 resume grammar/host tests.
Scoped warnings-denied Clippy, formatting and documentation checks pass with
temporary parent-owned module/caller wiring restored before worker commits.
The grammar checks also pass 96 actual command tests and 263 CLI unit tests
with five private-helper ignores. An original missing grammar check handle
was replaced with a focused run; no unobserved result was claimed.

Picker composition uses native catalog effects and observed identities without
allocating a startup writer until selection or Escape. Its first combined
focused run passes 16 picker tests after correcting a test-module import path.
Input tests additionally preserve a scope's query while rejecting stale chunk
remainders. A composed regression proves input received before menu flush cannot
gain selection authority after acknowledgement. Startup tests distinguish empty
Enter, Escape creating one writer, and Ctrl-D creating none; in-session tests
preserve drafts, queued work and current identity when opening is refused or
the picker is dismissed. Search/paging tests use the actual owned reader and
native store, retain cached rows on failed refresh, and preserve unfiltered
cursor boundaries.

The initial full CLI run passes 283 tests and fails one old help-text assertion
that still described the picker as unwired. Updating that behavior assertion
produces 284 passing unit tests with five private-helper ignores. All 96 command
tests pass. Initial scoped Clippy findings were style and function-size issues;
startup presentation ownership was extracted without lint suppression, and
replacement all-target/all-feature CLI Clippy passes. Formatting, bounded docs
and diff checks pass. Private helper dispatch uses exact `b2b0291`, not a claim
of fresh-release acceptance for the picker.

Read-only transition inspection found a remaining blocking-lock path through
observed preparation/adoption and candidate preference publication. The native
owned same-store-access integration addresses that complete path before picker
acceptance; initial preflight alone is insufficient. These component records
are not final full-feature review or remote gates.

Owned same-store preparation/adoption and preference publication are integrated
at `99544ee`. Workspace warnings-denied Clippy and a fresh locked release build
pass; that binary passes all 96 command tests. The combined CLI runtime check
rejects this checkpoint: four tests encounter fresh-startup `Resume(Busy)` under
default concurrency, and one picker fixture accepts the loading frame before
catalog rows arrive. Waiting for a nonzero acknowledged view revision corrects
the fixture, whose exact focused test then passes. Native scoped lock release
is repaired separately; the release checks do not override these failures.

The CLI retry correction retains the exact request identity for busy, changed
or missing picker rejection receipts until acknowledgement or native-free final
presentation. It does not suppress unrelated failures, failed settled turns or
indeterminate outcomes. A composed native-store regression changes a displayed
row's revision and verifies unchanged current identity, no provider calls,
retained rejection under blocked output, acknowledged receipt retirement, normal
successful shutdown, and explicit successful selection after a fresh display.
Its focused test passes all three receipt/retry scenarios; CLI all-target and
all-feature warnings-denied Clippy passes. Combined replacement checks remain
separate from this focused evidence.

Component `b4ee3d7`, integrated as `bd270e5`, replaces close-only session lock
release with scoped advisory unlock, including EINTR-only retry. A surviving
duplicated-descriptor regression fails against the old implementation and passes
after the fix; actual active holders still produce `Busy`. Final component checks
pass six store tests, 27 resume tests with one private-helper ignore, 284 CLI
tests with five private-helper ignores, and native warnings-denied Clippy.
Earlier repeated concurrent runs also pass with the parent's loading-frame
fixture correction, restored before the native-only component commit. The clean
native worktree was integrated and removed, retaining its commit.

Combined `bd270e5` passes 285 CLI unit tests with five private-helper ignores in
three default-concurrency runs, workspace all-target/all-feature warnings-denied
Clippy, formatting, bounded documentation and diff checks. Its fresh locked
release build completes in 9m16s. That exact binary passes all 96 command tests,
all seven production-input tests and all 285 CLI unit tests with five private-helper
ignores. The complete combined CLI feature still requires its remaining command
scenarios and final exact local, fresh-review and remote gates.

## Configured allowlist composition

Config component `c9c484a`, integrated as `136c808`, adds strict schema v6 with
lossless workspace identities, explicit-empty local shadowing and exact-byte-CAS
mutations. It preserves legacy input labels, duplicate rule order, model fields,
remove-last versus reset-empty behavior, and uncertain publication receipts.
The common config publication path also explicitly unlocks surviving duplicated
file descriptions; Busy remains nonblocking and only EINTR is retried.

Scoped checks pass 35 config-unit, 32 public config, 17 public store and five
store-unit tests, including duplicated-descriptor release and post-rename
ambiguity. Native all-target/all-feature warnings-denied Clippy, supported WASI
no-default-feature lint, formatting, bounded documentation and diff checks pass.
Initial old-current-schema assertions and test-only lint findings are corrected
without removing legacy coverage or relaxing the 64 KiB file limit. An additional
WASI all-feature probe fails in the existing target-gated Tokio reviewer path;
the supported no-default-feature surface passes and no reviewer code is changed.
Temporary exports were restored before the component commit; integration supplies
the parent-owned public exports. The clean component worktree is removed.

Native service component `03b512d`, integrated as `61de79c`, and lint follow-up
`9baa1c6`, integrated as `c3314b2`, pass 15 allowlist, nine permission-controller
and 50 owner/control tests plus all-feature native warnings-denied Clippy.
The temporary exports were restored and the clean service worktree removed.
CLI composition passes ten focused allowlist cases, including real saved/local
shadowing, malformed-rule presentation, pending receipts and blocked final output.
Combined execution passes 295 CLI unit tests with five private-helper ignores
and all 96 command tests; scoped CLI Clippy, formatting and diff checks pass.
These are component checks, not fresh allowlist release proof or full-feature
acceptance. The existing exact `bd270e5` release helper was used only by the
private terminal fixtures; Cargo built the updated command-test executable.

### All-feature WASI build remediation

The previously noted all-feature failure was reproduced as E0433 at
`permission_reviewer.rs:268`: the Tokio clock was feature-gated but its dependency
was also non-WebAssembly-gated. The clock type, implementation, public export and
native-only timer test now share that target guard. Portable injected reviewer
and clock contracts remain available; no Tokio dependency or fake timer is added
on WebAssembly. Native reviewer behavior and timer ownership are unchanged.

Rust 1.94.1 warnings-denied Clippy passes for the all-feature WASI library and
portable reviewer integration tests, and separately for the unchanged minimal
WASI library plus unsupported background/terminal tests. All 21 native reviewer
tests pass, including the real Tokio timer. The existing selected CI job now
contains the all-feature regression command; 17 classification tests and the
focused native-manifest contract test pass. Documentation-only routing remains
unchanged. This closes the reproduced compile mismatch, not the complete feature
or remote acceptance gates.

### Allowlist integration release evidence

The locked Rust 1.94.1 release build at `d55a45a` completes in 9m27s. Its exact
binary passes all 96 command tests and seven production-input tests. All 295 CLI
unit tests also pass with that binary as the production terminal helper, with
five private-helper ignores. Workspace all-target/all-feature warnings-denied
Clippy passes. The scoped WASI-fix review reports zero actionable findings and
its clean isolated checkout is removed. These close the allowlist checkpoint's
release evidence, not the combined CLI feature or its final review/remote gates.

## Workspace authority integration

Context-preparation component `e3571cb`, integrated as `a2a14ca`, forwards the
exact session/incarnation/turn/call tuple through effect-free preparation and the
native history/permission wrappers. Existing tools retain their default preflight.
All core tests and two doc tests, 61 tool-loop tests, eight history-wrapper and
four permission-wrapper tests pass. Combined CLI checks pass 295 unit tests with
five private-helper ignores and all 96 command tests, using Cargo's new command
executable and the exact `d55a45a` terminal helper. Workspace warnings-denied
Clippy, core all-target WASI lint, formatting and documentation checks pass.
The clean preparation worktree is removed after integration.

Authority component `79f2553`, integrated as `a18661b`, passes 17 descriptor/scope
tests, formatting and documentation checks. Its standalone lint run has exactly
two integration-only unused findings: the private install method and its manager
token, pending the real operation-service caller. They are not suppressed and
that run is not called green. Public module wiring remains part of integration.
The component's clean worktree is removed.

Configuration component `0c0a5c3`, integrated as `df7bdb5`, passes 44 config-unit,
19 store-unit, 32 config-integration and 18 store-integration tests. Latest-state
directory edits retain before/intended sets through uncertain publication and
check saved-plus-staged-launch capacity under the writer lock. Existing model
and permission CAS routes remain unchanged. Native all-target/all-feature Clippy,
minimal and all-feature WASI lint, formatting and bounded documentation checks
pass. Temporary exports were restored before commit and its clean worktree is
removed. Actual service, turn binding and tool/CLI integration remain required.

Turn component `b5b976d`, integrated as `d0d3524`, passes six scope and three
control tests plus 37 conversation, 39 runtime and seven permission-context
regressions. Its clean tree is removed. Service component `46d5bc8`, integrated
as `ac5536a`, closes the actual install/control callers and passes 78 combined
workspace tests, native all-target/all-feature warnings-denied Clippy and bounded
docs/format checks. The focused directory regression now proves a previously
missing source can first resolve through a symlink, become active, and retain
that canonical identity after its source is retargeted. The service tree remains
active for the separately identified latest-config alias-capacity refinement.
The WASI remediation is rechecked on the integrated branch: both target feature
variants, all 21 native reviewer tests and all 29 CI/manifest tests pass.

Owned startup `b65c1ae` passes six new tests within the 84 workspace-related
checks. Scoped `read_file` passes four actual engine/descriptor tests, including
root-qualified permission capability and content, same-name roots, retained
access across rename, next-turn scope replacement and expired/foreign context
rejection. Nineteen public read regressions and 14 permission-target tests pass;
the new ordinary-target regression checks the exact contextual preparation tuple.
Replacement native all-target/all-feature Clippy and all-feature WASI lint pass
after fixing wildcard imports, two unnecessary fixture clones and a portable
unused-self warning. These remain component checks, not final feature gates.
