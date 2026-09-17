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
