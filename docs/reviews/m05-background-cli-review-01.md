# Milestone 05 `background` CLI review history

This is the compact historical review record for the contract in
[`../background-cli.md`](../background-cli.md). Current phase, delivery,
workflow, and next-gate status is maintained only in the
[`implementation plan`](../implementation-plan.md#current-delivery-state).

## Accepted product candidate

Three fresh review tracks inspected exact candidate
`a665289da640d69ae88d0a4a336ad90ece889086`. The candidate passed the complete
exact Rust and Cargo 1.94.1 local gate before review.

| Track | Product verdict | Evidence retained |
| --- | --- | --- |
| Correctness, API, and compatibility | Green | Zero findings after strict command forms, stable rendering, exact record schema, duplicate and unknown field rejection, canonical stored paths, normalized configured bases, fixed error taxonomy, and pinned compatibility review. |
| Lifecycle, platform, and effects | Green | Zero findings after descriptor-relative no-follow traversal, Linux `openat2` and `O_NOATIME`, macOS `O_NOFOLLOW_ANY` and ACL policy, bounded environment reads, read-only operation, unsupported FreeBSD/WASI behavior, and native macOS tests. |
| Performance and resources | Green | Zero findings after streaming preflight, fixed depth/node/record/file/identifier/path/argument bounds, early oversized-input rejection, bounded rendering, and no-write regression review. |

## Rejected candidates and remediation

| Candidate range | Decisive findings | Replacement |
| --- | --- | --- |
| `7600463` through `2080351` | Persisted records and stored paths were too permissive, and the unsupported-platform terminal surface failed warnings-denied WASI compilation. | `09e49ed` through `2733b0c` made record decoding strict and bounded, rejected non-canonical stored paths, streamed JSON preflight, added no-follow state-base traversal, fixed the compile incompatibility, and enforced FreeBSD/WASI CI. |
| `2733b0c` through `548eff5` | Large identifier tokens were scanned before a byte bound, macOS ACL handling admitted unsafe record policy, and configured state bases did not preserve documented lexical normalization. | `6325d9a` restored bounded normalized environment bases and retained early identifier and descriptor ACL checks. |
| `6325d9a` through `8b45806` | Raw environment values lacked a pre-decode byte limit, and macOS deny-read ACL classification needed identity-stable preflight. | `a665289` bounded raw bases before decoding, added identity-checked macOS policy handling, and aligned the durable failure contract. |

Every accepted finding rejected its candidate. The complete replacement local
gate and three entirely fresh review tracks were rerun for the accepted
candidate. All isolated review worktrees were verified clean, removed, and
pruned after their iteration.

## Remote delivery evidence

The exact accepted candidate passed feature CI `33454785472` and Benchmark
evidence `33454785491`, then exact-main CI `33455374424` and Benchmark evidence
`33455374426`. Both benchmark runs retained unexpired upstream and bootstrap
artifacts whose names identify the exact accepted behavior SHA.

This review seal makes no package, release, or comparative-performance claim.
The implementation plan remains the sole live source for delivery and workflow
gates.

## Complete interactive feature: rejected first review candidate

The expanded interactive controls and combined legacy/terminal history were
independently reviewed at `77b6f34814887bdb1b1775c6c1d088ee1c3dea91`, tree
`b5e4207b73e16575013aec44b88464b593c2df83`, against accepted parent
`79d3d4d426aa52816ce6dd70020b7f6a0866da38`. This was not a delivery.

Before review, the exact Rust 1.94.1 local gate passed: macOS workspace
4,644 passed, 18 ignored; Linux workspace 4,650 passed, 17 ignored; explicit
workspace doctests 3 passed on each platform. Counts use top-level harness
summaries, excluding nested child-process summaries. The harnessless terminal
CLI target also exited successfully. Fresh locked release binaries, formatting,
warnings-denied Clippy, 269 repository Python tests (14 expected skips),
documentation policy, pinned compatibility/Unicode drift, dependency policy,
vulnerability audit and the three FreeBSD/WASI compile gates passed.

The Linux gate required correcting two environment-only failures: root bypassed
a permission-denial fixture, then root-owned shared temporary fixtures denied
the corrected unprivileged test user. After isolating those stale fixtures,
the complete replacement run passed with a real unprivileged NSS account,
private home, normal shell profiles and container init. Failed and successful
logs were exported and hash-verified before removing the container. No source
fix, test skip or deadline relaxation concealed either failed invocation.

Three fresh local fallback reviewers inspected clean isolated worktrees of the
same candidate; the host did not expose the named Bugbot service. Their static
reviews ran no builds or tests and produced these actionable findings:

| Track | Agent | Findings |
| --- | --- | --- |
| Correctness/API | `background_review_correctness_01` | P2: a truncated capture can open a partial URL. |
| Lifecycle/platform | `background_review_lifecycle_01` | P2: read-only profile-lock contention can discard exit output; P2: the same truncated-URL defect. |
| Performance/resources | `background_review_resources_01` | P2: the same truncated-URL defect. |

The two distinct defects reject this candidate. At
`background_commands/service.rs`, URL detection discarded the captured window's
truncation evidence: 65,515 spaces followed by
`http://localhost:3000/private` and LF could open the root URL instead.
At `terminal_registry.rs`, an exited backend fell into no-persistence cleanup
when the new read-only inspector's shared profile lock made its transaction
busy, discarding the final drained bytes. The cleanup fallback predates the
feature; routine read-only history inspection introduced the new trigger.
All three completed review worktrees were verified clean and removed.

### First-review remediation

`4ac1b8e1` carries explicit snapshot/truncated boundary evidence into URL
detection, preserving earlier delimited candidates while rejecting an
unterminated trailing one. The composed regression failed on the old behavior,
then passed twelve byte-limit, page-limit and scripted-gap scenarios covering
ordinary/oversized candidates and earlier complete URLs. Thirteen detector
tests and eight nonbrowser launcher tests also passed. Standalone default-feature
native tests additionally needed their composed-service module to use the same
feature gate as its production service; workspace feature unification had hidden
the mismatch. Warnings-denied Clippy rejected the regression adapter's fourth
boolean, so `7ad99073` represents gap injection as an optional read boundary.

The independently implemented `b9ef174e` defers known-exit cleanup on transient
profile `Busy`, retaining the backend and tail until a later bounded pump pass.
Its shared-read-only-lock regression failed before the fix and passed afterward,
proving repeated deferral, durable tail recovery and exactly-once cleanup.
All 64 registry tests passed, including retained permanent-error and shutdown
fallbacks. These are focused remediation results, not replacement feature
acceptance or authorization to merge.

### Replacement review and remote helper-path rejection

Candidate `16f04f3b0e4b274f0a7cc930674d7e4459debbcc`, tree
`066915b64b76c405be4375573d89748306637d76`, passed the complete replacement
Rust 1.94.1 local gate. macOS had 4,647 non-doctest passes and 18 ignored;
Linux had 4,652 non-doctest passes and 17 ignored. Each platform also passed
three included workspace doctests, three explicit doctests, and the separate
harnessless terminal CLI target. Counts exclude nested child-test summaries.
Formatting, warnings-denied Clippy, fresh release scenarios, repository Python
checks, documentation and upstream drift, dependency policy/audit, unsupported
compilation and default-feature native test compilation passed. Linux used an
unprivileged account from its first runtime invocation. Exported logs were
hash-verified before removing its container and clean source worktree.

Three fresh independent local static reviewers—
`background_review_correctness_02`, `background_review_lifecycle_02`, and
`background_review_resources_02`—reported zero actionable findings against the
same candidate and accepted parent. No named Bugbot service was available.
All three clean review worktrees were removed before pushing the feature.

Feature Benchmark `34472264651` succeeded with both exact-SHA artifacts, but
feature CI `34472264644` exposed a native test configuration defect. The new
`fresh_cli_inspects_producer_histories_without_recovery_or_live_control` test
looked at `MACHINE_GOD_CLI_TEST_BINARY` and then `target/release`; platform CI
instead correctly exports `MACHINE_GOD_TERMINAL_RELEASE_BINARY` pointing into
the exact target triple's release directory. The x86_64 Linux job failed its
binary-existence assertion before invoking the scenario. The local recipe set
both variables and therefore did not expose this mismatch. Remediation uses
the existing native-test helper contract without changing CI, skipping the
scenario, weakening assertions, or changing product behavior. This candidate
was not merged; the helper-selection correction requires replacement gates.

### Helper correction gate and rejected cancellation review

Candidate `8c1fcbfb53fc86df23590ceb289c6f2184301a5d`, tree
`e94e5ebfeb3a533f99c4c49f39a506d595c1f70f`, passed the complete replacement
local gate. macOS had 4,647 non-doctest passes and 18 ignored; Linux had 4,652
non-doctest passes and 17 ignored. Each platform also passed three included
doctests, three explicit doctests and the separate harnessless terminal CLI
target. The fresh background-history CLI scenario passed with only the native
helper variable set on both platforms; macOS also passed with an intentionally
invalid unrelated CLI override. Linux's fresh helper was outside the source
tree's fallback location. All other canonical local checks passed, including
269 Python tests with 14 expected skips. Neither runtime required retries or
changed deadlines. Logs were retained and the clean validation environments
removed before the review iteration finished.

Three new independent static local fallback reviewers inspected the complete
feature against parent `79d3d4d426aa52816ce6dd70020b7f6a0866da38`:

| Track | Agent | Findings |
| --- | --- | --- |
| Correctness/API | `background_review_correctness_03` | P2: background-control cancellation suppresses simultaneous turn/admission cancellation. |
| Lifecycle/platform | `background_review_lifecycle_03` | Zero actionable introduced findings. |
| Performance/resources | `background_review_resources_03` | Zero actionable introduced findings. |

The early return in `NativeInteractiveSession::request_cancel` cancelled the
background token without setting `cancel_requested` for an existing admission
or turn. Control admission permits those operations to coexist, so the turn
could resume after a cancelled background operation settled. This contradicted
the existing public turn-cancellation contract. The finding rejects this
candidate despite its complete local gate; it was not pushed or merged. All
three clean review worktrees were released and removed. Remediation must retain
both cancellation requests and the control's completion receipt, preserve
admission settlement, and avoid cancelling future queued work.

Five deterministic regressions reproduced the defect before correction: the
active-turn case failed because cancellation never reached its owned turn;
the pending-admission case settled as completed instead of cancelled. The three
background-only and shutdown/closed receipt cases already passed. The fix
guards closed/shutting-down owners first, cancels an existing background token,
and independently latches cancellation only for an already-owned admission or
turn. It does not drop the control future or set cancellation for later queued
prompts. These red results establish the defect, not replacement acceptance.

### Cancellation correction gate and macOS cleanup failures

Candidate `78300fc631ac93c60f767f17e5b84beda4516259`, tree
`bf5aaf9c0b1f46f937ff8aaedd14600cdfff9c92`, passed the full replacement local
gate: macOS 4,652 non-doctest passes and 18 ignored; Linux 4,657 passes and
17 ignored. Each platform also passed three included and three explicit
doctests, the harnessless terminal CLI target, focused cancellation/helper
selection regressions, fresh release checks and the other canonical checks.
The first macOS local attempt had one inventory-readiness timeout. Three
unchanged focused repetitions and an unchanged full replacement passed; its
cause was not established and the failed log was retained. Linux passed its
first runtime attempt. Exported Linux logs were hash-verified before cleanup.

Three new independent local static fallback reviewers inspected the complete
56-file feature against parent `79d3d4d426aa52816ce6dd70020b7f6a0866da38`:
`background_review_correctness_04`, `background_review_lifecycle_04`, and
`background_review_resources_04`. All reported zero actionable introduced
findings. They ran no tests or remote checks and made no changes; this was not
a named Bugbot-service review. Their clean isolated worktrees were removed.

Feature Benchmark `34484595531` passed with unexpired exact-SHA upstream
artifact `10155441963` and bootstrap artifact `10155213261`. The downloaded
evidence matches the exact candidate/tree and pinned fx revision, but remains
non-claim-eligible regression evidence. Feature CI `34484595470` did not pass:

- Attempt 1: Apple ARM job `102895629230` failed
  `long_quoted_artifact_paths_preserve_command_and_commandless_startup` while
  closing clean zsh after exit 17. A subsequent inventory-service restart in
  Drop strongly supports a failed close-time query, but does not identify its
  error or prove eventual cleanup. All other jobs passed.
- One unchanged failed-job retry, attempt 2: Apple ARM job `102904758206`
  passed that earlier test but failed
  `real_owned_backend_preserves_fast_signal_and_cleans_reparented_jobs` while
  closing its reparented-job case after shell exit 0. Its erased error cannot
  distinguish inner terminal close from private-server retirement. Each failed
  native unit run had 2,250 passes, one failure and 12 ignored tests.

Read-only diagnosis found the traced startup, PTY, tmux, cleanup, inventory,
binding and helper paths unchanged from the accepted parent. Interactive
cancellation changes are outside these tests' direct backend paths. Indirect
suite/resource interactions remain possible; neither runner load nor a shared
inventory cause is proven. Both failed logs are retained, and no third blind
retry or merge followed. New error-only test diagnostics retain typed failures
and identify query/close/server-retirement stages without new process observations,
deadline changes, retries or alternate cleanup paths. They are diagnostic
remediation, not a claimed source fix or feature acceptance.

Diagnostic candidate `fb1d617d` built its fresh release helper and native test
executable, but focused Clippy rejected two functions exceeding the line limit.
No runtime tests were started on that candidate. The correction separates tmux
transport observation from namespace retirement and groups inventory diagnostic
timing context; it does not suppress lints or change cleanup/query budgets.

Candidate `173d800a` passed focused native library Clippy, fresh release/test
builds, 27 inventory tests, 29 tmux tests and three repetitions of each original
CI-close fixture. Its serial terminal group failed with 775 passes, one failure
and five ignored tests. `full_command_boundary_executes_as_one_argument_and_reaps`
received zero inventory-readiness bytes over about 1.995 seconds; the subsequent
bounded reap also timed out. This is startup, before command transmission or an
inventory query, not either remote close failure. Native all-target Clippy and
full-gate expansion did not start after that failed command.

Read-only predecessor analysis found no persistent state mutation establishing
a cause. A bounded external observer then accompanied the exact two-test
predecessor/target pair (two passes) and PTY prefix (12 passes), without source
changes or adjusted deadlines. Seven PID-only snapshots were attempted. One
validated helper briefly reported state `U` and was absent at the next sample;
the remaining absence or executable-mismatch stops do not identify a loader,
kernel, scheduling or product cause. Raw failed and controlled logs remain
separate. These passing controlled runs do not replace the failed gate or
establish remediation. The same candidate also passed all 269 repository Python
tests with 14 expected skips, pinned compatibility/Unicode drift checks,
dependency policy and vulnerability audit.

One additional observed full-context run selected the same 781 terminal tests
and passed: 776 passes, five ignored, no failures, in 219.01 seconds. Observation
remained limited to the target fixture's three inventory-service spawns, with
four PID-only snapshots and no signals, suspension, source changes or deadline
changes. One exact helper reported state `S` before disappearing at the next
sample; no failing startup or close was captured. This bounds the reproduction
attempts and supports proceeding to a complete replacement gate, not a source
fix or feature-acceptance claim.
