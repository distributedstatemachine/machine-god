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
