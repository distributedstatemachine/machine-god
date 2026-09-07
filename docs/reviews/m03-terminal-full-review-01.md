# Full terminal feature: internal candidate history

This ledger preserves historical evidence compacted from the implementation
plan. Statements about pending work describe each recorded candidate, not live
project status. The [implementation plan](../implementation-plan.md) is the
only current delivery and gate ledger. No internal candidate below constitutes
a separate delivered slice.

## Recorded candidate sequence

Candidate `3414e55742b175f88cf8c803e3a573e8b31c9d09` passed the complete pinned
workspace and doctest gate, strict Clippy, 623 release-helper terminal tests,
CLI release smoke, repository Python checks and supported FreeBSD/WASI compile
checks. Intermittent startup failures did not reproduce in isolated diagnostics
or the complete replacement run; no speculative timeout change was made.
The full-feature review rejected that candidate for three concrete issues:
raw `list.workspace_root` predicates bypass native resolution, later runtime
failure can hide an already-committed operation receipt, and grid suffix limits
can admit complete cell text that the screen contract rejects. The integrated
corrections resolve supplied list filters on scoped workers, preserve executed
receipts without masking initialization failures, and enforce the same complete
UTF-8 cell bound during projection and checkpoint restoration. Their focused
regressions and strict native Clippy checks pass; completed worktrees are removed.
The correction requires the complete replacement local gate, three independent
review tracks and exact remote CI/Benchmark gates; this is not a delivery.

Candidate `b55098ffd4497eb99c71f893b596c96f75f426a9` passes the correction
regressions, strict workspace Clippy, doctests, repository Python checks,
supported cross-compilation, release CLI smoke and 633 release-helper terminal
tests. Its full workspace run failed one tmux bootstrap read; bounded isolated
and full-order diagnostics have not reproduced that failure. The diagnostic
full native run instead exposed a PTY host close failure, reproduced in a
focused run as expiry of the 250 ms macOS inventory budget. The integrated
optimization rejects unrelated session rows before expensive identity queries,
while retaining the exact identity sandwich, prior descendant pins and scan
bounds. It reduces unnecessary queries but does not promise that bounded native
operations cannot fail under host load. The composed fixture now observes the
retained failure and output gap before a bounded, explicit close of the same
session; it never replays start or write. An injected inventory failure verifies
the initial error, retained history, successful later cleanup and one command
execution. Full-feature review and remote delivery remain pending.

Candidate `1f8afe77edb25677ef0065601babefee52c12fa9` passes pinned formatting,
strict workspace Clippy, doctests, all 259 repository Python checks (14 expected
skips), supported cross-compilation, dependency checks, release CLI smoke and
634 release-helper terminal tests. Two full workspace attempts failed native
helper preparation or a composed probe wait; the earlier tmux bootstrap case
passed. Twenty bounded isolated PTY executions and one complete instrumented
native-order run passed, without establishing the intermittent failure cause.
The retained test-only preparation diagnostics identify stages and bound helper
output capture; they change no startup deadlines, retries or product behavior.
They are diagnostic evidence, not a claimed fix. The complete replacement local
gate, independent reviews and exact remote delivery gates remain open.

Candidate `0535195239c0b37da3a6bdc6ed9828e38f11212f` passes the build,
formatting, strict Clippy, doctest, Python, cross-compilation, dependency and
release-smoke prerequisites. Its full workspace run identifies one PTY failure
at reaping admission, before helper spawn, and another factory preparation
timeout. The admission correction distinguishes lost wait authority from a
bounded probe timeout: it never signals a known-stale handle and accepts a
timed-out probe only after positive exact-child reaping and a final cancellation
check. The existing probe/cleanup bounds and per-start proof remain intact.
The factory tests separate setup from late cancellation, expired commit and
expired shell acknowledgement while asserting the unchanged supplied deadline.
Strict workspace Clippy, seven admission regressions, cancellation cleanup,
eight factory tests and twenty PTY tests pass. The real stalled-probe regression
crosses the unchanged observation bound and proves successful exact-child reap;
it does not establish the subcase behind every earlier intermittent failure.
Complete replacement gates remain required; no delivery is claimed.

Candidate `2ad56777ca402c32f33a211c25026e2b1141eacd` passes the build,
formatting, strict Clippy, doctest, Python, cross-compilation, dependency and
release-smoke prerequisites. Its full workspace native run passes 1,362 tests
with four expected ignores, but fails two macOS inventory deadlines and one
custom-probe publication wait. A single instrumented probe run establishes
that successful, timely evidence is rejected because dispatch reuses an older
pump timestamp. The correction admits a fresh validated owner time before job
effects, without changing evidence timestamps or probe deadlines. Inventory
collection retains the fixed child boundary and limits, using pipe readiness
to remove sleep-polling latency and checking expiry before accepting any
snapshot. This does not promise success under arbitrary host load. Focused
checks pass for nine collector cases, three deterministic clock regressions,
the previously failing custom probe, and the owner, runtime, host-probe,
startup, tmux, staged-start, factory and PTY groups. Known loss of collector
wait authority also discharges its stale handle before cleanup. Strict workspace
Clippy passes after extracting the bounded test diagnostics. Replacement local
gates, independent reviews and exact remote delivery remain required.

Candidate `15112d88c5759b2e75b358a1b5c08f52873102a3` passes the complete
pinned local gate, including 1,376 native workspace unit tests with four
expected ignores and 639 release-helper terminal tests with four expected
ignores. Three independent local review tracks reject delivery for two cleanup
defects, stale probe-grant admission and a scheduling-sensitive collector test.
A failed close can discard captured descendant identities; a late cleanup
failure can reap the shell yet leave retries unable to converge. Probe grants
can outlive retired monitor identities until housekeeping and incorrectly block
replacement admission. Corrections must retain exact cleanup authority and
post-reap progress, reconcile live grant identities before admission, and use
the existing injected clock for backoff assertions. No timeout or quota
increase, command replay, remote push or delivery is claimed.

The integrated correction retains cleanup identities/pidfds across retries and
fallback queue transfer, requires positive shell exit before final quiescence,
and retains exact exit receipts before releasing authority. Linux final scans
retain newly acquired pins even when a later scan step fails. macOS reserves
one existing reap slot for inventory; shared cleanup restores the originating
scope without leaking it into unrelated jobs. Live probe grants reconcile before
monitor/start admission, including paused templates and reused namespaces.
The focused gate passes 25 PTY tests, four scope-attribution tests, two probe
quota regressions, one inventory-capacity regression and ten staged-start tests.
The PTY gate includes late-proof failure/retry, post-capture SID escape, scoped
fallback settlement and denied-signal escalation. A reproduced macOS TERM EPERM
during natural exit is handled by bounded escalation and positive completion
proof, not by treating denial as disappearance. Strict workspace Clippy passes.
Linux library/test cross-compilation and supplemental no-default-feature lint
pass (the latter excludes dormant-feature dead-code warnings); these are not
native Linux execution evidence. Full replacement local, platform, review and
remote gates remain pending; completed agent worktrees are removed.

Candidate `13ba049f4cc58947bb50cbad8061b7f36dda57e3` passes pinned formatting,
strict workspace Clippy, the complete workspace/doctest gate, all 259 repository
Python checks (14 expected skips), dependency checks, supported FreeBSD/WASI
compilation and fresh release smoke. Native unit execution passes 1,388 tests
with four expected ignores. The release-helper matrix passes 645 tests with
four expected ignores but fails a captured-exec fixture: file existence races
the shell's PID write, producing an empty integer parse before cancellation is
tested. The test-only correction waits for a complete framed PID within the
unchanged readiness bound; it does not alter product timing or cleanup behavior.
Replacement validation and three fresh reviews remain required before push.

Candidate `c2d1243b2592c86a0ad2c433d6b0e2549cb627af` passes the complete
replacement local gate, including 1,389 native unit tests and 647 release-helper
terminal tests (four expected ignores in each), and three fresh independent
source-review tracks with zero actionable findings. The feature branch is
pushed. Exact CI `34089297053` rejects delivery: Linux quality and both Linux
native jobs fail the same full-host constructor; macOS ARM fails an elapsed-time
assertion around cancellation, cleanup and joining. The Linux failure reproduces
as a non-root user in a local exact-1.94.1 container. Pinned rustix rejects
`chmodat(SYMLINK_NOFOLLOW)` on Linux, so new terminal directories require a
supported descriptor-bound restoration path that preserves hostile-umask and
identity checks. The timing fixture must separate writer cancellation from
native cleanup stages without changing product bounds. Benchmark evidence run
`34089297104` and any other passing jobs do not satisfy the failed CI gate;
`main` and the delivered count remain unchanged.

The integrated Linux correction restores only the retained new directory via a
validated procfs descriptor link, preserving mode-000 creation without changing
umask or following the ordinary entry for chmod. Non-root Linux execution passes
all 18 catalog tests, including hostile umask, substitution and untrusted-proc
regressions, plus the previously failing CLI settlement fixture. The timing
correction separates deterministic writer cancellation/expiry from native
cleanup and retains both real-helper exact-reaping assertions. Its three
focused tests pass on Linux and macOS. Complete replacement gates and fresh
reviews remain pending; all completed agent worktrees are removed.

Candidate `5d8b6e474792c915db3dfd7b4b277d75707ecf2a` passes the complete
macOS workspace gate (1,390 native unit tests, four expected ignores), strict
workspace and Linux native Clippy, doctests, repository Python checks and
supported FreeBSD/WASI compilation. Broader non-root Linux execution exposes a
second compatibility defect: host-side `TIOCGSID` on the pane slave returns
`ENOTTY`. The integrated correction obtains a fixed controlling-terminal
receipt from the authenticated pane helper and verifies it against the host's
retained descriptor before launch frames. All 21 focused tmux checks pass on
Linux and macOS, with one expected helper ignore. A later tmux close failure
is traced to an adopted zombie owned by a subreaper host that the tmux inventory
never reaps. The integrated correction uses exact pidfd reaping, including retained
descendants that change sessions, and a fresh full anchored snapshot without
consuming unrelated child status. All 24 focused Linux tmux tests pass, with
two expected helper ignores, and strict native Clippy passes. Regressions cover
not-yet-adopted exits, retained session changes, saturated descriptor budgets
and older-kernel exit observations. Replacement delivery gates remain open.
Two startup fixture corrections use durable history for close-drained output
and a stable builtin-only profile before forced expiry; each passes 20 non-root
Linux repetitions. The Linux shell-ack fixture observes the expected unreaped
abort exit under its original deadline before one close, passing 30 Linux
repetitions without a cleanup retry. A pre-existing same-length copy fixture now explicitly
changes its timestamp rather than depending on a filesystem clock tick.
Container execution uses a reaping init: omitting it caused a distinct
orphan-descendant fixture failure, which passes with init enabled. These test
arrangements do not weaken process identity checks or increase product bounds.
The complete replacement local, independent review and remote gates stay open.

Candidate `7496e22b54776ce8360eff09a5242219538d3f25` passes the complete
pinned local gate, including 650 release-helper tests and non-root Linux native
and CLI unit suites. Fresh review rejects it: tmux discards acquired descendant
handles after a later inventory error, and foreground exec failures are marked
as successful tool results. A deterministic Linux reproduction proves premature
cleanup success with the escaped child still alive. Resource review reports no
actionable findings. The integrated corrections retain each authenticated pin
immediately and preserve failed-exec result flags. Focused Linux/macOS tmux and
adapter checks pass; replacement gates remain required. No delivery is claimed.

Earlier internal evidence remains historical, not delivery status:
`a327580b754e25b8c1beaa5edf744b2b2252c620` passed the pinned workspace gate,
421 terminal component checks and release-helper startup/PTY coverage;
`de480453718169adf5e458a34c727833dbf32cd1` passed the pinned local gate,
release command round-trip and three component-review tracks after the decoder
profile correction. Full-feature review does not inherit those component seals.

## Review of `7496e22`

The exact candidate was `7496e22b54776ce8360eff09a5242219538d3f25`, reviewed
against merge base `46e5b70f6c5ba76a4699f5bd8ba424a1fa3813be` after its complete
local gate. Three independent local review tracks used the review skill's
direct fallback; no Bugbot service was available.

- Correctness/API (`terminal_7496_correctness`): one P2 at
  `terminal_action_tool.rs:481`. The existing exit-code-7 fixture reproduced
  `is_error=false` instead of the pinned foreground-failure semantics.
  Decoder, engine, provider, archive and CLI paths had no further established
  finding; 43 contract/input/output tests passed.
- Lifecycle/platform (`terminal_7496_lifecycle`): one P1 at
  `terminal_tmux_process.rs:355`. A deterministic non-root Linux reproduction
  established `retained=false`, `premature_absence=true` and an alive escaped
  child after a later refresh error. Independent exact-handle cleanup removed
  the fixture child. No additional finding was established in the assigned
  process, startup, catalog, runtime or platform scope.
- Resources/performance (`terminal_full_correctness`): no actionable finding
  in accounting, retention/recovery, grid bounds, scheduling and probe quotas.
  Authored startup/cancellation fixtures were excluded and covered by the
  lifecycle peer. This was source review, not a new performance claim.

Corrections `e34001efee6e7cc2c015090e74ad5a02b0e00269` and
`2ceadf6cc83457e521fcc1bafddd57dcb4d54b0a` were integrated with durable docs in
`49e26c835a6785924196c61b34dda06439f8a579`. Focused adapter checks passed 17
cases; tmux checks passed 49 on Linux and 45 on macOS, with expected private
helper ignores. Strict native Clippy passed on both platforms.

## Local validation of `49e26c8`

Pinned formatting, strict workspace and Linux Clippy, doctests, FreeBSD/WASI
checks, dependency policy/audit, standalone test support, 259 Python checks
(14 expected skips), drift/documentation checks and the fresh release build
passed. Eight release-launch checks and the release CLI smoke also passed.
The complete runtime gate rejected this candidate before review: macOS native
units passed 1,396 cases but failed a stale host-fixture success assertion;
Linux passed 1,463 cases but failed that assertion and a startup-timeout
cleanup test. Expected ignores were four and five respectively. The Linux
cleanup failure reproduced on the second untraced isolated attempt; broad
syscall tracing changed timing and did not establish its cause. No remote push,
review seal or delivery was claimed for this candidate.

Failure-only diagnostics reproduced the Linux case on attempt four in 0.09 s:
the captured startup-marker child disappeared during the shell's initial
process-tree inventory, before cleanup delivery. The forced command-ACK timeout
closed the private connection, letting bash reap its aborting marker while the
test immediately requested cleanup. This established a fixture-ordering race,
not expiry of the product inventory deadline. The scoped fixture correction
observes the exact unreaped Linux shell's exit 125 under its original deadline
before the existing single force-close; macOS behavior and product code stay
unchanged. Temporary process diagnostics are not retained.

Fixture commits `55d0b5db3d56929d14f25b9238a8641b6f7087b8` (explicit expected
host-result failure) and `a75126beacc4974a30c8e02072b137d9697df8c5` (ordered
Linux abort observation) preserve the original inertness, exit, output and
cleanup assertions. The corrected host case and unchanged macOS startup path
each pass their exact pinned test; the uninstrumented Linux startup case passes
30 consecutive runs and strict native Clippy. These focused checks are not a
replacement full-gate or review seal.

## Review of `8546972`

Candidate `8546972df17de9a06b13870e1b8d362eba869bff` passed the complete
pinned local gate: macOS native units 1,397/four expected ignores, full workspace
and doctests, release helpers 654/four expected ignores, eight release-launch
checks, fresh CLI smoke, non-root Linux native units 1,465/five expected ignores
and CLI units 141/four expected ignores, strict Linux/macOS Clippy, dependency
checks, 259 Python tests/14 expected skips, drift and FreeBSD/WASI checks.
Three fresh independent local reviewers compared the entire feature against
merge base `46e5b70f6c5ba76a4699f5bd8ba424a1fa3813be`:

- `terminal_8546972_api`: no actionable finding; 43 cached contract/admission
  regressions passed. Scope included decoder, core, Gateway, archives and CLI.
- `terminal_8546972_resources`: no actionable finding in accounting,
  retention/recovery, grid, scheduling and probe bounds. No benchmark claim.
- `terminal_8546972_lifecycle`: one P1 at `background_process.rs:1082` and
  `:2679`. Authenticated ancestry captures remain temporary across later
  fallible SID scans and union merges. The exact Linux reproduction captured
  two descendants, injected a later SID-snapshot failure, then let one child
  escape/reparent. Retry returned `Ok(Signaled(9))` while an independent pidfd
  proved the escaped child alive. Its guard killed/reaped the fixture afterward.
  The bounded reproduction took 0.05 seconds on non-root Linux/Rust 1.94.1.
  Related nested Linux capture stages and macOS identity/session snapshot
  prefixes were source-confirmed variants; the macOS variant was not runtime
  reproduced by this review. Existing Linux final-quiescence and tmux
  post-snapshot retention paths already retain their captures.

This rejected candidate was not pushed or delivered. Review used the local
direct fallback, not a Bugbot service; no previous review seal was inherited.

Correction `2e50a98b1e8c0da8313d70236d04d77119aee7ef` retains proved prefixes
across Linux ancestry/SID/capacity failures and macOS PTY/tmux inventory errors.
Ordinary callers keep their no-observer snapshot semantics. Permanent Linux
tests cover escape/reparent/retry after three capture stages and rejection of
unproved root/parent rows; macOS tests cover pre/final inventory-prefix failures.
Exact native Clippy passes on both platforms. Focused Linux groups pass 106
background/tmux, 22 PTY and 17 captured-exec cases; macOS passes 67, 25 and 17
respectively, with expected private-helper ignores. These checks do not replace
the integrated candidate's complete local, fresh review or remote gates.

## Local validation of `17a41fe`

Candidate `17a41feef4d1894890ece0f766a499144e2e276a` integrates the cleanup
prefix correction with its durable contract. Pinned formatting, strict
Linux/macOS workspace Clippy, doctests, FreeBSD/WASI checks, dependency checks,
259 Python tests/14 expected skips, drift/documentation checks and fresh release
build pass. Full non-root Linux native execution passes 1,467 cases with six
expected helper ignores, but fails `copy_file`'s postcommit corruption/source
mutation fixture: its expected error is instead a successful eight-byte copy.
All new cleanup regressions pass in that run. Pending Linux CLI and macOS
runtime sequences do not constitute evidence; the candidate was rejected
before review or remote push. The bounded diagnosis distinguishes the
hash-verified destination case from metadata-fingerprint source revalidation.

Correction `641cfeaeb25b1fbac44926de72d5188c1c1926bf` is test-only. Linux
repetition 14 proved the source rewrite retained identical device, inode, mode,
size, mtime and ctime. Both source-mutation fixtures now explicitly advance mtime
while preserving their eight-byte payloads. Destination/stage hash fixtures,
publication and parent-sync assertions, product code and timeouts are unchanged.
Rust 1.94.1 copy-file suites pass 25 tests on each platform, the Linux postcommit
case passes 200 repetitions, and strict native lint passes on both platforms.
These focused results do not replace the integrated candidate's complete gate.

## Local validation of `0ecf940`

Candidate `0ecf940b7bb1be5b547b64ebf427d672e2e9c7eb` passes pinned formatting,
strict Linux/macOS workspace lint, doctests, FreeBSD/WASI checks, dependency
checks, 259 Python tests/14 expected skips, drift/documentation checks and the
release build. Non-root Linux passes 1,468 native tests/six helper ignores and
141 CLI tests/four helper ignores. The full macOS workspace passes, including
1,398 native tests/five helper ignores, eight release-launch tests and the fresh
release CLI smoke. Its final release-helper matrix passes 654 cases/five helper
ignores but fails `real_tmux_cwd_formats_are_literal_and_owned_drop_collects_live_jobs`
at `terminal_tmux_startup.rs:1677`: commit returns `Process` after macOS inventory
output collection exceeds its existing deadline (259.052 ms elapsed). This
rejects the candidate before review or push; successful ordinary workspace
execution does not replace the failed release-helper gate.

Correction `701b2b9b83128e86405a33bcd2606c39ef461a4b` resets macOS collector
backoff after actual bytes or the EOF transition only. The deterministic test
proves old code rejects EOF at 222 ms and successful child exit at 223 ms because
stale 32 ms backoff consumes the unchanged 250 ms deadline. All three progress
cases pass after the fix; spurious readiness and interruptions without progress
retain bounded backoff. All ten collector tests, ten exact release-helper
repetitions, strict native macOS Clippy, formatting and diff checks pass under
Rust 1.94.1. Only `background_process.rs` changes. This establishes avoidable
collector latency, not exclusive attribution of the historical 259 ms failure;
the complete replacement gate and fresh reviews remain required.

## Local validation of `d70b8f6`

Candidate `d70b8f69243948dc787d576b52e1864070840e5d` integrates the macOS
progress correction. Formatting, strict Linux/macOS workspace lint, doctests,
FreeBSD/WASI checks, dependency checks, 259 Python tests/14 expected skips and
drift/documentation checks pass. Non-root Linux native execution passes 1,467
tests/six helper ignores but fails
`stale_owner_timestamp_cannot_acknowledge_after_actual_deadline` at
`terminal_startup.rs:1667`: immediate force-close returns `Cleanup` while the
fixture shell is still running after refused command acknowledgement. The
Linux CLI sequence is not run, and macOS runtime gates are not started after
this rejection. The shared bootstrap-abort path is being diagnosed before a
replacement candidate; no review or remote push is claimed.

The release build also passes (11 min 44 sec). Correction
`74521a231955b460bd6c4c885c8577d5ff2c5ccb` is confined to startup tests.
Failure-only Linux diagnostics twice reproduce the failure on attempt three:
the marker disappears during cleanup, then the exact unreaped bash parent
reports exit 125 within the saved original startup deadline. A shared Linux
fixture helper observes that exit without reaping before the existing single
force-close, covering both refused command-ACK fixtures and cancelled/dropped
shell ACKs. No production deadline, close retry or no-execution/artifact
assertion changes. Rust 1.94.1 startup suites pass 20 tests on each platform,
three Linux callers pass 50 repetitions each, and the macOS suite passes all
20 again with the fresh release helper. Strict native lint on both platforms,
formatting and diff checks pass; full replacement gates remain required.

## Review of `fc3b196`

Candidate `fc3b1969bcbfaff5810286fd109d2afb32e047a6` passes the complete
Rust 1.94.1 local gate: Linux native 1,468/six helper ignores and CLI 141/four
helper ignores; macOS full workspace/native 1,399/five helper ignores, eight
release-launch cases, fresh CLI smoke and release-helper matrix 655/five helper
ignores; strict lint, doctests, cross-platform checks, dependencies, 259 Python
tests/14 expected skips, drift and documentation checks. Three fresh direct
local reviewers inspect the full feature against merge base
`46e5b70f6c5ba76a4699f5bd8ba424a1fa3813be`, without inherited seals:

- `terminal_fc3b196_api`: zero findings; 43 cached contract/admission tests pass.
- `terminal_fc3b196_resources`: zero findings; source/test review, no benchmark
  claim or independent runtime rerun.
- `terminal_fc3b196_lifecycle`: one P1 at `terminal_tmux.rs:1116`. Close requires
  successful absence/validation discovery before native signal can use retained
  exact-process pins. Persistent capture-budget pressure therefore blocks
  cleanup of the very pins occupying that budget. Two bounded non-root Linux
  reproductions use the production backend and real native ownership with a
  two-descriptor budget: three failed force-close attempts leave one owned pin
  alive with zero signals, while direct native signaling kills it and still
  reports the inventory error. Wrong identity is rejected without capture or
  signal; diagnostics restore quotas and kill/reap their owned fixture children.

This rejects the candidate before push. All review worktrees are clean and
removed; no Bugbot service was used. The correction must separate retained
cleanup progress from fallible discovery without granting authority on identity
mismatch or accepting incomplete quiescence.

Correction `64017813b68c66c45cd3a67bcb499d4c9663df03` adds default-deny
cleanup-only delivery to the tmux process seam. Native delivery checks full
retained identity and uses authenticated handles without discovery. Failed close
preserves its original error and ownership; a prior ordinary signal attempt
suppresses duplicate fallback. Permanent production regressions cover running
and exited shells under capture pressure, untouched unproved jobs, identity
mismatches and successful retry without increasing quota. Portable cases cover
abort/validation/absence/signal failures and default denial. Exact Rust 1.94.1
checks pass: Linux 53/two helper ignores and 20 native quota repetitions;
macOS 48/one helper ignore; strict native lint on both, formatting and diff
checks. Only the three assigned tmux modules change. Full replacement local
and remote gates plus three fresh independent reviews remain required.

## Review and remote validation of `f29ddc7`

Candidate `f29ddc7f79fcf32083acb6bd63f96f35f22ea487` passes the complete
pinned local gate: Linux native 1,472/six helper ignores and CLI 141/four helper
ignores; full macOS workspace/native 1,402/five helper ignores, eight release
launches, fresh CLI smoke and production-helper matrix 658/five helper ignores;
strict lint, doctests, FreeBSD/WASI checks, dependencies, 259 Python tests/14
expected skips, drift/documentation and release build. Three fresh direct local
reviewers (`terminal_f29ddc7_api`, `terminal_f29ddc7_lifecycle`, and
`terminal_f29ddc7_resources`) report zero findings against merge base
`46e5b70f6c5ba76a4699f5bd8ba424a1fa3813be`. API additionally passes 35 cached
confirmations (15 action, 12 Gateway, eight archive); peers rely on supplied
runtime evidence, without performance claims. Review worktrees are removed.

The feature push starts CI `34126012291` and Benchmark `34126012351`.
Benchmark succeeds with unexpired artifacts `10020326854` (pinned upstream) and
`10020213499` (bootstrap), both bound to this exact SHA and expiring
`2026-12-06T13:11:42Z`. Failed CI jobs prevent main advancement:

- Linux ARM native and Ubuntu quality tests each fail only the captured-shell
  profile fixture; Linux x86_64 also fails native-launch cleanup after exit 23.
- macOS ARM fails tmux fast-signal/reparented-job close at
  `terminal_tmux_startup.rs:1812`; macOS x86_64 subsequently passes its complete
  job. The CI aggregate finishes failed.
- Missing zsh is reproduced with the identical native executable in fresh
  non-root Linux: tmux-only provisioning fails at captured-exec line 1431 in
  0.10 seconds; installing zsh makes the unchanged case pass in 0.12 seconds.
  The workflow correction explicitly installs/verifies shells without skipping
  profile coverage. All 14 focused CI-classification tests pass.
- An unchanged full Linux run with CI-like default concurrency and a two-CPU
  quota passes 1,472 cases. Fifty factory repetitions under simultaneous test
  load and five concurrent terminal-suite iterations also pass. No matching
  cleanup failure is reproduced; the diagnostic host is Linux arm64, not the
  remote x86_64 host. Diagnostics are restored without a speculative fix.
- The exact macOS CI-mode case passes 100 repetitions, both fast-signal and
  reparented-job variants each time. No failure diagnostic fires; no cause is
  established. Diagnostics and worktrees are removed, and cached diagnostic
  executables must be rebuilt before new exact-source gate evidence.

This is not delivery evidence. The corrected candidate requires replacement
local checks with matching Linux concurrency, three fresh reviews and exact
remote gates before main can advance.

## Concurrent deadline-fixture correction after `b1a7245`

Candidate `b1a7245c0c4381050faf6595a1e653a2f62c77e0` passes formatting,
strict Linux/macOS lint, doctests, FreeBSD/WASI compilation, dependency checks,
release build, 260 Python checks/14 expected skips, drift and bounded docs.
macOS passes the workspace (native 1,402/five helper ignores), eight release
launches, CLI smoke and 658 production-helper tests/five helper ignores.
After a package-scoped rebuild discards potentially cached diagnostic code,
Linux's default-concurrent native run fails four deadline-ordering fixtures:
1,468 pass, four fail and six helpers are ignored in 28.01 seconds. CLI does
not run after that failure. No review or push seals this candidate.

Correction `59d2052875913830274df030bb25582c7d0e092a` changes only tests in
`terminal.rs`. Original 1–5 ms deadlines could expire during filesystem
admission before mock execution existed. Arbitration fixtures now construct
admitted execution explicitly with live/expired deadlines, preserving output
byte counts, timeout and typed cleanup-error assertions without sleep-based
ordering. A separate expired-admission regression confirms no executor starts.
The independent timer test requires initial Pending, a real timer wake and
executor drop, using a one-second test deadline without retries. Production
deadline, admission and cleanup behavior are unchanged.

The isolated worker passes exact Rust 1.94.1 strict native lint and the full
default-concurrent Linux suite (1,473 pass/six helper ignores, 26.91 seconds),
39 focused cases and 20 repetitions, formatting and diff checks. Containers
are removed. macOS focused checks belong to the root's complete replacement
gate; three fresh reviews and exact remote success are still required.

## Linux detached-task record correction after `cd7ef9c`

Candidate `cd7ef9cb075225961086578a6c4ab5160f5703ba` passes all prerequisite
checks, including 39 macOS deadline cases, strict Linux/macOS lint, portability,
doctests, dependencies, 260 Python checks/14 skips, drift/docs and release build.
Its concurrent Linux run fails tmux reparented-job close: 1,472 pass, one fails,
six helpers are ignored in 29.08 seconds. CLI and full macOS runtime are not
run after rejection; no review or push seals the candidate.

Isolated diagnosis reproduces failure inside the scope scan, before transport
or server retirement, then captures a valid `6009 (sleep) X 0 -1 -1 ...` record
rejected by the nonnegative PGID/SID parser. Actual kernel is
`6.12.76-linuxkit`. Linux's [proc-stat implementation](https://github.com/torvalds/linux/blob/v6.12/fs/proc/array.c#L442-L510)
retains those defaults after [final task detachment](https://github.com/torvalds/linux/blob/v6.12/kernel/exit.c#L120-L199).
Sub-millisecond scan failure excludes deadline exhaustion in the captured case.

Correction `3abf14e9fadf39a57565991a853ac0192e769d76` changes only
`background_process.rs`. Exact state `X`, parent `0` and paired `-1` IDs map to
no group/session authority, with full PID/start-time validation and independent
retained-handle cleanup proof unchanged. Live/zombie states, malformed records,
mismatched PIDs and other negative sentinel combinations remain rejected.
The captured-record test is red before and green after the fix; all ten focused
proc-stat tests, strict Linux/macOS native lint and formatting pass. Diagnostics
are removed. Seven full concurrent Linux suites pass 1,475 cases/six helper
ignores each (25.95–27.60 seconds); 100 loaded tmux repetitions pass in 107.55
seconds, and macOS production-helper tmux tests pass 48/one ignore in 26.58
seconds. This does not establish the historical macOS failure's cause.

The eighth full Linux iteration fails a separate PTY fixture,
`queued_startup_suffix_obeys_deadline_and_close_discards_it`, at line 1525:
native close reports Cleanup while Running, then the fixture unwraps Process.
That run has 1,474 passes, one failure and six ignores in 26.37 seconds. The
campaign is not green; bounded diagnosis and replacement gates remain required.
Worker containers are removed and caches released.

## Job-control ancestry correction after `a007600`

Parser correction is integrated as `a0076004f22f947baba62b69099d771446a26bb8`
with the durable lifecycle contract and historical evidence. The next isolated
diagnosis reproduces the separate PTY close failure after 170 loaded fixture
repetitions, only in the expired-startup variant. Initial capture succeeds;
force-phase capture fails before KILL. Failure-only diagnostics prove bash moves
a new sleep descendant from group 5798 to 5800 while PID 5800, parent 5798 and
start identity 178978 remain identical. Whole-snapshot equality incorrectly
treats this ordinary job-control transition as changed ancestry.

Correction `b782556a902ea727bd52161d7d0a1f608e9a175c` changes only
`background_process.rs`: retained-directory parent revalidation ignores only
mutable PGID, retaining exact PID, process identity, parent PID and parent
identity. Descendant delivery remains pidfd-bound; root-group admission and
delivery checks are unchanged. No deadline, retries, output or no-execution
assertions change. The retained-directory regression is red before and green
after the correction, with negative controls for all four identity/ancestry
fields; temporary diagnostics are removed.

Exact Rust 1.94.1 Linux proc/ancestry tests pass 39/one helper ignore, strict
Linux/macOS native lint and formatting pass, and five default-concurrent Linux
suites pass 1,476/six helper ignores each (26.27–35.47 seconds). All 500 loaded
PTY suffix repetitions pass in 35.53 seconds. macOS production-helper PTY tests
pass 25/25 in 11.03 seconds. The complete replacement local gate, three fresh
reviews and exact remote success remain required; no delivery is claimed.

## Review and Ubuntu runner correction after `d49db6d`

Candidate `d49db6d81395b517049f0b43726bed764417f2c4` passes the complete
exact-1.94.1 local gate: default-concurrent Linux native 1,476/six helper ignores
and CLI 141/four helper ignores; macOS workspace/native 1,403/five helper ignores,
eight release launches, fresh CLI smoke and 658 production-helper tests/five
helper ignores; formatting, strict Linux/macOS workspace lint, doctests,
FreeBSD/WASI, dependencies, 260 Python checks/14 skips, drift/docs and release.
Three fresh direct local reviewers (`terminal_d49db6d_api`,
`terminal_d49db6d_lifecycle`, `terminal_d49db6d_resources`) report zero findings
against merge base `46e5b70f6c5ba76a4699f5bd8ba424a1fa3813be`. API also passes
37 cached confirmations (17 action, 12 Gateway, eight archive); all tracks
retain their stated runtime/performance limits. Review worktrees are removed.

The feature push starts CI `34137286794` and Benchmark `34137286780`.
Benchmark succeeds with unexpired exact-SHA artifacts `10024753482` (upstream)
and `10024581934` (bootstrap), expiring `2026-12-06T15:14:39Z`. Quality and both
Linux native jobs fail the same four zsh startup cases: 1,472 pass, four fail,
six helpers are ignored (57.20–65.07 seconds). Logs show global `compinit`
prompting for permission to continue, consuming the queued bootstrap prefix.
macOS ARM completes successfully; macOS x86_64 is still running at integration.
Main does not advance.

The [exact hosted Ubuntu image setup](https://raw.githubusercontent.com/actions/runner-images/ubuntu24-arm64/20260831.111/images/ubuntu/scripts/build/configure-system.sh)
makes `/usr/share` recursively writable; Ubuntu's global zsh configuration
enables completion initialization unlike the local Debian policy. In an
isolated container, unchanged d49 code passes the four matrices with Ubuntu
completion enabled and normal permissions, fails all four after making the
completion tree writable, then passes all four after the bounded repair.
The whole default-concurrent native suite subsequently passes 1,476/six helper
ignores in 26.35 seconds.

Correction `8d8fa6281c52b9511eb79895fe3c9bb2465ac04f` adds permission repair
only under `/usr/share/zsh` and a required noninteractive `compaudit` to both
Linux shell-provisioning paths. No Rust, product startup, User/Clean profile,
deadline or skip behavior changes. All 15 CI regression tests, YAML/bash syntax,
formatting and diff checks pass; the new regression rejects missing repair.
Diagnostic container/configuration is removed and shared binaries are unchanged.
Complete replacement local checks, three fresh reviews and exact remote gates
remain required.

## Bounded macOS inventory replacement after `b7f69b4`

CI `34137286794` ultimately fails on Intel macOS too: job `101791132224`
reports collector expiration at 253.454726 ms in
`real_factory_retains_full_command_and_large_paste` and 253.513073 ms in
`real_tmux_shared_bootstrap_preserves_profiles_and_durable_command_gate`.
The native result is 1,401 passes, two failures and five helper ignores in
296.65 seconds. Its ARM macOS counterpart passes. Benchmark success does not
override these failures, and main does not advance.

Ubuntu repair is integrated as `b7f69b466de7c712e5dc48fbc119af5bf4cd5ed5`.
Replacement prerequisites and default-concurrent Linux native 1,476/six ignores
and CLI 141/four ignores pass. The full macOS/review/remote replacement gate is
held for bounded inventory diagnosis; that candidate is not pushed or sealed.

Local ARM and Rosetta runs of the two exact Intel-failing fixtures pass.
Rosetta is not native Intel CI evidence. A production-helper ARM terminal sweep
passes 657 cases with five ignores but fails separately in
`dropped_pty_retains_scope_until_quarantined_cleanup_converges`: the original
startup deadline expires before frame writing. No inventory failure is reported
in that sweep. This separate timeout is not attributed to the inventory fix.

Thirty ARM observations during an x86_64 diagnostic build, with about 1,381
processes, measure `/bin/ps` at 31.67/36.78/51.57 ms min/median/max and fixed
`KERN_PROC_ALL` at 0.102/0.105/0.596 ms. Raw records occupy 894,888 bytes;
PID text is about 8.3 KiB. Source inspection confirms avoidable ps task/thread
queries. These observations motivate removing that work, not an exclusive
timeout-cause claim or an M07 threshold claim. Temporary diagnostics are removed.

Binding component `2cb1a0c921cb4a3d3c1a1e7113c0481f40468e96` adds the fixed
read-only query under the new narrow ADR 0004. It has an 8 MiB initialized
scratch cap, at most three data attempts and no partial-error publication.
Fifteen binding tests, strict exact-1.94.1 ARM64/x86_64 lint and both SDK ABI
assertions pass. The retained C fixture is wired into the existing selected
Apple-binding CI step. Its isolated worktree is integrated and removed.
Native/CLI capability wiring and complete replacement gates remain required;
no candidate acceptance or delivery is asserted by this component evidence.

The native integration's focused inventory group passes 11 tests in 0.65 seconds,
and the CLI exact-private-flag case passes. A broader 84-selection macOS run
is accidentally executed concurrently: 59 pass, 22 fail and three helpers are
ignored, with multiple collector timeouts. The unchanged code then passes the
same selections under the configured macOS CI policy (`--test-threads=1`):
81 pass, three helper ignores, no failures in 65.73 seconds. This includes both
earlier Intel-failing fixtures, PTY/captured/tmux ownership, prefix retention and
ten collector regressions. No deadline or production edits intervene. Concurrent
test contention is not proof of the original serial Intel timeout's cause.
Strict exact-1.94.1 native/CLI lint passes on macOS and Linux; the Linux cache is
package-cleaned before checking and its temporary container is removed.

Native/CLI component `bffc330f144df0f3fda3b6567ee7aa878dfe89bc` explicitly wires
the private helper through both cleanup ownership paths, preserves full helper
clones and legacy no-capability/group-only behavior, and rejects malformed
helper output before identity admission. Source-included background and PTY
component harnesses each pass five selected tests (0.94 and 0.93 seconds).
Formatting and diff checks pass; all worker processes stop and its clean,
integrated worktree is removed. Fresh release execution, the complete local
gate, three independent reviews and exact remote proof remain required.

## Composed-host test dispatch after `d55a7cb`

Integrated candidate `d55a7cbdbf1449cab8e4ca676ae9cfe647724c19` passes exact
Rust 1.94.1 formatting, both SDK ABI checks, strict Linux/macOS workspace lint,
doctests, FreeBSD/WASI, dependency policy/audit, 261 Python checks/14 skips
(376.075 seconds), pinned drift and documentation policy. Its fresh release
build completes in 9m06s; eight release-helper launch cases pass in 27.99 seconds
and CLI smoke passes. Clean default-concurrent Linux native passes 1,476/six
helper ignores in 27.16 seconds; CLI passes 141/four ignores in 0.77 seconds.

The full serial macOS native run fails: 1,400 pass, seven fail and five helpers
are ignored in 427.73 seconds. Six composed-host failures repeatedly reject
inventory collection before expiration. Source inspection proves their fake CLI
dispatches captured/PTY/marker modes but omits the new inventory mode. Merely
adding a branch would also expose libtest framing on the PID protocol stream;
the test-only correction must use the existing raw-output helper entrypoint.

After those host failures retain cleanup retries, the control-C PTY fixture
fails only at its final close: inventory expires at 250.923667 ms while Running,
then 250.684083 ms while Exited(0), and 252.61275 ms during drop. This is not
evidence that control-C delivery failed, nor proof that the host retries are its
exclusive cause. The separate clean-process confirmation remains required.
The run terminates normally with test exit 101; the later production-helper
terminal sweep is not executed. This candidate has no fresh review or remote
acceptance and is not pushed. Main remains unchanged.

The correction adds an exact-single-argument inventory branch to the fake CLI,
execing the existing raw-output entrypoint with libtest framing kept off the
protocol pipe. A new pre-host regression exercises the actual bounded collector
with that script. Direct first execution of a fresh printf-only script takes
0.46 seconds locally, versus 0.00 seconds through an explicit `/bin/sh` on a
separate fresh script. The regression therefore explicitly registers that
interpreter, as existing shell-backed helper fixtures do; the full host's direct
CLI path, production code and 250 ms limit remain unchanged.

With the same explicit-shell registration, the regression fails before the
dispatch fix in 0.08 seconds (valid mode rejected after 15.49 ms, not expired)
and passes afterward in 0.09 seconds. It checks canonical PID output and rejects
missing/extra arguments and an expired transferred deadline. The serial host
group plus the exact control-C case pass 10/one existing helper ignore in 6.03
seconds in a clean process, including all six earlier host failures. Cleanup
assertions pass and no matching helper processes remain. This does not establish
the exclusive cause of the earlier control-C close timeout.

Correction `05399d9ecb6ed36db7e6d0937a2ce4044b5a0207` changes only the native
test wrapper/regression and a macOS test-only adapter to the existing collector.
Strict exact-1.94.1 native all-target/all-feature Clippy and formatting pass.
The bounded native/CLI dispatcher audit finds no other generic fake CLI missing
the new mode. Temporary instrumentation/scripts are removed; its clean integrated
worktree is removed. The complete replacement gate and fresh reviews remain
required before any feature push or delivery.

## Exact `932b2cf` gates and subsequent corrections

Candidate `932b2cf324d1a402a68c4c87d52784f0cc7c4d89` passes the complete
exact-1.94.1 local gate: formatting, both SDK ABI assertions, strict Linux/macOS
workspace lint, doctests, FreeBSD/WASI, dependency policy/audit, 261 Python
checks/14 skips, pinned drift and documentation policy. Its fresh release build
and smoke pass, including eight release-helper launch cases. Default-concurrent
Linux native passes 1,476/six helper ignores (27.44 seconds), CLI 141/four
ignores (0.77 seconds). Serial macOS workspace native passes 1,408/five ignores
(325.42 seconds), terminal component 643/two ignores (151.57 seconds), and the
production-release-helper terminal sweep 659/five ignores (202.33 seconds).
All composed-host, control-C close and dropped-PTY cleanup cases pass.

Three fresh independent direct local reviews (`terminal_932_api`,
`terminal_932_lifecycle`, `terminal_932_resources`) inspect that full feature
against merge base `46e5b70f6c5ba76a4699f5bd8ba424a1fa3813be` and report zero
actionable findings. These are source/test reviews, not Bugbot or independent
runtime/performance measurements. Clean review worktrees are removed.

Feature CI `34148085502` fails; Benchmark `34148085579` succeeds with both
unexpired exact-SHA artifacts: `10028544765` (upstream), `10028457623`
(bootstrap), expiring `2026-12-06T17:33:18Z`. Quality, dependency, documentation,
FreeBSD/WASI, Linux x86_64 and ARM macOS jobs succeed. Main does not advance.

Linux ARM job `101824362043` passes 1,475 cases with six helper ignores but
fails `inline_reentrant_wake_completes_on_the_worker_without_self_joining` at
its initial-Pending assertion (58.79 seconds). Source inspection shows the
exit-zero helper may validly publish before the first poll registers a waker.
Correction `040bae2cc854437a1bc1b6f9fc06ec4f4567890a` changes only the existing
Linux test module: an existing before-spawn barrier orders waker registration,
the callback proves named-worker execution, and a separate already-published
case preserves valid immediate Ready. Both settle their worker permit tails.
Controlled publication-before-poll rejects the old Pending expectation in
0.01 seconds. All 16 system tests and 12 API tests pass; both scheduling cases
pass 100 repetitions each. Strict exact-1.94.1 native lint passes on Linux and
macOS; the active macOS unsupported-platform API test, formatting and diff
checks pass. No product behavior or deadlines change; owned containers stop.

Intel macOS job `101824362102` passes 1,406 cases with five helper ignores but
fails two inventory collections (319.71 seconds):
`explicit_owned_signal_kills_shell_and_final_drain_preserves_bytes` expires at
253.202581 ms during close, and
`real_tmux_shared_bootstrap_preserves_profiles_and_durable_command_gate`
expires at 252.483291 ms during a zsh clean commandless startup write-load.
The latter still has 3.876 seconds of its separate startup budget. Both failures
precede PID decoding/identity scanning; logs do not distinguish helper startup,
query, output or reaping delay. The inventory change is not accepted as a
complete fix. Bounded phase diagnosis and all replacement gates remain required.

Bounded isolated sampling subsequently compares 30 launches per path under the
unchanged 250 ms cutoff. On ARM, shell-to-libtest totals are 17.30/21.24/25.56 ms
min/median/max, direct libtest 11.44/14.79/18.56 ms, and the fresh ARM release
helper 6.68/8.80/12.08 ms. Spawn is at most 1.60 ms; first output dominates and
EOF/reaping generally follow within 1.2 ms. Direct helper query/decoding/sorting
takes 3.64–8.77 ms, with encoding adding about 0.37–1.11 ms.

Another 90 launches from a Rosetta x86_64 diagnostic build also finish before
the cutoff. Shell-to-x86-libtest totals are 58.18/67.32/85.18 ms; direct x86
libtest is 29.20/33.85/53.30 ms. The ARM release child under a Rosetta parent is
6.52/8.64/10.65 ms, not an x86 release comparison. Direct helper query/decoding/
sorting is 4.44–11.19 ms and encoding about 0.78–1.47 ms. Neither experiment
reproduces the native Intel failure or establishes its cause; no M07 claim is
made. Temporary ignored sampling tests and helper telemetry are removed.

Parent-only `cfg(test)` diagnostics extend the existing failure report with
helper selection and reservation/spawn/collector-entry times. A fixed-size
failure-only record captures first output, byte count, EOF, last child
observation, wait counts and last phase. No executable arguments/PIDs, added
files/subprocesses, helper protocol, injected-clock calls, deadlines, acceptance
or cleanup ownership change. Ten existing collector tests and one deterministic
counter test pass. Both exact Intel-failing cases pass with libtest helpers
(PTY 0.16 seconds, tmux 16.68 seconds) and release helpers (0.12 and 10.70
seconds). An initial wrong-module selector ran zero new tests; the corrected
exact selector runs and passes one. Complete replacement gates remain required.
Diagnostic component `394cb57768aa214ff4fb3f4d10816255fe181ede` passes strict
exact-1.94.1 native all-target/all-feature Clippy (1m23s), formatting and diff
checks. Its worktree is clean and all runtime sessions/cache leases are released
before integration. This evidence does not establish an Intel compatibility fix.

## Production-helper matrix selection after `d49f481`

Integrated diagnostic candidate `d49f48118d50462f6715912adf894144f8848602`
passes all prerequisite gates, 261 Python checks/14 skips (341.632 seconds),
fresh release build (8m26s), eight release-helper launch cases (28.04 seconds)
and release CLI smoke. Default-concurrent Linux native passes 1,477/six helper
ignores (26.87 seconds), CLI 141/four ignores (0.81 seconds).

The serial default-helper macOS workspace native run passes 1,408 cases with
five helper ignores but fails `durable_history_resize_matches_the_real_pty_and_survives_recovery`
(328.69 seconds). The PTY helper does not send its readiness byte within its
original two-second startup deadline: ready-read takes 1.982361583 seconds after
about 18 ms of prior preparation, and captured output remains unavailable.
No inventory-collection failure is reported. The run exits 101 and does not
reach later component suites or the production-helper terminal sweep. A fresh
isolated invocation passes with libtest (0.20 seconds) and release helper
(0.15 seconds); this does not explain or erase the failed gate.

The Apple matrix is changed to use the already-supported production-helper
fixture selection for all selected native tests. It builds the locked release
CLI for the exact matrix target, verifies the absolute executable and exports
`MACHINE_GOD_TERMINAL_RELEASE_BINARY` before the unchanged serial test command.
This tests the shipped dispatch and removes the measured libtest launch overhead
from default helper selection. Explicit protocol/failure fixtures, all tests,
deadlines, Linux commands/concurrency and package selectors remain intact.
Apple jobs without native/full-workspace selection do not build the helper;
documentation-only routing remains lightweight. The complete replacement gate
and native Intel evidence remain required, with phase diagnostics retained.
CI component `10d548dc151d0bce555a1920ef38efe19829dbb7` changes only the workflow
and its focused regressions. All 17 classification tests pass, including both
Apple target paths, selection boundaries and fail-closed build/executable
checks; no worker Rust builds or shared-cache writes occur. Its integrated
worktree is clean and removed.

The production-helper workspace comparison of the unchanged diagnostic Rust
code subsequently fails: 1,408 native tests pass, one fails and five helper
entrypoints are ignored (314.12 seconds). In
`real_failed_close_keeps_exact_artifact_cleanup_for_retry`, the configured
release inventory helper's reservation takes 11.75 microseconds, spawn returns
at 189.091875 ms and collection begins at 189.098084 ms. Collection spends
63.768541 ms with no bytes or EOF; the last child observation is Running at
32.9245 ms into collection. The original deadline expires at 252.877084 ms
total. The failed close therefore retains its child, contradicting the fixture's
expected later artifact-only failure state. This proves that recurring process
startup can consume most of the real release helper's inventory budget; it is
not proof of the earlier native Intel failure's exclusive cause. No gate is
accepted. The next correction removes recurring startup through an explicitly
owned, bounded reusable helper, with startup charged to the existing terminal
startup budget and query/identity deadlines preserved. Framing and lifecycle
changes require the full product gate; production-helper selection alone is
not recorded as a compatibility fix.

## Owned reusable inventory service integration

Protocol component `c9f2fec14a5912fccd9a0dc908f8dc5d4c6ea90f` adds the explicit
service flag, bounded big-endian framing, conservative absolute deadline
transfer and an independently killable helper loop. Its twelve protocol tests
and exact CLI dispatch test pass. Initial scoped lint uses a temporary
command-line dead-code allowance pending client integration; that is not a
strict integrated lint gate. The clean protocol worktree is removed after its
patch is integrated as `78d3fd8`.

Client component `e0c19ab` prepares the service before user-process startup,
shares weak-cached registrations within a host, and retains strong leases only
through native ownership. Exact child reap metadata blocks replacement through
quarantine without retaining a host scope in configuration. PTY positive close
releases its lease while preserving idempotence; tmux retirement cannot start
another inventory helper. Captured, PTY and tmux host paths share one explicit
registration. Legacy one-shot capabilities remain distinct and unchanged.

Test components `a54bf1f` and `d57f3df` add same-scope reuse, foreign-scope
rejection/rebind, last-lease settlement, pending-retirement barriers, malformed
replies, sequence exhaustion, delayed readiness, stalled queries and post-spawn
startup cancellation. Delayed readiness exceeds the query budget but is charged
to the original startup deadline; subsequent queries assert a single spawn.
No timeout is increased and no in-call retry is added. Native all-target/
all-feature compilation passes before the final framing and reap-ticket test
additions. Integrated focused runtime, strict lint, complete local checks,
fresh independent reviews and exact remote acceptance remain required.

The first integrated Linux strict lint run rejects a used underscore binding
and an oversized host-composition function before runtime. Follow-up components
`8a0e2aa`, `b1dc48c` and `be13748` add caller-level scope rebind and retired
PTY/tmux reuse assertions, extract shared helper registration, and fix those
lint failures plus a macOS test-only move from a drop-bearing reap permit.
Formatting and diff checks pass; replacement compile/runtime evidence is still
required. No failed check is treated as acceptance.

Focused service and protocol groups subsequently pass twelve cases each on
macOS; the service group includes eleven substantive regressions and its inert
helper entrypoint (0.90 seconds). Strict Linux workspace lint and both native/
CLI test builds pass, followed by both complete default-concurrent runtime
suites (CLI: 141 passed, four ignores, 0.77 seconds). The saved native output
was truncated, so no native duration is asserted. These are integration checks,
not the complete frozen-candidate gate.

Static lifecycle inspection then finds captured execution's host-stop and
dropped-response signals were not forwarded into the new readiness wait.
Component `8885686` threads those existing signals through startup checks,
bounded lock acquisition and readiness without adding a thread or deadline.
Its caller regression positively observes helper spawn, then requires scope
settlement under a five-second test bound rather than the original thirty-
second execution timeout, retaining executor/configuration and asserting that
the user command never starts. Exact macOS native all-target/all-feature strict
Clippy passes (1m22s); focused caller/PTY/tmux/host runtime is still pending.

On that same implementation content, nine exact macOS focused cases pass with
no failures or ignores (14.49 seconds; test build 1m58s): quarantine-ticket scope
settlement, retired tmux absence/Drop without respawn, captured scope rebinding,
stalled-startup host-stop and dropped-response cancellation, dormant and
unpolled-receipt host shutdown, idempotent PTY close, failed-close artifact retry,
and the real tmux eight-profile/bootstrap matrix. Installed tmux is exercised;
these are not unavailable-backend early returns. The full frozen local and
remote gates remain required.

## Reusable-service candidate `d6c1522`: local green, lifecycle rejection

Candidate `d6c15229be7ab4e16a6e07367a38dd1759cb89ff` passes the complete
exact-1.94.1 local gate: strict workspace lint on Linux/macOS, both Apple ABI
checks, formatting, doctests, FreeBSD/WASI checks, dependency checks, 263 Python
tests with fourteen platform skips (348.757 seconds), pinned upstream drift and
documentation policy. Fresh release build takes 8m33s; all eight release launches
pass (28.37 seconds), followed by release CLI smoke. Default-concurrent Linux
passes 1,477 native/six helper ignores (25.86 seconds) and 141 CLI/four ignores
(0.79 seconds). macOS workspace succeeds, including 1,435 native/five ignores
(380.34 seconds) and 668 component/two ignores (136.22 seconds); the final
release-backed all-feature terminal sweep passes 661/five ignores (200.38
seconds). A pre-test AI Gateway process delay resolves without intervention;
its 46 tests pass in 0.13 seconds. No runtime failure or retry is waived.

Two fresh independent direct local source/diff/test reviews inspect that exact
candidate against `46e5b70f6c5ba76a4699f5bd8ba424a1fa3813be`. Correctness/API
establishes zero actionable findings. Lifecycle/platform establishes one P1:
`CommandProcess::abort` and `NativeTerminalTmuxServer::retire` perform blocking
`Child::wait()` after kill. An unreapable child can indefinitely block the single
terminal owner, sequential cleanup of other sessions and host completion. The
candidate is rejected. Neither reviewer independently executes tests or claims
remote/native-Intel evidence, and neither review is attributed to Bugbot. A
third reviewer cannot start because the host reports an agent-thread limit on
both attempts; three-review success is not claimed. All clean review worktrees,
including the unused third checkout, are removed. The candidate is not pushed.

The correction replaces those raw owners with bounded exact-child ownership,
reserves existing reap capacity before spawn, and retains failed explicit
cleanup for retry. Drop transfers unresolved ownership and server-artifact
keepalive through the original scoped permit without restarting a spent grace.
Pending kill delivery can resume only after a later successful running-child
observation. New caller tests defer exact-child observation through a per-permit
test hook, checking retained command/server ownership, retry, quarantine scope
settlement and namespace artifact lifetime. Integration inspection also catches
nested cleanup refreshing the grace and interrupted observation being converted
to successful running-child observation. The correction preserves one exact
deadline across retries and retains an interrupted child without signaling. The
PID-targeted interruption regression fails on the earlier wrapper before the
correction. Worker commits `bc4c39e` and `d45ad68` pass exact-1.94.1 native
all-target/all-feature strict lint (34.66 seconds), formatting and all eighteen
focused cases (15.00 seconds, zero ignores). Seven real tmux/composed-host cases
use the explicitly selected `d6c1522` release helper; this is focused protocol
evidence, not a fresh candidate release gate. Complete replacement checks plus
three fresh reviews remain required before any delivery claim.
