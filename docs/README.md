# Documentation

Bounded Milestone 03 slice 35, native `ask_user_question`, is **CYCLE 10
REJECTED — CYCLE 11 REMEDIATION IN PROGRESS**
from exact delivered base
`5846799b665d62fc8301b33520da5cda33e850b3`. Ordinary-question input is a
strict bounded batch, normalized for terminal-safe display and passed to an
explicit rootless `QuestionPrompter`. Ordered answers may be bounded free-form
values and need not equal an option. Preparation explicitly requires no
policy-governed authority; the injected prompter separately owns its host
interaction authority. `permission_request_id` approval escalation is rejected
and deferred. No CLI UI, timeout, detached work, durable terminal expansion,
`vision`, or `read_tool_result` is added. Historical behavior head `a76818e`,
tree `f44def5`, passed its recorded local gate. Formal cycle 1 rejected exact
candidate `6c54ec3bf2c23983f14b0a4edeac723321a97900`, tree
`bea90245a559e8e223cc5bb45e0ddfa15e426ee6`, with a deduplicated
0 blocker / 1 high / 3 medium / 3 low union. Independent evidence component
`c77b336` (integrated as `0dd1128`) and production component `9d2e0f2`
(integrated as `47e9505`) remediate those findings. Cycle-1 finding docs compose
with them at exact behavior head `c8718c6`, tree `c27463b`, whose complete
exact-1.94.1 local gate was green with 55 focused tests. Formal cycle 2 rejected
exact candidate `910d7bc84cfd7800fb4daf9ab8537bf269027896`, tree
`503a91f334156dbcf2470560b9bb456c3491fd3d`, with a deduplicated `0/0/2/2`
union. Cycle-3 source `cf531d1`/`b7b4358`, evidence `3e3c0c7`/`f3f6f9d`, and
documentation `bfdf05b` compose at exact behavior head `8bdc33d96bf88f5986c0e01b3979a2cef0427e82`,
tree `7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`. Complete pre-trim answer
lengths are bounded before scanning, exact reachable-maximum evidence replaces
the invalid result-guard claim, and the complete exact-1.94.1 local gate is
green with 57 focused tests. Legal inputs reach 41,102 serialized result bytes;
the unreachable 49,152-byte guard remains authoritative defense in depth. The
formal cycle-3 candidate `746e510c7d8eb93229996e74f91827f489e5bb31`, tree
`c49221efbea66c840b333f0de0161aa686aad52f`, was rejected with a deduplicated
`0/0/3/2` union. Cycle-4 core component `e569514`/`4c8cff3`, native component
`53c05cd`/`1857a3f`, and finding-documentation component `b057958` compose at
exact behavior head `cb93bff35271e6dfc3f4c27ac7a72e621941845c`, tree
`fa402acb75c6d364c41db66f6b55595aa1d0e59a`. The public capability accessor is
total and optional; host answers occupy a private fixed four-slot container
that admits zero through four entries; and prompt-future plus cancellation
waiter/Waker destruction remains inside the active permit. The exact-1.94.1
local gate is green with 171 named focused executions, all required and
extended checks, and the unchanged locked release artifact. The tool requires
no policy-governed authority, while its injected prompter separately owns host
interaction authority. Formal cycle 4 rejected exact candidate
`42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
`b761f7b93d535a1580910f43ff509c40aa07415b`: correctness/API reported
`0/0/0/1`, lifecycle/platform `0/0/1/1`, and performance/resources
`0/0/1/0`, deduplicated to `0/0/2/1`. Cancellation could become observable
only while the final registered Waker was synchronously destroyed, after the
last cancellation check, and a concurrently moved Waker callback could finish
after outer drop released the permit. The reference-host record also retained
stale cycle-3-pending lineage. Cycle-5 evidence `ad47fcb`/`bcce292`, source
`80382d8`/`e0fd8e0`, and finding docs `ba53f55`/`b870731` compose at exact
behavior head `b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
`0b025f8e42e18006a72d89becf0e395d35c91a57`. Activity-backed cancellation
Waker ownership now extends through callback return, cached registration plus
prompt/waiter/Waker teardown remains under activity, and direct returns make a
final post-teardown cancellation check. Deterministic evidence covers both
accepted races. Its exact-1.94.1 required and extended local gates are green;
32 direct question tests and the integrated engine/host suites pass. The only
Cargo delta is the existing audited reentrant-Waker helper added as a native
test-only path dependency, with no production normal/build dependency change.
Formal cycle 5 reviewed exact `54b1aab5660e90096b95518bde4ebffb93f28fa6`,
tree `54586d2256c8a3d2289b92bc9bc842eed9ce4d07`: correctness/API and lifecycle/
platform were `0/0/0/0`; performance/resources and the deduplicated union were
`0/0/1/0`. The candidate is rejected because arbitrary retained activity-Waker
clones can independently forward concurrent blocking downstream callbacks
while consuming one prompt slot. Cycle 6 must add an activity-backed single-
flight coalescing notifier, lossless replay, stale-target close, capacity
ownership through callback/clone teardown, and deterministic owned-future many-
clone evidence. Finding docs `7dee269`/`e20023c`, evidence
`b007ada`/`4a929c4`, and source `0488d71`/`707a794` compose at exact behavior
head `707a794230758374fa2dab6d65eaf27449c7c477`, tree
`1e60299e21f45079f4e8cf27468a28d1ab4fe227`. One shared notifier serializes
delivery, coalesces concurrent notices into one replay, closes stale delivery,
and retains the prompt slot through callback/final-clone ownership. The two
deterministic 16-clone/replay and completion/close/capacity regressions pass.
Focused, required pinned, extended, and unchanged release-smoke gates are
green, including 34 direct tests. That exact historical pre-review checkpoint
was **CYCLE 6 LOCAL GATE GREEN — FORMAL REVIEW PENDING**. Formal cycle 6
rejected exact `85058a8aa88fab6912d9313f1ce71e2778cc937f`, tree
`fd3c5072c9473c7fe8767cc2692238eacb8a0f43`: correctness/API `0/0/0/1`,
lifecycle/platform `0/0/1/0`, performance/resources `0/0/1/0`, and
deduplicated union `0/0/1/1`. Reentrant replay can repeatedly rearm itself and
amplify one wake into unbounded synchronous callbacks. Cycle-7 rejection docs
`6128f03`/`1d354ff`, evidence `acca13c`/`b75fc54`, and source
`3d48ce8`/`fbb3f5c` compose at exact behavior head `fbb3f5c`, tree `7cee96e`.
Observation now gates one later serialized replay; callback count remains
constant-bounded before observation, close/panic clears state, activity remains
owned, and foreign Waker work stays outside the lock. The base consumes 65
callbacks for a finite budget of 64 while the fix consumes one; refined
positive evidence proves lossless post-observation replay. Focused, required
pinned, extended, status 10/0, and release-smoke gates are green. The current
status is **CYCLE 10 REJECTED — CYCLE 11 REMEDIATION IN PROGRESS**. Formal cycle
7 rejected exact `617672984fbb897f2efec63de6a05bb32db9a3db`, tree
`f2cd844449193b46cfa1473ae21edad68664157e`: correctness/API and performance/
resources were `0/0/0/0`, lifecycle/platform was `0/0/1/0`, and the union is
`0/0/1/0`. Replay B is selected before prior target A is dropped, so A's
destructor can panic and wedge `notifying` or reentrantly close/replace the lane
before stale B delivery. Cycle-8 docs `22d5702`/`3650dba`, evidence
`cf4abfd`/`5681bab`, and source `a1b3d23`/`d8075ff` compose at exact behavior
head `d8075ff`, tree `fa32564`. A is destroyed under `catch_unwind` outside the
lock while lane/activity ownership remains; only afterward does arbitration
read the then-current lifecycle, pending state, and target. Destructor close or
replacement wins, every callback/target-drop panic clears the lane, callback
panic wins if both occur, and maximum callback concurrency remains one. The
rejected-base B=0 wedge/B=1 stale-delivery regressions now recover and suppress
respectively while retaining capacity. Focused direct 37/engine one, complete
pinned/extended, status 10/0, and unchanged release-smoke gates are green.
Formal cycle 8 rejected exact `e929b5ea7e3264c2b56066a416bc2a979a03b214`,
tree `cfadc42814688a29c4d512e5fd91c843423821d4`: correctness/API was
`0/0/0/0`, lifecycle/platform and performance/resources were each `0/0/1/0`,
and the union is `0/0/2/0`. The distinct mediums are secondary panic-payload
destruction overriding the promised callback panic, and synchronous re-poll/
re-notify extending one activation to 257 callbacks for budget 256. Cycle 9
must suppress or forget the secondary payload while proving primary marker,
lane, capacity, and fresh delivery; and cap an explicit notify activation at
initial plus one replay while retaining residual pending work for a later
activation. Cycle-9 docs `2faedc7`/`5296dcc`, evidence `cf2e220`/`ee25455`,
and source `527e10d`/`0279b8c` compose at exact head `0279b8c`, tree `50b2423`.
Explicit activation is initial plus at most one replay; residual pending work
survives for later activation; close/panic clears; concurrency remains one; and
capacity remains held. Dual panic forgets the opaque secondary payload to
preserve callback-primary precedence, while a lone target-drop panic propagates.
Marker and residual-work evidence passes with 39 direct tests: 257 callbacks on
the base becomes two and then four total after later activation. Focused,
pinned, extended, status 10/0, and unchanged release-smoke gates are green.
Analogous preexisting `terminal` code is outside this slice and is not claimed
fixed. Formal cycle 9 rejected exact `1eeab670a552bc15b5602319b0bb1ce27d2be497`,
tree `5c86e624cf3c0e6d521382c377a9ed9b0500ee5b`: correctness/API and lifecycle/
platform each reported the same `0/0/1/0` medium, performance/resources
reported `0/0/0/0`, and the deduplicated union is `0/0/1/0`. After the callback
plus one replay budget, a post-poll wake can remain pending without downstream
scheduling until unrelated explicit activity; the committed test progresses
only by manually calling `retained_wakers[2]`. A last self-wake or cancellation
wake can therefore leave the no-timeout prompt pending indefinitely. Cycle-10
docs `216c3b4`/`895c9d4`, final evidence `74a8497`/`5e46f56` (superseding
`5cbd9b0`), and source `b043364` compose at exact head
`72e8e75ba2490d4dfa0f680d9dca0b4e10a0401a`, tree
`5405180e5b3b4b59c4d7e712f614bdbc958a9d75`. The lifecycle is
`Open`/`DeliveryResourceExhausted`/`Closed`, one serialized nonrecursive lane,
and exactly 256 callbacks per activation. Short chains now reach callback 3
autonomously; a continuing chain reaches terminal callback 256, then
cancellation wins or the existing redacted nonretryable prompt-failed error is
returned. Focused direct 41 and all pinned/extended/release-smoke gates are
green. No thread, queue, dependency, or public API is added. Formal cycle 10
rejected exact `4ea1c1f5be3586ce9bee696b12c4120dc2a72018`, tree
`78e781ffd7b03aafdf295ae79f4090120971c248`: correctness/API and lifecycle/
platform each reported `0/0/1/0`, performance/resources `0/0/0/0`, and the
distinct union is `0/0/2/0`. Queue-only delivery resets the notify-local budget
to one on every self-waking `Pending` poll, so 256 is never reached. Separately,
suppressed/nonselected opaque cleanup-panic payloads can replace the primary or
double-panic abort during unwind. Cycle 11 must make the budget prompt-lifetime
and forget every suppressed payload, with exact queue/cancellation and marker/
no-abort/lane/capacity evidence. No cycle-11 source or evidence exists yet.
No cycle-11 gate, review, integration, or delivery exists yet.
The normative
boundary is
[`ask-user-question.md`](ask-user-question.md), with status in the
[`slice-35 review ledger`](reviews/m03-ask-user-question-review-01.md).

Bounded Milestone 03 slice 34, native `terminal`, is **DELIVERED**
from exact delivered base `52b5885f275c9f6f4f16b378f71780c29f2ebab2`.
It freezes only bounded foreground `exec`: fixed `/bin/sh -c`, a descriptor-
retained workspace-relative starting directory, exact environment-snapshot
identity in the process capability, bounded separate stdout/stderr, timeout,
cancellation, fail-fast concurrency, an independent deadline guardian,
and bounded process-group cleanup. Its first-poll timeout governs controllable
userspace phases but is not a wall-clock ceiling across blocked host syscalls,
synchronous executor poll/drop, or Waker callbacks. Public system-executor
construction is Linux-only; the private non-Linux host entry stays advertised
and returns fixed unsupported at allowed execution before cwd lookup or spawn.
It adds no PTY, durable session, sandbox, CLI command, benchmark, product-
performance, or fx-equivalence claim. Exact cycle-5 behavior head `3bfc0bf`,
tree `d4ed10d`, passed its complete exact-1.94.1 replacement gate. Formal cycle
5 rejected exact candidate `0c04859`, tree `b0359fb`, with a deduplicated
`0/1/0/1` union: one high self-target notifier-cycle defect and one low stale-
status defect. Cycle-6 remediation prevents the supplied notifier Waker from
becoming its own target, explicitly closes notification on completion, and
refreshes these status summaries. Exact cycle-6 behavior head `8705811`, tree
`8a319c1`, passes the complete exact-1.94.1 replacement gate. The immutable
candidate was `28292b7`, tree `487f160`; formal cycle 6 rejected it with a
deduplicated `0/0/1/0` union because pending outer-future drop can leave an
independently retained supplied Waker able to deliver to the stale host target.
Cycle-7 frame-owned RAII closes notification on normal return, pending future
drop, and unwind without releasing activity owned by a retained supplied-Waker
clone. Exact behavior head `9810ee9`, tree `cf7390e`, passes the complete exact-
1.94.1 replacement gate. Formal cycle 7 is **GREEN** on exact candidate
`3f07389`, tree `f048cdc`:
correctness/API, lifecycle/platform, and performance/resources each reported
`0/0/0/0`. Review-exempt delivered record `ddd6a89`, tree `4a227b3`, passed
feature CI/Benchmark runs `33040148977`/`33040148920` and main CI/Benchmark
runs `33040487021`/`33040487046`; every run succeeded for that exact SHA and
both benchmark runs retained two unexpired exact-SHA artifacts. The delivered
count is 34. No product-performance or fx-equivalence claim is made. See the
[`terminal` contract](terminal.md) and
[`live review ledger`](reviews/m03-terminal-review-01.md).

Bounded Milestone 03 slice 33, native `web_search`, is **DELIVERED**
from exact delivered base `4ba9f5afde89b9666fe9929bb81fbabcaa834334`.
Its frozen strict input is required `query` plus optional mutually exclusive
`allowed_domains` / `blocked_domains`. Effect-free preflight prepares the exact
configured AI Gateway `NetworkTarget`; one allowed execution makes at most one
required Perplexity worker request through the shared injected transport, and a
separate bounded native codec admits its one provider-executed call/result.
Production, independent evidence, and these docs compose through behavior
precursor `3d2984000301e58762e0940504159aeb55b2389e`, tree
`5222c3e009e9fe440097a86fd46889d1bb2e1434`. Its complete exact-1.94.1 local
gate is green. Formal cycle 1 rejected exact candidate `89c5ec95`, tree
`8d91a55`, with a deduplicated `0/2/5/2` blocker/high/medium/low finding union.
Source remediation is composed from exact isolated components `096b11c4` and
`ca0b990a`. Exact composed remediation precursor `e662fa8047c5ca321d622b9b5920166804a35c27`,
tree `6c0ace98ea9931af9d16cc9fb2ade969df477d3c`, passes the complete local gate;
cycle 2 reported `1/1/1/1` on exact candidate `399f5f7`, tree `99a88a4`.
Primary-source adjudication rejects the layer-confused provider-envelope
blocker, leaving the accepted `0/1/1/1` union under remediation. The slice is
remediated through exact precursor `366cef966d7dcf1b11101a37d4493099e6f421a7`,
tree `40c05cb2999c641bc7ccbdc369fc6d9251b989b7`, whose complete exact-1.94.1
replacement gate is green. Formal cycle 3 rejected exact candidate `aef6abe`,
tree `5abcef3`, with a deduplicated `1/0/2/2`. Remediation is composed from exact
isolated components `5d45dca` and `454f8fd`. Exact composed precursor `b834205`,
tree `f3557a5`, passes the complete replacement gate. Formal cycle 4 rejected
exact candidate `cc1d3d1`, tree `ad0c3d3`, with a deduplicated `0/0/1/1`;
exact finish-envelope remediation component `dc79c8d`, tree `e2fed70`, is
composed with maintained documentation and exact host-fixture component
`9f6c474`. Exact precursor `2e9c44d`, tree `3e25daa`, passes the complete
replacement gate. Formal cycle 5 is green on exact `782aa54`, tree `b1ba692`,
with a `0/0/0/0` union. Review-exempt delivery record `52b5885`, tree
`148b358`, passed exact feature CI/Benchmark runs
`33023313461`/`33023313463` and main CI/Benchmark runs
`33023812814`/`33023812808`. All four runs succeeded for that exact SHA; each
benchmark run retained two unexpired exact-SHA artifacts. The delivered count
is 33. The fixed
resource/platform/deferred boundary is the
[`native web search contract`](web-search.md), and status is tracked in its
[`live review ledger`](reviews/m03-web-search-review-01.md). The slice is
unmeasured and makes no product-performance or fx-equivalence claim.

Current bounded Milestone 03 slice 32, strict `session <id> [--json]`, is
**DELIVERED**. Cycle 4 rejected exact `df72e084`, tree `99bf524`, with
correctness/API, native effects, and performance/resources each at `0/0/1/0`;
its deduplicated `0/0/2/0` union is the ordinary/streamed wire-form mismatch
plus eager approximately 8.9 MB
tracker allocation. Exact remediation `1f96c4bf`, tree `b320f552`, makes
`StoredEnvelope`, `StoredRecord`, `StoredMessage`, `StoredToolCall`, and
`StoredToolOutput` object-only and `Role` string-only, preserves the canonical
writer, and grows fixed-fingerprint tracker storage fallibly with unique keys,
with at most 65,536 tracker entries. Exact gate-record candidate `8f533cde`, tree
`8215fb94`, passed the complete exact-1.94.1 local gate without fallback:
focused 24 native/64 CLI process/16 differential, Python 135/8 skips, byte-
stable pinned fx `b1774fb`, WASI/FreeBSD with only the established `read_file`
warning, documentation 85/147/626/81 checks,
`cargo-deny` 0.20.2 with three established duplicate warnings, `cargo-audit`
0.22.2 over 211 dependencies/1,226 advisories with zero vulnerabilities, the
unchanged 364-line production graph, and diff/inventory/no-added-unsafe checks
are green. Its 3,985,216-byte release binary has SHA-256
`c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c` and
passed the ledger's 16-category/20-record, 12-grammar, missing/no-create,
held-lock, engine-over-default, and 8,650,857-byte near-cap matrix; the native
near-cap probe passed 1/1. Allocator
total/current/maximum tuples are `12/2/7` and `819/14/645` bytes for empty/
short/long text, `14/2/8` and `1,427/14/1,059` for short/long JSON, and
`35/2/9` and `2,228,435/14/1,606,083` for 5,000 keys.

Cycle 5 rejected exact `8f533cde`: correctness/API `0/0/0/1`, native effects
`0/0/0/0`, and performance/resources `0/0/0/1`, deduplicated to one low stale
cross-document-summary finding. That documentation remediation is already
composed in exact cycle-6 candidate
`5332d6a841521f3aa3c26b7c2b9a0e77cb1f7e31`, tree
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
slice stays non-equivalent, unmeasured, and claim-ineligible; no
product-performance or fx-equivalence claim is made. See the
[`live ledger`](reviews/m03-session-cli-review-01.md).

- [Implementation plan](implementation-plan.md)
- [Architecture](architecture.md)
- [Provider-neutral core API](core-api.md)
- [Deterministic testkit](testkit.md)
- [Command-line interface](cli.md)
- [Delivered top-level `session` CLI contract](session-cli.md)
- [Delivered top-level `doctor` CLI contract](doctor-cli.md)
- [Delivered top-level `models` CLI contract](models-cli.md)
- [Delivered top-level `permissions` CLI contract](permissions-cli.md)
- [Delivered top-level `sessions` CLI contract](sessions-cli.md)
- [Native configuration and schema-v3 credential source](configuration.md)
- [Native `read_file` tool](read-file.md)
- [Native `list_files` tool](list-files.md)
- [Native `file_info` tool](file-info.md)
- [Native `glob_files` tool](glob-files.md)
- [Native `grep_files` tool](grep-files.md)
- [Native `write_file` delivered contract](write-file.md)
- [Native `edit_file` tool](edit-file.md)
- [Native `delete_file` tool](delete-file.md)
- [Native `rename_file` tool](rename-file.md)
- [Native `copy_file` contract](copy-file.md)
- [Native `create_folder` delivered contract](create-folder.md)
- [Native `open_file` delivered contract](open-file.md)
- [Native `web_fetch` delivered contract](web-fetch.md)
- [Native `web_search` slice-33 contract](web-search.md)
- [Native `terminal` slice-34 foreground-exec contract](terminal.md)
- [Injected-transport AI Gateway provider](ai-gateway.md)
- [Optional native AI Gateway HTTP transport](ai-gateway-http.md)
- [Native AI Gateway credential discovery](ai-gateway-credentials.md)
- [Native file session store](session-store.md)
- [Native ask permission handler](ask-permission.md)
- [Native reference-host composition](native-reference-host.md)
- [Native root selection and preparation](native-root-selection.md)
- [Native by-ID session lifecycle](native-session-lifecycle.md)
- [Native session listing](native-session-listing.md)
- [Delivered native session inspection](native-session-inspection.md)
- [Compatibility](compatibility.md)
- [Performance](performance.md)
- [Security](security.md)
- [Architecture decisions](decisions/README.md)
- [Adversarial reviews](reviews/README.md)
- [Milestone 01 completion evidence](reviews/m01-milestone-review.md)
- [Milestone 02 completion evidence](reviews/m02-milestone-review.md)
- [Milestone 03 config/status CLI delivery review](reviews/m03-config-status-cli-review-01.md)
- [Milestone 03 native host configuration schema-v2 review](reviews/m03-native-host-config-review-01.md)
- [Milestone 03 native reference-host composition review](reviews/m03-native-reference-host-review-01.md)
- [Milestone 03 configured credential-source review](reviews/m03-configured-credential-source-review-01.md)
- [Milestone 03 native root-selection delivery review](reviews/m03-native-root-selection-review-01.md)
- [Milestone 03 native session-lifecycle review](reviews/m03-native-session-lifecycle-review-01.md)
- [Milestone 03 native session-listing delivery review](reviews/m03-native-session-listing-review-01.md)
- [Milestone 03 native `file_info` delivery review](reviews/m03-file-info-review-01.md)
- [Milestone 03 native `glob_files` delivery review](reviews/m03-glob-files-review-01.md)
- [Milestone 03 native `grep_files` delivery review](reviews/m03-grep-files-review-01.md)
- [Milestone 03 native `write_file` delivery review](reviews/m03-write-file-review-01.md)
- [Milestone 03 native `edit_file` delivery review](reviews/m03-edit-file-review-01.md)
- [Milestone 03 native `delete_file` delivery review](reviews/m03-delete-file-review-01.md)
- [Milestone 03 native `rename_file` delivery review](reviews/m03-rename-file-review-01.md)
- [Milestone 03 native `copy_file` delivery review](reviews/m03-copy-file-review-01.md)
- [Milestone 03 native `create_folder` delivery review](reviews/m03-create-folder-review-01.md)
- [Milestone 03 native `open_file` delivery review](reviews/m03-open-file-review-01.md)
- [Milestone 03 native `web_fetch` review ledger](reviews/m03-web-fetch-review-01.md)
- [Milestone 03 native `web_search` live review ledger](reviews/m03-web-search-review-01.md)
- [Milestone 03 native `terminal` live review ledger](reviews/m03-terminal-review-01.md)
- [Milestone 03 `permissions` CLI review ledger](reviews/m03-permissions-cli-review-01.md)
- [Milestone 03 `models` CLI delivery review ledger](reviews/m03-models-cli-review-01.md)
- [Milestone 03 `doctor` CLI live review ledger](reviews/m03-doctor-cli-review-01.md)
- [Milestone 03 `sessions` CLI live review ledger](reviews/m03-sessions-cli-review-01.md)
- [Milestone 03 `session` CLI live review ledger](reviews/m03-session-cli-review-01.md)
