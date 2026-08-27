# Milestone 03 native `terminal` review ledger

Status: **IN PROGRESS — CONTRACT FROZEN**

Slice 34 begins from exact delivered base
`52b5885f275c9f6f4f16b378f71780c29f2ebab2`. Its normative boundary is
[`../terminal.md`](../terminal.md). It implements only bounded foreground
`exec`; every other pinned-fx terminal action remains deferred.

## Initial composition

The review-exempt frozen-contract checkpoint is exact `79ae1b7`. Exact isolated
host component `ecf3e78c4abc70bb4f3329a6f8dffa9237ff130b`, production precursor
`ea216db80cb38601da268d84dd05b962c802c5df`, independent-evidence component
`785b193e2735401ffd3966ed6ef84db637891d59`, and lifecycle remediation
`64a48504275afc0e6989cee03eeeeb8174267225` are followed by configurable-system
component `13dfd28b9f49424c5272545b7ce6ceb75f445b92` and expanded Linux-evidence
component `ebc631e136bd052a41ff104534b5ecc0d5613d93`. They compose through local
exact `59b069a84e7d4dc4d76ac65520b9045603cae8af`.

Early independent evidence found that an inline reentrant Waker makes the
literal no-thread-tail wording impossible: a worker cannot join itself from its
own callback. Before formal review, the contract consequently allowed a
resource-free notification callback tail to self-detach while still claiming
every original process-group member was gone. Formal cycle 1 below rejected
both the self-only Waker exception and the stronger group-disappearance claim.
Production had also remediated pre-spawn deadline, reader-join, escaped-writer,
exit/signal-range, duration, pending-executor deadline, cancellation-precedence,
and portability gaps before that formal candidate.

Exact focused evidence is green for four private limit/deadline/outcome tests,
nineteen portable contract/lifecycle tests, two engine permission/durability
tests, one unsupported-platform test, two workspace clone/failure tests, and
eight reference-host tests. Seven real Linux process tests compile and await
Linux execution.

## Cycle 1 candidate gate

Exact precursor `ec8bbc97c23022db3f1884cac083c3cbe4460825`, tree
`afe5445ffc0bef629db82ce8a678558c470a675d`, passed the four required Rust gates
but failed the first supplemental Python run in three stale manifest assertions.
They still expected `sha2` to be optional and HTTP-feature-scoped even though
terminal now requires it for environment-snapshot identity in every native
feature topology. Exact test/documentation remediation
`80c6ee0d09c7fe5feaed96504a2f931aeb131208`, tree
`027c4bc27b90526e172b1306d0ef3700dd39a745`, corrects those assertions without
changing product Rust. Its complete replacement gate is green under exact Rust
and Cargo 1.94.1 without fallback:

- all four required commands pass: formatting, warnings-denied workspace all-
  target/all-feature Clippy, 1,147 listed non-documentation tests, and two
  doctests;
- all 136 Python tests pass with eight expected macOS skips, and regeneration
  against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` is byte-stable;
- exact `cargo-deny` 0.20.2 passes with the three established duplicate
  warnings, while `cargo-audit` 0.22.2 loads 1,226 advisories, scans 211 lockfile
  dependencies, and reports zero vulnerabilities;
- FreeBSD and WASI baseline checks pass, with only the established WASI
  `read_file` warning. Linux terminal test checking and warnings-denied Clippy
  pass. Foreign all-feature Linux compilation stops in `aws-lc-sys` because the
  macOS host lacks Linux C headers, before product Rust; native Linux CI remains
  authoritative;
- documentation integrity covers 89 Markdown files, 312 fence markers, 678
  parsed links, and 515 repository-relative targets with zero missing targets;
- the exact 29-file base diff is +4,484/-192, adds no unsafe Rust, and leaves
  workflows, benchmarks, generated compatibility data, the root manifest, and
  `Cargo.lock` unchanged; and
- a fresh locked 3,985,216-byte arm64 Mach-O release binary has SHA-256
  `b515ce0951f44a1e30171ee69c400cb9e750430e3a9d4959028ab49a16a55383` and
  passes help and status smoke paths.

This evidence makes no product-performance, compatibility-promotion, or fx-
equivalence claim.

## Formal cycle 1 review

Three adversarial product tracks reviewed exact candidate
`fba499e1410f307a9ec79ac3d1df72e82008a7ff`, tree
`8679917ddab5225a92dc17d8ac19ff815c34b46f`, and **REJECTED** it:

- correctness/API/schema reported `0/0/1/2`;
- native process/lifecycle/platform reported `0/1/3/1`; and
- performance/concurrency/resources reported `0/2/2/1`.

Overlap deduplicates to `0 blocker / 3 high / 3 medium / 4 low`:

1. **High:** deadline, output-limit, and direct-exit races can produce a non-
   output-limit status with more than 1 MiB observed, which outcome validation
   collapses to a generic executor invariant.
2. **High:** process-group cleanup dispatches `SIGKILL` without bounded
   observation or error discrimination, so its documented disappearance claim
   is stronger than the implementation and Linux process model can prove.
3. **High:** synchronous cwd/proc lookup and the spawn boundary can block beyond
   the advertised absolute deadline before or beneath the guardian. Safe Rust
   cannot preempt a blocked host syscall; remediation must narrow that claim and
   retain deadline enforcement around every controllable phase.
4. **Medium:** executor and guardian destruction can cancel the token after
   `await_executor` returns, but no post-await or final pre-publication check
   arbitrates that cancellation.
5. **Medium:** readers continue hardware-rate draining during termination grace,
   and overflow does not immediately stop both readers, so the 1 MiB work cutoff
   and post-stop read claim are false.
6. **Medium:** normal exit reaps the leader before signalling the numeric process
   group, allowing theoretical PID/PGID reuse to target an unrelated group.
7. **Low:** nested `~` cwd components are accepted despite the frozen grammar,
   while the docs omit the implemented U+2028/U+2029 rejection.
8. **Low:** public non-Linux system construction fails, but private reference-
   host composition intentionally advertises a terminal that fails at execution;
   the contract does not state that exception.
9. **Low:** injected environments are content-sorted before individual and
   aggregate size validation, allowing rejectable oversized inputs to cause work
   outside the advertised construction bounds.
10. **Low:** Linux system evidence does not yet prove retained-root replacement,
    cwd-dependent execution, or the exact snapshotted environment.

All findings are accepted for source, evidence, or honest-contract remediation.
The replacement contract makes a finished publisher tail exception for any
arbitrary inline or blocking Waker only after command resources are gone. It
starts timeout accounting on first poll and independently enforces it around
controllable userspace phases without claiming safe Rust can preempt a blocked
host syscall. Observed aggregate output above 1 MiB is authoritative and stops
both readers under deterministic read-count/overshoot bounds. Group cleanup
retains the leader identity through signal dispatch, distinguishes absence from
ambiguity, observes after KILL, and reaps the direct child. Successful cleanup
claims no observed signalable original-group member remains; adopted zombies,
credential-escaped or unsignalable members, `setsid` descendants, and
uninterruptible waits are not claimed away. Fixed cleanup ambiguity returns an
observable `terminal_wait_failed`; drop has no result channel and does not claim
ambiguous members disappeared. Cancellation is rechecked after executor/
guardian teardown and directly before `ToolOutput` return. Public system
construction fails off Linux, while private host composition advertises
`terminal` and fails fixed unsupported at allowed execution before cwd lookup or
spawn.

Cycle 2 requires a complete replacement gate and three fresh exact-SHA reviews.

## Cycle 2 remediation and replacement gate

The three accepted remediation tracks were implemented in non-overlapping
isolated worktrees from exact cycle-1 record `49567ec`. Independent evidence
component `8c58c29418f4bf74cb646790c21539a8891db57b`, tree
`5e34b9db3ff456f4f1b6f98d5b7ffbb5bac28a85`, integrated as exact `ac4c3c3`.
Documentation component `ddccdb2759210ae2cccc2a53668da59cf7811edf`, tree
`8831c0c1787166d34c41f9a5f67e34a2fc1df50b`, integrated as exact `e30a195`.
Production component `3725aedb066ec99240c5db2d4263ad6952e97aba`, tree
`f23bd94df13d110996bc9350abefab12341ffe97`, integrated as exact
`b03ecb03d9786be3d43cca7dc6885e273aa36b5d`. Each source worktree was clean
before integration; all three worktrees and temporary branches were then
removed and pruned.

The replacement behavior head `b03ecb03d9786be3d43cca7dc6885e273aa36b5d`,
tree `06a87fd085bb1024572ac04abbb68f1ee958e9a0`, passes its complete local gate
under exact Rust and Cargo 1.94.1 without fallback:

- the two regressions that failed on the rejected candidate now pass, followed
  by 21 portable terminal integration tests and five private terminal tests;
- all four required commands pass, with warnings denied across every workspace
  target and feature, 1,150 listed non-documentation tests, and two doctests;
- all 136 Python tests pass with eight expected macOS skips, and regeneration
  against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` is byte-stable;
- exact `cargo-deny` 0.20.2 accepts advisories, bans, licenses, and sources with
  the three established duplicate warnings; `cargo-audit` 0.22.2 loads 1,226
  advisories, scans 211 lockfile dependencies, and reports no vulnerability;
- Linux native-library and terminal-test checks and warnings-denied Clippy pass;
  FreeBSD warnings-denied Clippy passes; WASI no-default and all-feature checks
  pass with only the established unrelated `read_file` warning;
- documentation integrity covers 89 Markdown files, 312 fence markers, 678
  parsed links, and 515 repository-relative targets with zero missing targets;
- the exact 29-file base diff is +5,130/-192, adds no unsafe Rust, and leaves
  workflows, benchmarks, compatibility data, the root manifest, and
  `Cargo.lock` unchanged; and
- a fresh locked 3,985,216-byte arm64 Mach-O release binary has SHA-256
  `b515ce0951f44a1e30171ee69c400cb9e750430e3a9d4959028ab49a16a55383` and
  passes the available help and status smoke paths. The terminal tool is not a
  CLI subcommand, so its user-visible boundary is exercised through the native
  and engine integration targets rather than a fabricated CLI route.

This gate makes no formal cycle-2 review, remote workflow, integration,
performance, sandbox, or fx-equivalence claim. Formal reviewers identify the
exact immutable candidate and tree below.

## Formal cycle 2 review

Three fresh adversarial product tracks reviewed exact candidate
`a566db2b44a40cc64e4c4029e42a664c80eea074`, tree
`785c0841d15171c1efeecd9fdfd1c64690f535b6`, and **REJECTED** it:

- correctness, public API, schema, capability, and engine integration reported
  `0/1/0/1`;
- native process, cancellation, lifecycle, and platform behavior reported
  `0/1/0/2`; and
- performance, concurrency, memory/output bounds, and resource ownership
  reported `0/2/1/1`.

Overlap deduplicates to `0 blocker / 2 high / 1 medium / 2 low`:

1. **High:** the outer deadline guardian checks expiry before a ready executor
   and rewrites every non-timeout result after the deadline. A valid
   `output_limit` can therefore become a constructor-invalid `timed_out` result
   with more than 1 MiB observed. Production readers also expose no cause marker
   until group cleanup and outcome publication finish, so a deadline during
   overflow cleanup can discard an already-observed authoritative overflow.
2. **High:** worker and deadline threads invoke arbitrary Wakers inline, then
   detach an unfinished notification tail after the outer future consumes the
   result. A legal permanently blocking Waker can therefore leave one OS thread
   and stack per completed execution outside the active-execution limit.
3. **Medium:** ordinary Linux exit retains the zombie leader and consequently
   sees a successful group TERM even when no live descendant exists, then
   unconditionally sleeps through the full 250 ms grace before KILL and reap.
   Every trivial command therefore pays that latency.
4. **Low:** the grammar rejects components spelled exactly `~`, but the source
   also rejects every leading tilde-prefixed literal such as `~cache` while
   accepting the same literal when nested.
5. **Low:** public `TerminalExecutor` Rustdoc still promises an unconditional
   absolute bound and joining every worker, contradicting the maintained
   controllable-phase and bounded-notification-tail contract.

All findings are accepted. Replacement source must publish the production
output cause before cleanup, arbitrate cancellation first and observed output
second, and never mutate a validated outcome into an invalid status/counter
combination. Blocking notification callbacks must retain a per-tool active
lease until return so repeated legal callbacks fail fast at the configured cap
instead of accumulating without bound. Normal-exit cleanup must preserve the
leader identity through its final signal without imposing the cancellation/
timeout grace on an already-exited foreground leader. Grammar and public
Rustdoc must align with the frozen contract, with independent deterministic
regressions for each behavior.

All three review worktrees were read-only and clean; each was removed and
pruned with its temporary branch immediately after its verdict. Cycle 3
requires a complete replacement gate and three fresh exact-SHA reviews.

## Cycle 3 remediation in progress

Accepted remediation is split across non-overlapping production, evidence, and
maintained-documentation worktrees. The maintained contract now records the
required replacement semantics:

- the production output-limit cause is visible to the guardian before cleanup
  and wins a concurrent deadline, while final arbitration cannot turn a
  validated outcome into a contradictory status/counter combination;
- a worker or deadline thread blocked in an arbitrary Waker callback retains
  its originating per-tool active slot until return, bounding callback threads
  and stacks at configured capacity and making later calls fail fast as busy;
- cancellation, timeout, overflow, and drop use TERM grace then final KILL,
  while an already-exited foreground leader receives TERM and immediate final
  KILL with its identity retained through signal dispatch, then reap and bounded
  disappearance observation;
- only a component spelled exactly `~` rejects; literal `~cache` is valid in a
  leading or nested position; and
- the public timeout remains non-unconditional across blocked host syscalls,
  synchronous executor poll/drop, and arbitrary callback execution.

This is a remediation-status record only. It does not assert that source and
evidence components have been composed, name a replacement candidate or tree,
claim a green local gate or review, or advance remote delivery.

## Cycle 3 remediation and replacement gate

Cycle-3 remediation was completed in three non-overlapping isolated worktrees.
Documentation component `2bf62a332534844d9d50298d843e2da3f3bdb4e8`, tree
`4554f012`, was integrated as `9e2ed710`. Independent evidence component
`6a1543855b8e2f7d569369de886299156af05952`, tree `c74b58`, was integrated as
`6ec06ed3`. Production component
`44f13efe473ef3a92fce71a04bfdc57c7bdad2c3`, tree `372f466`, was integrated as
`281f5e96`. Exact follow-up `62d0fe17e765b86be12efe8f99c96f871b47ba0c`,
tree `0e6f94b`, preserves timeout priority for a non-output executor error that
becomes ready after the deadline and narrows the public callback-tail wording.
Every source worktree was clean before integration; every source worktree and
temporary branch was removed and pruned immediately afterward.

The exact replacement behavior head
`62d0fe17e765b86be12efe8f99c96f871b47ba0c`, tree
`0e6f94ba8bb822f3bf5dd0f7ab79c875214aa0fd`, passes its complete local gate
under exact Rust and Cargo 1.94.1 without fallback:

- all four required commands pass, including warnings-denied workspace
  all-target/all-feature Clippy, 1,157 listed non-documentation Rust tests, and
  two doctests;
- the external terminal suite passes 24 portable cases and the native library
  includes nine private terminal cases. Deterministic regressions cover
  output-limit/deadline priority, timeout/error priority, literal tilde-prefixed
  directories, blocked worker and deadline Waker leases, Linux overflow during
  process-group cleanup, and ordinary-exit latency;
- all 136 Python tests pass with eight expected macOS skips, while regeneration
  against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` remains
  byte-stable;
- exact `cargo-deny` 0.20.2 accepts advisories, bans, licenses, and sources with
  the three established duplicate warnings; `cargo-audit` loads 1,226
  advisories, scans 211 lockfile dependencies, and reports no vulnerability;
- Linux native-library and terminal-test checks and warnings-denied Clippy pass;
  FreeBSD warnings-denied Clippy passes; WASI no-default and all-feature checks
  pass with only the established unrelated `read_file` dead-code warning;
- documentation integrity covers 89 maintained Markdown files, 312 fence
  markers, 678 parsed links, and 515 repository-relative targets with zero
  missing targets;
- the exact 29-file base diff is +5,926/-193, adds no unsafe Rust, and leaves
  workflows, benchmarks, compatibility data, the root manifest, and
  `Cargo.lock` unchanged; and
- a fresh locked 3,985,216-byte arm64 Mach-O release binary has SHA-256
  `b515ce0951f44a1e30171ee69c400cb9e750430e3a9d4959028ab49a16a55383` and
  passes help and isolated no-residue status smoke. The terminal boundary is
  exercised through native and engine integration tests because it is not a
  CLI subcommand.

The composed implementation publishes an output-limit cause before process
cleanup and lets that validated cause win a simultaneous deadline. Cancellation
still has first priority; a ready non-output error after the deadline remains a
timeout. Each worker or deadline callback holds a shared per-tool active lease
until its Waker returns, so arbitrary blocking callbacks consume capacity and
cannot accumulate outside the configured admission bound. Ordinary observed
foreground exit sends TERM and immediate final KILL before reap rather than
waiting through the cancellation grace. Only a component exactly equal to `~`
is rejected. Public documentation explicitly excludes blocked host syscalls,
synchronous executor poll/drop, and arbitrary callback tails from an
unconditional wall-clock guarantee.

This replacement gate makes no formal cycle-3 review or remote-delivery claim.
The immutable review candidate and tree are recorded only after the gate record
is committed, and three fresh product-review tracks must inspect that exact
object.

## Formal cycle 3 review

Three fresh adversarial product tracks reviewed exact candidate
`3215b9eeef9a8d139be84761aec1f08d4f321bb0`, tree
`3fafcade3056131ea978e98d43cf01a17adc58aa`, and **REJECTED** it:

- correctness, public API, schema, capability, and engine integration reported
  `0/1/0/1`;
- native process, cancellation, lifecycle, and platform behavior reported
  `0/1/1/1`; and
- performance, concurrency, memory/output bounds, and resource ownership
  reported `0/0/1/0`.

Overlap deduplicates to `0 blocker / 2 high / 2 medium / 0 low`:

1. **High:** after the deadline timer becomes ready and the guardian's second
   executor poll returns pending, a production reader can set the overflow
   marker before timeout publication. Dropping the executor then joins and
   observes authoritative output limit but discards it, so the outer result is
   an empty timeout. Existing Linux evidence establishes overflow earlier and
   does not control this final interleave. The replacement needs a shared cause
   arbiter or equivalent close-and-recheck protocol plus a deterministic
   barrier regression.
2. **High:** a public injected executor has no access to the private callback
   lease authority. It may legally publish, retain the supplied task Waker, and
   block while invoking it; a concurrent poll consumes the result and releases
   the originating permit. Repeated calls can therefore accumulate injected
   publisher threads and stacks outside configured capacity. Terminal-owned
   polling must supply an opaque capacity-backed notifier/Waker, or the public
   seam and detached-tail contract must be narrowed consistently.
3. **Medium:** the deadline and production publishers independently increment
   the active counter. One admitted execution at capacity one can therefore
   leave two callback threads/stacks after its permit releases, despite the
   maintained exact-capacity statement. Replacement ownership must share or
   transfer the one originating admission slot through all retained Waker
   families and callback returns rather than allocate a counter entry per
   callback.
4. **Medium:** an unfinished stopped deadline thread or published worker thread
   is detached even when it never found a Waker and acquired no callback lease,
   or after its callback lease ended just before thread return. Repeated fast
   ready executions under scheduling delay can leave unaccounted thread tails.
   Replacement code must synchronously join an unfinished no-callback tail or
   keep the originating slot alive through actual thread completion, with
   deterministic publication-gap and fast-ready regressions.

The accepted unified remediation is one execution-activity ownership protocol:
one admitted active slot is shared by the outer execution, every TerminalTool-
supplied Waker family, and both native guardian/worker threads until all of them
finish. Public injected executors receive only the wrapped task Waker during
polling, so a retained or in-flight callback also retains that originating
slot. No publisher allocates a second active count. Native threads hold the same
activity through return even without a registered Waker. Final cause selection
checks cancellation first, then uses one linearized close: overflow observed
before timeout closes must complete a valid output-limit outcome, while a
timeout that closes first remains authoritative against later overflow.

Every review worktree remained read-only and clean. Each worktree and temporary
branch was removed and pruned immediately after its verdict.

## Cycle 4 remediation in progress

Cycle-4 source, independent-evidence, and maintained-documentation remediation
is in progress in non-overlapping isolated worktrees. The intended replacement
has one admitted execution activity consuming one active slot. That same slot,
without a second active-count increment, is owned by the outer call, its request
and executor, every TerminalTool-supplied Waker clone and callback, and native
worker/deadline threads through actual return. TerminalTool wraps the task Waker
before polling a public injected executor, so that executor needs no private
counter authority. Retained requests or Wakers and native threads that observed
no Waker keep the same activity; later admissions fail fast as busy until its
last owner returns or is dropped.

Final cause selection uses one linearized close after cancellation is checked
first. Overflow observed before the timeout close wins and completes a valid
output-limit outcome. A timeout close that wins first remains authoritative
against overflow observed later. Publication cannot expose a contradictory
status/counter pair.

This is a remediation-status record only. It does not assert source/evidence
composition, name a replacement candidate or tree, claim a green gate or
review, or advance remote delivery. Cycle 4 still requires a complete
replacement gate and three fresh exact-SHA reviews.

## Cycle 4 composition and replacement gate

Cycle-4 remediation completed in three non-overlapping isolated worktrees.
Maintained-documentation component
`0aba476fcfc03fd428a3053e30ebb2c40aef005d`, tree
`a0713151f29447f9de9f3f55d1c056ead3274a22`, integrated as `35c094f`.
Independent-evidence component
`99ed96ec07b2dab202b1c314cdb7afa73330ca93`, tree
`7c90d23fabee4d9d2ed681feb700e05f9e8e41e6`, integrated as `b1dc1e6`.
Production/private-evidence component
`b204217f803de0f937602b190cd905f4995f2054`, tree
`b4b9785785731ffd8242ae899cb384c1034b1f67`, integrated as exact behavior
head `4fda23571ed98269469ada74c1c90d074b5beae7`, tree
`6907c2742b0c0ee6c9a240cb4cbac9dab34fbc79`. Every worktree was clean before
integration; all three worktrees and temporary branches were then removed and
pruned.

The composed behavior uses one `Arc`-owned execution activity for one admitted
active slot. The outer execution, owned request/executor, transparent Waker
wrappers supplied to injected executors, and native worker/deadline threads all
share that activity. Retained Waker families, in-flight callbacks, requests,
and no-Waker native thread tails therefore keep the same single count occupied
until their last owner returns or is dropped; publishers never increment a
second callback count. A shared atomic cause provides a single output-limit or
timeout close. Cancellation remains first. Output limit claimed before timeout
closes keeps the guardian pending until the executor publishes its validated
bounded outcome; timeout that closes first remains authoritative over later
overflow. A terminal executor error after an output claim becomes a fixed
invariant failure rather than a fabricated contradictory result.

The exact behavior head passes its complete replacement gate under exact Rust
and Cargo 1.94.1 without fallback:

- all four required commands pass, including warnings-denied workspace
  all-target/all-feature Clippy, 1,163 listed non-documentation Rust tests, and
  two doctests;
- focused exact tests pass 27 external terminal, 12 private terminal, two
  engine, and one unsupported-platform cases. The two independent activity-
  ownership regressions deterministically failed the rejected cycle-3 base and
  now pass. Private evidence controls the exact post-second-poll/pre-timeout-
  close output claim and the no-Waker thread lifetime;
- all 136 Python tests pass with eight expected macOS skips, while regeneration
  against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` remains
  byte-stable;
- exact `cargo-deny` 0.20.2 accepts advisories, bans, licenses, and sources with
  the three established duplicate warnings; `cargo-audit` loads 1,226
  advisories, scans 211 lockfile dependencies, and reports no vulnerability;
- Linux native-library and terminal-test checks and warnings-denied Clippy pass;
  FreeBSD warnings-denied Clippy passes; WASI no-default and all-feature checks
  pass with only the established unrelated `read_file` dead-code warning;
- documentation integrity covers 89 maintained Markdown files, 312 fence
  markers, 678 parsed links, and 515 repository-relative targets with zero
  missing targets;
- the exact 29-file base diff is +6,712/-193, adds no unsafe Rust, and leaves
  workflows, benchmarks, compatibility data, the root manifest, and
  `Cargo.lock` unchanged; and
- a fresh locked 3,985,216-byte arm64 Mach-O release binary has SHA-256
  `b515ce0951f44a1e30171ee69c400cb9e750430e3a9d4959028ab49a16a55383` and
  passes help plus isolated no-residue status smoke.

This gate makes no formal cycle-4 review or remote-delivery claim. Three fresh
product-review tracks must identify the immutable candidate and tree recorded
after this gate result is committed.

## Formal cycle 4 review

Three fresh adversarial product tracks reviewed exact candidate
`105259befa46a26e2854820a60bfbbec0c7e39bb`, tree
`003262f0167e4a3eedbc5b2c55211b386db82986`, and **REJECTED** it:

- correctness, public API, schema, capability, and engine integration reported
  `0/2/0/0`;
- native process, cancellation, lifecycle, and platform behavior reported
  `0/2/1/0`; and
- performance, concurrency, memory/output bounds, and resource ownership
  reported `0/1/1/1`.

The performance track's low-severity wording discrepancy and the lifecycle
track's medium-severity early activity release are the same underlying defect.
After overlap deduplication at the higher severity, the union is
`0 blocker / 2 high / 2 medium / 0 low`:

1. **High:** the outer cancellation future is polled with the raw caller Waker.
   Since cancellation invokes registered Wakers inline, a blocking cancellation
   callback can outlive a concurrent completion and escape execution-activity
   accounting. Repeated calls can accumulate callback threads and stacks beyond
   configured capacity.
2. **High:** after output limit claims the shared cause, executor cleanup may
   still fail with a specific wait or pipe error. Before the deadline that error
   is preserved, but after the deadline the guardian converts it to a generic
   executor invariant. Classification therefore depends on elapsed time after
   an already-linearized cause rather than on one documented precedence.
3. **Medium:** each transparent activity Waker clone forwards independently.
   One contract-valid injected executor can clone the supplied Waker and start
   arbitrarily many simultaneous blocking callbacks. Those callbacks retain one
   activity count but still create unbounded threads and stacks for one admitted
   execution, contradicting the configured-capacity resource bound.
4. **Medium:** the explicit final activity drop releases capacity before bounded
   rendering, the last cancellation check, and public return. Maintained text
   and acceptance evidence say the outer call owns the activity through return,
   while an existing test encodes release before publication.

All findings are accepted. Cycle-5 replacement uses one shared activity-backed
coalescing notifier for every terminal-owned Waker registration: cancellation,
injected/system executor polling, and deadline notification. Retained clones
keep the activity, but at most one underlying caller-Waker callback is in flight
for an execution; concurrent notifications coalesce without holding internal
locks across arbitrary clone, drop, or wake behavior. An output-limit claim
closes out timeout competition but does not fabricate an output-limit result:
successful cleanup publishes the validated output-limit outcome, while cleanup
failure preserves its specific typed error on either side of the deadline.
Cancellation remains first. The outer activity remains owned through bounded
rendering, the final cancellation check, and function return.

Each review worktree remained read-only and clean. Every cycle-4 review
worktree and temporary branch was removed and pruned immediately after its
verdict. Cycle 5 requires non-overlapping source, independent-evidence, and
maintained-documentation remediation, a complete replacement gate, and three
fresh exact-SHA reviews.

## Cycle 5 remediation in progress

Cycle-5 remediation is split across isolated worktrees. Production owns the
shared coalescing notifier, unified registration path, stable cleanup-error
precedence, and through-return activity lifetime. Independent evidence owns
deterministic public saturation/recovery and callback-fan-out regressions.
Maintained documentation owns the exact notifier, precedence, and publication
contract. This is a remediation-status record only: it does not assert composed
behavior, a green gate or review, or remote delivery.

## Cycle 5 composition and replacement gate

Cycle-5 remediation completed from non-overlapping isolated components.
Maintained-documentation component
`774c725e4ea03871e687e57f0c202161fb5702d7`, tree `df8a6e0`, integrated as
`d8d6708`. Independent-evidence component
`d854c1435c24930b43eb2e010e72c3c348e6348a`, tree `b774248`, integrated as
`02d9969`. Production/private-evidence component
`0453fa8b67accfd079a5cd6dafa9ec09011cc09a`, tree `598d84e`, integrated as
`93796de`. The stale dual-callback test was corrected by exact component
`ecff5dc407faa94c0844f58eb5a55ac3768d172a`, integrated as `9021971`.

Composition then exposed two distinct issues. The external retained-publisher
fixture installed a fresh supplied Waker on its final poll but did not remove it
on future drop, correctly keeping the activity busy forever under the product
contract. Exact evidence component
`80fc24e3f8bcd17ba41497b99b4a608670c4e27d`, tree `bf7ba0a`, integrated as
`ac6d3e9`, makes that fixture deregister its retained Waker and accepts at most
one serialized callback replay. Production also discarded every notice during
an in-flight callback. A legal inline poll could observe the first notice before
a later deadline or executor notice was discarded, losing the required later
wake. Exact source component
`a4845aad049b7293d2177ade7fd7f618469ff1c5`, tree `ca0795f`, integrated as
`2e86fb6`, adds poll-observed pending arbitration: notices before a re-poll
coalesce into the current callback, while a notice after that observation gets
one serialized replay to the latest target. Callback concurrency remains one.
Exact `dc82875` factors the bounded recovery probe for warnings-denied Clippy,
and `3bfc0bf` aligns the maintained contract.

Every component worktree was verified clean before integration. Each worktree
and temporary branch was removed and pruned immediately after its iteration.
Exact behavior head `3bfc0bfaf7bed4d27a6cc3588d1edb72d04778c7`, tree
`d4ed10ddbb2f8ca9eaee57511555d1418592c352`, passes the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback:

- all four required commands pass, including warnings-denied workspace all-
  target/all-feature Clippy, 1,170 listed non-documentation Rust tests, and two
  doctests;
- focused suites pass 29 external terminal, 17 private terminal, two engine,
  and one unsupported-platform cases. The cycle-4 cancellation and fan-out
  regressions fail on the rejected base and pass here; private evidence covers
  typed cleanup errors on both sides of the deadline, serialized wake replay,
  replay suppression after an observing poll, panic recovery, and target-before-
  activity destruction;
- all 136 Python tests pass with eight expected macOS skips, while regeneration
  against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` remains
  byte-stable;
- exact `cargo-deny` 0.20.2 accepts advisories, bans, licenses, and sources with
  the three established duplicate warnings; `cargo-audit` 0.22.2 loads 1,226
  advisories, scans 211 lockfile dependencies, and reports no vulnerability;
- Linux native-library and terminal-test checks and warnings-denied Clippy pass;
  FreeBSD warnings-denied Clippy passes; WASI no-default and all-feature checks
  pass with only the established unrelated `read_file` dead-code warning;
- documentation integrity covers 89 maintained Markdown files, 312 triple-
  backtick occurrences, 678 parsed links, and 515 repository-relative targets
  with zero missing targets;
- the exact 29-file base diff is +7,717/-193, adds no unsafe Rust, and leaves
  workflows, benchmarks, compatibility data, the root manifest, and
  `Cargo.lock` unchanged; and
- a fresh locked 3,985,216-byte arm64 Mach-O release binary has SHA-256
  `b515ce0951f44a1e30171ee69c400cb9e750430e3a9d4959028ab49a16a55383` and
  passes 672-byte help plus 355-byte isolated status smoke with empty stderr and
  no state or temporary-directory residue.

This gate makes no formal cycle-5 review or remote-delivery claim. Three fresh
product-review tracks must identify the immutable candidate and tree recorded
after this gate result is committed.

## Formal cycle 5 review

Three fresh adversarial product tracks reviewed exact candidate
`0c04859ca48f81ff1fbaf89327e74231dee5e77c`, tree
`b0359fb7209e150d2cd0eef9a316148e58e6b772`, and **REJECTED** it:

- correctness, public API, schema, capability, and engine integration reported
  `0/1/0/0`;
- native process, cancellation, lifecycle, and platform behavior reported
  `0/0/0/0`; and
- performance, concurrency, memory/output bounds, and resource ownership
  reported `0/0/0/1`.

The deduplicated union is `0 blocker / 1 high / 0 medium / 1 low`:

1. **High:** the outer host may legally re-poll the terminal future using the
   TerminalTool-supplied Waker retained by an injected executor. The shared
   notifier then binds its own Waker as its target, forming an `Arc` self-cycle.
   Later executor, deadline, or cancellation notifications recurse into the
   notifier and coalesce without reaching the original task Waker. The future
   may hang and the activity slot remains permanently busy.
2. **Low:** top-level [`../../README.md`](../../README.md) and
   [`../README.md`](../README.md) still describe cycle-4 remediation as in
   progress and deny that a replacement composition, gate, or candidate exists,
   contradicting this ledger and the implementation plan.

Both findings are accepted. Cycle-6 source must recognize the supplied notifier
Waker and never install it as the notifier target, preserving the last external
target. Completion must explicitly close the notifier, remove and destroy its
target outside the lock, suppress later delivery to the completed task, and
still let every independently retained supplied-Waker clone own the activity
until that clone is dropped. Public evidence must re-poll with the retained
supplied Waker and prove executor, deadline, and cancellation delivery plus
capacity recovery without recursion or a cycle. Maintained status entry points
must describe the exact cycle-5 gate and formal-review-pending cycle-6
remediation state.

Every cycle-5 review worktree remained read-only and clean. Each worktree and
temporary branch was removed and pruned immediately after its verdict.

## Cycle 6 remediation in progress

Cycle-6 remediation is split across non-overlapping isolated source,
independent-evidence, and maintained-documentation worktrees. This is a
remediation-status record only; it does not assert composed behavior, a green
replacement gate or review, or remote delivery.

## Cycle 6 composition and replacement gate

Cycle-6 remediation completed from non-overlapping isolated components.
Maintained-documentation component
`31669fd316ad346429f54f7532547210ae0ea48c`, tree `c00ed8d`, integrated as
`42238ae`. Independent-evidence component
`059188df0366c18dbb7b4c18e84ea1cb7e781a75`, tree `3e71255`, integrated as
`fe94963`. Production/private-evidence component
`1f147172c12dfa03e14f166df9ba724a130f5ebf`, tree `b284812`, integrated as
`8705811`. Every component worktree was verified clean, removed, and pruned
immediately after integration; only the primary worktree remained.

Exact behavior head `87058114c68bab601a0940114ba7687fa7aea664`, tree
`8a319c135ee2cfaaa4c8f385334dde7471500c05`, passes the complete replacement
gate under exact Rust and Cargo 1.94.1 without fallback:

- all four required commands pass, including warnings-denied workspace all-
  target/all-feature Clippy, 1,175 listed non-documentation Rust tests, and two
  doctests;
- focused suites pass 32 external terminal, 19 private terminal, two engine,
  and one unsupported-platform cases. Three public regressions prove that an
  executor result, deadline, and cancellation still notify the original host
  after an outer re-poll through the retained supplied Waker; each also proves
  post-completion delivery suppression plus busy/recovery ownership across a
  retained clone;
- all 136 Python tests pass with eight expected macOS skips, while regeneration
  against pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef` remains byte-
  stable;
- exact `cargo-deny` 0.20.2 accepts advisories, bans, licenses, and sources with
  the three established duplicate warnings; `cargo-audit` 0.22.2 loads 1,226
  advisories, scans 211 lockfile dependencies, and reports no vulnerability;
- Linux native-library and terminal-test checks and warnings-denied Clippy pass;
  FreeBSD warnings-denied Clippy passes; WASI no-default and all-feature checks
  pass with only the established unrelated `read_file` dead-code warning;
- documentation integrity covers 89 maintained Markdown files, 312 triple-
  backtick occurrences, 680 parsed links, and 517 repository-relative targets
  with zero missing targets;
- the exact 29-file base diff is +8,324/-193, adds no unsafe Rust, and leaves
  workflows, benchmarks, compatibility data, the root manifest, and
  `Cargo.lock` unchanged; and
- a fresh locked 3,985,216-byte arm64 Mach-O release binary has SHA-256
  `b515ce0951f44a1e30171ee69c400cb9e750430e3a9d4959028ab49a16a55383` and
  passes 672-byte help plus isolated status smoke with empty stderr and no state
  residue.

This gate makes no formal cycle-6 review or remote-delivery claim. Three fresh
product-review tracks must identify the immutable candidate and tree recorded
after this gate result is committed.

## Formal cycle 6 review requirements

Production, independent tests, and maintained documentation are owned in
non-overlapping isolated worktrees. Each component must be committed, verified
clean, integrated, and then removed and pruned. The composed candidate must pass
focused tests, all four exact-1.94.1 required commands, portability and
release-mode evidence before review.

Three fresh read-only adversarial product tracks review one exact immutable
candidate and tree:

1. correctness, public API, schema, capability, and engine integration;
2. native filesystem/process effects, cancellation, lifecycle, and platform
   behavior; and
3. performance, concurrency, memory/output bounds, and resource ownership.

Findings are recorded as blocker/high/medium/low. Confirmed findings are fixed
and the complete gate plus three fresh tracks repeat until each track and the
deduplicated union report `0/0/0/0`. This is ordinary terminal-agent product
review, not a cybersecurity assessment.

## Delivery gate

Only a review-green exact candidate may be pushed as the feature branch. Its
exact feature CI and Benchmark evidence SHA must pass before `main` is
fast-forwarded without force. Exact main CI and Benchmark evidence must then
pass, with the expected exact-SHA artifacts retained. No package or GitHub
release is authorized.

The final record will append exact component commits, candidate/tree, local
evidence, every review report and adjudication, workflow IDs and SHAs,
integration result, and worktree cleanup. Documentation-only result and
delivery seals follow the user's review exemption and do not restart product
review.
