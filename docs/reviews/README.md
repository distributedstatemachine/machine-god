# Adversarial reviews

Bounded slice 35, native `ask_user_question`, is **CYCLE 13 FORMAL REVIEW GREEN —
FEATURE WORKFLOWS PENDING** from exact delivered base `5846799`. It is limited to
ordinary questions through an injected rootless prompter: strict 1-4/2-6 input,
terminal-safe bounded text,
bounded ordered free-form answers, explicit no-policy-authority preparation,
separately owned prompter interaction authority, fixed
redacted outcomes, and default-one/hard-eight fail-fast prompt concurrency.
`permission_request_id`, CLI presentation, timeout, detached work, approval
escalation, and later-tool scope are deferred. Cycle-2 production, independent
evidence, and its exact local candidate gate were completed, but three formal
tracks rejected exact candidate `910d7bc`, tree `503a91f`, with a deduplicated
`0/0/2/2` union. Cycle-3 source `cf531d1`/`b7b4358`, evidence
`3e3c0c7`/`f3f6f9d`, and docs `bfdf05b` compose at exact behavior head
`8bdc33d96bf88f5986c0e01b3979a2cef0427e82`, tree
`7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`. The complete exact-1.94.1
local gate is green with 57 focused tests. Formal cycle 3 rejected exact
candidate `746e510c7d8eb93229996e74f91827f489e5bb31`, tree
`c49221efbea66c840b333f0de0161aa686aad52f`: correctness/API reported
`0/0/1/2`, lifecycle/platform `0/0/0/2`, performance/resources `0/0/2/0`, and
the deduplicated union is `0/0/3/2`. Cycle-4 core `e569514`/`4c8cff3`, native
`53c05cd`/`1857a3f`, and finding docs `b057958` compose at exact behavior head
`cb93bff35271e6dfc3f4c27ac7a72e621941845c`, tree
`fa402acb75c6d364c41db66f6b55595aa1d0e59a`. Total optional capability
inspection, private fixed four-slot answer ownership, and prompt/cancellation-
Waker teardown under the active permit pass the complete exact-1.94.1 local
gate with 171 named focused executions. Formal cycle 4 rejected exact candidate
`42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
`b761f7b93d535a1580910f43ff509c40aa07415b`: correctness/API reported
`0/0/0/1`, lifecycle/platform `0/0/1/1`, performance/resources `0/0/1/0`, and
the deduplicated union is `0/0/2/1`. Accepted findings are post-check
cancellation during final-Waker destruction, a concurrently moved callback
tail escaping permit ownership, and stale reference-host lineage. Cycle-5
evidence `ad47fcb`/`bcce292`, source `80382d8`/`e0fd8e0`, and finding docs
`ba53f55`/`b870731` compose at exact behavior head
`b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
`0b025f8e42e18006a72d89becf0e395d35c91a57`. Activity-backed Waker/callback
ownership, cached registration, teardown under activity, the final post-
teardown cancellation check, and deterministic race evidence are implemented.
All required exact-1.94.1 and extended local gates are green. The only Cargo
delta is an existing audited path fixture added to native tests; production
normal/build dependencies remain unchanged. Formal cycle 5 reviewed exact
candidate `54b1aab5660e90096b95518bde4ebffb93f28fa6`, tree
`54586d2256c8a3d2289b92bc9bc842eed9ce4d07`: correctness/API and lifecycle/
platform were `0/0/0/0`; performance/resources and the deduplicated union were
`0/0/1/0`. The candidate is rejected because arbitrary retained activity-Waker
clones independently forward concurrent blocking downstream callbacks while
consuming one prompt slot. Cycle 6 requires an activity-backed single-flight
coalescing notifier, lossless replay, stale-target close, capacity retention
through callback/clone ownership, and deterministic owned-future many-clone
evidence. Finding docs `7dee269`/`e20023c`, evidence `b007ada`/`4a929c4`, and
source `0488d71`/`707a794` compose at exact head `707a794`, tree `1e60299`.
One shared notifier serializes callbacks, coalesces one replay, closes stale
delivery, and owns capacity through callback/final-clone teardown. Deterministic
tests cover 16 concurrent retained clones and completed-prompt close/capacity.
Independent `236dd90`/`94b9fdd` passes direct 34, engine one, and native Clippy.
Focused, required pinned, extended, and unchanged release-smoke gates are
green. The test-only helper/one lock-list-line delta remains explicit;
production normal/build dependencies are unchanged. Formal cycle 6 reviewed
exact `85058a8aa88fab6912d9313f1ce71e2778cc937f`, tree
`fd3c5072c9473c7fe8767cc2692238eacb8a0f43`: correctness/API `0/0/0/1`,
lifecycle/platform `0/0/1/0`, performance/resources `0/0/1/0`, and
deduplicated union `0/0/1/1`. The medium is unbounded synchronous replay
amplification when every self-notice rearms the replay loop. The low is the
stale cycle-5 status in three operative opening summaries. Cycle 7 requires
observation-aware constant-bounded callback delivery, lossless observation-
then-notice replay, deterministic finite-budget evidence, consistent openings,
a focused status scan, new gates, and three fresh exact-SHA reviews. Rejection
docs `6128f03`/`1d354ff`, evidence `acca13c`/`b75fc54`, and source
`3d48ce8`/`fbb3f5c` compose at exact head `fbb3f5c`/`7cee96e`. Entry clears
state, bind observes, one later notice earns one replay, and close/panic clears
state; callback concurrency stays at most one without lock-held Waker work.
Finite-budget evidence rejects the base at 65 callbacks for budget 64 while
the fix uses one; positive evidence proves one lossless post-observation replay.
Focused direct 35/engine one, all pinned/extended, status 10/0, and release-
smoke gates were green. Formal cycle 7 reviewed exact
`617672984fbb897f2efec63de6a05bb32db9a3db`, tree
`f2cd844449193b46cfa1473ae21edad68664157e`: correctness/API and performance/
resources were `0/0/0/0`, lifecycle/platform was `0/0/1/0`, and the union is
`0/0/1/0`. Replay B was selected before prior target A was dropped; an
A-destructor panic can wedge `notifying`, and an A-destructor reentrant close or
replacement can permit stale B delivery. Cycle 8 must drop A before selecting
replay, catch and settle unwind and lane flags, and admit only the then-current
replay after successful destruction, with deterministic panic-recovery and
reentrant-close-suppression evidence. The analogous preexisting terminal path is outside
this bounded slice and is not claimed fixed. Rejection docs
`22d5702`/`3650dba`, evidence `cf4abfd`/`5681bab`, and source
`a1b3d23`/`d8075ff` compose at exact head `d8075ff`/`fa32564`. A is destroyed
under `catch_unwind` outside the lock while the lane/activity remains; then-
current replay arbitration follows. Destructor close/replacement wins, callback
or target-drop panic clears flags, callback panic wins if both occur, no foreign
work runs under lock, and concurrency stays at most one. Deterministic evidence
records the rejected-base fresh B=0 wedge and stale B=1 delivery, then proves
recovery, close suppression, and capacity retention. Focused direct 37/engine
one, all pinned/extended, status 10/0, and unchanged release-smoke gates are
green. Formal cycle 8 reviewed exact `e929b5ea7e3264c2b56066a416bc2a979a03b214`,
tree `cfadc42814688a29c4d512e5fd91c843423821d4`: correctness/API was
`0/0/0/0`, lifecycle/platform and performance/resources were each `0/0/1/0`,
and the union is `0/0/2/0`. One medium is secondary panic-payload destruction
overriding the promised callback panic; the other is synchronous re-poll/re-
notify extending an activation to 257 callbacks for budget 256. Cycle 9 must
suppress/forget the secondary with primary-marker/lane/capacity/fresh-delivery
evidence, and cap activation at initial plus one replay while retaining residual
pending work for later activation. The analogous preexisting terminal path is
outside this slice and is not claimed fixed. Rejection docs
`2faedc7`/`5296dcc`, evidence `cf2e220`/`ee25455`, and source
`527e10d`/`0279b8c` compose at exact head `0279b8c`/`50b2423`. Activation is
initial plus at most one replay, residual pending survives for later explicit
activation, close/panic clears, A drops before arbitration, concurrency remains
one, and capacity stays retained. Dual panic forgets the opaque secondary
payload to preserve callback-primary precedence; lone target-drop panic
propagates. Marker and residual-work tests pass with direct 39: the base's 257
callbacks become two, then four total after later activation. Focused,
pinned/extended, status 10/0, and unchanged release-smoke gates are green.
Formal cycle 9 rejected exact `1eeab670a552bc15b5602319b0bb1ce27d2be497`,
tree `5c86e624cf3c0e6d521382c377a9ed9b0500ee5b`: correctness/API and lifecycle/
platform reported the same `0/0/1/0` medium, performance/resources reported
`0/0/0/0`, and the deduplicated union is `0/0/1/0`. After the callback-plus-
one-replay budget, a legal wake after the replay poll remains pending with no
downstream scheduling until unrelated explicit notify activity. The committed
test manually invokes `retained_wakers[2]`; a last self-wake or cancellation
wake can otherwise leave the no-timeout prompt pending indefinitely. Cycle 10
now provides autonomous post-poll progress through an
`Open`/`DeliveryResourceExhausted`/`Closed` serialized nonrecursive lane capped
at exactly 256 callbacks. Rejection docs
`216c3b4`/`895c9d4`, final evidence `74a8497`/`5e46f56` (superseding
`5cbd9b0`), and source `b043364` compose at exact behavior head
`72e8e75ba2490d4dfa0f680d9dca0b4e10a0401a`, tree
`5405180e5b3b4b59c4d7e712f614bdbc958a9d75`. Short residual/cancellation
chains progress through callback 3; continuous chains reach terminal callback
256, then cancellation wins or the existing redacted nonretryable prompt-
failed error returns. Direct 41 and the complete local gate are green. No
thread, queue, dependency, or public API is added. Formal cycle 10 rejected
exact `4ea1c1f5be3586ce9bee696b12c4120dc2a72018`, tree
`78e781ffd7b03aafdf295ae79f4090120971c248`: correctness/API and lifecycle/
platform each reported `0/0/1/0`, performance/resources `0/0/0/0`, and the two
distinct mediums produce `0/0/2/0`. A queue-only callback returns before its
poll, so every queued self-wake resets the notify counter to one and bypasses
the finite cap. Separately, cleanup-panic payload destruction during unwind can
replace the primary or double-panic abort. Cycle 11 must enforce a prompt-
lifetime budget, explicitly forget suppressed/nonselected payloads, and add
queue-bound/cancellation plus primary-marker/no-abort/lane/capacity evidence.
Cycle-11 docs `d839d18`/`f8342db`, evidence `3692d19`/`adf2b93`, and source
`83dd836` compose at exact behavior head
`b8b721a065f4b14f5f3678a22ee5b0bd2267ca2f`, tree
`46721503429685e5feb8e4ac33f74e865acf0c2a`. Lifetime-wide callback accounting
and centralized cleanup precedence correct the queue budget and unwind defects.
Four tests plus a subprocess child prove the rejected base's callback-256
Pending, callback-257 cancellation, replaced primary, and `SIGABRT` cases.
Direct 46 and the complete local gate are green. Formal cycle 11 rejected exact
`b1d454ba21d2a380a4198bb1253c4cb1bc34d4a6`/`26d90d8`: correctness/API
reported `0/0/0/1`, lifecycle/platform `0/0/1/0`, performance/resources
`0/0/1/0`, and the union is `0/0/2/1`. The mediums are release panic `abort`
invalidating shipped recovery and a forgotten Waker-owning panic payload
retaining capacity forever. Cycle-12 docs `5047d40`/`e582331`, source
`0684f3e`/`54d0af0`, and evidence `87d175b` compose at exact behavior head
`696dccfa84b9ce0a57ca4f764a6f05aefedb39f3`, tree
`f8734a8815b424f07d59f668f5ccd2a59319a8b1`; disposable composition is
`8378a47`/`522d0a4`. Release `unwind`, the product probe, detachable state-held
capacity, and local callback/close guards make closed forgotten Wakers inert.
Direct 46, manifest eight, all exact/extended/portability gates, and the fresh
release/smokes pass. Formal cycle 12 rejected exact
`3dec7a2f073fa85479af19765b03b06cdfd9da8c`/`c34d20a` with correctness/API
`0/0/1/1`, lifecycle/platform `0/0/1/2`, performance/resources `0/0/1/0`, and
deduplicated union `0/0/1/2`. The shared medium is that the release probe covers
only a prompt-poll panic under `NoopWake`, not cleanup-panic precedence, a
target-drop secondary payload retaining the supplied Waker, stale-lane
suppression, or detached-capacity recovery. Cycle-12 rejection docs
`70c929f15d345431b4673f799a29b2b45eee2c5d`/`f74ebaf` and cycle-13 evidence
`c9f9535892441cc6b0f4a99f115365f10a7c8426`, integrated as
`c252620f55eb75edbb1f771950200168671ef0f3`/`a921449`, replace the release
evidence and rename one stale test without production source, API, or dependency
changes. The release probe proves ordinary prompt-drop and ambient primaries,
the supplied-Waker-owning target-drop secondary, destructor panic control,
product suppression/forget, and exact `2/2/0/2` drops/callbacks/stale-wakes/
fresh-capacity totals. Exact stdout is 193 bytes with empty stderr. Focused
46/1/9/1/8 plus native Clippy, all exact/extended gates, and fresh CLI SHA-256
`a568e58e07b02a3b9739f1210794ad698faa8c6aec9933247150e19fa67799b4` are
green. Formal cycle 13 reviewed exact
`a4f1bb91c00064e0ceb6975e1c9e7b4a09b1ff95`/`72a0303`; all three tracks and
the union reported `0/0/0/0`. Reviewers validated the real in-close target drop,
both primaries, secondary Waker ownership/control/suppression/forget, zero stale
plus fresh capacity before closed identity release, lifetime-256/resource
bounds, and current unwind wording. All prior findings are resolved. This docs-
only result seal is review-exempt and makes no behavior, performance,
equivalence, or delivery claim. Feature workflows, integration, `main`
workflows, and delivery remain pending.
See
[`m03-ask-user-question-review-01.md`](m03-ask-user-question-review-01.md).

Bounded slice 34, native `terminal`, is **DELIVERED** from exact delivered base
`52b5885`. It implements only bounded
foreground `exec` with a Linux-only system executor. Formal cycle 5 rejected
exact candidate `0c04859`, tree `b0359fb`, with a deduplicated `0/1/0/1`
union: a high self-target notifier cycle and a low stale-status defect. Cycle-6
source, independent evidence, and maintained documentation were completed in
non-overlapping worktrees and compose through exact behavior head `8705811`,
tree `8a319c1`. Its complete exact-1.94.1 replacement gate is green. The
immutable candidate was `28292b7`, tree `487f160`. Formal cycle 6 rejected it:
correctness/API reported `0/0/1/0`, while lifecycle/platform and performance/
resources each reported `0/0/0/0`. The sole medium is stale host-Waker delivery
after pending outer-future drop. Cycle-7 source, independent evidence, and
maintained documentation compose through exact behavior head `9810ee9`, tree
`cf7390e`, whose complete exact-1.94.1 replacement gate is green. The immutable
candidate was `3f07389`, tree `f048cdc`. Formal cycle 7 is **GREEN**:
correctness/API, lifecycle/platform, and performance/resources each reported
`0/0/0/0`. Review-exempt delivered record `ddd6a89`, tree `4a227b3`, passed
feature CI/Benchmark runs `33040148977`/`33040148920` and main CI/Benchmark
runs `33040487021`/`33040487046`; each benchmark run retained exactly two
unexpired exact-SHA artifacts. The delivered count is 34. Scope and worktree
cleanup are recorded in
[`m03-terminal-review-01.md`](m03-terminal-review-01.md).

Bounded slice 33, native `web_search`, is **DELIVERED** from exact delivered
base `4ba9f5a`. Its frozen
local-tool contract performs exact Gateway-network preflight, then at most one
approved required Perplexity worker request through the shared transport and a
dedicated bounded native provider-executed decoder. Composed behavior precursor
`3d298400`, tree `5222c3e`, passed the complete exact-1.94.1 local gate. Formal
cycle 1 rejected exact `89c5ec95`, tree `8d91a55`; the three tracks reported
`0/1/3/2`, `0/2/3/1`, and `0/0/3/0`, deduplicated to `0/2/5/2` in
blocker/high/medium/low order. Source remediation is composed from exact
isolated components `096b11c4` and `ca0b990a`. Exact composed precursor
`e662fa8`, tree `6c0ace9`, passes the complete local gate. Cycle 2's accepted
`0/1/1/1` union was remediated through exact precursor `366cef9`, tree
`40c05cb`. Formal cycle 3 rejected exact candidate `aef6abe`, tree `5abcef3`,
with a deduplicated `1/0/2/2`. Exact isolated components `5d45dca` and
`454f8fd` compose its remediation. Exact precursor `b834205`, tree `f3557a5`,
passes the complete replacement gate. Formal cycle 4 rejected exact `cc1d3d1`,
tree `ad0c3d3`, with a deduplicated `0/0/1/1`; the remediation passed formal
cycle 5 with a `0/0/0/0` union. Review-exempt delivery record `52b5885`, tree
`148b358`, passed feature CI/Benchmark runs `33023313461`/`33023313463` and
main CI/Benchmark runs `33023812814`/`33023812808`; every run succeeded for
the exact SHA, and both benchmark runs retained two unexpired exact-SHA
artifacts. The delivered count is 33. Review scope, fixed resources, deferrals,
and the per-iteration committed/
integrated/clean worktree-removal invariant are tracked in
[`m03-web-search-review-01.md`](m03-web-search-review-01.md). No performance or
fx-equivalence claim is made.

Current bounded slice 32 is **DELIVERED**. Cycle 4 rejected exact
`df72e084`, tree `99bf524`, with correctness/API, native effects, and
performance/resources each at `0/0/1/0`; the deduplicated `0/0/2/0` union is
the wire-form mismatch plus eager approximately 8.9 MB tracker allocation.
Exact remediation `1f96c4bf`, tree `b320f552`, makes `StoredEnvelope`,
`StoredRecord`, `StoredMessage`, `StoredToolCall`, and `StoredToolOutput`
object-only and `Role` string-only, keeps the canonical writer unchanged, and
grows fixed-fingerprint tracker storage fallibly with unique keys, with at most
65,536 tracker entries. Exact gate-record candidate `8f533cde`, tree `8215fb94`,
passed the complete exact-1.94.1 local gate without fallback: focused 24
native/64 CLI process/16 differential, Python 135/8 skips, byte-stable pinned
fx `b1774fb`, WASI/FreeBSD with only the established `read_file` warning, docs
85/147/626/81, `cargo-deny` 0.20.2 with three established duplicate warnings,
`cargo-audit` 0.22.2 over 211 dependencies/
1,226 advisories with zero vulnerabilities, the unchanged 364-line production
graph, and diff/inventory/no-added-unsafe evidence are green.

The 3,985,216-byte release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c` and
passed 16 equivalence categories across 20 records, 12 grammar cases, missing/
no-create, held-lock, engine-over-default, and 8,650,857-byte near-cap evidence;
the native near-cap probe passed 1/1. Allocator
total/current/maximum tuples are `12/2/7` and `819/14/645` bytes for empty/
short/long text, `14/2/8` and `1,427/14/1,059` for short/long JSON, and
`35/2/9` and `2,228,435/14/1,606,083` for 5,000 keys. Cycle 5 rejected exact
`8f533cde`: correctness/API `0/0/0/1`, native effects `0/0/0/0`, and
performance/resources `0/0/0/1`, deduplicated to one low stale cross-document-
summary finding. That documentation remediation is already composed in exact
cycle-6 candidate `5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
`d2fec0815b60c61368298e7f4f0d7bef0fc2e097`. Formal cycle 6 rejected it:
correctness/API, native effects, and performance/resources each reported
`0/0/0/1`; the deduplicated `0/0/0/1` is solely that these pages described the
committed remediation as pending. There was no additional production, API,
native, or performance finding. Formal cycle 7 rejected exact
`399e75eda0f61501fe179a22de6a0f4f2abfce06`, tree
`d056b96ef8361e841c936c5f61c138de913b5fff`: correctness/API and native effects
each reported `0/0/0/0`, while performance/resources reported `0/0/0/1`; the
deduplicated union is `0/0/0/1`. The sole low corrects resource wording:
shadowed duplicate values may parse more nodes than survive in the final tree.
The 65,536 caps apply separately to tracker entries and aggregate final decoded-
tree logical-node accounting, while the 8,651,165-byte file ceiling bounds
total parse work. Production and resource behavior were otherwise green. The
wording correction is present. Formal cycle 8 is **GREEN** on exact reviewed
candidate `d724b6195324349cc5628a47f8ab7fa496123cd5`, tree
`6439863a9b7fd1720156c790fedc4798256c2b6d`: correctness/API, native/effects,
and performance/resources each reported `0/0/0/0`, with a deduplicated
`0/0/0/0` union. Independent evidence included 598 release-binary
differentials with zero mismatches plus focused 24 native, 14 CLI unit, and 24
CLI process tests; native also reconfirmed focused 24 and green WASI/FreeBSD
checks with the established `read_file` warning; performance confirmed the
exact allocator tuples above and the corrected cap semantics. This
documentation-only result seal is review-exempt: it records review of
`d724b61` / `6439863` and does not imply that the seal commit itself was
reviewed. Review-exempt documentation seal
`b6db9a67c070f7ef599d994c44b4a21731a004c5`, tree
`59dd628fd0552c5083449f7a31aa4241a8ecb952`, passed feature CI run
`32965947722` and Benchmark evidence run `32965947723`, was integrated on
`main`, and passed main CI run `32966531225` and Benchmark evidence run
`32966531319`. All four runs succeeded for exact `b6db9a6`; each benchmark
run retained exactly two unexpired exact-SHA artifacts. Slice 32 is delivered,
and the delivered count is 32. No product-performance or fx-equivalence claim
is made. Zig is used only to build the pinned upstream fx comparison input;
`machine-god` remains a Rust product and is neither written in nor shipped as
Zig. This final delivery-record commit is docs-only and user-exempt from
adversarial review. A commit cannot contain its own future workflow IDs, so
the exact workflow IDs for this record will be reported at handoff. The
slice remains non-equivalent, unmeasured, and claim-ineligible; no product-
performance or fx-equivalence claim is made.

- [`m03-session-cli-review-01.md`](m03-session-cli-review-01.md) tracks the
  delivered bounded slice-32 `session <id> [--json]` composition from exact
  delivered base `6e687b6`. It retains the full rejected cycle-4, remediated
  gate-record, rejected cycle-5 through cycle-7 evidence, and formal green
  cycle-8 result summarized above. The review-exempt result seal records the
  reviewed `d724b61` / `6439863` candidate. Exact seal `b6db9a6` completed the
  feature and main delivery gates recorded above.
  Historical initial composition was `852fec7`, focused remediation advanced
  through precursor `c0c16a7`, and gate precursor `fa099f7`, tree `64d6a72`,
  passed its local and supplemental gates before formal cycle-1 review.

- [`m03-sessions-cli-review-01.md`](m03-sessions-cli-review-01.md) tracks the
  delivered bounded slice-31 `sessions [--json]` candidate. Cycle 1 rejected
  exact candidate `9448738` with `0/0/0/3` deduplicated findings. Exact
  replacement `a527652`, tree `0249dd0`, passed three fresh cycle-2 reviews at
  `0/0/0/0` each. Seal `b5b9116`, tree `3e61754`, passed exact feature and main
  CI/benchmark delivery gates.

Each feature and milestone receives fresh correctness/API, security/abuse, and
performance/concurrency reviews. Store reports as `mNN-feature-review-NN.md` and
record the exact reviewed commit, findings, resolutions, and rejected rationales.

- [Milestone 01 completion evidence and review status](m01-milestone-review.md)
- [Milestone 02 core-contracts review](m02-core-contracts-review-01.md)
- [Milestone 02 deterministic testkit review](m02-testkit-review-01.md)
- [Milestone 02 bounded tool-loop review](m02-tool-loop-review-01.md)
- [Milestone 02 completion evidence and review status](m02-milestone-review.md)
- [Milestone 03 config/status CLI delivery review](m03-config-status-cli-review-01.md)
- [Milestone 03 bounded native configuration loading review](m03-native-config-load-review-01.md)
- [Milestone 03 capability-aware tool preflight review](m03-tool-preflight-review-01.md)
- [Milestone 03 confined native read_file review](m03-read-file-review-01.md)
- [Milestone 03 Linux containment marker remediation review](m03-benchmark-containment-marker-review-01.md)
- [Milestone 03 confined native list_files review](m03-list-files-review-01.md)
- [Milestone 03 injected-transport AI Gateway review](m03-ai-gateway-review-01.md)
- [Milestone 03 native AI Gateway HTTP transport review](m03-ai-gateway-http-review-01.md)
- [Milestone 03 native file session store review](m03-session-store-review-01.md)
- [Milestone 03 native ask permission handler review](m03-ask-permission-review-01.md)
- [Milestone 03 native AI Gateway credential discovery review](m03-ai-gateway-credential-review-01.md)
- [Milestone 03 native host configuration schema-v2 review](m03-native-host-config-review-01.md)
- [Milestone 03 native reference-host composition review](m03-native-reference-host-review-01.md)
- [Milestone 03 configured credential-source review](m03-configured-credential-source-review-01.md)
- [Milestone 03 native root-selection delivery review](m03-native-root-selection-review-01.md)
- [Milestone 03 native session-lifecycle review](m03-native-session-lifecycle-review-01.md)
- [Milestone 03 native session-listing delivery review](m03-native-session-listing-review-01.md)
- [Milestone 03 native `file_info` delivery review](m03-file-info-review-01.md)
- [Milestone 03 native `glob_files` delivery review](m03-glob-files-review-01.md)
- [Milestone 03 native `grep_files` delivery review](m03-grep-files-review-01.md)
- [Milestone 03 native `write_file` delivery review](m03-write-file-review-01.md)
- [Milestone 03 native `edit_file` delivery review](m03-edit-file-review-01.md)
- [Milestone 03 native `delete_file` delivery review](m03-delete-file-review-01.md)
- [Milestone 03 native `rename_file` delivery review](m03-rename-file-review-01.md)
- [Milestone 03 native `copy_file` delivery review](m03-copy-file-review-01.md)
- [Milestone 03 native `create_folder` delivery review](m03-create-folder-review-01.md)
- [Milestone 03 native `open_file` delivery review](m03-open-file-review-01.md)
- [Milestone 03 native `web_fetch` review ledger](m03-web-fetch-review-01.md)
- [Milestone 03 native `web_search` live review ledger](m03-web-search-review-01.md)
- [Milestone 03 native `terminal` live review ledger](m03-terminal-review-01.md)
- [Milestone 03 `permissions` CLI review ledger](m03-permissions-cli-review-01.md)
- [Milestone 03 `models` CLI delivery review ledger](m03-models-cli-review-01.md)
- [Milestone 03 `doctor` CLI live review ledger](m03-doctor-cli-review-01.md)
