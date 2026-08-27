# Native `ask_user_question`

Status: **CYCLE 10 REJECTED — CYCLE 11 REMEDIATION IN PROGRESS**.

Bounded Milestone 03 slice 35 starts from exact delivered base
`5846799b665d62fc8301b33520da5cda33e850b3`. The comparison input is pinned
fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`. This first slice asks only
ordinary, bounded questions through an explicitly injected, rootless
`QuestionPrompter`. It does not provide a terminal UI or approval escalation.

## Local implementation checkpoint

Core no-authority component `de1ce26`, frozen contract `13cd366`, contract
correction `399f960`, initial production component `b24a673`, and initial
evidence `a76818e` formed the rejected cycle-1 candidate. Cycle-2 independent
evidence component `c77b336a378b349f51eaddc60cb342f805fd7e21` is integrated
as `0dd1128b914b00f15a17be3cbf2b6f7edccf605b`; production component
`9d2e0f234fd96beb2b2ce5b7dd5a6c123905fbf6` is integrated as
`47e9505f463b5ca9f4f418198022a4805757621b`. Cycle-1 finding documentation
composes with both at exact behavior head
`c8718c60ead54b4e66916cecb1d382c1e8f82934`, tree
`c27463b76607ae048363327e163c2077e296b898`.

Cycle-3 production component
`cf531d1692a2946442a37e2049507369c4e12b5c` is integrated as
`b7b4358525ce1f8864e501a8176b8c3fbdf3790e`; independent evidence component
`3e3c0c7ea06131adaeca027053c677a670f1a09b` is integrated as `f3f6f9d`; and
documentation component `bfdf05b6db1a343c8b4ab15cad98476986a77552` is
integrated at exact behavior head
`8bdc33d96bf88f5986c0e01b3979a2cef0427e82`, tree
`7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`.

That exact cycle-3 behavior head passes the complete exact-Rust-1.94.1 local
gate, including all four required workspace commands and 57 focused tests: 28
direct tool, one engine, 15 affected configuration, three root-selection, nine
reference-host, and one reference-host lifecycle test. Repo-wide Python passes
136 tests with eight intentional skips; pinned-fx drift, dependency policy,
audit, native/FreeBSD/WASI portability, documentation, protected diff/no-added-
unsafe, locked release, and isolated missing-root smoke checks are green.
`cargo audit --no-fetch` checks 1,226 advisories across 211 dependencies with
zero vulnerabilities. Documentation integrity is 91 Markdown files, 318 fence
markers, 701 parsed links, 534 local links, and zero missing targets. The fresh
locked release binary is 3,985,216 bytes with SHA-256
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
isolated `help`, `doctor`, and `sessions` runs do not create the missing root.
Formal cycle 3 reviewed exact candidate
`746e510c7d8eb93229996e74f91827f489e5bb31`, tree
`c49221efbea66c840b333f0de0161aa686aad52f`, and rejected it.

Cycle-4 core component `e569514028cae3b3e6d7b2ba86bf9a738b8d5210` is
integrated as `4c8cff3`; native component
`53c05cdf5e64e9e26266b89d78c8a20a2ac160df` is integrated as `1857a3f`; and
finding-documentation component `b057958f950b8a2a1412ecbc83b6f452d6571a2f`
is integrated at exact behavior head
`cb93bff35271e6dfc3f4c27ac7a72e621941845c`, tree
`fa402acb75c6d364c41db66f6b55595aa1d0e59a`. That exact head implements every
accepted cycle-3 correction and passes the complete local gate under Rust and
Cargo 1.94.1 exactly, without fallback:

- all four required formatting, workspace warnings-denied Clippy, workspace
  test, and workspace doctest commands are green;
- named focused runs are green for 66 core contract, 22 testkit double, 30
  direct question, one engine, 29 affected configuration, 11 plus two root-
  selection, nine reference-host, and one lifecycle executions (171 total);
- repo-wide Python passes 136 tests with eight intentional skips, pinned-fx
  regeneration is byte-stable, `cargo deny` passes with the three established
  duplicate-dependency warnings, and `cargo audit --no-fetch` checks 1,226
  advisories across 211 dependencies with zero vulnerabilities;
- native no-default, all-feature WASI, and warnings-denied no-default FreeBSD
  checks are green, with only the established unrelated WASI `read_file`
  warning;
- documentation integrity is 91 Markdown files, 318 fence markers, 701 parsed
  links, 534 local links, and zero missing targets; protected inputs and Cargo
  files are unchanged, and no Rust `unsafe` is added; and
- the fresh locked release binary remains 3,985,216 bytes with SHA-256
  `04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
  isolated missing-root `help`, `doctor`, and `sessions` runs pass without
  creating the root.

Formal cycle 4 reviewed exact candidate
`42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
`b761f7b93d535a1580910f43ff509c40aa07415b`, and rejected it. Correctness/API
reported `0/0/0/1`, lifecycle/platform `0/0/1/1`, and
performance/resources `0/0/1/0`; the deduplicated union is 0 blocker, 0 high,
2 medium, and 1 low. The historical cycle-4 local gate remains valid regression
evidence for that rejected candidate, not approval. Cycle-5 source and evidence
are established at the exact local-gate-green behavior head recorded below.
Formal cycle 5 nevertheless rejected its later exact candidate; cycle-6
remediation, a new complete local gate, three fresh formal reviews, remote CI,
benchmark workflow, integration, and delivery remain pending. No product-
performance, compatibility-promotion, or fx-equivalence result is established.
No release-binary prompt exercise applies because this library-only slice adds
no CLI prompt UI.

Formal cycle 1 reviewed exact candidate
`6c54ec3bf2c23983f14b0a4edeac723321a97900`, tree
`bea90245a559e8e223cc5bb45e0ddfa15e426ee6`, and rejected it. The
deduplicated result was 0 blocker, 1 high, 3 medium, and 3 low findings.
Cycle 2 implemented every accepted cycle-1 correction and passed its local
gate, but formal review rejected exact candidate
`910d7bc84cfd7800fb4daf9ab8537bf269027896`, tree
`503a91f334156dbcf2470560b9bb456c3491fd3d`, with a deduplicated 0 blocker,
0 high, 2 medium, and 2 low findings. Cycle 3 corrected every accepted cycle-2
finding and passed the local gate above, but formal review rejected its
immutable candidate with a deduplicated 0 blocker, 0 high, 3 medium, and 2 low
findings. The complete prior outcome and remediation history are in the
[`review ledger`](reviews/m03-ask-user-question-review-01.md).

## Formal cycle-4 outcome and cycle-5 remediation target

Correctness/API reported `0/0/0/1`, lifecycle/platform `0/0/1/1`, and
performance/resources `0/0/1/0` on exact cycle-4 candidate
`42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
`b761f7b93d535a1580910f43ff509c40aa07415b`. Overlap deduplicates to
`0/0/2/1`. Accepted findings are:

- cancellation triggered synchronously by destruction of the final registered
  cancellation Waker became observable only after the last cancellation check,
  allowing a direct success or error to escape;
- concurrent cancellation could move and execute the registered Waker callback
  after outer drop and permit release, allowing callback tails outside the
  configured active limit; and
- the reference-host document retained stale cycle-3-pending lineage.

Cycle 5 freezes activity-backed cancellation ownership. Every registered or
cached equivalent cancellation Waker clone and callback retains the originating
permit through callback return. Prompt, waiter, and cached-Waker teardown stays
under that activity. After teardown, execution makes one final cancellation
recheck before every direct success or error return. Deterministic race evidence
exercises both synchronous final-Waker cancellation and a concurrently moved
callback.

Independent evidence component
`ad47fcb1a6eb751e4953d84933afa1c12dddfbd7`, integrated as `bcce292`, and
source component `80382d8f3f4df53fea867f66c53620f1d6592c6d`, integrated as
`e0fd8e0`, compose with finding-documentation component
`ba53f5539b68817d2ebe920039ccb5c8303d8b34`, integrated as `b870731`, at exact
behavior head `b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
`0b025f8e42e18006a72d89becf0e395d35c91a57`. Direct cross-composition at
`e1947c1495c7cbdc69236b8f7ab1599dda80ca07`, tree `e63f8272`, passed all 32
direct question tests. At the integrated head, formatting; 32 direct tests; one
engine test; nine all-feature reference-host tests; one reference-host session-
lifecycle test; native all-target/all-feature warnings-denied Clippy; and six
native-manifest tests are green.

That exact head also passes all four required exact-1.94.1 workspace commands.
The extended gate passes 136 Python tests with eight intentional skips, the
pinned compatibility drift check, dependency policy with only established
duplicate warnings, and audit of 1,226 advisories across 211 dependencies with
zero vulnerabilities. Native no-default, all-feature WASI library, and
warnings-denied no-default FreeBSD checks pass; WASI emits only the established
unrelated `read_file::check_cancellation` dead-code warning. Documentation
integrity is 91 Markdown files, 318 fence markers, 701 parsed links, 534 local
links, and zero missing targets. Diff checks are clean, protected `.github`,
benchmark, and compatibility inputs are unchanged, and no Rust `unsafe` is
added.

The authorized manifest/lock delta is development-only: native tests add the
existing audited `machine-god-reentrant-waker-test` path dev-dependency. The
package already existed in `Cargo.lock`; only its native dependency-list line
changed. The production normal/build dependency graph is unchanged, and audit
still covers 211 dependencies. A fresh locked release binary is 3,985,216 bytes
with SHA-256
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`.
Isolated `--help`, `doctor --json`, and `sessions --json` runs against missing
XDG roots succeed without creating files. This checkpoint is regression and
delivery evidence, not formal approval, a benchmark result, or an fx-
equivalence claim. Formal cycle 5 later rejected the exact candidate below.

## Formal cycle-5 outcome and cycle-6 remediation target

Three fresh read-only tracks reviewed exact candidate
`54b1aab5660e90096b95518bde4ebffb93f28fa6`, tree
`54586d2256c8a3d2289b92bc9bc842eed9ce4d07`. Correctness/API and lifecycle/
platform each reported `0/0/0/0`; performance/resources reported `0/0/1/0`.
The deduplicated union is `0/0/1/0`, so the candidate is rejected.

The one medium is a product resource/capacity defect: arbitrary retained
activity-Waker clones independently forward concurrent blocking downstream
callbacks while consuming one prompt slot. Cycle 6 must introduce an activity-
backed single-flight coalescing notifier, allow at most one callback in flight,
replay notifications without loss, close the stale downstream target, and
retain capacity until callback and clone ownership is gone. Deterministic
owned-future evidence must cover many retained clones.

## Cycle-6 implementation and local checkpoint

Finding docs `7dee2694660b3d16340f20de272c6631abdcbcef`/`e20023c`, evidence
`b007ada85ce58727ea5d38ab810495dc68e57ef0`/`4a929c4`, and source
`0488d71e2ca1b6b0877d5dc5e1e29ce059f1c5ff` compose at exact behavior head
`707a794230758374fa2dab6d65eaf27449c7c477`, tree
`1e60299e21f45079f4e8cf27468a28d1ab4fe227`. Independent cross-composition
`236dd90`/`94b9fdd3980a413c594538fc9222b09007518bce` is green for 34 direct
tests, one engine test, and native warnings-denied Clippy.

One activity-backed notifier now shares the prompt and cancellation Waker
family. It serializes downstream callback delivery, coalesces a concurrent or
reentrant burst into one lossless replay, and closes its target and replay state
when the prompt completes or the outer future drops. Target clone/drop occurs
outside the state lock. Retained notifier clones and in-flight callbacks keep
the originating prompt permit until their ownership ends. The deterministic
`cloned_prompt_wakers_coalesce_blocking_callbacks_and_replay_once` regression
wakes 16 retained clones concurrently and proves maximum callback concurrency
one plus exactly one replay. The deterministic
`completed_prompt_closes_retained_waker_delivery_until_every_clone_drops`
regression proves stale post-completion delivery is closed and capacity remains
held through callback return and final-clone drop.

The integrated focused gate passes exact-1.94.1 formatting, direct 34, engine
one, all-feature host nine, host lifecycle one, native manifest six, and native
all-target/all-feature warnings-denied Clippy. All four pinned workspace gates
are green. The extended gate passes Python 136 with eight skips, pinned drift,
dependency policy/audit (1,226 advisories, 211 dependencies, zero
vulnerabilities), native/FreeBSD/WASI portability with only the established
unrelated WASI `read_file::check_cancellation` warning, documentation
91/318/701/534/0, clean protected/no-added-unsafe checks, and fresh locked
release/missing-root smoke. The 3,985,216-byte binary SHA-256 is
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`.
The dev-only reentrant-Waker path dependency and one native lock dependency-
list line remain the only authorized manifest/lock delta; production normal/
build dependencies and the 211-dependency audit inventory are unchanged.
Formal cycle 6 later rejected the exact candidate below, so this historical
local gate does not approve it.

## Formal cycle-6 outcome and cycle-7 remediation target

Three fresh read-only tracks reviewed exact candidate
`85058a8aa88fab6912d9313f1ce71e2778cc937f`, tree
`fd3c5072c9473c7fe8767cc2692238eacb8a0f43`. Correctness/API reported
`0/0/0/1`; lifecycle/platform and performance/resources each reported
`0/0/1/0`. The shared medium deduplicates across those two tracks, producing a
`0/0/1/1` union and rejecting the candidate.

The medium is unbounded synchronous replay amplification. Every reentrant
notice sets the pending bit; when a replayed callback self-notifies, it rearms
the loop again. One original `wake` can therefore execute an unbounded sequence
of synchronous callbacks while consuming one prompt slot. Cycle 7 must make
coalescing observation-aware: callback count stays bounded by a constant when a
callback continually self-notifies, while a new notice after an outer
observation is replayed without loss. A deterministic finite-budget regression
must prove both the bound and the observation-then-notice replay before new
gates and three fresh reviews.

The low is inconsistent operative opening status. `README.md:9`,
`docs/README.md:3`, and `docs/architecture.md:3` still described cycle 5 while
later summaries described cycle 6. The rejection-doc checkpoint aligned every
operative opening to its then-current rejection/remediation status. This local-
gate checkpoint advanced all ten operative status regions to its then-current
local-gate status; its focused consistency scan reported ten current and zero
stale. The cycle-7 review outcome below supersedes that operative status.

## Cycle-7 implementation and local checkpoint

Rejection docs `6128f03adddfa566a8fc8f3b326fc16e927b0b05`/`1d354ff`, evidence
`acca13c0613e12c2a20e903abbb768e87253c5b6`/`b75fc54`, and source
`3d48ce852db57afe32601ebdd90bc8ef42d4a0fd` compose at exact behavior head
`fbb3f5c5f40d0726b444b1ebc6f25fb1ee1fee36`, tree
`7cee96e0701d11925360f3d1b6315f5801bbd807`. Independent cross-composition
`c0c9eb0`/`6fd79edfac2705e8dfe79bbe43011ab83dc4cd94` is green for formatting,
35 direct tests, one engine test, and native warnings-denied Clippy.

Callback entry clears observed and pending state, so every notice before an
outer observation coalesces into the callback already in flight. An outer bind
marks observation; only a later notice earns one serialized replay, whose entry
clears observation again. Close and panic clear both observation and pending
state. Callback concurrency is at most one, retained Wakers and callbacks keep
the prompt permit, and arbitrary downstream Waker clone/drop/callback work
never runs while the notifier lock is held.

`reentrant_prompt_wake_before_outer_repoll_has_constant_callback_work`
rejects the cycle-6 base at 65 callbacks for a finite budget of 64 and records
one callback for cycle 7. The refined
`cloned_prompt_wakers_replay_once_after_outer_repoll_observes_the_burst`
proves that one notice after an outer re-poll is delivered as one replay.

The focused gate passes exact-1.94.1 formatting, direct 35, engine one, all-
feature host nine, host lifecycle one, native manifest six, and native all-
target/all-feature warnings-denied Clippy. All four pinned workspace gates are
green. The extended gate passes Python 136/8 skips, pinned compatibility,
dependency policy/audit 1,226/211/zero, native/FreeBSD/WASI portability with
only the established unrelated `read_file` warning, documentation
91/318/701/534/0, status consistency 10 current/zero stale, clean protected/no-
unsafe checks, and a fresh locked release plus three missing-root smokes. The
3,985,216-byte release SHA-256 is
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`.
The existing audited reentrant-Waker path fixture and one native lock dependency-
list line remain the exact dev-only Cargo delta; the production graph is
unchanged. Formal cycle 7 later rejected the exact candidate below, so this
historical checkpoint does not approve it.

## Formal cycle-7 outcome and cycle-8 remediation target

Three fresh tracks reviewed exact candidate
`617672984fbb897f2efec63de6a05bb32db9a3db`, tree
`f2cd844449193b46cfa1473ae21edad68664157e`. Correctness/API and performance/
resources each reported `0/0/0/0`; lifecycle/platform reported `0/0/1/0`;
the deduplicated union is `0/0/1/0`. The nonzero medium rejects cycle 7.

The product lifecycle defect occurs between one delivery and replay. Replay
target B is selected before prior target A is dropped. If A's destructor
panics, unwinding bypasses lane settlement and leaves `notifying` wedged. If
A's destructor reentrantly closes the notifier or installs a replacement, the
already selected B can still receive stale delivery after that lifecycle
transition.

Cycle 8 must drop A before selecting any replay target, catch and settle a drop
unwind plus all lane flags, and admit the then-current replay only after A was
destroyed successfully. Deterministic tests must prove recovery after a
panicking destructor and suppression when A's destructor reentrantly closes the
notifier. A complete replacement gate and three fresh reviews are required.

The analogous notifier/destructor ordering in native `terminal` predates this
finding and is outside this bounded slice. This remediation target makes no
claim that the terminal path is fixed.

## Cycle-8 implementation and local checkpoint

Rejection docs `22d570286f76067971504ee2283ee40d49eab8a1`/`3650dba`, evidence
`cf4abfd7385904ff4c32c503ff7d8f3823225032`/`5681bab`, and source
`a1b3d231077a67a63f8984cbd3fe4f8cc2370108` compose at exact behavior head
`d8075ffee2d6765df2ce7842300e26bb7127d52b`, tree
`fa32564476ce6a74cd3ba09c48a4b98af602cb72`. Independent cross-composition
`01d9a06`/`c917dce7856e9a1736651fa01696c5ad7e42fbcb` is green for formatting,
37 direct tests, one engine test, and native warnings-denied Clippy.

Target A is destroyed under `catch_unwind` outside the notifier lock while the
lane and originating activity remain retained. Only after A's successful
destruction does replay arbitration read the then-current lifecycle, pending
notice, and target. A destructor's close or replacement therefore wins.
Callback panic or target-drop panic clears the lane flags; callback panic wins
deterministically if both occur. Foreign callback, clone, drop, and Waker work
stays outside the lock, and callback concurrency remains at most one.

`replay_target_drop_panic_clears_lane_for_a_fresh_notification` records the
rejected base with fresh B at zero callbacks because `notifying` remains wedged,
then proves cycle 8 clears the lane and delivers the fresh notice.
`replay_target_drop_close_suppresses_selected_replay_and_retains_capacity`
records one stale B delivery on the rejected base, then proves the fix lets A's
reentrant close suppress B and retain capacity through destruction.

The focused gate passes exact-1.94.1 formatting, direct 37, engine one, all-
feature host nine, host lifecycle one, native manifest six, and native all-
target/all-feature warnings-denied Clippy. All four pinned workspace gates are
green. The extended gate passes Python 136/8 skips, pinned compatibility,
dependency policy/audit 1,226/211/zero, native/FreeBSD/WASI portability with
only the established unrelated `read_file` warning, documentation
91/318/701/534/0, status consistency 10 current/zero stale, clean diff/
protected/no-unsafe checks, and a fresh locked release plus three missing-root
smokes. The 3,985,216-byte release SHA-256 is
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`.
The existing audited reentrant-Waker fixture and one native lock dependency-
list line remain the exact dev-only Cargo delta; the production normal/build
graph is unchanged. Formal cycle 8 later rejected the exact candidate below, so
this historical checkpoint does not approve it. The analogous preexisting
terminal path remains outside this bounded slice and is not claimed fixed.

## Formal cycle-8 outcome and cycle-9 remediation target

Three fresh tracks reviewed exact candidate
`e929b5ea7e3264c2b56066a416bc2a979a03b214`, tree
`cfadc42814688a29c4d512e5fd91c843423821d4`. Correctness/API reported
`0/0/0/0`; lifecycle/platform and performance/resources each reported
`0/0/1/0`; the deduplicated union is `0/0/2/0`. Both distinct mediums reject
cycle 8.

First, when both the callback and target drop panic, the captured secondary
panic payload is eventually destroyed. Its destructor can itself panic and
override the callback panic that cycle 8 promised would win. Cycle 9 must
safely suppress or forget that secondary payload so the primary callback panic
survives. Deterministic marker evidence must also prove lane cleanup, retained
capacity, and fresh delivery after recovery.

Second, a callback can synchronously re-poll the outer future and then
re-notify. Each replay can repeat that sequence, extending one notify activation
to 257 callbacks for a budget of 256. Cycle 9 must bound every explicit notify
activation to the initial callback plus at most one replay, retain residual
pending notification for a later explicit activation, keep callback concurrency
at most one, retain capacity, and add deterministic large-budget evidence for
both the bound and later delivery.

Any analogous native `terminal` path is preexisting and outside this bounded
slice. It is not claimed fixed.

## Cycle-9 implementation and local checkpoint

Rejection docs `2faedc764c9cc3caa7813babed0abf0f2f867c90`/`5296dcc`, evidence
`cf2e2207d7f298a9aa102476673d9ab33a42024c`/`ee25455`, and source
`527e10dcc53cb609de394ac59d3fe2641ceed627` compose at exact behavior head
`0279b8cb744b8d5cee92d2bfc263abcca60a9987`, tree
`50b2423637fc9eb8f0cd6792874a2385ff32fd06`. Independent disposable cross-
composition `13eccf9`/`56695d8d7c2daaa38355c22a04276b583b93a815` is green
for formatting, 39 direct tests, one engine test, and native warnings-denied
Clippy.

One explicit notify activation executes the initial callback plus at most one
replay. Post-observation pending work produced by that replay remains after lane
release and is eligible only when a later explicit activation arrives. Close
and panic clear state, callback concurrency is at most one, target A is dropped
before replay arbitration, and the lane/activity retains permit capacity.

When both the callback and A's drop panic, cycle 9 intentionally forgets the
opaque secondary payload so its destructor cannot replace the callback panic.
The callback panic remains primary. A single target-drop panic still propagates,
and no foreign work executes under the notifier lock.

`callback_panic_precedes_panicking_replay_target_payload_drop` proves primary
marker identity, lane cleanup, capacity retention, and fresh delivery.
`one_notify_activation_has_one_replay_and_leaves_residual_pending_work` rejects
base `e929b5e` after 257 callbacks exhaust budget 256; cycle 9 performs two
callbacks, then reaches four total only after a later activation while residual
pending work is decremented.

The focused gate passes exact-1.94.1 formatting, direct 39, engine one, all-
feature host nine, host lifecycle one, native manifest six, and native all-
target/all-feature warnings-denied Clippy. All four required exact-1.94.1
workspace commands pass without fallback. The extended gate passes Python
136/8 skips; byte-stable compatibility; deny with only `core-foundation`,
`cpufeatures`, and `syn` duplicates; audit 1,226/211/zero; native, FreeBSD, and
WASI portability with only the established unrelated `read_file` warning;
documentation 91/318/701/534/0; status 10/0; clean diff/protected/no-unsafe;
and a fresh locked release plus three missing-root smokes. The 3,985,216-byte
release SHA-256 is
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`.
The authorized Cargo delta is the native dev-only test-fixture line and one
native dependency-list line in `Cargo.lock`; the production normal/build graph
is unchanged. Formal review, workflows, integration, and delivery remain
pending. This is not benchmark, product-performance, compatibility-promotion,
or fx-equivalence evidence. The analogous terminal path remains out of scope
and is not claimed fixed.

## Formal cycle-9 outcome and cycle-10 requirement

Formal cycle 9 reviewed exact candidate
`1eeab670a552bc15b5602319b0bb1ce27d2be497`, tree
`5c86e624cf3c0e6d521382c377a9ed9b0500ee5b`. Correctness/API reported
`0/0/1/0`, lifecycle/platform reported `0/0/1/0`, and
performance/resources reported `0/0/0/0`. The correctness and lifecycle
reports describe one defect, so the deduplicated union is `0/0/1/0`.

After one explicit activation has used its initial callback and one replay, a
legal wake emitted after the replay poll sets only
`pending_after_observation`. Notify releases the lane without scheduling a
downstream callback. That wake is consumed only when an unrelated later
explicit notify activates the lane. The committed
`one_notify_activation_has_one_replay_and_leaves_residual_pending_work` test
manually calls `retained_wakers[2]` to make progress; it does not demonstrate
autonomous progress from the retained wake.

Consequently, a self-waking prompt or cancellation transition can remain
`Pending` indefinitely when its wake is the last external activity. The tool
sets no timeout, so nothing else is required to arrive. The candidate is
rejected.

Cycle 10 must ensure every wake emitted after its corresponding poll schedules
progress without unrelated activity. Delivery must remain bounded and
nonrecursive, callback execution single-flight, established callback/target-
drop panic ordering unchanged, and the prompt permit retained through all
activity ownership. A deferred/trampoline dispatcher may be necessary; a
public contract redesign is acceptable only with an explicit, justified
progress rule. At that rejection checkpoint, no cycle-10 source, evidence,
gate, review, workflow, integration, or delivery result was claimed.

## Cycle-10 implementation and local checkpoint

Cycle-9 rejection docs `216c3b4479d51dbe1052c2f9a6723089e600f77b`/`895c9d4`,
final evidence `74a849791e311759630d0204d692190a39da279c`/`5e46f56`
(superseding `5cbd9b0`), and source
`b0433648b1c836a8db6151f64b461196830fea92` compose at exact behavior head
`72e8e75ba2490d4dfa0f680d9dca0b4e10a0401a`, tree
`5405180e5b3b4b59c4d7e712f614bdbc958a9d75`. Final disposable composition
`a8acbf4`/`54124807ac991cc93dc15db28bad21ac8e2a19ae` passes formatting and all
41 direct tests.

`ActivityWake` has `Open`, `DeliveryResourceExhausted`, and `Closed` states
around one serialized, nonrecursive callback lane. An activation permits
exactly 256 downstream callbacks. Calls 1 through 255 carry normal observation-
aware source-wake progress. If a wake after poll 255 would continue, sticky
exhaustion is recorded before callback 256, which schedules a terminal outer
poll. That poll checks cancellation first, then returns the existing
nonretryable `Execution` error with kind `ask_user_question_prompt_failed` and
redacted message `ask_user_question prompt failed` if exhausted.

Exhaustion suppresses retained binds/wakes until close. Short residual chains
now progress autonomously through callback 3. Close/panic behavior, A-drop-
before-arbitration, callback-primary dual-panic handling, lone target-drop panic
propagation, permit retention, and no foreign Waker operations under the mutex
remain. No threads, queues, dependency, or public API change is added.

`observed_residual_wakes_progress_without_an_unrelated_activation` records two
callbacks on the rejected base and three now.
`continuously_rewaking_prompt_stops_at_the_delivery_limit_with_redacted_error`
records two on the base and the exact terminal 256 now.
`cancellation_in_the_residual_wake_window_progresses_and_closes_delivery`
records base two/new three and proves autonomous cancellation progress,
cancellation-first precedence, and closed delivery.

The exact-1.94.1 focused gate passes formatting, direct 41, engine one, host
nine, host lifecycle one, manifest six, and native warnings-denied Clippy. All
four required exact-1.94.1 commands pass without fallback. Extended Python
136/8, byte-stable compatibility, deny with established duplicates, audit
1,226/211/zero, native/WASI/FreeBSD portability, docs 91/318/701/534/0,
status 10/0, diff/protected/no-unsafe, and release-smoke gates are green. The
locked release remains 3,985,216 bytes with SHA-256
`04daccd31dc0c97c49c1af09471f9b37ba51590d4293b050972c0bf786da25cf`;
three isolated missing-root smokes create no files. The authorized Cargo delta
remains the dev-only native fixture plus one native lock dependency-list line;
the production graph is unchanged. Formal review, workflows, integration, and
delivery remain pending. This is not benchmark, product-performance,
compatibility-promotion, or fx-equivalence evidence.

## Formal cycle-10 outcome and cycle-11 requirements

Formal cycle 10 reviewed exact candidate
`4ea1c1f5be3586ce9bee696b12c4120dc2a72018`, tree
`78e781ffd7b03aafdf295ae79f4090120971c248`. Correctness/API reported
`0/0/1/0`, lifecycle/platform reported `0/0/1/0`, and
performance/resources reported `0/0/0/0`. The two mediums are distinct, so the
deduplicated union is `0/0/2/0`.

First, `callbacks_started` resets inside every notify. A queue-only downstream
Waker returns after enqueue, allowing the lane to clear `Open` before the queued
outer poll. A prompt that self-wakes and returns `Pending` on every queued poll
therefore starts every notify at counter one. It never reaches 256, can run
indefinitely, and retains capacity. Existing evidence covers only synchronous
reentrant callback execution that waits for its poll. Cycle 11 must retain a
hard finite delivery budget across callback-return and queued-poll activations
for the prompt lifetime, with queue-driven exact-bound and cancellation
evidence.

Second, the prompt-poll panic handler catches `drop(doomed)` but discards its
`Err` payload. Destruction of that captured opaque cleanup panic can replace the
primary or double-panic and abort. `PromptActivity::drop` and analogous local
selectors can also destruct suppressed/nonselected payloads during unwind.
Cycle 11 must explicitly forget every suppressed or nonselected opaque cleanup
payload, choose a documented primary only when not already unwinding, and prove
prompt-poll primary identity, no abort, lane closure, and capacity recovery.

No cycle-11 source, evidence, gate, review, workflow, integration, or delivery
result is claimed.

## Product boundary

`ask_user_question` is a provider-visible native tool for one ordered batch of
one to four blocking decisions. Each question has two to six model-supplied
options. Options guide the user but do not constrain the returned answer: a
host may expose an `Other` path, and any bounded nonempty free-form answer is
valid. The number and order of answers must exactly match the prepared batch.

Pinned fx also accepts `permission_request_id` to enter an action-bound
approval flow. Machine-god's current configuration has only ask-mode
permission handling and no exact auto-denied continuation authority. This
slice therefore rejects `permission_request_id` whenever the field is present,
regardless of its value or type. It never treats question, option, answer, or
conversation text as authorization.

The tool is read-like and reversible in product metadata, but it is not a
filesystem read and does not require permission. Effect-free preparation uses
core's explicit `PreparedToolCall::without_authority` form. Core consequently
constructs no permission request ID, emits no `PermissionRequested` or
`PermissionResolved` event, and never invokes the permission handler for this
call. Argument validation, cancellation, tool events, result limits,
persistence, and recovery remain unchanged. Every existing tool retains the
permission-required default.

That no-authority name is scoped to policy-governed authority; it does not
claim that the injected prompter lacks terminal, UI, or transport authority.
Cycle 4 makes `PreparedToolCall::capability()` a total optional accessor:
permission-required calls return their exact capability and explicit no-policy-
authority calls return no capability. Inspecting either valid public state must
not panic.

## Model-visible schema

The root is a strict object with exactly one allowed field:

```json
{
  "questions": [
    {
      "question": "Which implementation should proceed?",
      "options": [
        {
          "label": "Bounded native seam",
          "description": "Add only the injected library boundary."
        },
        {
          "label": "Defer the slice",
          "description": "Leave the native tool inventory unchanged."
        }
      ]
    }
  ]
}
```

Objects are strict at all three levels:

| Object | Required fields | Optional fields | Unknown fields |
| --- | --- | --- | --- |
| root | `questions` | none | rejected |
| question | `question`, `options` | none | rejected |
| option | `label` | `description` | rejected |

`permission_request_id` is not an ordinary unknown-field case. Its presence
returns the fixed deferred-feature error below. JSON values have already been
decoded by core, so duplicate JSON object keys are outside this tool's
observable boundary; strictness applies to the resulting object.

The schema advertises one to four questions and two to six options per
question. It does not advertise `permission_request_id`, a timeout, default,
multi-select, option ID, selected label, or CLI presentation property.

## Fixed limits

All byte counts are UTF-8 bytes. `raw` below means the decoded string after
trimming only leading and trailing ASCII space, tab, carriage return, and line
feed. `rendered` means the terminal-safe representation described in the next
section. Serialized sizes are compact `serde_json` bytes, including JSON
punctuation and escaping.

| Resource | Inclusive maximum |
| --- | ---: |
| Incoming serialized arguments | 32,768 bytes |
| Questions | 4 |
| Raw question text | 1,024 bytes each |
| Rendered question text | 4,096 bytes each |
| Options per question | 6 |
| Total options | 24 |
| Raw option label | 128 bytes each |
| Rendered option label | 512 bytes each |
| Raw option description | 512 bytes each |
| Rendered option description | 2,048 bytes each |
| Aggregate rendered presentation text | 32,768 bytes |
| Serialized normalized prepared arguments | 49,152 bytes |
| Complete pre-trim host answer | 4,096 bytes each |
| Aggregate complete pre-trim host answers | 4,096 bytes |
| Aggregate rendered answers | 16,384 bytes |
| Reachable serialized `ToolOutput` maximum | 41,102 bytes |
| Serialized `ToolOutput` defense-in-depth guard | 49,152 bytes |
| Default simultaneous active prompts per tool | 1 |
| Hard simultaneous active prompts per tool | 8 |

Aggregate presentation text is the checked sum of every normalized question,
label, and present description, excluding vector and object overhead. The
separate normalized-argument serialization check includes that overhead and
JSON escaping. Neither ceiling substitutes for the other.

The raw question total is at most 4,096 bytes. Answer limits measure the
complete strings returned by the host before any trim or character scan: each
answer and their aggregate are at most 4,096 bytes. Checking `String::len`
before ASCII-edge trimming prevents an arbitrarily large whitespace-only host
response from causing unbounded synchronous scanning. Terminal encoding
expands one accepted answer byte by at most four rendered bytes, and compact
JSON needs at most one additional escape byte per rendered byte.

Under every legal question and answer bound, the exact reachable maximum
serialized `ToolOutput` is 41,102 bytes. Evidence must prove that reachable
maximum, not claim a first-over rejection that legal inputs cannot construct.
The separate 49,152-byte complete-result check remains an authoritative
defense-in-depth guard over the final envelope, but is unreachable under the
other frozen limits. Overflow uses checked arithmetic and fails rather than
truncating.

The input ceiling is checked by a bounded serialization pass before semantic
traversal or string cloning. The rejected cycle-1 candidate scanned a complete
oversized JSON string value or object key before testing the remaining 32/48
KiB budget. Cycle 2 makes string and key accounting remaining-budget-aware and
stops as soon as the applicable ceiling is exceeded. Per-field raw ceilings
are checked after trim and before terminal encoding. Rendered
per-field and aggregate ceilings are checked during encoding. The normalized
serialized ceiling is checked last. An input can fit 32 KiB and still fail a
later rendered or normalized ceiling; no field is silently shortened.

## Canonicalization and terminal safety

Preparation visits questions and options in input order and applies this exact
pipeline:

1. require the strict object and field type;
2. trim only ASCII `0x20`, `0x09`, `0x0d`, and `0x0a` from both ends;
3. reject an empty required question or label;
4. check the raw per-field byte limit;
5. encode terminal-unsafe scalar values without truncation;
6. check the rendered per-field and aggregate byte limits; and
7. build the strict normalized arguments and check their compact serialized
   size.

An optional description must be a string when present. After ASCII trim, an
empty description canonicalizes to absence; a nonempty description follows the
same raw/rendered checks. This is intentionally stricter than pinned fx, which
discards a non-string description.

Terminal-safe encoding preserves printable ASCII and printable valid Unicode.
It replaces ASCII C0 control bytes and `DEL` with lowercase `\xnn`. It replaces
C1 controls `U+0080..U+009F`, `U+061C`, `U+200B..U+200F`,
`U+2028..U+202E`, `U+2060..U+206F`, and `U+FEFF` with lowercase
`\u{nnnn}` using at least four hex digits. JSON strings are valid UTF-8, so
invalid UTF-8 cannot reach this stage. The encoder never interprets ANSI
sequences, applies Unicode normalization, or removes visible text.

Labels must be unique within their question after trim and terminal-safe
encoding. Comparison is bytewise ASCII case-insensitive: `Yes` conflicts with
` yes `, and raw ESC conflicts with the literal rendered text `\x1b`, while
non-ASCII case variants are not folded. Duplicate questions and labels reused
in different questions are allowed.

The normalized values supplied to `QuestionPrompter` are the exact normalized
values retained in prepared execution arguments. Direct `execute` accepts only
a canonical normalized value with an incoming preimage satisfying the same
raw-field and incoming-serialization bounds as preparation. The rejected
cycle-1 candidate admitted a printable 4,096-byte question that preparation
would reject at the 1,024-byte raw limit. Cycle 2 decodes a terminal-safe
preimage, rechecks its raw and canonical rendering bounds, and verifies the
complete incoming preimage under 32 KiB before invoking the prompter.

## Injected prompt boundary

`QuestionPrompter` is an object-safe, `Send + Sync`, executor-neutral host
boundary. The native tool supports owned and shared construction, including an
`Arc<dyn QuestionPrompter>`. The prompt request owns the complete normalized
ordered batch and exposes read-only getters. Its `Debug` output is structural
and does not include question, option, description, or answer text.

Calling `Tool::execute` creates an inert future. On first poll it checks
cancellation, attempts one fail-fast active-prompt admission, and only then
invokes the prompter exactly once. Capacity exhaustion does not queue, register
a capacity Waker, or invoke the prompter. A successful permit belongs to the
execution activity until the outer future and every cancellation Waker clone or
callback originating from it have returned or been destroyed.

The prompt future remains owned by the tool future. Dropping an unpolled tool
future invokes no prompt. Cycle 5 requires pending-return, drop, and unwind
paths to tear down the prompt future, cancellation waiter, and cached equivalent
Waker under one activity. Registered and cached cancellation Waker clones, plus
callbacks that cancellation has moved out of the waiter, retain that activity
and its originating permit through callback return. Thus outer-future drop does
not admit a replacement while an old callback tail still runs. The adapter
starts no thread, task, channel, timer, runtime, retry, or detached work. A
conforming prompter must keep interaction work owned by the returned future or
perform its own complete drop cleanup.

There is no tool timeout. The host/executor may apply an outer deadline, but
this slice neither accepts nor starts one. A stalled or blocking injected
prompter can therefore stall its call; that violates the injected boundary and
cannot be repaired by the portable adapter.

The default active-prompt limit is one. An explicit constructor may select
one through eight. Zero and values above eight fail construction with a fixed,
data-free invalid-limits error. Each tool instance owns its counter; there is
no process-global registry.

## Outcomes, answers, and precedence

The prompter returns one of three structured outcomes. Cycle 4 stores answered
values behind a private bounded container whose checked construction admits
zero through four strings only:

- `Answered` with that ordered bounded answer container;
- `Cancelled` for an explicit user cancellation; or
- `Unavailable` for a noninteractive host.

The zero-to-four construction range intentionally includes count mismatches for
every legal one-to-four-question batch so execution still owns mismatch
validation, but a malformed host cannot transfer an arbitrarily large vector
whose synchronous destructor escapes the resource contract. `Answered` must
contain exactly one answer per prepared question. Each complete
host-returned string is checked against the per-answer and aggregate 4,096-byte
limits before scanning or trimming. Within-bound answers are ASCII-trimmed and
must remain nonempty. Machine-god then applies the same terminal-safe encoding
and aggregate rendered limit used above. It deliberately does not
require an answer to equal an option label. This admits a bounded `Other`
answer. The only claimed parity with pinned fx's answer codec is the absence
of option-label membership enforcement; machine-god additionally ASCII-trims
answers, rejects empty answers, enforces raw/rendered/result bounds, and
applies its own terminal-safe encoding.

Successful answers produce deterministic ordered JSON as the `content` of a
non-error `ToolOutput`:

```json
[
  {
    "answer": "Bounded native seam",
    "question": "Which implementation should proceed?"
  }
]
```

The representation intentionally inserts `answer` and then `question` for each
object; array order equals input question order. This order does not depend on
the selected `serde_json::Map` implementation, lexical map behavior, or
feature unification. The rejected cycle-1 candidate inserted `question` before
`answer` and happened to serialize in the documented order only with the
current lexical-map dependency behavior. Cycle 2 expresses and tests the
intended insertion directly. Questions are the
exact normalized strings shown to the prompter. No option, description,
internal ID, timing, or host metadata is returned. Pinned fx emits the same two
object members in the opposite textual key order; JSON object order is not
semantic, so this slice makes no byte-level fx result claim.

An explicit user `Cancelled` outcome returns the successful string content
`(user cancelled the question)`. `Unavailable` returns the successful string
content `(ask_user_question is only available in the interactive shell; ask the
user freeform instead)`. These sentinels are not answer arrays and cannot be
misread as authorization.

Engine cancellation has precedence at first poll, immediately before prompt
invocation, after every ready prompt outcome before interpretation, and after
prompt/waiter/cached-Waker teardown immediately before every direct return. The
rejected cycle-1 candidate checked cancellation and then cloned up to 16 KiB of
question presentation text before invoking the prompter. Cycle 2 adds the
adjacent pre-invocation cancellation check after that last intervening work,
so observable cancellation prevents UI invocation.
Cancellation that is observable in the same poll as answers, user
cancellation, unavailability, host failure, or final cancellation-Waker
destruction wins and returns the fixed cancelled tool error. After cancellation
wins, no answer result is published. The cached equivalent registration avoids
recreating an untracked Waker family while still permitting the final check to
observe cancellation caused synchronously during teardown.

For a non-cancelled ready result, precedence is:

1. redacted prompter failure;
2. answer-count mismatch;
3. per-answer complete pre-trim byte limit in question order;
4. aggregate complete pre-trim answer limit;
5. ASCII-edge trim and empty-answer rejection;
6. terminal rendering and aggregate rendered-answer limit;
7. serialized result defense-in-depth guard; and
8. ordered success publication.

No partially validated or partially encoded answer array is returned.

## Fixed failures and redaction

| Condition | `ToolErrorKind` | Code | Message | Retryable |
| --- | --- | --- | --- | --- |
| Malformed shape, type, field, empty required text, or duplicate label | `InvalidInput` | `ask_user_question_invalid_arguments` | `ask_user_question arguments are invalid` | no |
| Incoming, field, presentation, normalized, answer, or result limit | `InvalidInput` | `ask_user_question_resource_limit` | `ask_user_question resource limit exceeded` | no |
| `permission_request_id` present | `InvalidInput` | `ask_user_question_permission_request_unsupported` | `ask_user_question permission escalation is not supported` | no |
| Invalid configured active limit | construction error | n/a | `invalid ask_user_question limits` | n/a |
| Active prompt limit already full | `Unavailable` | `ask_user_question_busy` | `ask_user_question prompt capacity is exhausted` | yes |
| Prompter failure | `Execution` | `ask_user_question_prompt_failed` | `ask_user_question prompt failed` | no |
| Wrong answer count or malformed answer | `Execution` | `ask_user_question_invalid_response` | `ask_user_question prompt returned an invalid response` | no |
| Engine cancellation | `Cancelled` | `ask_user_question_cancelled` | `ask_user_question was cancelled` | no |

Argument precedence is serialized-input limit, root type, deferred
`permission_request_id`, other root keys, `questions`, and then each ordered
question/option pipeline. Resource-limit failures at a field's exact check take
precedence over later semantic checks. Errors retain no input, answer,
question, label, description, prompter diagnostic, session identity, or
executor text. Tool, request, prompter-error, limits-error, and prompt-outcome
debugging is fixed or structural and never invokes user-defined `Debug`.

## Platform and host composition

The tool and injected seam use safe standard Rust, allocation, atomics, and
core futures only. They are exported by `machine-god-native` without an HTTP,
Unix, or non-WebAssembly feature gate and must compile on the repository's
native, FreeBSD, and WASI library targets. The slice supplies no browser bridge
or WASI terminal interaction; a caller must inject a portable deterministic or
unavailable prompter there.

The production `NativeReferenceHost` remains gated to its existing
Linux/macOS, non-WebAssembly, `ai-gateway-http` boundary. Its constructors gain
an explicit shared `QuestionPrompter`, register `ask_user_question` first in
the alphabetical tool catalog, and do not discover a terminal, TTY,
environment variable, file, or runtime for it. The exact sixteen-tool order is
`ask_user_question`, `copy_file`, `create_folder`, `delete_file`, `edit_file`,
`file_info`, `glob_files`, `grep_files`, `list_files`, `open_file`, `read_file`,
`rename_file`, `terminal`, `web_fetch`, `web_search`, and `write_file`.
Thirteen are descriptor-backed; the question and web tools are rootless.

## Pinned-fx relationship and deferrals

Pinned fx supplied the 1-4/2-6 schema, question/label ASCII trimming,
case-insensitive label deduplication, terminal-safe presentation, ordered
answer JSON, cancellation sentinel, noninteractive sentinel, and a result
codec that does not require answers to match option labels. Machine-god uses a
different stricter answer boundary: it trims and rejects empty answers,
applies explicit byte bounds, and terminal-safe encodes them. This slice also
adds strict unknown-field/type validation and explicit resource/concurrency
bounds. It intentionally omits fx's `permission_request_id` approval
escalation.

Deferred work includes:

- `permission_request_id`, auto-denied continuation, approval revalidation,
  grant caching, and permission modes beyond the delivered ask-only policy;
- a concrete terminal, graphical, browser, remote, or CLI question UI;
- timeout, detached or background prompts, prompt persistence/history, resume,
  notification, and multi-process capacity;
- multi-select, default choices, answer membership enforcement, and unbounded
  open-ended input;
- durable terminal actions, `vision`, `read_tool_result`, Milestone 05 skills,
  MCP, ACP, and subagent surfaces; and
- benchmark-workload changes, compatibility promotion, product-performance,
  or fx-equivalence claims.

Zig remains only the pinned upstream fx build input. Machine-god remains a Rust
product and neither ships nor executes Zig.
