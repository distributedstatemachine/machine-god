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
