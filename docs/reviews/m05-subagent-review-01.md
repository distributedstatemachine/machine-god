# Milestone 05 `subagent` review history

This is the compact historical review record for the contract in
[`../subagent.md`](../subagent.md). Current phase, delivery, workflow, and
next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`ba52dbfeb7d05b67ee314934905abee7e0c35ffc`, tree
`b31b79e92acebef4c18cff921038f529a3c88c68`. The candidate passed the complete
exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after strict create-shape decoding, NUL and byte bounds, fixed error taxonomy, structural context identities, canonical preparation, typed execution, pinned behavior, host composition, testkit, and durable-contract review. |
| Lifecycle, platform, and effects | Green | Zero findings after inert-before-poll authority and fixture review, effect-free preparation, same-poll cancellation precedence, wake/drop and permit release, fail-fast admission, inert native defaults, injected constructors, macOS smoke paths, FreeBSD, and WASI review. |
| Performance and resources | Green | Zero findings after raw and serialized JSON bounds, escaping amplification, iterative rejected-value destruction, output bounds, allocation passes, global/per-parent contention, mutex/static state, cancellation/wakers, and testkit capacity review. |

## Rejected candidates and remediation

| Candidate | Decisive findings | Replacement |
| --- | --- | --- |
| `0ffa79f` | The scripted authority recorded and consumed before first poll; input resource failures used the execution error class; security omitted structural context identities; and preparation documentation claimed a typed request too early. | `5a03a16` moved authority work into the future, separated input and execution resource failures, and corrected both durable contracts. |
| `5a03a16` | The core API omitted structural context identities, while core and native review fixtures still mutated state before their returned futures were polled. | `c6806e1` documented the identities as non-authoritative and made every fixture future inert with direct unpolled regressions. |
| `c6806e1` | The testkit overview overpromised string error codes for the kind-only subagent boundary, and the input contract omitted its NUL exclusion. | `ba52dbf` documented the exact `Failed`/`ResourceLimit` mappings and the no-NUL input rule. |

Every accepted finding rejected its candidate. The complete replacement local
gate and three entirely fresh review tracks were rerun for each replacement.
All isolated gate and review worktrees were verified clean, removed, and
pruned after their iteration.

## Remote delivery evidence

The exact accepted candidate passed feature CI `33440115994` and Benchmark
evidence `33440115963`, then exact-main CI `33440894118` and Benchmark evidence
`33440894154`. Both benchmark runs retained unexpired upstream and bootstrap
artifacts whose names identify the exact accepted behavior SHA.

This review seal makes no package, release, or comparative-performance claim.
The implementation plan remains the sole live source for delivery and workflow
gates.

## Complete managed-agent feature: first reviewed candidate

Candidate `437a19b06e862b973716b6f344ef052a03185fc4` was reviewed against
`6d6364c6ee3b1540505749b10b2527df39ddf5be` after its complete local gate.
This is a rejected product candidate, not a delivery or performance claim.

Exact Rust 1.94.1 Linux and macOS formatting, warnings-denied workspace Clippy,
fresh locked release builds, workspace and doc tests passed. Linux ran with
default concurrency under unprivileged UID 10001 and a reaping init; macOS
process tests ran serially. Focused production-helper input, managed/MCP PTY and
agent-driver tests passed on both platforms. Repository Python tests (269),
documentation policy, upstream/Unicode drift, dependency deny/audit, FreeBSD/WASI
checks and both Apple ABI compilations plus the native arm64 ABI probe passed.

Logs remain under `/tmp/mg-managed-implementation.V0ZGg1/`, notably
`managed-full-runtime-linux-437a19b0-r2.log`,
`managed-full-runtime-macos-437a19b0.log`, and
`managed-focused-macos-437a19b0.log`. The initial Linux invocation failed before
tests because the container lacked `rg`; the corrected diagnostic used `grep`.
The initial audit invocation omitted its `audit` subcommand and was corrected.
Both failed invocations remain retained; neither is a product-test failure.

The preceding macOS release-PTY failure on `e00da357` was traced to XNU setting
the shared TTY descriptor's `FWASWRITTEN` bookkeeping bit on its first output
write. Candidate `437a19b0` ignores only that macOS bit when comparing status
flags; all other bits remain exact. New regressions cover every status bit and
the real first-write/reopen sequence. Diagnostic logs and rejected runtime
evidence remain retained; temporary diagnostic source was removed.

Three fresh general review agents inspected the entire feature, read-only,
without executing tests or performance measurements:

| Track | Reviewer | Result |
| --- | --- | --- |
| Correctness/API | `m65_r1_correctness` | P2: production never writes the typed events selected by event inspection; receipt sequences identify journal pages without public events. |
| Lifecycle/platform | `m65_r1_lifecycle` | P1: abandoned shared MCP refresh can retain a private peer in the admission cohort while cleanup waits for that cohort. P2: ordinary foreground preparation/enrollment drops candidate custody and its reservation after cleanup failure instead of fencing. |
| Performance/resources | `m65_r1_resources` | No concrete introduced findings; reviewed budgets, scheduling/dependencies, notices, paging, undo, blocked output and weak ownership. |

These findings reject the candidate. Passing local tests did not establish
coverage of the identified scenarios. No remote delivery was attempted for it.

The remediation combines isolated event-history and foreground-custody work with
principal-owned abandoned-admission settlement. Event mutations and receipts now
share durable typed evidence, including truthful suppressed/duplicate milestone
records. Ordinary foreground failures use the retained settlement/fence path;
the public opening failure carries any untransferred startup owner. MCP admission
settlement drives original preparation without closing a persistent child's
publication, and excludes idle or already-transferred turn cohorts. Added
regressions cover command-level event paging/recovery, exact worker custody,
cleanup timeouts, public failure ownership, and non-emitting milestone rendering.
These implementation changes still require replacement local gates and reviews;
the rejected candidate's passing tests do not verify them.

Coordinator inspection of `7b04dcf2` found that the initial repair inferred
abandoned preparation from outstanding worker tickets and could miss a retained
refresh before worker admission. Both platform Clippy checks passed, but the
subsequent test builds were intentionally interrupted before runtime tests.
The replacement consumes explicit preparation custody once per admission,
including worker-free cancellation, without affecting later idle controls.

Inspection of `ff10fc22` then traced the same retained-refresh ownership through
actual MCP tool execution. Its Linux formatting, Clippy, test compilation and
release build passed; the macOS test build was intentionally interrupted after
Clippy passed, before either platform ran runtime tests. The repair shares one
preparation-settlement claim between admission-first foreground cleanup and
turn-first child cleanup, including actual turns. Binding regressions cover
exactly-once transfer/failed-admission custody; factory regressions exercise both
settlement orderings with held actual workers. Controller tests separately cover
retained refresh retirement and non-cancelling deferred activation. These are
compositional tests, not a claimed full MCP tool-to-manager deadlock reproduction.

## Complete managed-agent feature: second reviewed candidate

Candidate `6d87002d00d8cd0ef57dc07939385332cce79614` passed the complete
replacement local gate before fresh whole-feature review against
`6d6364c6ee3b1540505749b10b2527df39ddf5be`. Both exact Rust 1.94.1 builds,
formatting, Clippy, release helpers, workspace/doc tests, focused runtime checks,
269 repository Python tests, documentation/upstream/Unicode checks, dependency
policy/audit and platform/Apple ABI checks passed. Linux used unprivileged default
concurrency and macOS serial runtime execution, without overlapping their runtime
gates. Logs remain under `/tmp/mg-managed-implementation.V0ZGg1/`, including
`managed-full-runtime-linux-6d87002d-r2.log` and
`managed-full-runtime-macos-6d87002d.log`. The initial Linux invocation failed
before tests because `grep -q` closed a test-list pipe early. Its log is retained;
the corrected preflight consumes the full listing. This was a gate-command error,
not a source fix or product-test failure.

Fresh read-only general reviewers rejected this candidate:

- `m65_r2_correctness`: P1 nonresident close permanently retries a policy-invalid
  restore after accepting archive intent; P2 production child runtimes omit model
  capability catalogs and suppress named effort; P2 delayed relationship consent
  can publish a now-archived parent.
- `m65_r2_lifecycle`: P1 production create/restore/reconciliation performs blocking
  store work on the manager's polling thread. In particular, a held transcript
  lock stalls sibling execution, cancellation and host progress. The separate
  preliminary failed-preparation/TLS suspicion was not confirmed as a finding.
- `m65_r2_resources`: P1 journal directory-entry limits are checked only after
  publication, leaving an unreconcilable ambiguity at capacity; P2 a suppressed
  duration deadline drops the sole timer without arming a later child's deadline.

These were source-established findings, not runtime reproductions. Neither review
performed builds, tests or measurements. Their passing local candidate evidence
does not cover the identified failures, and no remote delivery was attempted.
All three finished review worktrees were verified clean and removed. Isolated
implementation worktrees are separate from those frozen review checkouts.

Replacement implementation combines coordinator-owned relationship eligibility,
cleanup-only policy restriction, shared-deadline rearming and prepublication
directory-entry admission with isolated catalog-capability and controlled-worker
preparation repairs. Regression tests cover delayed consent after archive,
nonresident close under a stricter caller, suppressed duration wakeups, directory
pressure/orphan/reopen headroom, actual provider effort selection, held transcript
locks and actual worker/TLS completion. These are added tests pending execution,
not a claim of passing replacement gates. An early native-only compile found a
test referencing an HTTP-gated public catalog export; its import now uses the
internal module path. The failed compile log is retained as
`managed-r2-coordinator-compile-preparation-pending.log` in the same log directory.

Candidate `c0cd4da3` passed static policy/audit and platform/Apple ABI compilation,
but both Linux and macOS Clippy rejected one unused import in the new preparation
regressions. Neither platform reached runtime tests. The import was removed;
both failed build logs remain retained for the replacement gate.

Candidate `08c9a943` passed both complete build gates and the native Apple ABI
probe. Focused execution then rejected the two relationship-consent regressions
on Linux and macOS: their direct journal assertions raced manager notice replay
and returned `Busy`. The other 15 selected tests passed on both platforms; neither
full runtime gate started. The tests now settle replay before out-of-band journal
reads, without changing consent or publication assertions. Failed and remaining
focused logs are retained under the same log directory. Replacement verification
is still required; passing builds alone do not establish feature acceptance.

Candidate `e9522d58` passed both complete build gates, static/platform checks,
the native Apple ABI probe, 17 focused repair tests on each platform, and the
full Linux runtime/doc gate with 269 Python tests. The serial macOS runtime gate
failed `background_process::integration_tests::abort_and_drop_revoke_pipe_authority`
at its prepared-child abort: group TERM returned EPERM, the phase snapshot proved
only the leader, NOWAIT had no exit status, and the first bounded reap later
succeeded. The native suite reported 4,015 passing tests and one failure;
remaining integration/doc tests were not reached. The original test passed ten
unchanged isolated repetitions, but broader background-process execution then
reproduced the identical failure in
`pipe_mode_requires_controller_before_release_and_null_rejects_attachment`.
Neither repetition establishes a cause or source fix. All failed and diagnostic
logs remain under `/tmp/mg-managed-implementation.V0ZGg1/` with the candidate SHA.

Read-only source diagnosis found a feasible Darwin transition: `P_REF_DEAD`
precedes waitable `SZOMB`; group iteration can skip that leader and return EPERM,
while direct-PID signaling accepts it through `pzfind`. The evidence is Apple's
[direct-PID signaling](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/bsd/kern/kern_sig.c#L1381),
[process lookup](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/bsd/kern/kern_proc.c#L2631),
and [exit ordering](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/bsd/kern/kern_exit.c#L2269)
in public XNU `12377.121.6`, not the exact installed `12377.160.73` build.
This is a source-supported mechanism, not instrumented attribution of either
failure. A signal-zero probe cannot distinguish it from a running leader.

The correction instead retries the actual same signal directly against the
retained leader, only on macOS after EPERM, the sole-leader phase proof and a
fresh successful NOWAIT observation without status. It accepts only successful
direct signaling, preserves every direct error and observation failure, adds no
wait or global scan, and leaves final quiescence and actual reap obligations
intact. Replacement focused/full gates and three fresh whole-feature reviews
remain required; diagnosis is not an acceptance review.

Candidate `22d0068c` passed both complete build gates, static/platform checks,
the native Apple ABI probe and both new Darwin dispatch regressions. Its first
macOS background filter reported 112 passes and one failure: the valid one-shot
inventory helper produced no output within the original 250 ms deadline.
Collection correctly rejected it; the delay before first output remains
unattributed. The source paths involved are unchanged from the feature base.
Ten isolated repetitions and a later complete background filter passed unchanged
(113 passes, two private-helper entrypoints ignored), as did all 17 remaining
managed repair tests. Those repetitions are not an inventory cause or source fix.
The initial failure and diagnostic logs remain retained alongside the recheck.

The subsequent Linux run passed its focused native and CLI selections, then
failed the complete CLI suite: 597 passes, one failure and six private-helper
entrypoints ignored. The managed PTY child-conversation test sent Enter after
catalog creation/refresh and received the stale/busy/not-displayed input notice
instead of opening the conversation. The full native/doc/Python runtime stages
and full macOS runtime were not reached. Its focused earlier invocation passed;
this does not resolve the failure under normal CLI-suite concurrency. Logs use
the candidate SHA under the same retained directory. No fresh acceptance review
or remote delivery was attempted for this failed gate.

Read-only diagnosis established a visible-frame/flush-acknowledgement gap:
the renderer writes the composer before the separate output flush, and input
polling can precede consumption of even an already-ready flush acknowledgement.
An Enter received there binds no displayed frame and is rejected permanently.
The captured failure does not distinguish that ordering from a still-pending or
partially rendered catalog. Waiting for composer bytes alone cannot establish
native display acknowledgement. The physical fixture now observes one complete
nonbusy catalog, including its expected row and final composer, rather than
combining substrings from different frames. Product repair and deterministic
delayed-acknowledgement coverage are separate from that fixture correction.

The replacement retains at most one bare catalog selection with the original
editor, native frame and local render revision. It waits for that exact real
flush acknowledgement and revokes on intervening mixed/edit/partial input,
navigation, replacement, modal ownership and shutdown. Commands, messages and
close confirmations do not defer; native polling order and progress are unchanged.
Nine deterministic regressions force delayed/already-ready acknowledgements,
stale native/local revisions, receipt-before-decode ordering and revocation.
The existing blocked-output child-progress regression retains its execution
assertions and now checks pending selection rather than the old rejection.
This implementation and fixture correction still require execution and fresh
whole-feature acceptance; earlier passing repetitions are not their evidence.

Candidate `8b54fecd` passed static policy/audit and platform/Apple ABI compilation,
but both build gates stopped at Clippy: the added pending-selection handling
expanded `agents_event` to 117 lines against the existing 100-line limit.
Pending-selection and navigation-toggle handling are extracted into helpers
without changing admission or revocation conditions. An intermediate focused
Clippy run still counted 102 lines after only the first extraction; its log is
retained too. Neither build reached runtime tests; both failed build logs
remain retained under the candidate SHA.

Candidate `a4b0ca24` passed both complete build gates and static/platform checks.
Its nine acknowledgement regressions and both managed PTY scenarios passed on
Linux and macOS; the complete Linux CLI suite passed 607 tests with six private
helper entrypoints ignored. The full Linux native suite then parked in
`closing_nonresident_child_restricts_repair_without_changing_saved_policy`.
Read-only live debugger captures located the wait in its subsequent reopen
command, with `Active::Replay`, a `Journal` retry issue, a registered waker and
`closing=false`; the only other native threads were idle cleanup collectors.
The journal had already retained the archived child and unchanged saved policy.

The fixture read the private journal directly after close without settling
manager-owned replay. Source tracing shows that these operations can compete for
the same slot and leave replay behind the explicit journal-repair gate. The
original losing error was not retained in the live future. The fixture now waits
for owned journal settlement before its independent assertion read, following
the existing neighboring test pattern; product retry behavior is unchanged.
After capturing state, the exact owned test binary was explicitly terminated,
and Cargo exited 101 from SIGTERM. This is an interrupted gate, not a passing
native suite or a deadline-only restart. Full macOS runtime was not started.
The original runtime and three `managed-live-hang-*-linux-a4b0ca24.log` captures
remain in the retained log directory. Fresh replacement gates remain required.

Candidate `ecdba60e` passed both complete build gates, static/platform checks,
focused macOS checks and the complete Linux runtime gate, including all 269
repository Python tests. The Linux native suite passed 4,011 tests, including
the previously parked close/reopen fixture under normal concurrency. The full
macOS CLI suite passed 609 tests; the native suite then finished with 4,017
passes, one failure and twelve private-helper entrypoints ignored. Remaining
macOS integration and doc tests were not reached. Both exact runtime logs remain
under `/tmp/mg-managed-implementation.V0ZGg1/`; no acceptance review or remote
delivery was attempted for this candidate.

The failure was `close_signal_denial_requires_positive_exit_and_retains_force_retry`:
its graceful-close assertion observed `Signaled(15)` rather than `Signaled(9)`.
Read-only diagnosis traced the fixture's EPERM injection to group dispatch only.
The new macOS retained-leader fallback legitimately delivered the same TERM
directly, so the injection no longer represented a denied signal. The correction
adds a separate test-only direct-dispatch denial and arms both paths in this
fixture. Existing group-only fallback tests and production signaling remain
unchanged. The original assertions still require graceful escalation to KILL,
failed force-close retention and actual reap after a subsequent force retry.
Fresh focused and complete replacement gates remain required.

During replacement-candidate preparation at `9a735e2e`, read-only investigation
found a separate reachable ACP defect in model attach/reparent consent. Core
ends the ordinary permission-review scope before admitted tool execution. The
relationship authorizer then emitted a new synthetic permission request, but
ACP required the exact still-authorizing request for every permission prompt.
That lookup necessarily failed stale, and the connection driver treated the
projection failure as a protocol error. This finding is source-backed; no
runtime reproduction was claimed at discovery. Human relationship commands and
detach do not take this second-consent path.

The remediation separates exact-proposal execution consent from policy
authorization. Its native request must originate in the actual claimed managed
invocation, retain the frozen proposal and expire with its original job, turn,
principal or cancellation. CLI and ACP expose only one-shot approve/reject,
with no saved rule or reusable grant. Ordinary ACP permission snapshots remain
strict. Composed tests drive actual model calls through native admission and
the inbox, including CLI flush acknowledgement and ACP reply correlation;
structural test contexts are not positive consent evidence. This correction
requires replacement full local gates and three fresh whole-feature reviews
before delivery; the prior candidate is not accepted.

The exact `9a735e2e` Linux/macOS build stages and static/platform checks finished
successfully before integration. With its fresh macOS release helper, the
complete signal-denial fixture, close/reopen settlement fixture and both
retained-leader/group-denial fallback regressions passed (four focused tests).
The retained log is
`/tmp/mg-managed-implementation.V0ZGg1/managed-denial-repair-macos-9a735e2e.log`.
This verifies those narrow corrections, not whole-feature acceptance; full
runtime gates were not started for the candidate with the known ACP defect.

Candidate `1386575a2fb398b43fd2580754d5d37d4af33fb4` integrated exact
execution-time consent, the incoming repository banner changes and bounded
verified-mirror failover for the pinned Zig benchmark download. Its complete
local Linux/macOS build, runtime, static and platform gates passed. Linux native
passed 4,015 tests; macOS native passed 4,022 tests. The CLI suites passed 609
and 611 tests respectively, and all 275 repository Python tests passed.
Logs remain under `/tmp/mg-managed-implementation.V0ZGg1/`, including
`managed-full-runtime-linux-1386575a.log` and
`managed-full-runtime-macos-1386575a.log`. These are local regression results,
not remote delivery or performance evidence.

The fresh R3 correctness reviewer (`m65_r3_correctness`) rejected that candidate
with one source-backed P1: a persistent child's default 64-item FIFO returned
`JournalError::Limit` on the next submission, but durability confirmation parked
that command as capacity pressure. Its active command paused the target and
occupied the sole journal lane needed to remove accepted FIFO work, admit
cancellation or publish sibling writes. Explicit retry reused the same full
snapshot. No runtime reproduction was claimed by the source-only reviewer.
The other two review tracks did not start because the host rejected new threads
with its thread-limit error; this was not a three-track acceptance cycle.

The correction distinguishes definite preacceptance Limit rejection from Busy
retry and already-accepted settlement. A rejected command returns `ResourceLimit`
without holding the journal lane. Internal writes after durable acceptance and
ambiguous publication retain their original custody. The R3 reviewer then
became a fix author and is not an independent acceptance reviewer of that fix.
Focused regression results and complete replacement acceptance remain required.

Candidate `8dd596c452390aa2b7499e54d21fd5d4306e8bbc` passed the complete
replacement Linux/macOS local gate, including the three FIFO-limit regressions.
Linux native passed 4,018 tests and macOS native passed 4,025; CLI suites passed
609 and 611 respectively, and all 275 repository Python tests passed. Exact
build, focused, static/platform and full runtime logs remain under
`/tmp/mg-managed-implementation.V0ZGg1/` with the `8dd596c4` suffix. These are
local regression results, not delivery or performance evidence.

The fresh R4 correctness reviewer (`m65_r4_correctness`) rejected that candidate
with one source-backed P2: notice addressing required a live parent principal.
An idle-evicted persistent parent retained its durable child relationship, but
work admitted to that child registered a detached notice tracker. Restoring the
parent before completion refreshed only the parent's own tracker, so the child's
enabled completion notice remained suppressed and no original existed to replay.
The source-only reviewer ran no tests. The other two review tracks did not start
because new agent threads were unavailable; this was not three-track acceptance.
The R4 reviewer subsequently became the fix author and cannot count as an
independent acceptance reviewer of its correction.

Integration tracing of the durable-target correction also identified a related
direct-inbox boundary: snapshots selected by display ID/generation alone could
expose an old original to a sequential replacement with a different transcript
incarnation, before historical source-acknowledgement validation rejected it.
The correction must retain target incarnation in each immutable envelope and
filter live snapshots before bounded selection, alongside historical replay and
acknowledgement validation. This source-backed finding is not an executed
regression or acceptance result.

The correction persists the parent generation with its journal relationship and
the target incarnation in immutable notice envelopes. Actual prompt snapshots
filter by incarnation before bounded selection; saved prompt/outbox records and
historical replay/acknowledgement validate the same original target. Eviction
does not change addressing, and reopen does not silently retarget an old
relationship. The obsolete weak live-principal addressing cache is removed.
Regression coverage includes actual residency pressure and restoration, restart
replay, close/reopen and explicit reparent/detach, consent across parent-generation
change, malformed relationship triples, historical target mismatches and direct
foreign-incarnation exclusion. Compilation, focused execution, full replacement
gates and independent acceptance remain required for the integrated correction.

Candidate `58fa92f551278792bbfd9ac1f3fca73316847f09` passed both complete
build stages, including fresh release helpers, and static/platform checks.
Focused macOS eviction/restoration, direct-inbox and historical-target,
relationship-consent, and prompt-context regressions passed. The notice staging
suite then rejected its new fixture's relationship update with `StaleSource`:
the fixture changed incarnation but reused relationship revision 1. Production
correctly requires a strictly increasing relationship revision. The fixture now
asserts that stale rejection before advancing to revision 2; original-envelope
immutability and pre-count incarnation-filtering assertions remain unchanged.
The exact focused failure log remains
`/tmp/mg-managed-implementation.V0ZGg1/managed-focused-macos-58fa92f5.log`.
Neither complete runtime suite nor acceptance review ran for this candidate.

Candidate `022c0083e495a8e2435057bd014ec05116c89c2e` passed the complete
replacement Linux/macOS local gate after the staging fixture correction.
Linux native passed 4,028 tests and macOS native passed 4,035; CLI suites passed
609 and 611 respectively, and all 275 repository Python tests passed. Exact
build, focused, static/platform and full runtime logs remain under
`/tmp/mg-managed-implementation.V0ZGg1/` with the `022c0083` suffix. These are
local regression results, not remote delivery or performance evidence.

The fresh R5 correctness reviewer (`m65_r5_correctness`) rejected that candidate
with one source-backed P2: host shutdown could move a settled child into
retirement before its parent-notice source acknowledgements and delivery-outbox
clear settled. Clear dispatch excludes closing children and cannot address
retirees; completed retirement can drop the original notice context even while
a clear future retains the runtime. The shutdown predicate could then report
success with an uncleared saved outbox. Explicit later restoration can recover
that original outbox; the finding is false graceful-shutdown completion, not
history deletion. The source-only reviewer ran no tests. The lifecycle track
could not start because the host rejected new threads; the resource track did
not start. This was not three-track acceptance, and the candidate was not pushed.
The R5 reviewer subsequently became the correction's fix author and cannot count
as an independent acceptance reviewer of the replacement candidate.

The correction uses complete runtime notice-custody checks before explicit
archive, shutdown retirement and resident removal. Rejected restored preparations
can own recovered deliveries too: their exact retained runtime/context remains
available to the existing clear driver after peer/worker closure. Final shutdown
also checks retained clear and uncertain context custody. Resolution retains
normal runtime admission and exact context matching; no label-based authority or
new execution is introduced. Seven regressions cover acknowledgement pending,
paused clear and capacity, committed-error retry, dropped clear repair, saved and
uncertain recovery, explicit close, and rejected restored admission.

The test-only component `4c206f573537f9d63b1c6fc7ed91aa990d8fb846`, applied to
unchanged `022c0083` production code, compiled with exact Rust 1.94.1 and a fresh
release-helper build. On macOS the exact
`shutdown_ack_pending_clears_child_outbox_before_success` regression failed in
0.16 seconds at its assertion that successful shutdown must clear the original
outbox (exit 101). The retained log is
`/tmp/mg-managed-implementation.V0ZGg1/managed-shutdown-red-runtime-4c206f57.log`.
This is executed reproduction of the finding, not a fixed-code acceptance result.
The rejected-restoration test component `7b1254ca` and production correction
`2d0e12f1` are integrated with it. Focused execution, complete replacement gates
and three fresh independent reviews remain required.

Candidate `119b70318a70e78029b71c708a7446b6e7f8a3a9` passed both complete
build stages, including fresh release helpers, and static/platform checks.
All seven new shutdown/retirement regressions passed on macOS in 3.86 seconds,
including the previously failing original-outbox assertion. Earlier CLI consent,
selection, PTY/restart and durable-parent checks also passed. The existing
`actual_parent_checkpoint_is_source_acknowledged_before_outbox_clear` test then
hung during fixture shutdown, after its explicit external clear had succeeded.
Sampling the live process confirmed `Fixture::drop` waiting in `block_on`.
The external context had dropped before the manager observed its successful
clear; `Parent.clear` retained a stale receipt without independent confirmation,
and the strengthened shutdown predicate could never accept it. A dead context
alone must not establish successful clearing, so blanket receipt disposal is not
an acceptable correction.

The exact diagnosed test process was terminated with SIGTERM after retaining
the stack sample; the focused gate exited 143 and stopped all later stages.
Neither complete runtime suite nor acceptance review ran. Logs and the stack
remain under `/tmp/mg-managed-implementation.V0ZGg1/`, including
`managed-focused-macos-119b7031.log` and
`managed-delivery-stall-119b7031.sample.txt`. The candidate was not pushed.
The same fix author is correcting exact confirmed-clear receipt settlement;
this is continued remediation, not a fresh independent review.

The follow-up component `88f81e153c3e24d580530589da3363df87805d9e` retains
monotonic confirmed-clear evidence on the original delivery record. Only the
matching operation after successful metadata publication sets it; notification
wakes the waiting manager after releasing the context lock. The manager retains
that exact receipt through clear/retry futures and observes its confirmation
before discarding stale state. Dead context alone does not clear a receipt,
and registration preserves unconfirmed custody. Two regressions cover external
clear waking shutdown before context retirement and context drop without clear
confirmation; existing error/drop tests assert confirmation only after repair.
Formatting and diff checks passed in the fix-author worktree. Focused and full
replacement execution and three fresh independent reviews remain required.

Candidate `4c7c452b3ba71eed06556f76bb8164c183df0d5e` stopped at warnings-denied
Clippy on both Linux and macOS: the new external-clear regression used similar
local names `wakes` and `waker`. Static policy and platform checks passed;
no runtime stage started. Renaming the counter to `notifications` preserves
every assertion and changes no production behavior. The replacement candidate
still requires the complete local gate and three fresh whole-feature reviews.

Candidate `a042eac208ab5972a2b0d753bd7b5f8a0f17d783` passed the complete exact
Rust 1.94.1 local replacement gate. Linux passed 609 CLI and 4,037 native tests,
workspace integrations and doctests, plus all 275 Python checks. macOS passed
611 CLI and 4,044 native tests, workspace integrations and doctests. Both fresh
release helpers, focused regressions, static policy/audit and platform checks
passed. All six delivery and seven shutdown regressions passed on both platforms,
including the previously hanging external-clear/context-retirement scenario.
Logs remain under `/tmp/mg-managed-implementation.V0ZGg1/` with `a042eac2` suffixes.

Fresh R6 correctness review rejected this candidate with one P2 finding at
`managed/manager/command/lifecycle.rs:165`: explicit cancellation of accepted
queued or interrupted work can take the direct intent/head-settlement path
without publishing its frozen enabled cancellation notice. After host shutdown
and restart, nonresident interrupted work can therefore be successfully cancelled
while its parent permanently misses the notice. Only runtime completion sets
the terminal-notice flag; replay cannot recover a notice never journaled. The
review was source-only, not an executed reproduction. No other actionable finding
was reported. Lifecycle review could not start despite an initial attempt, retry,
and another attempt after an old thread disappeared; each hit the host thread
limit. Resource review did not start. Unused review worktrees were removed.
There was no three-track acceptance or push. The correctness reviewer now owns
the correction and is not independent for replacement acceptance.

Test-only component `f515ab3beb6e3ce290dfa8ddfc7b2e9a6cd23796`, against unchanged
`a042eac2` production code, reproduced both cancellation-notice omissions on
unprivileged Linux with exact Rust 1.94.1 and the fresh release CLI/helper.
`cancellation_before_initial_execution_publishes_original_without_a_turn` and
`nonresident_cancel_after_shutdown_preserves_frozen_enabled_notice` both failed
at the assertion requiring one original cancellation notice: observed zero,
expected one (exit 101; two failures in 0.10 seconds). The test-only build passed.
Logs are `managed-cancel-red-build-f515ab3b.log` and
`managed-cancel-red-runtime-f515ab3b.log` in the retained gate directory above.
This establishes the original failure, not acceptance of a correction.

Correction component `c60878b291fc69768b25e12b916090e0e07547e5` publishes the
cancellation original and FIFO-head removal in one journal transaction. The
manager derives frozen notification policy and actual attempt identity from
durable originals, with the exact current relationship target. This also handles
nonresident and never-started work without restoring a runtime or executing a
provider turn. Confirmed cancellation retires the live emitter so replay cannot
duplicate the terminal notice. Only explicit cancel intent creates the notice;
archive/close stays silent. Disabled policy and detached relationships stay silent.
Eight manager regressions cover pre-start/restart, frozen policy, FIFO, idle
repeat, active cancel, explicit retry and close. Two store regressions cover
exact target rejection and all four publication fault phases, reconciliation
failure and restart. Author formatting and diff checks passed; fixed-code runtime
execution and replacement independent acceptance have not yet run.

Integrated candidate `6f90dd864e74f8c8fbcba04e42b3456ed78c273f` stopped at
warnings-denied Clippy on Linux and macOS: cancellation preparation checked
`parent_owner.is_some()` before unwrapping it, and a store assertion cloned a
notice into a one-element array. Static policy/audit and platform checks passed;
no runtime stage started. Binding the parent with `if let` and using
`std::slice::from_ref` preserve behavior and assertions. The replacement still
requires focused execution, the complete local gate and fresh independent review.

Candidate `71dc23728fa74488fad26ce925b269ce98c167fd` passed the complete exact
Rust 1.94.1 replacement gate. Linux passed 609 CLI and 4,047 native tests,
workspace integrations and doctests, and all 275 Python checks (219.350 seconds).
macOS passed 611 CLI and 4,054 native tests (828.26 seconds for native), remaining
workspace integrations and doctests. Both platforms passed all eight manager
cancellation and two store cancellation regressions, six delivery and seven
shutdown regressions. Warnings-denied all-feature Clippy, fresh release helpers,
static policy/audit and platform checks passed. The native-only test builds still
reported their 26 existing unused-code warnings. Logs remain in the retained
gate directory with `71dc2372` suffixes; the complete gate process exited zero.

Fresh R7 correctness review started in an isolated read-only checkout of this
candidate. Concurrent lifecycle launch and its single retry both hit the host
thread limit, so that unused clean worktree was removed. Resources review had
not started. Review acceptance requires three fresh independent results; a
later completed-reviewer state may permit the remaining new reviewers to run
sequentially. This is no three-track acceptance or delivery claim. Remote main
was still `7cadf2f2ea13ef392797903ad190c0ce3ba92654`; the feature was not pushed.

The R7 correctness agent subsequently hit a usage limit before completing its
review. Its partial report identified a possible full-residency mailbox deadlock;
it supplied neither a final finding inventory nor acceptance. Coordinator tracing
confirmed `manager/pump/admission.rs` retained an unaccepted capacity-needing
command even with no evictable child or retirement in progress. `capture_mailbox`
then withheld later cancel/close controls behind that same head.

Test-only component `a09b472e542244283840746217fef6e12126a179`, with unchanged
`71dc2372` production, reproduced this on exact Rust 1.94.1 unprivileged Linux
with a fresh release helper. The full-residency create-then-cancel regression
failed in 0.09 seconds with `unaccepted create parked ahead of the only
cancellation` (exit 101), without a provider response or caller retirement.
Logs are `managed-residency-red-build-a09b472e.log` and
`managed-residency-red-runtime-a09b472e.log` in the retained gate directory.
The clean failed-review worktree was removed; no review acceptance was inferred.

Remediation rejects unaccepted resident requests when neither safe idle eviction
nor actual ongoing retirement can supply a slot, and preserves foreground
reservation priority without parking commands. Already-owned retirement still
settles before reuse. Unresolved saved-notice custody stays retained; the
requester receives a limit result and explicitly retries after repair. Replacement
tests, the complete local gate and three fresh whole-feature reviews remain
required. Coordinator remediation is not an independent acceptance review.

Candidate `22cd2154cc9f6d831c2fcb51c86a4ea257559901` stopped at Linux and macOS
warnings-denied Clippy (both exit 101): the changed foreground regression lacked
the `ManagedFailureCode` import and retained an unused `ManagedSubagentAuthority`
import. The replacement corrects those test imports without changing production
behavior. Neither runtime tests nor independent reviews started for this candidate.

Candidate `a3a2641f63f0996b2f9489b146ab8ab4d409a5d3` passed the complete exact
Rust 1.94.1 replacement gate. Linux passed 609 CLI and 4,050 native tests,
workspace integrations/doctests and all 275 Python checks (204.983 seconds).
macOS passed 611 CLI and 4,057 native tests (814.62 seconds), workspace
integrations and doctests. Both fresh release helpers, focused managed tests,
all-feature warnings-denied Clippy, static policy/audit and platform checks
passed. The native-only builds retained their 26 existing unused-code warnings.
The macOS release build took 27m35s; a process sample established active LLVM
code generation, not a hung build. Gate logs retain the `a3a2641f` suffix.

Three fresh R8 reviewers independently completed source review of that exact
candidate against `7cadf2f2ea13ef392797903ad190c0ce3ba92654`. These were
ordinary isolated adversarial agents, not Bugbot; they did not rerun runtime
tests. All four findings reject the candidate:

- Correctness (`m65_r8_correctness`), P2: denied child tool attempts had no
  journal producer. Core emitted neither started nor finished events for denial,
  and manager permission observations retained no exact tool-call identity.
- Lifecycle (`m65_r8_lifecycle`), P2: hidden Models/Skills menu state leaked
  into modal composer context, imposing the 256-byte picker limit on valid
  answers, retaining submitted text and misinterpreting LF/Ctrl-J.
- Resources (`m65_r8_resources`), P1: a full ordinary notice inbox prevented
  terminal publication and actual manager shutdown without a parent prompt ACK.
- Resources, P2: original ACK and historical-parent validation could scan whole
  histories inside one active replay future, withholding the journal lane from
  sibling control commands and shutdown.

The same three reviewers subsequently authored component fixes and therefore
are not fresh reviewers for replacement acceptance. The coordinator owns replay
fairness and integration. No M65 branch push or three-track acceptance followed.

Lifecycle test-only `48dd08e7` reproduced both modal bugs on Rust 1.94.1:
long answers returned `TooLong`, and submitting `1` retained draft `1`.
Component `76dbf1bb` gates menu/history/form context from the actual retained
input owner, preserving an earlier agent-owned paste. Its two regressions,
32 raw-input tests and 35 composer tests passed. Driver fixture checks stopped
at the missing fresh release-helper prerequisite; they were not counted as
passing or replaced with weaker checks.

Notice-pressure test-only `d872c5f7` had an incorrect setup predicate: a
persistent child becomes Idle, not Completed. The coordinator stopped only that
owned test process; it is not product-failure evidence. Corrected bounded
test-only `3393f9292011aa63ac4324f99319f8c5f5019a53` failed on unchanged
production under unprivileged Linux/Rust 1.94.1 in 2.10 seconds with
`full notice inbox prevented actual manager shutdown` (exit 101). The log is
`managed-notice-pressure-red-3393f929.log` in the retained gate directory.
Component `528f7caa` adds a separately charged single durable-publication slot,
retains exact originals and snapshot charges, and wakes capacity waiters.
Replacement focused execution and complete gates remain required.

Correctness's initial regression reused provider call IDs inside one turn,
which core rejects, and waited for Idle. That exact test was stopped and its
fixture corrected to reuse IDs across two actual FIFO turns with a terminal
failure assertion. Clean test-only `c2acfc53a494f312a1d5923cef408244961d1ab8`
then failed on unchanged production with four approved activity records versus
six expected records including two denials (Rust 1.94.1, exit 101, 1.43 seconds).
Neither the invalid fixture nor author work constitutes replacement acceptance.

Replay test-only `235c29da` initially failed compilation because its sibling
module could not access the existing fixture. Corrected test-only `b456d211`
widens only test-fixture visibility and reproduced the finding on unchanged
production under unprivileged Linux/Rust 1.94.1: one admission scanned the
entire historical relationship (exit 101, 0.14 seconds). Its build retained the
existing native-only warnings plus a test-only private-interface warning;
the integration narrows that helper method rather than suppressing the warning.
The fresh production helper came from the same unchanged production source.
Logs retain `managed-replay-red-` and the complete test-only SHA suffix.
The coordinator correction joins original/ACK/parent validation in resumable
100-record steps, checks exact snapshots between admissions, and parks failed
read-only retries outside the command lane. Mutation reconciliation retains its
existing original custody. Fixed-code execution and fresh acceptance remain due.

Correctness component `f45533c9` adds the explicit core denial observation,
managed durable activity producer and ACP failed-call projection. Author checks
passed four managed event tests, 17 ACP projection tests, 98 core unit tests
and 71 testkit permission/tool-loop tests under Rust 1.94.1. Component
`c931cac4` adds a bounded integrated notice regression for real parent checkpoint
ACK/outbox clear, retained snapshot pressure, deferred replay with a live tracker
and shutdown without another provider turn; it has not yet been executed.
All component edits are integrated into the feature branch. Author checks do
not replace the full integrated local gate or three fresh independent reviews.

Integrated candidate `403a7563c1fc5fdd16d5149705341b5387bccfb1` stopped at
Linux and macOS warnings-denied Clippy (both exit 101): the notice-pressure
regression used a redundant closure around `NoticeBatchEntry::token`. Static
policy/audit and platform checks passed. No runtime stage or independent review
started; replacing the closure with its method reference preserves the test.

Coordinator integration inspection also found that the new publication charge
could undercount transient copies/serialization for very small configured notice
limits. The correction checks copied payload size before staging, counts exact
JSON bytes without an encoded allocation, and rejects oversized replay envelopes
before cloning. Added tests compare Unicode/escaping counts with actual JSON,
exercise exact and undersized limits, and check full charge refunds after tiny
limit rejection. These are implementation checks, not independent acceptance.

The parked-read retry also receives exact issue-observation ownership: dropping
a superseded validation wait clears its own stale blocked status without
clearing a newer wait or authorizing a mutation retry. A focused ownership test
covers both ordinary drop and overlapping same-kind waits.

Candidate `b741edbfe70971becdecfce6d9f080fa6f985436` passed Linux and macOS
warnings-denied all-feature Clippy, static policy/audit and platform compilation.
Its early Linux replay run failed one of four tests (exit 101): extracting
`history_fixture` had dropped the actual session before returning its weak
notice-context witness. The missing pending notice therefore reflected an
invalid test lifetime, not evidence to weaken production witness validation.
The other three replay tests, 31 notice tests, nine notice-shutdown tests and
four managed-event tests passed. This includes the previously unexecuted
`c931cac4` deferred-replay integration regression. The failed candidate is not
accepted; the macOS release build was still active when these checks completed.

Test-only component `c433e698ba8438313ff20a7c89353ee377968b2c` retains the
actual session in both fixture callers and adds live/dropped witness assertions
without changing production validation or existing replay assertions. With a
fresh locked release-helper selection on exact Rust 1.94.1 unprivileged Linux,
all 48 focused tests passed: four replay, 31 notice, nine notice-shutdown and
four managed-event tests (exit zero). Logs are
`managed-r8-focused-linux-b741edbf.log`,
`managed-r8-adjacent-linux-b741edbf.log` and
`managed-r8-focused-linux-c433e698.log` in the retained gate directory.
These focused checks do not establish the full replacement gate or independent
whole-feature acceptance.

### R9 complete-feature review and replacement work

Candidate `52c68c4208e660678ab8f8eb4a299e9895a2dee4` passed the full replacement
local gate on Rust 1.94.1: Linux CLI 611/native 4,065 tests, macOS CLI 613/native
4,072 tests, workspace integrations/doctests and 275 repository Python tests.
Formatting, warnings-denied all-feature Clippy, fresh locked release helpers,
static policy/audit and FreeBSD/WASI/Apple platform checks passed. Exact logs
retain the `52c68c42` suffix in `/tmp/mg-managed-implementation.V0ZGg1`.
The earlier `b741edbf` macOS focused runner stopped on a shell quoting error
before executing tests, after its release build succeeded. Corrected replacement
commands passed shell syntax checks before the complete candidate run.

Three fresh ordinary independent source reviewers inspected this exact clean
candidate against `7cadf2f2ea13ef392797903ad190c0ce3ba92654`; none edited files
or claimed to rerun the supplied gates. Their complete reports reject it:

- Correctness: a second cancel/close can displace the first accepted control
  observer while actual cleanup remains pending (one P2).
- Lifecycle: a whole-child FIFO waiter retains an obsolete target-run dependency,
  allowing a successor wait cycle to evade rejection (one P2).
- Resources: confirmed journal growth can exhaust all subsequent operation
  admission permanently (P1); continuous mailbox commands can starve accepted
  child writes/starts and completed waits (P2); source ACK validation holds the
  journal lane across lifetime-sized history scans (P2).

The coordinator's delivery regression reproduced the last finding on unchanged
production: one admitted delivery scanned the entire 360-record retained history
before returning (one test failed, exit 101, 1.12 seconds). Evidence is retained in
`r9-delivery-red-exact.log`. The initial short-name `--exact` invocation ran zero
tests and is not evidence; the corrected fully qualified invocation ran the test.
Replacement code makes ordinary source validation resumable between bounded
pages, retains exact original custody and parks failed reads outside the lane.
Author tests, complete replacement gates and fresh reviews remain required.

The coordinator's delivery component subsequently passed all nine delivery tests
and all 84 default-feature manager tests under Rust 1.94.1 (3.16 and 20.31 seconds).
The additional regressions cover restarting a changed source frontier without a
stale ACK and retaining in-flight delivery when another parent registration
prunes dead observers. Retained progress uses bounded cursors and bitsets, not a
full head per parent. Logs are `r9-delivery-focused.log` and
`r9-delivery-manager-focused.log`. All-feature Clippy identified one
`single_match_else` style lint; the equivalent `if let` correction is included.
These author checks do not establish the full replacement gate or acceptance.

Author component `1bdda588` preserves accepted control custody, rotates seven
durable admission lanes, fences target writes and avoids resetting replay for
unchanged inspections. Its overlapping-cancellation regression reproduced the
baseline failure. All 79 default-feature manager tests passed on Rust 1.94.1 in
the isolated author tree, including continuously replenished four-slot inspection
traffic with accepted child progress, completed-wait response and replay completion.
Coordinator integration retains the resumable delivery signature and shutdown
custody checks; its exact combined checks remain due. The journal/pressure and
FIFO-successor fixes are separate components of the same rejected-candidate repair,
not new delivered features.

Integrated `dd9e954e` passed all 86 default-feature manager tests. The selected
helper run is retained in `r9-integrated-delivery-pump-focused-selected-helper.log`;
an earlier invocation had a mistyped unused helper path and is not the selected
helper evidence. All-feature Clippy found two admission-refactor style issues:
the lane constant followed statements, and infallible admission returned `Result`.
The coordinator moved the constant and removed the unnecessary wrappers through
the pump caller. Exact replacement checks remain due after the remaining fixes.

Store component `42554e7d26f0840562409e7a2707ea03867bd69b` passed 46 store,
six manager-limit and four catalog tests on exact Rust 1.94.1. Its regression
reproduced the legal confirmed-publication/read-admission failure before the fix.
Credit-inclusive tests cover actual protected-byte spending, fourteen exact
cleanup/ACK directory credits, exhausted readable reopen, accepted FIFO archive,
pressure interruption without user intent, seventy idle histories under unchanged
defaults, and archived exact-ACK owner-epoch transfer. The integrated manager
producer cutoff and complete replacement gates remain required.

Wait component `0f55e1b5da164bdca958f903cb4dc92615897f1b` passed 18 scheduler
and 83 default-feature manager tests (0.01 and 18.74 seconds) after reproducing
the FIFO-successor cycle failure. It retains the original request/deadline and
cancellation custody through fair reacquisition and successor validation; tests
include cancellation in the between-leg gap and unadmitted-run readiness.
Both author trees were clean at handoff; neither ran the full replacement gate.

Coordinator integration skips generic recovery for archived replay sources and
uses the store's exact archived-ACK repair exception in ordinary delivery. A
related remaining path was also identified: nonresident close/reopen's saved
lifetime helper loops through delivery pages inside one command admission.
It requires a retained lifecycle continuation through manager-owned delivery and
actual cleanup before this repair series can claim bounded delivery integration.

Pressure components `24f0240d`, `a2ee5366`, `162bf2e2` and `4e3dc970`
were integrated with the delivery and store changes. Author combined `b1f904ac`
passed 85 default-feature manager tests, including all eleven pressure tests,
on Rust 1.94.1 in 41.85 seconds. The restart fixture originally requested an
unsupported `queue` inspection section; correcting it to `status` changed only
the fixture. No full replacement gate or fresh review is claimed.

Coordinator integration makes all inspections read-only and projects lost-owner
live work as interrupted without changing the snapshot used for validation.
Store component `6fbad133` permits exact reserved ACK-only owner repair for
quiescent sources as well as archived ones; replay and delivery skip generic
recovery only when it would not change work state. Actual owner-reopen integration
tests and the saved-lifetime continuation remain part of the same R9 repair.

Actual owner-reopen test component `eb3beca0` and follow-up `8ba78a7f` cover
archived/quiescent ACK credit use, rejection of a foreign checkpoint, required
live-work recovery, preservation of accepted intent, and read-only lost-owner
projection. The quiescent delivery and replay regressions first failed because
generic recovery consumed an unavailable publication allowance. On the integrated
consumer code, all four delivery, five replay and 46 store tests passed.

Coordinator `549bdb14` compiled the native unit-test binary successfully; its
all-feature Clippy gate rejected `transaction::publish` at 108 lines. Component
`43fdfc36` extracts cohesive page validation/encoding without moving publication
effects or reservation custody; all 55 focused tests passed again (0.65, 0.38
and 9.27 seconds respectively). Logs are `r9-delivery-recovery-focused.log`,
`r9-readonly-integration-build.log` and `r9-readonly-integration-clippy.log` in
the retained gate directory. The coordinator intentionally terminated the
superseded intermediate release build before integrating this correction; no
fresh release or full replacement gate is claimed for that rejected candidate.
The same component's default-feature targeted `too_many_lines` Clippy passed;
its existing dead-code warnings remain outside that narrow check. Root formatting
and bounded documentation checks passed after integration. The clean resource
worktree was removed after exact scoped comparison, with commits retained.

Saved-lifetime test component `4a12fd25` reproduced a nonresident close holding
the command lane during an actual parent outbox clear, blocking an unrelated
configure. Component `989573d4` parks old preparation and cleanup, transfers close
intent to resident archive settlement, and returns reopen through an eighth fair
lane only after old delivery/resources settle. Exact retiring transcript owners
fence duplicate preparation; pending targets reject competing mutations without
blocking inspection. The first ten nonresident and subsequent 95 manager tests
passed. A broader run passed 97 of 99: two shutdown fixtures incorrectly expected
pending observers after mailbox closure, which intentionally returns Unavailable
while retaining manager cleanup. Corrected fixtures preserve actual-cleanup,
original-intent and no-new-generation assertions; their final run remains due.
Root integration proceeds to the full replacement gate on this frozen source,
not to delivery or a new independent acceptance claim.

The corrected frozen `989573d4` manager run passed all 99 tests in 57.15 seconds;
`r9-saved-lifetime-manager.log` retains that exact result. Integrated `84df58f3`
passed static policy/audit and FreeBSD/WASI/Apple compilation, then both Linux
and macOS all-feature Clippy rejected the same three style issues: manual
let-else, a 104-line delivery-clear function and a non-Copy test-only mode enum.
Neither build reached release/runtime gates. Follow-up `1fe6c15e` uses let-else,
derives Copy/Clone for that enum and shares the exact-context runtime lookup,
preserving lookup order and the distinct fresh-clear idle-child condition.
The corrected source requires a complete replacement gate before fresh review.

Integrated `bae06b54d6a6c522ea6e956909b3ec5cb165ec18` passed both platforms'
warnings-denied all-feature Clippy, workspace/native test compilation and fresh
locked release builds, plus static policy/audit and FreeBSD/WASI/Apple compilation.
The Linux focused managed and CLI filters passed. Its full workspace run stopped
in the CLI suite at 610 passed, one failed and six ignored: the actual release
process-restart scenario rejected reopening an archived agent instead of returning
`LifecycleChanged`. A standalone run of the same exact test reproduced the failure
in 10.24 seconds. Logs are `managed-full-runtime-linux-bae06b54.log` and
`r9-reopen-release-red-bae06b54.log` in the retained gate directory. The remaining
workspace tests, Rust doctests, Python suite and macOS runtime stages were not reached;
no full-gate success or fresh review is claimed. The correction must preserve
original observed admission while distinguishing owned lifecycle progress from
external stale changes, then pass the complete replacement gate.

A native observed-human regression reproduced the same `StaleGeneration` rejection
after actual journal-owner restart: its valid reopen failed while an external
recovery-before-admission negative passed (one failed, one passed, 0.19 seconds).
`reopen-observation-red.log` retains this unchanged-production result. The later
acknowledgement-provenance cases were not part of that two-test result and require
their own fresh execution. The repair must retain the post-admission expected
head without reapplying the historical UI revision after owned progress, while
preventing an external mutation from being adopted through a later delivery ACK.

Component `06037517dc7e282355f2b896ed578c82928d25b2` retains that expected-head
fence and records exact before/after delivery publications. Three of its four
native cases passed; the remaining test's direct journal read raced an admitted
replay and returned Busy. Follow-up `03cc3c5d8c4db986859c5eeeb6b3ceeebb351d83`
waits for journal-lane idleness and moves a test-only import into the test module,
without changing production behavior. Its four regression cases passed in 0.52
seconds, followed by all 108 manager tests in 46.62 seconds. The original process
scenario also passed in 0.23 seconds using its unchanged harness and a freshly
built Linux `06037517` release CLI. Logs are `reopen-observation-green-03cc3c5d.log`,
`reopen-observation-manager-03cc3c5d.log` and
`reopen-release-cli-green-06037517.log`. Native all-target/all-feature
warnings-denied Clippy passed on exact `03cc3c5d` in 5m09s;
`reopen-observation-clippy-03cc3c5d.log` retains the result. These component checks
do not replace the complete integrated gate or three fresh whole-feature reviews.

Integrated `240f992f79a01b0a68967b3aa5f09959d1755ce4` passed both platforms'
warnings-denied all-feature Clippy, workspace/native test compilation and fresh
locked release builds, plus static policy/audit and FreeBSD/WASI/Apple compilation.
Linux focused filters passed, including 342 matching managed/skills tests. The
full CLI suite passed 611 tests with six ignored, including the original actual
release-process archive/reopen regression. The native suite then failed with
4099 passed, one failed and eleven ignored in 47.94 seconds. The child skill-menu
test indexed the first provider request while that collection was still empty
(`reference_host/mcp/tests/managed_host/owner/interactive/skills.rs:303`). A
standalone unchanged-binary run reproduced the same failure in 0.06 seconds.
Logs are `managed-full-runtime-linux-240f992f.log` and
`skill-picker-unchanged-240f992f.log` in the retained gate directory. The remaining
gate stages and fresh whole-feature reviews were not reached. The failing
observation needs an established cause and correction, not acceptance by retry.

Fixture correction `afe8a6cde294298f173b97694e849fa311d1bfc1` establishes that UI
submission can coexist with the previous idle projection and zero provider
requests. It requires the exact `MessageQueued` receipt, then an authoritative
settled inspection whose enqueue event and completed work transition identify
that same message before asserting one provider request and the selected skill
body. The draft and stale-frame assertions remain. No production behavior was
changed. Linux all-feature Clippy rejected this first component's 123-line test;
follow-up `fe282396691351bb6ecee5fb5f3e12c460caf8d5` extracts the cohesive receipt
settlement helper without weakening assertions. Focused validation and the
complete replacement gate remain required before fresh whole-feature reviews.

Integrated `aa6d2c4716e7a22aea07ba52151bde66f40b01dc` passed the Linux full
gate: 611 CLI tests, 4100 native tests, workspace integrations/doctests and all
275 repository Python tests. Static policy/audit and FreeBSD/WASI/Apple
compilation passed. macOS formatting, all-feature Clippy, workspace test
compilation, fresh locked release and focused filters passed, including all four
skill-picker cases. An extra native-only no-run compilation, not used by the
canonical workspace runtime, was intentionally stopped after the required
workspace compilation succeeded; its exit 101 is not reported as a passing
command. The separately completed release build passed.

The macOS full workspace runtime passed 613 CLI tests and 116 CLI integrations,
then failed in the native suite: 4105 passed, two failed and twelve ignored in
1054.50 seconds. Both `terminal_pty::tests::explicit_owned_signal_kills_shell_and_final_drain_preserves_bytes`
and `terminal_pty::tests::full_command_boundary_executes_as_one_argument_and_reaps`
failed during preparation's child-reaping admission, before their intended PTY
assertions. The no-op child remained unobserved through the 500 ms probe window
and the separate 500 ms cleanup window; neither yielded an exact reap receipt.
The remaining workspace integration and doctest stages were not reached. No
whole-feature review, push or full-gate acceptance followed this failure.

Unchanged exact-binary diagnostic runs passed both tests individually (0.09 and
0.42 seconds), then all 27 PTY module tests in serial order (6.67 seconds).
These results do not establish a cause, source correction or full-gate success.
A later host snapshot showed high unrelated CPU load, but does not prove what
delayed the failed children; unrelated processes were not stopped. Logs are
`managed-full-runtime-macos-aa6d2c47.log`,
`pty-unchanged-focused-aa6d2c47.log` and `pty-unchanged-module-aa6d2c47.log` in
the retained gate directory. The failed observation and exact cleanup outcome
remain part of the candidate's evidence, without relaxed deadlines or assertions.

Independent read-only tracing located the failure in the environment-cleared
`/bin/sh -c 'exit 0'` probe, before PTY/inventory helper spawning. Admission,
polling, settlement, cleanup and both failing test bodies are unchanged from
the actual `7cadf2f2` base. Seventeen `Ok(None)` observations, zero interrupted
waits and no kill error precede quarantine; the outer PTY deadline had not
expired. No source defect was established. An unchanged passing replacement
gate cannot be represented as a causal fix for this intermittent observation.

Candidate `dc2198859bd444248cb0a0c78c13df5dbde20214` passed replacement
build/static/platform checks, the complete Linux gate (including 275 Python
tests), and macOS focused tests. A CLI test-listing process waited in
`_dyld_start`, then resumed unchanged; separate signature verification passed.
The full macOS run subsequently failed with 612 CLI tests passed, one failed
and six ignored in 1033.66 seconds. `blocked_output_signals_exit_after_terminal_drain`
timed out waiting for its helper's readiness marker, before sending test signals.
Remaining workspace stages were not reached. No review or push followed.

The unchanged focused test passed in 0.82 seconds. Independent source tracing
found its helper, guardian, scripted stream/output and dependencies unchanged
from `7cadf2f2`; the helper does not compose a native host or DNS/PTY machinery.
A sample from the later, eventually passing composed-session test located its
wait in system DNS configuration's macOS bundle-directory enumeration. That
sample and host-load observations do not establish the failed helper's cause.
Both the original run and diagnostic sample completed without intervention.
Logs are `managed-full-runtime-macos-dc219885.log` and
`cli-ready-unchanged-dc219885.log`; the retained sample is
`machine_god-f2107f13ee78d8d5_2026-09-18_094230_XBcg.sample.txt` under `/tmp`.

The follow-up adds only the helper mode/PID to captured test diagnostics and
the PID to the readiness assertion. It changes neither the ten-second window,
helper behavior nor outcome assertions, and is not a causal fix. Its purpose
is to identify the exact helper for observation if the failure recurs.

### R10 whole-feature review and regression evidence

Candidate `8247ab74939519c7374eb305e3a86411175110ce` passed the complete
replacement local gate: exact Rust 1.94.1 formatting, all-target/all-feature
Clippy, workspace test compilation and fresh locked release builds on Linux
and macOS; static policy, dependency audit, pinned drift and platform checks;
Linux focused/workspace/doctests and all 275 Python tests; macOS focused,
serial workspace and doctests. Linux reported 611 CLI and 4100 native tests
passed; macOS reported 613 CLI and 4107 native tests passed. All seven gate
commands terminated with exit zero. Logs use the `managed-*-8247ab74.log`
suffix in `/tmp/mg-managed-implementation.V0ZGg1`.

The previously failing readiness and PTY cases passed in that exact full run.
This is replacement-gate evidence, not a causal fix for the earlier failures.
Separate samples observed temporary loader waits at `_dyld_start` and an
eventually passing native HTTP-catalog test waiting in system DNS configuration's
bundle-directory enumeration. Those processes resumed unchanged; no unrelated
processes were terminated or deadlines relaxed.

Three fresh independent agents reviewed that exact candidate against actual
base `7cadf2f2ea13ef392797903ad190c0ce3ba92654`. Each reported one finding,
rejecting the candidate despite the passing local gate:

- Correctness/API (`m65_r10_correctness`, P1): a successor prompt can receive a
  terminal admission error while actual prior cleanup or worker capacity is
  pending, yet remain queued for later execution, losing interactive/ACP
  cancellation and request correlation.
- Lifecycle/platform (`m65_r10_lifecycle`, P2): ordinary resident close reports
  success after archiving but before the retired runtime's resources close.
- Performance/resources (`m65_r10_resources`, P2): escaped message and skill
  reference text can form a valid accepted immutable record above 512 KiB,
  which history pagination rejects without advancing its cursor.

Regression-only `f266e4ed` reproduced early close success on Linux. Combined
test-only `fcef25b26bced071a0a6031c12d5438b1649396f` compiled the workspace
and reproduced both foreground/ACP admission failures and history's `Limit`
failure. Logs are `managed-close-red-f266e4ed.log` and
`managed-r10-red-fcef25b2.log`; these are expected failing regressions, not
acceptance. No feature push or remote gate was performed for rejected `8247ab74`.
The correctness and resources reviewers subsequently became repair authors;
they cannot serve as fresh reviewers of their replacement implementation.

The repair series transfers the ordinary close observer and exact archived
receipt into resource retirement (`b9b78936`), retains foreground admission and
its cancellation/request identity across cleanup and cohort-capacity waits
(`d999ee42`), and makes an oversized accepted journal record a bounded lossless
singleton page (`ceda1102`). Tests cover close failure/shutdown/abandonment,
interactive and ACP cleanup/capacity with cancellation and exactly-once
execution, and skill-bearing acceptance through inspection, restart replay,
parent checkpoint, source ACK and outbox clear. External inspection output
bounds remain unchanged. These component commits require integrated green
validation and a new independent review cycle; no acceptance is inferred from
the earlier candidate's passing gate.

On `ea9bae2c`, deterministic Linux close tests passed ordinary closure,
cleanup-error retention and abandoned-observer custody. The new shutdown test
incorrectly expected a pending observer after mailbox closure. Existing mailbox
and saved-lifecycle contracts instead reject that observer with `Unavailable`
while retaining the accepted job's actual cleanup custody. The fixture now
asserts that rejection, retained job, pending shutdown and eventual archived
settlement; production shutdown behavior is unchanged. Both oversized-history
regressions passed, including composed inspection/restart replay/source ACK.
Logs are `managed-r10-pure-focused-linux-ea9bae2c.log` (three passed, one fixture
failure) and `managed-r10-history-focused-linux-ea9bae2c.log` (both passed).
Follow-up `3e606231` passed the corrected shutdown assertion, but the cleanup-error
fixture's direct journal inspection raced the manager's serialized replay read
and returned `Busy`. The close fixtures now establish journal-lane idleness
before direct inspection or fault injection; retired-child state alone is not
that proof. Product behavior and resource gates are unchanged.

Candidate `77316dc2` passed both platform builds, static/platform checks and the
new R10 focused regressions. Its broader Linux focused run then rejected the
older `foreground_next_turn_waits_for_original_run_settlement` fixture: it
expected a failed raw-runtime admission to retain its input for later replay.
The runtime contract already forbids requeuing failed or dropped admissions.
The corrected fixture verifies consumption, no provider call after cleanup
without resubmission, and an exact fresh prompt without the rejected input in
provider history. Interactive/ACP owners separately retain pending admission
and response identity; their four cleanup/capacity regressions passed.

A broader managed diagnostic on the same rejected candidate exposed a second
obsolete fixture assumption in
`retiring_saved_owner_fences_close_and_reopen_but_not_inspect`: it withheld
resource cleanup and synchronously awaited close success before releasing that
same cleanup. The coordinator terminated only that owned diagnostic process
after establishing this dependency in source. The fixture now retains the
pending original response, checks competing close/reopen fencing and inspection,
then releases cleanup before asserting the original success. The diagnostic is
not a passing gate; no production deadline or close-settlement guarantee changed.
The first compile of the strengthened prompt-history assertion (`127ad716`)
caught a testkit wrapper field error; the assertion now reads the recorded
request's `request.messages`. Static and platform checks passed on that rejected
candidate; runtime acceptance still requires the corrected candidate's gate.

### R11 whole-feature review and event-source repair

Candidate `7f0e1eea7853906db3c0293f129e51903f2ff16e` passed the complete
replacement local gate: exact Rust 1.94.1 builds, warnings-denied Clippy, fresh
release helpers, static/dependency/pinned-source and platform checks, full
Linux/macOS workspace tests and doctests, and 275 Python tests. Linux reported
611 CLI and 4111 native tests passed; macOS reported 613 CLI, 116 CLI integration
and 4118 native tests passed. Logs use the `managed-*-7f0e1eea.log` suffix in
`/tmp/mg-managed-implementation.V0ZGg1`.

The original full macOS runtime log ended mid-native test when its tool session
disappeared; the original shell and test processes were also absent. That log
is incomplete, not a passing run. The unchanged replacement is retained as
`managed-full-runtime-macos-7f0e1eea-recovery.log`; its final exact-SHA guard and
`GATE_EXIT ... code=0` prove complete workspace/doctest termination. Earlier
intermittent failures remain unresolved historical observations, not claims of
a causal source fix.

Three fresh read-only agents reviewed the entire feature against actual remote
base `7cadf2f2ea13ef392797903ad190c0ce3ba92654` in isolated clean worktrees:

- Correctness/API (`m65_r11_correctness`): one P2 finding. Events-only inspection
  suppresses an older immutable journal page's read failure, returning successful
  empty/end-of-history evidence. The current head/tail can remain valid while
  history traversal fails. Message and tool-activity sources already report
  errors, but the events source had no corresponding field or projection.
- Lifecycle/platform (`m65_r11_lifecycle`): zero actionable introduced findings.
- Performance/resources (`m65_r11_resources`): zero actionable introduced findings,
  including the bounded Zig provisioning fallback changes.

These are independent source/caller/test reviews, not Bugbot or additional
runtime evidence. The finding rejects the candidate despite its passing gate.
All three review worktrees were verified clean and removed; no feature push or
remote candidate gate followed. Regression-only `6460591d` exercises events-only,
continuation and mixed-source inspection with older fixture-owned pages missing
or corrupt while leaving current head/tail intact.

All three regressions failed as expected on Linux at `6460591d`: the wire
inspection omitted `events_error` despite failed traversal. The retained log is
`managed-r11-events-red-6460591d.log`; this is rejection evidence, not a green
gate. The fresh release helper was selected before execution.

The repair adds explicit `events_error` evidence alongside existing per-source
errors and renders it in the thin CLI. Healthy reads and unselected sources do
not invent errors. Other selected status data remains usable; no false history
cursor or stale-cursor restart is synthesized. Regression fixtures restore their
original page bytes and private permissions before postfailure validation.
The replacement candidate still requires focused and complete local gates and
three fresh whole-feature reviewers before remote acceptance.

### R11 replacement local acceptance

Behavior candidate `e432242b5c17cfdc892cdefe2810a9b8939ab14e` passed the
complete replacement local gate on exact Rust 1.94.1. Linux and macOS selected
their freshly built locked release CLI/helper before runtime checks. Focused
checks passed eight event-history tests (including four source-error cases),
one oversized-history/replay/acknowledgement test and five CLI renderer tests
on each platform. No deadline or assertion was relaxed.

Linux passed 611 CLI and 4115 native tests, workspace integration tests,
doctests and all 275 Python checks. macOS passed 613 CLI, 116 CLI integration
and 4122 native tests, the remaining workspace integration tests and doctests.
The Linux and macOS runtime logs both terminate with the exact candidate SHA
and `GATE_EXIT ... code=0`, after clean-tree guards. Platform runtimes ran
sequentially after owned builds completed. Warnings-denied Clippy, formatting,
FreeBSD/WASI checks, Apple C bindings, dependency policy/audit, documentation,
pinned upstream and generated Unicode checks also passed.

Evidence is retained under `/tmp/mg-managed-implementation.V0ZGg1` in
`managed-*-e432242b.log`. The macOS run remained live through quiet output
intervals and completed without restart. These results do not explain away
earlier intermittent failures or establish an M07 performance claim.

Coordinator inspection also found that the unchanged security overview still
described the removed foreground-only subagent API and deferred already
implemented MCP/ACP ownership. A documentation-only correction aligns that
overview with the normative contracts; it changes no product source or test
behavior. Its bounded documentation checks are separate from the exact behavior
gate above, under the repository's documentation-maintenance exemption. Fresh
whole-feature review and remote acceptance remain required.

### R12 whole-feature review

Three fresh independent read-only agents reviewed candidate
`741bcb0b808339370e155b4894ff2cf4168d59cc` against actual remote base
`7cadf2f2ea13ef392797903ad190c0ce3ba92654`. The candidate differs from the
fully gated `e432242b` only in the implementation plan, this review record and
the security overview; an explicit non-documentation tree comparison was empty.
The existing documentation checker, all ten documentation-policy tests and
diff whitespace checks passed. No additional Rust gate was required for those
documentation-only descendants.

- Correctness/API (`m65_r12_correctness`): zero established actionable findings
  after tracing commands/results, actual invocation and principal claims, FIFO,
  lifecycle/recovery, frozen policy, relationships, inspection, notice delivery
  and shared-host/CLI/ACP composition.
- Lifecycle/platform (`m65_r12_lifecycle`): one P2 finding. A transient original
  notice-outbox-clear failure parks its exact future behind the manager retry
  fence. The existing `shutdown_clear_committed_error_requires_original_retry`
  fixture demonstrates that shutdown cannot finish before explicit retry.
  Production interactive, one-shot and ACP retirement loops neither consume the
  blocked status nor invoke the native reconciliation retry API. Storage can
  recover while EOF/quit still waits indefinitely. Repair must retain the
  original receipt/owner, distinguish pending workers from recovery-required
  fences, and provide usable interactive and deliberate headless recovery without
  model-work replay or premature cleanup success.
- Performance/resources (`m65_r12_resources`): one P2 finding.
  `Reservation::refresh` in `managed/store/transaction.rs` rescans every retained
  page and rereads every head after each confirmed journal publication, under
  the exclusive operation slot. Repeated append cost grows with lifetime history,
  delaying unrelated commands, tool observations and durable cancellation intents.
  Repair must preserve exact byte/entry/orphan and settlement/ACK accounting while
  removing full-history scans from ordinary successful publications.

The resource reviewer ran an authorized standalone macOS diagnostic reproducing
directory iteration and page `openat`, three `fstat`, `fstatat` and `close` calls.
Private page stand-ins fit the accounting path; the scan does not read page
payloads. At 1,000/8,000/32,000/64,000 entries, observed scan times were
24–27 ms / 219–270 ms / 1.19–1.67 s / 2.64–3.34 s. These are not product
benchmark measurements: ACL checks, head decoding, publication/sync and scheduling
were omitted. Source and results remain in
`/tmp/mg-r12-scan-diagnostic.dWNArv`; generated fixture files were removed after
measurement and can be regenerated. No product files or build inputs changed.

Both findings reject the candidate. These reviews were source/caller/test
inspection, with the separately scoped diagnostic above, not Bugbot or new
runtime-suite acceptance. All three clean review worktrees were removed; no
candidate push or remote acceptance followed. Repair work is separated into
journal accounting and native/CLI/ACP recovery lanes. The replacement requires
regressions, the complete local gate and three new whole-feature reviewers.

### R12 repair implementation

Journal repair `e7080f97` replaces publication inventory scans with exact
before/after accounting for at most four touched names, preserving protected
settlement/ACK credits and original ambiguous-receipt custody. Eight regressions
cover constant accounting work, cached/inventory equivalence, staging/orphan reuse,
failure after accounting, namespace invalidation, damaged references, bounded
missing/restored history and independent low-head pressure. The default-feature
native all-targets Rust 1.94.1 compilation passed; it is not the full local gate.

The host recovery lane adds typed independent retry fences and one shared native
one-second timer, including during shutdown. It retries only retained operations,
never a new provider turn or implicit queued-work resume. Ctrl-R has recovery
priority before ordinary navigation admission while recovery is required.
Tests layer an exact committed-but-error notice-clear adapter and deterministic
clock assertions with ordinary filesystem-failure composition through native,
one-shot, ACP retirement and raw CLI owners. Raw cases retain an unacknowledged
output frame and exercise recovery input or EOF; these are composed-driver tests,
not claims of a live-provider production-executable fault injection.
These implementation records do not establish runtime or replacement acceptance.

Candidate `311ea3b6` passed exact Linux compilation/release preparation and 33
focused accounting, capacity, retry-state, input, manual-recovery and event/history
tests. Static and platform checks passed. Its new injected-clock test failed:
after an already-committed clear returned an error, the first retry reconciles the
newer stored revision and rejects the stale expected revision without saving.
The test incorrectly expected a save on that first tick. An uncaptured diagnostic
reported `saves: 6 != 7`; fixture shutdown then parked during panic unwinding.
Both confirmed-failed test processes were explicitly terminated and their logs
retained. This rejects the candidate; neither quiet waiting nor termination is a
passing test result. Repair must exercise separate rate-limited revision-repair
and save steps with observable failure diagnostics, preserving the original
receipt and actual successful cleanup. It must not bypass automatic recovery by
manually refreshing the session revision, or change production retry behavior to
satisfy the incorrect expectation.

Candidate `f5af0367` corrects that test to observe separate readback and save
stages, defer assertions until settlement, and report bounded failure diagnostics.
Its exact Rust 1.94.1 Linux and macOS formatting, warnings-denied Clippy, workspace
test compilation and locked release preparation passed, as did static/platform
checks. All 34 focused Linux regressions passed, including the repaired clock
test. The old macOS `311ea3b6` release build completed, but its final SHA guard
correctly rejected the changed checkout; the separate exact `f5af0367` build
passed. That old guard failure is not current-candidate acceptance.

The replacement Linux runtime gate then passed 54 store, three retry-gate and
17 notice/shutdown tests before the composed native-owner recovery case timed
out at `tests/interactive_session/support.rs:453`. A focused exact-candidate
macOS diagnostic reproduced the same setup timeout. Both processes exited 101;
neither complete runtime suite passed. The fixture blocked the foreground session
publication after observing its provider request, before confirmed turn completion,
and never observed its required `NoticeOutboxClear` fence. The repair must isolate
the intended metadata-clear failure from foreground finalization, retain the
original delivery evidence and successful cleanup assertions, and preserve the
existing deadlines. No new reviewers or remote acceptance followed this failure.

The shared-fixture repair waits for a successful foreground `Completed` outcome,
which releases the runtime's active lease, then blocks the same parent session
before the next manager poll. It checks the original outbox's parent/checkpoint
identity, nonempty source notices, unchanged retained outbox at the clear fence,
and unchanged provider count. Failure diagnostics include runtime and manager
state. This is a test-ordering repair, not a production recovery change; all five
composed owner/ACP/one-shot/raw-input cases still require replacement execution.

Candidate `86697a01` passed exact Linux/macOS build preparation and static/platform
checks. Its Linux runtime gate again passed the 74 store/retry/shutdown tests,
then failed immediately because the completed parent had no original notice
outbox. This rejected the candidate and showed the completion barrier alone was
insufficient. Diagnostic-only `94deddf8` traced the parent checkpoint, completed
record and provider request: none contained notice context. The batch was absent
at prompt admission, not cleared early.

Diagnostic-only `56ca38e5` added one ordinary inspect command targeting the exact
child before the single parent prompt. Command admission waits for that child's
pending journal writes, and the manager stages notices before admitting commands.
The trace then showed the original Started and Completed notices in the parent's
model-visible context, followed by the exact retained outbox, real clear failure,
bounded retry and successful shutdown. That focused Linux case passed; it does
not establish full-gate acceptance. The lasting repair must retain inspection,
notice identity and outbox assertions without the temporary trace output, and all
five composed recovery cases still need verification on the clean replacement.

Clean worker candidate `da135eb9` retained the receipt/source/model-context checks
without trace output. Exact Linux test compilation and release preparation passed;
the native-owner and ACP-retirement focused cases passed. The one-shot case then
failed before the intended clear because its wall-clock-created session was driven
with the shared fixture's fixed `100/101` timestamps, violating native session
metadata ordering. Raw-input cases did not run after that failure. The helper must
derive its test times from the actual session metadata and rerun all five cases;
neither this partial run nor a different-owner pass establishes final acceptance.

Worker candidate `0d148dd9` derives the helper's source timestamp from the actual
session metadata and uses a checked increment for the parent prompt. Linux build
preparation passed; native-owner, ACP-retirement and one-shot recovery cases
passed. The raw Ctrl-R case reached native closure without provider replay, then
timed out in final presentation cleanup: its fixture had consumed a deliberately
held flush without ever acknowledging it. This is not a successful raw test or a
reason to bypass output completion. The recovery-specific harness must acknowledge
that original flush only after proving native/host cleanup while output remains
blocked, then rerun all five focused cases on the clean replacement.

The separate EOF diagnostic reproduced the same final held-flush timeout after
native closure. Clean worker candidate `aca52045` fixes only the shared fixture
and the two recovery-specific raw test bodies: exact-child inspection, actual
session timestamps, matching model-visible and retained notice identities, and
release of the original flush acknowledgment after host cleanup. Exact Linux
workspace test compilation and locked release preparation passed. All five focused
cases passed on that SHA: native owner, ACP retirement, one-shot settlement, raw
Ctrl-R and raw EOF. Temporary trace output is absent. These scoped results justify
integration, not feature delivery; the integrated candidate still requires the
complete replacement local gate, three fresh reviews and exact remote gates.

Integrated candidate `2d53c5a1` passed exact Linux/macOS build preparation,
static/platform checks and the complete Linux runtime/Python gate. All five
composed recovery regressions also passed on macOS. Its full macOS native run
then failed seven terminal tests: the exact inventory-wrapper dispatch case
and six PTY cases. Five PTY failures exhausted a direct `/bin/sh -c 'exit 0'`
reap probe and its bounded cleanup; another exhausted inventory-service
readiness. The inventory-wrapper case observed its first bytes at 248.77 ms,
without EOF before the original 250 ms deadline. The remaining native result
was 4,129 passed and 12 existing ignored fixture entries. This was not a
successful full macOS gate.

Unchanged focused diagnostics passed all 27 PTY tests and the inventory-wrapper
case. A single unchanged full macOS rerun passed all six previously failing
PTY cases but reproduced the inventory-wrapper failure: zero observed bytes or
EOF before expiration at 252.61 ms, with the child still running at the last
observation. That native result was 4,135 passed, one failed and 12 existing
ignored fixture entries. The source audits found no demonstrated causal change
in the probe, collector or helper dispatch; neither an isolated pass nor this
partial full run establishes a source fix or acceptance.

Diagnostic-only `9b518c7e` retained the same wrapper, test-executable entrypoint,
arguments, protocol and deadline, adding opt-in bounded timing datagrams on a
separate channel. Its three exact-case attempts passed, passed, then failed.
Passing valid invocations reached helper entry after 87.49/151.57 ms, with
process enumeration taking 2.46/4.15 ms. The failing invocation expired at
252.73 ms with no protocol bytes or received helper-entry marker; the same
receiver subsequently recorded the deliberately expired helper. These
best-effort markers narrow the observed failing interval but do not prove that
helper entry never ran or identify an operating-system cause. Instrumentation
has observer overhead and is investigation evidence, not an acceptance result.
The fixture re-executed the approximately 154 MiB native test binary even when
the approximately 16 MiB production release helper was explicitly selected.
The clean fixture repair must honor that selection without weakening exact
argument validation, canonical output, original deadlines, EOF or positive reap.
Diagnostic instrumentation is not part of that lasting repair.

Clean worker candidate `3e2bcdf0`, based directly on `2d53c5a1`, changes only
fixture helper selection and three pure selector regressions. When an explicit
release helper is supplied, the wrapper executes that helper with the exact
private flag and raw production stdout; absent selection retains the explicitly
registered debug entrypoint. Invalid explicit selection does not generate a
debug fallback. The original four-case runtime test body is unchanged, including
the shared 250 ms deadline, canonical PID output, EOF and reap requirements.
Exact Rust 1.94.1 native test compilation passed, the actual fixture passed on
its first run in 0.10 seconds, and all three selector regressions passed.
No timing instrumentation is carried into this clean patch. These focused
results justify integration, not full feature or causal-root-cause acceptance.

Integrated `03a2f49f` passed the exact build/static/platform gates and complete
Linux workspace, doctest and 275-test Python gate. Its first macOS gate passed
the three selector regressions but failed the valid inventory fixture with zero
bytes and no EOF at 250.68 ms; the last observation still reported a running
child. The deliberately expired helper also exhausted its collection window,
while shell-side missing/extra argument rejection completed promptly. The full
macOS suite had not started. Three unchanged, externally observed focused runs
passed, but no child remained visible long enough for the observer to capture
a stack. Observation overhead is acknowledged; these are diagnostics, not a fix.

A read-only audit found that the selected release path, private flag, raw stdout
and original deadline reach the wrapper's exec attempt without fallback. That
does not prove entry into Rust `main`. The expired stamp is rejected before
process enumeration, so enumeration alone cannot account for both stalls. No
introduced collector, deadline codec or reaping change was established.

One unchanged macOS gate attempt passed every focused check, including the
inventory fixture, and passed that fixture again in the full native suite.
CLI results were 617 unit tests and 116 integration tests passed, with six
existing ignored unit fixture entries. Native results were 4,137 passed, two
failed and 12 existing ignored fixture entries in 626.45 seconds. The failures
were `durable_history_resize_matches_the_real_pty_and_survives_recovery` and
`expired_eof_observation_cannot_hide_a_retained_running_child`. Both failed
before PTY preparation: their direct `/bin/sh -c 'exit 0'` admission probes
remained running through 16/17 non-interrupted wait observations and then
exhausted the separate bounded kill/reap cleanup window. Both exact PIDs were
absent when checked after the suite ended. This is another rejected local gate,
not managed-agent acceptance or evidence of a causal source fix.

Two unchanged externally observed focused runs each passed all 27 PTY tests.
A separate Rust 1.94.1 diagnostic completed 64 direct shell probes without
exhausting either observation window. Its polling is deliberately identified
as a standalone diagnostic, not the product's cancellation/reaping machinery.
These results do not distinguish whole-suite effects from intermittent host
startup/exit delays. A passing retry must not erase either failed gate or be
presented as explaining it.

A serial full-native diagnostic on the same product source then passed 4,139
tests with zero failures and 12 existing ignored fixture entries in 815.63
seconds. External observers watched owned children and exact probe-timeout
messages; neither captured a stalled probe. A parent-process sample showed
active terminal-host test execution, not a demonstrated deadlock. This run
used `--nocapture` and external observation, so its timing includes diagnostic
overhead. It is not a complete workspace gate and does not establish a cause
or remediation for either earlier macOS failure.

The complete externally observed macOS gate on documentation-only descendant
`6cca5d49880f5f61ee05f363b32085a20be45905` then passed all focused checks,
workspace tests and the separate workspace doctest command, including its final
exact-SHA and clean-tree guards. The observer exited zero after 1,565.109 seconds;
the full native suite passed 4,139 tests with 12 existing ignored fixture entries
in 823.17 seconds. CLI unit and integration results were 617 and 116 passed,
respectively, with six existing ignored unit fixtures. No stalled-child stack
capture was produced. The complete log and external observations remain under
`/tmp/mg-m65-pty-reap-observation.UVN13K/full-gate-6cca5d49/`.

An explicit non-documentation comparison with `03a2f49f` was empty. Its previously
passed exact Rust 1.94.1 Linux/macOS build, static/platform and full Linux runtime
gates therefore apply to the same product/test source, including all 275 Python
tests. This completes local regression acceptance for a fresh whole-feature
review; it is not remote acceptance, a performance claim or a causal explanation
for the retained macOS failures. No deadlines, test selections or product source
were changed for the passing macOS retry; external observation adds overhead.

### R13 whole-feature review

Three fresh independent read-only agents reviewed candidate
`73c37802867e4492741204336201cf23f1108bfc` against actual remote base
`7cadf2f2ea13ef392797903ad190c0ce3ba92654`. The candidate adds only this
historical gate record and the implementation-plan transition to the fully
gated product/test source. The documentation checker, ten documentation-policy
tests and whitespace checks passed before review.

- Correctness/API (`m65_r13_correctness`): one P2 finding. Resume authorizes the
  frozen FIFO-head work configuration, but nonresident runtime preparation uses
  the mutable child-head configuration instead. An accepted Ask work interrupted
  before a later authorized Yolo configuration change can be resumed by an Ask
  caller while resident, but the same caller receives `HostUnavailable` after
  restart/eviction because factory selection checks the newer Yolo configuration.
  Restore the authorized work's exact frozen configuration without changing the
  saved defaults for future accepted work or weakening caller-policy admission.
- Lifecycle/platform (`m65_r13_lifecycle`): zero established actionable findings
  after tracing admission/settlement, workers, principal/MCP leases, preparation,
  recovery, blocked input/output and interactive/one-shot/ACP shutdown. A proposed
  maximum-millisecond Instant overflow was not established on supported platforms;
  pinned Rust uses signed 64-bit seconds for Linux and macOS monotonic instants.
- Performance/resources (`m65_r13_resources`): zero established actionable
  findings after scheduler/dependency, residency, mailbox, journal accounting,
  paging, notices, undo/MCP ownership and blocked-output inspection.

This is source and test inspection, not new runtime measurements or Bugbot.
The correctness finding rejects the candidate; no feature push or remote
acceptance followed. All three clean review worktrees were removed after the
reviews completed. The retained intermittent macOS failures remain unexplained.

The repair regression first ran against unchanged production behavior. Its
resident case passed, then the nonresident case captured the newer child name,
model, effort and Yolo mode with an Ask caller origin instead of the exact frozen
Ask work configuration. The separate frozen-policy escalation-denial regression
passed. This is a manager/factory-request mismatch reproduction, not a composed
production `HostUnavailable` observation. The red run exited 101 with one passed
and one failed test; its log remains under
`/tmp/mg-m65-r13-repair.11HqeQ/frozen-resume-red.log`.

The repair makes restoration configuration explicit: Resume supplies the
authorized work configuration, while new messages supply the current child
defaults. Captured caller origin, escalation checks, publication custody and
durable defaults are unchanged.

Worker repair `f4f645a3` passed exact Rust 1.94.1 native all-feature test Clippy
with warnings denied, formatting, both resume regressions, the separate actual
factory restore regression, all 116 manager tests and all 22 shared-factory tests.
The latter runtime groups ran serially with the explicitly selected production
helper. The manager tests cover resident/nonresident/journal-owner restart,
preserved future-message defaults and frozen-policy escalation denial. The
factory test separately checks actual restore selection, transcript identity,
policy rejection and absence of provider execution; it does not turn the manager
double into a composed `HostUnavailable` reproduction.

Initial test-accessor compilation errors and a fixture constructor's one-line
Clippy overrun were corrected before these passes. The latter repair extracts
identical private-directory setup without suppressing lint or changing effects.
Logs and diagnostic excerpts remain under `/tmp/mg-m65-r13-repair.11HqeQ/`.
These focused results are not replacement feature acceptance: the complete local
gate, three fresh whole-feature reviews and exact remote gates remain required.

### R13 repair replacement local gate

Integrated repair `d22691a3c8b544ac9b336b673032b4674ac8fd1f` passed the
exact Rust 1.94.1 Linux/macOS build, static and platform gates, complete Linux
workspace and doctests, and all 275 Python tests. Linux native results were
4,132 passed with 11 existing ignored fixtures; CLI results were 615 unit and
121 integration tests passed, with six existing ignored unit fixtures.
Stage logs remain under `/tmp/mg-managed-implementation.V0ZGg1/` as
`managed-{stage}-d22691a3.log`.

The first macOS attempt failed the focused inventory fixture before the full
workspace suite: valid and expired-stamp launches produced no bytes or EOF
within their original deadlines. Exact syspolicyd evidence records a concurrent
680 ms XProtect scan of the selected release helper. This supports launch delay
but does not establish where either failed process stopped before an entry
marker. Read-only source inspection established no introduced dispatch defect.

An unchanged full macOS retry passed every focused check, workspace tests and
the separate doctest command, with the final exact-SHA/clean-tree guard and
`GATE_EXIT` both successful. Native results were 4,142 passed and 12 existing
ignored fixtures; CLI results were 617 unit and 116 integration tests passed,
with six existing ignored unit fixtures. The observer ultimately returned zero.
Logs and samples remain under
`/tmp/mg-m65-pty-reap-observation.UVN13K/full-gate-repeat-d22691a3/`;
the rejected attempt is retained in sibling `full-gate-d22691a3/`.

Host power logs show repeated thermal-emergency and maintenance sleeps during
the retry. Samples separately captured two test processes in `_dyld_start` and
one CLI fixture in directory enumeration during system DNS initialization, not
a demonstrated managed-runtime deadlock. The high-frequency external observer
was paused without stopping the gate; a separate diagnostic watcher was stopped
after native tests passed. Resuming the observer after gate completion reaped
the finished gate process and returned its successful exit. No deadlines, test
selections, source, power settings or operating-system protections were changed.
These observations qualify wall time; they do not prove a causal fix for the
retained intermittent failures. This is local regression evidence, not remote
acceptance or an M07 performance claim.

### R14 whole-feature review

Three fresh independent agents reviewed frozen candidate
`5b6bdcf22773acd9b96ffe4b25ca48cd807088da` against actual remote main
`7cadf2f2ea13ef392797903ad190c0ce3ba92654`. Its non-documentation diff from
fully gated `d22691a3` was empty; documentation checks and ten policy tests passed.

- Correctness/API (`m65_r14_correctness`): zero actionable introduced findings
  after command/schema/result, authentic admission, FIFO/recovery/frozen work,
  relationship publication, paging and host/CLI/ACP integration inspection.
- Lifecycle/platform (`m65_r14_lifecycle`): zero actionable introduced findings
  after child/foreground settlement, factory/worker/reap custody, principal MCP
  and prompt leases, startup/replacement/shutdown and frame/flush inspection.
- Performance/resources (`m65_r14_resources`): one P2 finding. Replenished
  successful configuration changes on an idle sibling set the global replay-reset
  flag. With one admitted operation per poll, mailbox and replay admissions can
  alternate, but every replay admission discards its previous catalog/source
  frontier and performs only the initial catalog/inspect step. An unchanged
  source's confirmed unacknowledged completion notice is never exposed during
  that workload. Replay resumes after traffic stops or storage pressure rejects
  writes; this is starvation during valid traffic, not irreversible loss.

The proposed deterministic regression preserves an original through manager
restart, registers its exact parent context, and continuously replenishes four
admitted sibling configuration requests. The original must become visible before
that traffic ends. Static inspection established the finding; reviewers ran no
new builds or tests. The candidate is rejected and was not pushed.

Both resource and lifecycle reviewers dismissed a separate cleanup-timeout
hypothesis: cached terminal MCP cleanup failure is the explicitly documented and
tested fence, not a retry that may reset its deadline. Reviews were ordinary local
source/test reviews, not Bugbot or runtime/performance measurements.

The actual admitted-mailbox regression reproduced the finding on unchanged
production source: after 64 successful sibling configuration replies, the
original was still absent; it became visible once that traffic was stopped.
Exact Rust 1.94.1 reported zero passed, one failed and 4,154 filtered tests in
1.57 seconds, with the expected starvation assertion. The red run exited 101.

The repair coalesces invalidations into a later sweep without discarding current
catalog progress. Exact-head conflicts discard only the changed source; final
validation and visibility share serialized admission. Capacity-delayed originals
are revalidated after intervening writes, including each confirmed source ACK
within an unfinished multi-source delivery. Unchanged capacity waits remain inert.
Invalidation before the first catalog read coalesces with that initial scan,
instead of scheduling a redundant second sweep.

Focused exact Rust 1.94.1 runtime checks passed all 138 manager tests in 92.81
seconds, 31 notice tests and 25 prompt-context tests. New regressions cover the
replenished admitted-write workload, capacity-delayed ACK/archive and parent
retirement, changed-source traversal/revisit, late-created sources after an
exhausted catalog, initial-registration coalescing, and actual partial two-source
checkpoint acknowledgement.
An initial new capacity-fixture failure retained a weak deadline subscription
after replacing its notice registry. The corrected fixture releases that old
subscription so normal pump setup subscribes to the replacement; production
code and deadlines were not changed to fix the fixture. The first manager run
had 135 passing tests and that one failure; its log is retained alongside the
passing rerun under `/tmp/mg-m65-r14-repair.PPp71e/`.
Worker commit `4b81ef885056ae49e63fdeba60d4415134cb1484` also passed final
formatting and native all-feature test Clippy with warnings denied. The exact
commands, retained log index and focused-check limits are in that directory's
`evidence.md`. The repair worktree was clean with every execution handle closed
before integration.
These are focused repair results, not complete feature or remote acceptance.
