# Combined CLI feature history

This historical ledger preserves component evidence compacted from the
implementation plan. These internal checkpoints are not deliveries or final
adversarial review acceptance. Current phase, delivery state and gates belong only to the
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

## Integration snapshot retained from the agent-readiness base

The following narrative was retained from `3a0df99098f62e88aa6d2f29c826797a7320abab`
during ledger compaction. Statements about remaining work describe successive
historical checkpoints, not current assignments. Consult the implementation
plan for the consolidated boundary; these records do not close feature gates.

### Recorded component inventory and checks

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

Remaining work includes workspace handlers and actual multi-root tools,
migration/recovery/doctor cleanup, recording, and remaining
policy/workspace scenarios. Bare/latest/exact/picker startup, in-session picker
selection, shared cache/human prompts and owned raw input are composed.
Clear/new/reset allocate fresh IDs;
clear/new carry terminals forward, while reset and live resume stop/forget.
Started transitions and uncertain outcomes stay owned through settlement.
The complete combined feature and all final gates remain open.

Detailed component checks, including the first-run PTY failures and replacement
results, are retained in the [combined CLI history](m03-cli-full-review-01.md).
They are historical evidence, not separate deliveries or final product reviews.

Interactive composition includes owned raw terminal editing, atomic paste,
Unicode row rendering, native resize observation, contextual human prompts and
native-free final output. Canonical resumed text is streamed without inference
or repeated effects. Headless `ask` accepts whole pipes and retained regular files
through EOF with bounded validation and first-signal-preserving settlement.
Checkpoint `9273ac4` passes 42 native input tests, 254 CLI unit tests, 96 command
tests and ten replay tests, with three native/five CLI private-helper ignores.
Workspace and WASI CLI Clippy, formatting and bounded docs checks pass. Its fresh
locked release passes all 96 command tests and all seven production-input tests;
the 254 CLI unit tests also pass using that exact release helper.
Checkpoint `65886bf` composes `/undo` with the host's real file tracker and
owned worker scope, preserving typed receipts through active turns, blocked
output, transitions and shutdown. A tracked destination reconstruction no
longer makes a valid inverse rename report uncertainty; external replacements
remain rejected. Combined checks pass 48 undo and 34 owner/control tests,
257 CLI unit tests with five private-helper ignores, 96 command tests and ten
replay tests, plus workspace warnings-denied Clippy. Its fresh locked release
passes all 96 command tests and all seven production-input tests; all 257 CLI
unit tests also pass using that exact release helper.
The next integration composes `/copy` from an acceptance-time canonical snapshot
through independent native selection/process ownership and acknowledged CLI
receipts. Clipboard work neither blocks generation nor retargets after a session
change; shutdown settles it before native-free final output. Three component
trees are integrated and removed with their commits retained. Their focused
checks pass. Combined checks pass 34 clipboard tests with one private-helper
ignore, all 44 owner/control tests, 261 CLI unit tests with five private-helper
ignores, 96 command tests and ten replay tests. Replacement workspace Clippy,
WASI CLI Clippy, formatting and bounded docs checks pass. The fresh checkpoint
release passes all 96 command tests, all 261 CLI unit tests with five private-helper
ignores and all seven production-input tests. These are checkpoint checks, not
final feature acceptance.
Picker composition now includes startup without a provisional writer, pinned
resume aliases and raw keys, cached/paged loaded-summary search, exact observed
selection and output-acknowledged input identities. Combined CLI checks pass
284 unit tests with five private-helper ignores, all 96 command tests, scoped
warnings-denied Clippy, formatting and bounded documentation checks. Owned
nonblocking preparation/adoption and candidate preference publication are
integrated at `99544ee`. That checkpoint's concurrent fresh-startup contention is
fixed by scoped advisory unlock at `bd270e5`; the independent loading-frame fixture
and retryable picker receipts are also corrected. Replacement CLI checks pass
285 tests with five private-helper ignores across three default-concurrency runs,
plus workspace warnings-denied Clippy, formatting and bounded docs checks. The
fresh locked `bd270e5` release passes all 96 command tests, seven production-input
tests and all 285 CLI unit tests with five private-helper ignores. These checks
include blocked and acknowledged stale-row rejection receipts and explicit
refreshed-selection success; they are not full-feature acceptance.
Allowlist configuration/storage is integrated at `136c808`: strict schema v6,
workspace-local shadowing, bounded settings CAS, and scoped config-lock release.
Component checks pass 35 config-unit, 32 config-integration, 17 store-integration
and five store-unit tests, native warnings-denied Clippy and supported WASI lint.
Native grammar/runtime service is integrated at `61de79c` with lint corrections
at `c3314b2`. Its checks pass 15 allowlist, nine permission-controller and 50
owner/control tests plus warnings-denied Clippy. CLI routing and bounded rendering
pass ten focused allowlist tests, 295 unit tests with five private-helper ignores,
all 96 command tests and scoped warnings-denied Clippy. Both completed component
worktrees are removed. The fresh locked `d55a45a` release passes all 96 command
tests, seven production-input tests and all 295 CLI unit tests with five
private-helper ignores. Workspace configuration, retained multi-root authority
and context-aware tool preparation are integrated; actual
tool/approval/sandbox/search integration and CLI handlers remain required.
Contextual tool preparation is integrated at `a2a14ca`: core and wrapper tests,
61 tool-loop tests, 295 CLI unit tests with five private-helper ignores, all 96
command tests, workspace warnings-denied Clippy and core WASI lint pass. The clean
preparation worktree is removed. Descriptor scope component `a18661b` passes 17
focused tests; its public wiring and real install caller remain integration work.
Schema-v7 workspace persistence is integrated at `df7bdb5`, with 113 scoped tests,
native warnings-denied Clippy and both WASI feature checks passing. The clean
configuration and authority worktrees are removed. Exact-turn scope ownership is
integrated at `d0d3524`, with nine new and 83 existing tests passing; its clean
worktree is removed. Operation service `ac5536a` passes 78 combined workspace
tests and native all-target/all-feature Clippy against that actual turn owner.
Provisional directory sources now resolve once and remain pinned after retargeting.
Owned startup `b65c1ae` and explicit scoped `read_file` composition pass 84
workspace-related tests, four real scoped-read tests, 19 public read tests and 14
permission-target tests. Native all-target/all-feature Clippy and all-feature
WASI lint pass after the portable metadata-description fix. Top-level workspace
management is integrated at `fdb51de`; latest-state alias provenance at `0dd1767`
and first-use missing-config-parent handling at `68bd09c` correct real native
incompatibilities. Missing-parent component checks pass 101 focused/regression
tests and native all-target/all-feature Clippy. The combined CLI's twelve
workspace-related command tests now pass, including real listing, first add,
no-op clear and persistence. Scoped read/permission evidence passes six unit and
fourteen public target tests. Checkpoint `7026b88` passes all 99 command tests
through its fresh locked release binary, plus the exact CI release-smoke script
and 18 CI classification tests. That smoke now exercises real workspace
persistence and the implemented session JSON contract. The integrated CLI and
configuration worktrees are cleanly removed. Production host routing for the
complete tool set is still required. Mutation adapter `797bf6f` integrates 381
component regression tests; its shared projection also passes eight combined
root tests after permission-preparer wiring. Its clean worktree is removed.
Path-tool, enumeration and grep adapters are integrated at `49df27d`, `7586d38`
and `03a6bcf`; their clean worktrees are removed. Actual prepared-root host
composition now shares the exact workspace context with these tools, reads,
all five mutations, permission preparation, undo and history. State-directory
aliases compare retained object identities rather than spelling. Qualified
history paths and outer mutation approval stamps survive the real wrapper chain.
Vision (`5189a97`), scoped sandbox roots (`0697c59`) and Linux semantic routing
(`e9209e0`) are integrated and their clean component worktrees removed. The host
also connects actual vision and sandbox scopes, and native interactive startup/
transitions attach the selected workspace automatically. All 54 composed-host
tests, eight interactive-owner regressions, nine mutation tests, eight history-wrapper
regressions, native all-target/
all-feature Clippy and bounded docs checks pass. The sandbox component's first
broad PTY run had ten inventory-startup timeouts; both its serial diagnostic and
same-binary default-concurrency retry without competing builds pass 685 tests
with two existing helper ignores. No deadline or concurrency gate was weakened.
The macOS semantic reader/scan, terminal cwd routing and actual launch/slash
selection remain in integration. These checks are not full-feature acceptance.
The all-feature WASI build failure is corrected by matching the Tokio reviewer
clock's implementation, export and test guards to the non-WebAssembly dependency.
The portable reviewer remains compiled. Both all-feature reviewer and minimal
unsupported-tool WASI lint pass, as do 21 native reviewer tests and 18 focused
CI/manifest checks. The existing selected CI job now guards both WASI builds.
Typed historical cards and the remaining command/session scenarios still need
implementation; complete local, review and exact remote feature gates stay open.

## Combined-candidate local regression: `56d5de41`

The candidate integrated recording, session maintenance, configured permission
reporting, saved-rule interaction and positive startup-resume scenarios. Its
Rust 1.94.1 formatting, workspace warnings-denied Clippy, fresh locked release
build, bounded documentation checks, compatibility/Unicode drift, dependency
checks and repository Python suite passed (255 passed, 14 existing platform
skips). Exact FreeBSD/WASI compilation checks also passed. The fresh macOS
release passed help/version and nine permission/maintenance command scenarios.

The full serial macOS workspace run passed 370 CLI unit tests with six fixture
helpers ignored, 108 command tests, ten replay tests, core suites and 2,163
native unit tests with twelve fixture helpers ignored. It subsequently failed
`composed_semantic_search_preserves_catalog_and_returns_fixed_unsupported_result`
in the native reference-host integration suite: the newly supported macOS
scanner correctly returned successful searched-file evidence, while the old
test expected a fixed unsupported error. The run stopped there; later workspace
suites and doc tests were not accepted, and this candidate was not reviewed or
delivered.

The correction runs the existing retained-root/exact-result persistence scenario
on both Linux and macOS, and converts the obsolete macOS-only unsupported
scenario into a cross-platform exact empty-match success scenario. Catalog,
permission, completion and durable-result assertions remain; successful scanning
is checked through exact counters rather than an unsupported stub. No production
behavior, deadlines or test-runner policy changed.

## Combined-candidate review: `adee6877`

The replacement candidate passed the complete Rust 1.94.1 local gate. macOS
used serial runtime scheduling and the fresh release helper; Linux used its
default concurrency and a separately built release helper. Both workspace and
explicit doc tests passed. Repository Python checks passed 255 tests with 14
existing platform skips; dependency policy, vulnerability audit, pinned drift,
bounded documentation and FreeBSD/WASI compilation checks passed.

Three fresh independent read-only agents reviewed the complete delta against
merge-base `4659f0011e5add304bd24bd5cfd43e244edceac7`:

- `cli_full_review_api_01`: one P3 documentation finding. `docs/resume-cli.md`
  incorrectly rejected interactive `--record` and claimed only explicit-ID,
  one-prompt resume existed. No additional runtime/API defect was established.
- `cli_full_review_lifecycle_01`: zero actionable findings across workspace,
  permission, persistence, resume, input, recording and cleanup ownership.
- `cli_full_review_resources_01`: zero actionable findings across JSON bounds,
  scanning, undo, queues, catalogs, recording, CI and test topology.

These were local source/caller/test reviews, not independent runtime reruns or
remote acceptance. The documentation correction states the existing accepted
interactive recording grammar and explicitly scopes one-prompt behavior; it
changes no product code or tests. The candidate was not pushed or delivered.

## Combined-candidate runtime regression: `7e83a77e`

This candidate also corrected stale overview statements about configured
permissions and the explicit macOS sandbox. Its pinned-toolchain formatting,
Clippy, fresh release builds, standalone fixture checks, dependency checks,
documentation/drift, FreeBSD/WASI and Apple ABI checks passed. The complete
Python suite passed 255 tests with 14 existing platform skips, including the
fresh optimized cleanup probe.

The macOS full runtime run passed all CLI, core, integration and doc suites,
but native unit tests reported 2,159 passes, four failures and twelve fixture
helper ignores. The failures were the HTTP/TCP probe fixture and three PTY
helper startup/reaping fixtures. Linux's initial container lacked an init
reaper, invalidating its terminal cleanup evidence; a replacement container
with an init reaper passed those focused checks. Its subsequent full run still
failed the supervisor exact-owner signal fixture and two terminal deadline
fixtures. All seven macOS/Linux timing-sensitive failures passed individually
with unchanged binaries and deadlines after the competing platform run ended.
Observed host load was approximately 41 on sixteen logical CPUs, with unrelated
editor extension hosts consuming substantial CPU. Scheduling interference is
plausible, not proven; isolated retries are not replacement full-gate acceptance.

The initial Linux run additionally exposed a deterministic fixture ownership
problem: the catalog test dropped a raw locked file and assumed immediate
unlock. Retaining `File::try_clone()` reproduces the post-drop `Busy` failure
without a fork race or a timing assumption. Explicit unlock releases the shared
open-file-description lock even while that duplicate remains alive, matching
the production store guard's existing contract. The same assumption occurs in
three owned-resume fixtures. Their correction preserves the initial contention
and subsequent successful-operation assertions without retries or deadline
changes. No product review or remote delivery accepted this candidate.
Correction `e5de4e8f` passed seven catalog-reader and eight owned-resume tests,
with one existing child-fixture entrypoint ignored, plus formatting and native
all-target/all-feature warnings-denied Clippy. Its clean integrated worktree
was removed.

## Combined-candidate runtime regression: `6f6e6660`

Pinned formatting, workspace all-target/all-feature Clippy, fresh release
builds, repository policy and dependency checks, FreeBSD/WASI compilation and
Apple ABI checks passed. The Python suite passed 255 tests with fourteen
existing platform skips. Linux and macOS runtime windows were separated.

Linux passed 2,160 native unit tests with eleven existing helper ignores and
113 fresh-release CLI command tests. Its terminal integration target passed 97
tests and failed `linux_system_timeout_kills_a_term_ignoring_shell_before_publication`:
the expected `timeout.pid` did not exist. The public 100 ms budget starts before
admission and spawn, so a legitimate timeout need not produce a ready child.
The other workspace targets and explicit doc tests passed. The initialized
container had no remaining child processes after the run.

The macOS workspace command exited 101 and identified the native library test
target as failed. Later integration targets and workspace doc tests passed.
The retained tool output truncated the native failure details; it does not
establish a failure count or cause. This run is rejected, not accepted on the
basis of earlier isolated successes. The explicit doc-test command following
the failed workspace command was not reached.

Test-only correction `ed9eebda` preserves the exact public 100 ms budget and
bounded timeout-result assertions, and separately requires newline-complete
post-trap readiness before closing the real executor's existing timeout cause.
That scenario checks a live child and group before timeout, their absence at
publication, and bounded activity-slot release. It uses the existing owned
future for cleanup on assertion failure; no production hooks or deadline
changes are introduced. The component commit is not full-feature acceptance.

The correction passed pinned formatting, workspace all-target/all-feature
Clippy, a fresh release build, both focused scenarios, all fifty terminal unit
tests and all 98 terminal integration tests on Linux. No child processes
remained in the initialized container. The clean integrated component worktree
was removed. The unchanged `6f6e6660` macOS native executable subsequently
passed 2,163 tests with twelve existing helper ignores in 462.22 seconds, using
serial scheduling and the original release helper. Quiet diagnostic output
retained the complete result. This does not recover the missing earlier
failure details or establish their cause, and does not replace an exact
full-candidate gate.

## Combined-candidate local review and remote rejection: `a9321f6c`

The complete exact Rust 1.94.1 local gate passed. Linux retained default test
concurrency and passed 2,161 native unit tests with eleven helper ignores;
macOS used serial scheduling and passed 2,163 native unit tests with twelve
helper ignores. Both complete workspace runs and explicit doc tests passed
with freshly built release helpers. Repository Python checks passed 255 tests
with fourteen existing platform skips. Formatting, warnings-denied Clippy,
standalone-fixture checks, dependency policy/audit, pinned drift, documentation,
FreeBSD/WASI and Apple ABI checks passed.

Three fresh independent agents reviewed the complete delta against
`4659f0011e5add304bd24bd5cfd43e244edceac7`, each with zero actionable findings:
`cli_full_review_api_02`, `cli_full_review_lifecycle_02`, and
`cli_full_review_resources_02`. These were static local source/caller/test
reviews, not independent runtime reruns. The API track also checked dispatch
for all twenty primary commands across the five required CLI categories,
preserving documented differences and later-milestone deferrals. All clean
released review and verification worktrees were removed.

The unchanged candidate was pushed. Benchmark run `34403352536` passed and
retained both unexpired exact-SHA artifacts. CI run `34403352711` rejected the
candidate: quality and both Linux architecture jobs each passed 2,160 native
unit tests, ignored eleven existing helpers and failed only
`terminal::tests::linux_ready_term_ignoring_shell_is_reaped_before_timeout_publication`.
At `terminal.rs:7602`, the real executor returned `terminal_wait_failed` before
the fixture could inspect its outcome. Later suites were not reached in those
jobs. Both Apple architecture jobs passed. The aggregate CI gate failed;
`main` was not advanced.

Read-only diagnosis found process-wide child-subreaper setters in several
native unit fixture constructors, with no general reaper that consumes
unrelated adopted statuses. The new fixture signals readiness before forking
its transient sleep child. Killing the group while that child exists can leave
an adopted zombie owned by the test process, preserving group visibility after
the direct shell is reaped. A controlled isolated reproduction enabled a
subreaper only in its diagnostic test process and forced the sleep child to
exist before readiness. The real executor returned `terminal_wait_failed` in
0.53 seconds. `/proc` showed sleep PID 1051 as a zombie adopted by unit process
1044, still in shell group 1048. Reaping only that exact owned child returned
`SIGKILL`; the group then disappeared. This confirms the mechanism locally,
not recovered process-table evidence from the failed hosted jobs.

The test-only correction uses a private mode-0600 FIFO held read/write by the
shell and a builtin `read`, avoiding transient descendants. All readiness,
timeout-result, process/group disappearance, activity-slot and bounded-cleanup
assertions remain unchanged. The same corrected test executable passed normally
and through a scratch subreaper exec wrapper, each in 0.28 seconds. No diagnostic
subreaper setter, status inspection or manual reap remains in the source patch.
No production deadline, cleanup requirement or test concurrency is relaxed.
Correction `3fcdf29b` passed all fifty selected terminal unit tests and all 98
public terminal integration tests at default Linux concurrency, exact formatting,
workspace all-target/all-feature warnings-denied Clippy and a fresh locked release
build. Production binary SHA-256 remained
`ca299181c7593f11b5006e0ac4aeefad5ecabb7d5196bdd4314f0ac551d1937d`.
The clean component was fast-forward integrated. Complete replacement acceptance
remains a separate gate.

## Replacement Linux rejection: `889b8e65`

Exact pinned formatting, all-target/all-feature Clippy and the fresh locked
release build passed. The default-concurrency workspace run completed under
`--no-fail-fast`: 4,531 tests passed, four failed and seventeen existing helpers
were ignored. Explicit workspace doc tests passed all three cases. The corrected
FIFO timeout case passed. No test/helper processes or zombies remained afterward.

Native unit failures were `production_supervisor_signals_only_the_exact_live_owner`
(`Unavailable`), `captured_exec_accepts_exact_maximum_command_and_rejects_one_over`
(`Process`), and `frozen_shell_capture_supports_profiles_explicit_start_and_no_ambient_shell`
(`terminal_authority_unavailable`). One unchanged isolated run of each passed the
first two and reproduced the third. The newly initialized container executes as
UID 1000 but has no corresponding passwd/NSS record; current-user shell capture
requires that record. A container-only task account with UID/GID 1000, a real
`/bin/bash` login shell and private home was created after verifying those
records were absent. No `HOME` override or source change was made. The unchanged
host-authority test then passed in 0.02 seconds, confirming the environment cause.
No cause is established for the first two full-run failures from isolated success
alone.

The HTTP catalog test `request_timeout_is_resource_limited_and_tears_down_the_connection`
also failed: its strict stalled-body server observed peer EOF before a complete
request head, panicked and disconnected the completion channel. The public
25 ms attempt timeout includes admission and response-head work, so readiness
before expiry is not guaranteed. An unchanged isolated run passed. A narrow
test-fixture correction followed; production deadlines and mandatory-ready
cancellation/drop coverage remain unchanged. The complete run remains rejected;
there was no macOS replacement run, review restart, push or main advancement.

Correction `ad346115` adds a private deadline-bound timeout server with owned
worker joining. It permits no connection only after an explicit completion
notification and a subsequent empty accept probe; connected outcomes require
observed peer closure before headers, during the partial response, or after its
body prefix. Malformed input, unexpected data and fixture deadline expiry remain
errors. Accepted sockets explicitly use blocking I/O with the remaining absolute
fixture deadline. The existing bounded parser is reused through a generic `Read`
parameter without logic changes; the strict stalled-body fixture is unchanged.

A controlled peer that connected and closed before headers reproduced the old
strict fixture's exact `UnexpectedEof` failure. That diagnostic is not committed.
Four deterministic replacement-fixture regressions cover no connection, empty
and partial headers, acknowledged body-prefix closure and malformed complete
headers. All four and the complete twenty-test HTTP catalog suite passed on the
final source at default Linux concurrency, alongside pinned formatting,
workspace all-target/all-feature warnings-denied Clippy and a fresh locked
release build. The clean test-only component was fast-forward integrated;
complete replacement acceptance remains pending.

## Replacement Linux ordering failure: `145124f2`

The exact full default-concurrency Linux workspace run passed 4,538 tests and
failed one, with seventeen existing helper ignores. Native unit tests passed
2,161 cases, including both earlier process failures, the account-shell case
and the FIFO correction. All twenty catalog HTTP cases passed. Explicit doc
tests passed all three cases; pinned formatting, full Clippy and a fresh locked
release build passed. Static repository policy checks also passed. No test or
helper processes or zombies remained.

The sole failure was
`linux_output_limit_wins_when_deadline_expires_during_term_ignoring_cleanup`
at `tests/terminal.rs:6302`: its 225 ms request returned `timed_out` instead of
the asserted `output_limit`. The complete terminal integration target passed
97 other cases. No isolated retry or cause is asserted for this failure.
Later targets ran under `--no-fail-fast`; this is a rejected complete run, not
partial acceptance. macOS runtime, Python release-probe and portability gates
were not started for this candidate, and there was no review restart, push or
main advancement.

The retained unchanged terminal executable subsequently passed the exact case
once in 0.31 seconds. Code inspection found no prerequisite proving native
overflow was observed before timeout closure; final byte counts alone cannot
establish that ordering. A controlled private-entrypoint diagnostic retained
the same 225 ms request limit and overflow command, but supplied an explicitly
expired admission timestamp. Production returned `timed_out`, duration 225 ms,
zero stdout/stderr bytes, null exit/signal and no occupied slot. Those checks
passed; the old unconditional `output_limit` assertion then failed. This proves
the fixture's universal expectation invalid, not the exact scheduling cause of
the earlier full-run failure. The diagnostic is not part of the final patch.

Paired test-only correction `6c686453` retains the public 225 ms budget, original
command, two-second cleanup bound and strict status/count consistency for either
legitimate timeout-first or overflow-first outcome. Its separate mandatory
real-executor case uses a private FIFO to gate a builtin-only producer until the
TERM-ignoring shell, PID/group and activity slot are proven ready. Native pipe
readers must establish actual overflow before the test closes timeout; the real
result must preserve that cause, bounded overflow counts, process/group absence
and activity-slot release. No test code fabricates an overflow claim. Existing
deadline-arbitration fixtures remain unchanged; the new case proves cause-close
ordering, not literal wall-clock expiry during TERM grace.

The final source passed the real-overflow case normally and under an isolated
subreaper, all 51 selected terminal unit tests and all 98 terminal integration
tests, at default Linux concurrency. Exact formatting, full warnings-denied
Clippy and a fresh locked release build passed. Production code and binary
SHA-256 were unchanged. Complete replacement acceptance remains pending.

## Replacement macOS recording failure: `1b804b0b`

The exact candidate passed pinned formatting, full warnings-denied Clippy,
fresh locked release builds, policy/Python checks and FreeBSD/WASI/Apple ABI
checks. The full Linux workspace passed 4,540 tests with seventeen existing
helper ignores; explicit doctests passed all three cases. The macOS full
workspace passed 4,535 tests and failed one, with eighteen existing helper
ignores; its explicit doctests also passed all three cases. Nested helper
summaries are excluded from these counts. Both runtime runs completed without
fail-fast, on separate host runtime lanes.

The sole macOS failure was
`sigint_preserves_exit_status_records_signal_and_closes_tape_and_input_helpers`
in the CLI recording scenarios. Exit 130, exact termios restoration, PTY helper
closure, tape parsing and the recorded SIGINT assertion passed; the tape lacked
the asserted bracketed-paste disable bytes. All other CLI unit cases, 108 command
subprocess tests, ten replay tests and 2,163 native unit cases passed. The
fixture discarded physical PTY output and removed its tape, so the original
failure does not distinguish absent terminal output from absent recording.

Read-only inspection identified an eager interactive signal timer carried
across native/input joins as a candidate cause, not a proven explanation of the
original run. Controlled regression work follows without weakening the existing
recording assertion. No replacement product reviews, push or main advancement
followed this rejected candidate. The completed read-only diagnosis worktree
was removed; active correction work remains isolated.

Two controlled regressions then failed against unchanged production: active
driver shutdown and actual startup-picker interruption each joined the real
host/input owners, advanced virtual time by 200 ms before final presentation,
and supplied responsive output acknowledgements. Both incorrectly reported
stalled output. This establishes the carried-over timer defect, not the exact
cause of the historical PTY failure. The correction moves ownership of the
unchanged, non-renewable 100 ms presentation budget to the post-cleanup tail;
the subprocess assertion retains mandatory terminal-reset recording and adds
bounded physical-versus-recorded output diagnostics.

Preparation of integrated candidate `9e92afaa` stopped at the same
warnings-denied Clippy finding on Linux and macOS: the new startup signal test
used an underscore-prefixed fixture field. Renaming that field and its two uses
preserves the test and production behavior without suppressing the lint.

## Replacement background-signal failure: `e3fac2d4`

The corrected candidate passed exact pinned formatting, full workspace
warnings-denied Clippy, all test-target compilation and fresh locked release
builds on Linux and macOS. All four new timing regressions and eight real
recording/resume scenarios passed on both platforms, including the original
SIGINT case with mandatory physical and recorded terminal reset. Python ran
269 tests (255 passed, fourteen existing platform skips), including the genuine
fresh optimized cleanup probe; documentation/drift, dependency policy/audit,
FreeBSD/WASI, both Apple ABI/binding checks and standalone support checks passed.
The released implementation and policy worktrees were removed.

The complete default-concurrency Linux workspace run passed 4,543 tests and
failed one, with seventeen existing helper ignores. Explicit doctests passed
all three cases. The sole failure was
`production_supervisor_signals_only_the_exact_live_owner` at
`background_supervisor.rs:3768`: same-owner signal delivery returned
`Unavailable`. Native unit results were 2,161 passed, one failed and eleven
ignored. No-fail-fast ran the remaining targets; no test/helper processes or
zombies remained. An authorized single run of the unchanged exact failing case
then passed in 0.09 seconds. That isolated result neither accepts the full gate
nor establishes why it failed. Full macOS runtime and replacement reviews were
not started; there was no push or main advancement.

Read-only inspection identified a discriminating lifecycle hypothesis: a live
child admitted to Linux ancestry traversal can be reaped before its own queued
descendant scan, producing the fixed process error mapped to `Unavailable`.
The owner fixture continuously spawns and reaps one-second sleep children.
Skipping such an incompletely inspected node could conceal descendants, so
controlled diagnosis must preserve the fail-closed traversal and its original
budget. No cause or correction is inferred solely from the isolated pass.

A controlled diagnostic used the existing snapshot-capture callback, without a
new production hook, to reap an admitted child while its root remained alive.
Traversal returned the expected process error after 4.87 ms and the original
budget still passed preflight. This proves an incomplete-traversal path, not the
historical full run's exact cause; existing expired-budget tests cover the
separate deadline path.

Test-only component `dd16b8c8` retains that controlled case as a fail-closed
regression. The owner-routing fixture now uses builtin output/read with owned
piped input and requires its exact owner-bound readiness output before signaling.
Different-owner rejection, same-owner delivery, exit status and reaped-owner
rejection remain mandatory. Dedicated descendant tests and production behavior
are unchanged. Pinned formatting, full warnings-denied Clippy, fresh locked
release and both exact changed cases passed, along with 151 background-process
tests (three existing helper ignores) and all fifty supervisor tests at default
Linux concurrency. The production release hash was unchanged; complete
replacement acceptance remains open.
