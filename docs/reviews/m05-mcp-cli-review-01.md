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
