# Complete MCP CLI review history

This is historical evidence for the complete MCP feature, not a live delivery
ledger. The [implementation plan](../implementation-plan.md) owns current phase,
delivery identifiers and the next gate. Component commits are not deliveries.

## R1 candidate and local evidence

Candidate: `8583cd37585d82274cdec84994147dd48e1672eb`.
Review base: `6736070cc70ff6040cfa92275e80a0e9a5172852`.
Scope: the complete 484-file MCP feature diff, not only its last test fixes.

The exact candidate passed the full Rust 1.94.1 local gate: macOS serial and
Linux default-concurrency workspace/doc tests, fresh release CLI smokes on both
platforms, formatting, strict Clippy, repository Python checks, pinned upstream
and Unicode drift, dependency audit/policy, portable compilation and unsafe
conformance. Python reported 269 tests with 14 skips. CLI unit tests reported
511 passes and six ignores on both platforms; CLI integrations reported 111
passes on macOS and 116 on Linux. macOS native units reported 3,329 passes and
12 ignores. No remote acceptance follows from these local results.

Local logs were retained under `/private/tmp/mg-mcp-full-gate.hE0Sfz`, especially
`macos-8583-complete-gate-replacement.log`, `linux-8583-complete-gate.log`,
`linux-8583-release-smoke.log`, `python-8583-gate.log` and
`auxiliary-8583-gate.log`. The earlier macOS log without `replacement` records
an incorrect diagnostic binary path, not a successful gate.

Earlier rejected local runs exposed stale catalog search fixture geometry,
same-label test-directory deletion races under Linux concurrency, and an
immediate pipe-closure assertion sensitive to unrelated concurrent spawning.
The repairs respectively separated cardinality/retained/model budgets, used
exclusive unique directory ownership, and isolated three immediate pipe
assertions in bounded exact child tests without reducing parent concurrency or
weakening the assertions. The original unrelated pipe reader was not captured;
passing unchanged retries alone were not accepted as a repair. All three repairs
were included in the complete successful candidate gate above.

## R1 independent local review results

Three fresh reviewers received isolated checkouts of the same candidate after
the complete local gate. They were ordinary independent local reviewers, not
Bugbot. The review covered correctness/API, lifecycle/platform and resources.

| Track | Concrete finding |
| --- | --- |
| Correctness/API | Tool-call progress notifications accumulate across calls in the retained 64-entry peer queue. Ordinary calls and continuations need notification consumption without losing subscription invalidation. |
| Correctness/API | HTTP tool calls retain the discovery decoder's 65,536-node cap instead of selecting the complete tool-result codec's 262,144-node capacity. |
| Correctness/API | Human feature demand does not activate optional servers deferred by the supported native `AskStartup` phase. Ordinary interactive CLI startup uses `All`; that path was not claimed to reproduce this issue. |
| Lifecycle/platform | Configured stdio startup accepts positive `u32` milliseconds, but the shared helper encoder and decoder impose the terminal's 600-second cap before server launch. |
| Resources, partial | The execution contract incorrectly promises legacy URL-error input custody, although the result decoder keeps such errors as protocol failures. |

The correctness and lifecycle reviewers completed source review and prepared
coordinator-owned bounded diagnostic fixtures. Their final reports did not claim
runtime execution. The resource reviewer ended with a tool error before a final
report; its partial observations are not a completed third-review verdict.
R1 therefore did not satisfy acceptance.

The coordinator's original-candidate macOS startup diagnostic subsequently
completed: the immediate producer succeeded at ten seconds and failed with
`Transport(Process)` at 1,200 seconds. Cleanup settled before the assertion.
The log additionally located a 600-second inventory-bootstrap encoder ceiling,
before the captured-helper launch ceiling. The repair preserves that inventory
substage ceiling under the original deadline while allowing the configured MCP
deadline through its own helper. Evidence: `r1-lifecycle-diagnostic.log` in the
same retained directory. This baseline failure is not replacement-gate evidence.

The original-candidate notification diagnostic also reproduced the reported
failure: in one selected turn, the first tool call with 33 progress notifications
completed, while the second returned a tool error. Owned cleanup completed before
the assertion. Evidence: `r1-notification-diagnostic.log`; the diagnostic source
is retained on `agent/m63-mcp-r1-notification-diagnostic` at `f5637596`.

## Remediation decisions

Backward compatibility is not required. Correct the obsolete continuation
promise rather than restoring legacy behavior. Remove context-free compatibility
trait forwarding while preserving useful injected catalogs and feature handlers
through required explicit-context hooks.

Resource source inspection also identified repeated string-length scans and
uncharged character-class range comparisons. Store decoded scalar counts once
per parsed value and charge class comparisons to the existing shared pattern
budget. Small deterministic Unicode, retained-accounting and work-bound tests
cover these changes; no timing or measured speedup claim is made.

Replacement code requires its own focused checks, complete local gate, three
fresh completed independent reviews and exact remote evidence before delivery.

## Replacement candidate local findings

Candidate `7cd486a6ff38cd9ec4ad932dd1de89d73c35a7a0` passed pinned formatting,
strict workspace Clippy, compilation of all test targets and fresh release builds
on macOS and Linux. Pinned-source/Unicode drift, dependency audit/policy and the
selected FreeBSD/WASI Clippy checks passed. Repository Python reported 269 tests
with 14 skips; rendered documentation and the bounded policy check passed.

The focused macOS MCP batch then reported 785 passes and two failures, so this
candidate did not reach the complete runtime or fresh-review gates. Evidence is
retained in `macos-7cd-build.log`, `linux-7cd-build.log`,
`auxiliary-7cd-replacement.log`, `python-7cd-gate.log` and
`macos-7cd-focused.log` under the same evidence directory. The initial auxiliary
attempt failed because its old upstream checkout was unreadable; a fresh checkout
of the identical immutable pin supplied the successful replacement drift check.

- The no-responder continuation fixture assumed no clock reads after receiving
  the wire response. Notification settlement now legitimately checks the original
  operation deadline there. Compare against an ordinary complete-response control
  and require unresolved input to add no interaction-clock work; keep the exact
  result and single-request assertions.
- The new HTTP node-budget fixture used 70,000 null values, whose approximately
  350 KiB encoding exceeded the engine's independent default 256 KiB cumulative
  result-byte limit before a completion event could be emitted. Use compact
  values to exercise the same node count and actual archive path without crossing
  that unrelated byte boundary. Product limits remain unchanged.

These are fixture corrections, not proof of replacement runtime acceptance.

Candidate `b7f1bfc5a1762411ab63ae5af793eacd69a39b33` passed the replacement
focused batch, including all 787 native MCP tests, contextual integrations,
composed CLI cases and the captured-helper deadline regression. Both platform
builds, strict Clippy and auxiliary checks passed. Full runtime gates then
reported separate terminal/background lifecycle failures:

- Linux default concurrency: 3,337 native unit tests and 116 CLI integrations
  passed, but `blocked_deadline_waker_tail_retains_capacity_until_callback_returns`
  failed during its recovery poll; the other 97 tests in that integration suite
  passed. Its earlier blocked-callback capacity assertions had passed. The panic
  did not retain the returned output, so its exact status was not observed.
- macOS serial execution: 3,338 native unit tests passed, one failed and 12 were
  ignored. `abort_and_drop_revoke_pipe_authority` received `Cleanup` from
  `abort_and_reap`. An unchanged isolated retry subsequently passed in 0.33 seconds;
  that retry does not establish a cause or repair.

Evidence remains in `macos-b7f1bfc5-focused.log`,
`linux-b7f1bfc5-complete-gate.log`, `macos-b7f1bfc5-complete-gate.log` and
`macos-b7f1bfc5-abort-diagnostic.log` in the retained directory. Neither failed
full run reached its remaining integration, doc-test or release-smoke commands.

The terminal fixture source assumed an admitted pending executor could not
return a tool-level timeout during its first poll. The admission-inclusive
deadline permits that outcome. Its correction retains the 20 ms execution and
two-second recovery bounds and all blocked-tail assertions, requires exact
executor admission/poll/drop evidence for recovery, and validates only the empty
timeout projection. Pre-admission timeouts do not establish recovery. A second
case deliberately reaches the original deadline during the admitted recovery
poll. This remains a fixture correction requiring runtime verification, not an
assertion about the original unlogged output or a product deadline change.

The terminal correction subsequently passed both focused deadline-waker cases
and the full 99-test terminal integration suite with Linux default concurrency,
strict Clippy and a fresh canonical release helper at `3d2a64fa`.
Evidence: `linux-3d2a64fa-terminal-focused.log`. This focused success does not
replace the complete feature gate.

Source investigation of the separate macOS abort failure did not establish a
root cause. Its cleanup path uses bounded process-group snapshots and direct-child
reaping, not the MCP startup inventory helper. The original captured output had
neither existing collector-failure nor scan-expiry diagnostics. Failure-only
test instrumentation now distinguishes aggregation sites, snapshot failure stages,
quiescence rejection and reap outcomes/custody. It adds no production logging,
clock/process observations, changed deadlines or error outcomes; it gathers
evidence rather than claiming a repair for the unclassified original failure.

## Second full-feature review

Candidate `6bceb863376748247316fcb05dea692c6ba28166` passed the complete
replacement local gate: focused checks, pinned builds and strict Clippy, macOS
serial and Linux default-concurrency workspace/doc tests, fresh release smokes,
portable checks, dependency audit/policy and pinned-source drift. Python reported
269 tests with 14 skips; rendered documentation and bounded policy checks passed.
Evidence remains in the `macos-6bceb863-*`, `linux-6bceb863-*`,
`auxiliary-6bceb863-gate.log` and `python-6bceb863-gate.log` files in the retained
evidence directory. The prior macOS abort failure did not reproduce; this does
not establish its original cause.

Three fresh independent local reviewers compared the entire feature against
`6736070cc70ff6040cfa92275e80a0e9a5172852`. They performed source review, not
runtime reproductions or exhaustive line-by-line inspection of every changed
file. All three completed their assigned tracks and rejected the candidate:

- `mcp_r2_lifecycle`, P1: conversation admission waits for authentication refresh
  with a newly created token instead of the runtime's actual preparation token.
  Cancelling an admitted prompt or beginning quiescence therefore cannot stop
  that waiter while shared refresh is pending. Forward caller cancellation
  without cancelling the controller-owned shared refresh job.
- `mcp_r2_correctness`, P2: deferred tool-name allocation omits original builtin
  reservations when a direct runtime caller supplies no new reservations. The
  controller path resupplies them and is unaffected. Preserve the original set
  during allocation, not only in the resulting publication.
- `mcp_r2_resources`, P2: full replacement clears old-generation byte accounting
  when peers drain even though old tool routes/bindings can remain owned by
  registrations or native call continuations. Same-peer refresh tracks these
  owners weakly; full replacement does not. Arbitrary external snapshots could
  have a separate caller budget, but the finding also concerns retained native
  operation ownership. Extend accounting without duplicating peer charges.

These are source-supported findings, not measured performance claims. Repairs
require deterministic regressions, the complete replacement local gate and
three newly assigned reviewers before remote delivery.

The three reviewers subsequently authored non-overlapping repairs in isolated
worktrees: `2d060daa` forwards admission cancellation, `4867c07f` inherits builtin
reservations during allocation, and `0cbd9059` retains full-reload generation
accounting. These authors cannot count as fresh reviewers for the replacement.
Regressions cover active cancellation and quiescence during blocked shared
refresh, initial-only name reservations, retained native calls/bindings/server
routes, mixed drained and pending generations, byte/count rejection without
publication changes, and capacity recovery after final owner release. Their
formatting checks passed; build and runtime acceptance require the replacement
gate rather than the rejected candidate's evidence.

Integrated repair candidate `c86eff571ade84874016e37b5770758ad77d3b0e` passed
pinned formatting, strict workspace Clippy, all test-target compilation and fresh
release builds on both platforms. Drift, audit/policy and the three portable
Clippy selections passed. The focused macOS batch passed both admission-cancel
cases, the naming regression and four retirement-accounting cases. Its fifth
retirement case failed before the intended suspended-call boundary: the executor's
exchange flag was still false after one poll of the collected turn. This is the
observed failure, not proof of an accounting defect or of its cause.

A subsequent complete MCP diagnostic reported 794 passes and the same one
failure in 24.63 seconds. The candidate therefore did not pass its focused gate.
Evidence remains in `macos-c86eff57-build.log`, `linux-c86eff57-build.log`,
`auxiliary-c86eff57-gate.log`, `macos-c86eff57-focused.log` and
`macos-c86eff57-mcp-diagnostic.log`. No fresh product-review acceptance or remote
delivery follows from these partial results.

The remaining contextual/CLI/background/terminal focused diagnostic also passed
(`macos-c86eff57-remaining-diagnostic.log`). Source inspection explains why the
fixture's one-poll assumption is invalid: the actual submission writer wakes its
caller and returns `Pending` after writing, before a later flush poll. The test
now drives those real wakeups until the explicit post-exchange marker, rejects
premature turn completion and checks the actual `tools/call` request for `lookup`.
Peer-lane release, retained charge and final-owner cleanup assertions are unchanged.
Worker repair `504bec35` changes only the fixture; its runtime verification remains
required and is not inferred from this scheduling explanation.

Candidate `7aa6be7059a6567c2df52aca37a51bd20a63b46f` passed both platform
builds, strict Clippy and auxiliary checks. All eight R2 regressions and the
complete focused macOS batch passed, including 795 MCP cases. The full Linux
default-concurrency workspace/doc/release-smoke gate passed. The full macOS
native suite then reported 3,343 passes, four failures and 12 ignored tests in
678.60 seconds; its remaining integration/doc/smoke steps did not run:

- `abort_and_drop_revoke_pipe_authority`: failure-only instrumentation located
  `Cleanup` at the signal-phase operation error (`background_process.rs:5445`).
  Direct-child reaping succeeded on its first probe and discharged child/permit
  custody. The signal and errno were not captured, so the original cause remains
  unproven; this is distinct from a reap timeout.
- Three PTY cases failed during exclusive-child-reaping admission, before their
  history/EOF/signal behavior or actual PTY helper startup. `/bin/sh -c 'exit 0'`
  spawned successfully but returned no exit status during the initial 500 ms.
  Exact-child kill succeeded, but a further 500 ms cleanup did not obtain terminal
  status; the child and permit transferred to observation-only quarantine.
  The outer two-second startup deadline had not expired. These observations do
  not establish a source bug or host-load cause.

The three PTY cases were
`durable_history_resize_matches_the_real_pty_and_survives_recovery`,
`expired_eof_observation_cannot_hide_a_retained_running_child` and
`explicit_owned_signal_kills_shell_and_final_drain_preserves_bytes`.
Each of the four failed cases subsequently passed once in a separate unchanged
process. This diagnostic does not establish a repair or replace the failed gate.
Evidence remains in `macos-7aa6be70-complete-gate.log` and
`macos-7aa6be70-platform-diagnostic.log`; successful focused/Linux/auxiliary logs
use the same candidate prefix. Python and fresh product reviews were not started
from the failed macOS run.

Worker `9d7a57b7` adds failure-only test diagnostics for the rejected signal's
name/errno, the existing phase's sole-leader evidence and the already-performed
exit confirmation. It adds no process observation, retry, changed deadline or
production logging. The existing signal acceptance and early-error propagation
remain unchanged. This instrumentation is not claimed as a product repair.

All 130 compiled test binaries after the native unit suite subsequently passed
in a separate serial diagnostic (`macos-7aa6be70-remaining-diagnostic-replacement.log`).
The initial diagnostic invocation failed while parsing its compile-log inventory,
before running any tests; its log is retained separately. This additional
coverage does not erase the four failures or satisfy the complete macOS gate.

## R3 candidate, review and repairs

Candidate `aae6b49dcc8f8e8d770e287e73d04e73066afcba` passed the complete
Rust 1.94.1 local gate, including both platform builds, focused regressions,
Linux default-concurrency and macOS serial workspace/doc/release-smoke checks,
drift, audit/policy and portable compilation. The macOS native suite reported
3,348 passes and 12 ignored tests; all four previously failing platform cases
passed without establishing their earlier cause. Python reported 269 tests with
14 skips in 545.770 seconds; rendered documentation and bounded policy checks
passed with zero errors. Evidence is retained in the `aae6b49d`-named logs under
`/private/tmp/mg-mcp-full-gate.hE0Sfz`.

Three fresh ordinary local reviewers inspected the same 496-file feature diff
against `6736070cc70ff6040cfa92275e80a0e9a5172852` after that gate. Host thread
capacity delayed the resources launch until correctness completed; no author
was substituted. These were risk-directed, source-only full-feature tracks,
not Bugbot, exhaustive line-by-line coverage or additional runtime execution.

| Track | Result |
| --- | --- |
| Correctness/API | Zero actionable introduced findings established. |
| Lifecycle/platform | P1: production stdio capture selects the PTY helper argument, although its launcher sends the captured-execution descriptor handshake and uses persistent pipe input. |
| Performance/resources | P2: descriptor charges omit fixed shared records, descriptor slots and typed prompt-argument storage; the shared lazy cache consumes those understated charges. |

The coordinator confirmed both source paths. The accounting issue is finite,
bounded undercharging, not unbounded growth, measured heap usage or demonstrated
OOM. Separate cardinality limits do not make the shared cache's retained-byte
charge conservative. These findings reject the candidate despite its green local
gate; no remote acceptance follows.

The lifecycle reviewer subsequently authored regression `ddd07b44`, helper
selection fix `e520f1e2` and test-organization follow-up `3314733e`. Required
startup and optional first-demand cases use the actual production capture and
selected release helper, a modern stdio producer, model search/select/call,
persisted/provider-visible results, producer EOF and owned host completion.
Test-only baseline `904ab405` retains the original product code and includes the
fixture follow-up. Both cases failed at their post-cleanup success assertion in
0.54 seconds; the host-completion assertion passed first. The failure payload
was `()`, so the runtime log alone does not identify a narrower startup stage.
Evidence: `r3-captured-stdio-baseline.log`, using the original candidate's release
helper with SHA-256
`b9e7c4ac4af783da0c514ec757b230457f248374f0390fefee4b275168cc8337`.

The resources reviewer authored regression `84687782` and repair `eed82598`.
The repair adds size/layout-derived catalog, descriptor-owner/backing and prompt
argument charges before retention, preserving payload/schema charges and all
configured caps. Tests cover each variant, empty catalogs, argument arrays,
exact/one-under admission, shared cache pressure and charge release. Both authors
performed pinned formatting and diff checks, not runtime acceptance. They cannot
serve as fresh reviewers for the replacement candidate.

Test-only baseline `87df2017` adds the resource regressions without either
production repair. All four focused cases failed: the fixed-record lower bound,
prompt-argument charge growth (7,748 versus 12,868 bytes, omitting 5,120 bytes of
typed slots), the empty catalog's missing shared-record charge, and retention
under a payload-only lazy-cache budget. Evidence remains in
`r3-descriptor-charges-baseline.log`. These negative diagnostics establish the
tested regressions, not replacement acceptance; the integrated repair still
requires its complete local gate and three fresh reviewers.
