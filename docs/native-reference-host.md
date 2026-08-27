# Native reference-host composition

Status: **DELIVERED** through slice-34 `terminal`; slice-35
`ask_user_question` is **CYCLE 10 REJECTED — CYCLE 11 REMEDIATION IN PROGRESS**;
Milestone 03 remains **IN PROGRESS**.
Formal cycle 1 rejected exact candidate
`6c54ec3bf2c23983f14b0a4edeac723321a97900`, tree
`bea90245a559e8e223cc5bb45e0ddfa15e426ee6`, with a deduplicated
0 blocker / 1 high / 3 medium / 3 low union. Cycle-2 evidence
`c77b336`/`0dd1128`, production `9d2e0f2`/`47e9505`, and finding docs compose
at exact `c8718c6`, tree `c27463b`; the complete exact-1.94.1 local gate is
green with 55 focused tests. Formal cycle 2 rejected exact candidate
`910d7bc84cfd7800fb4daf9ab8537bf269027896`, tree
`503a91f334156dbcf2470560b9bb456c3491fd3d`, with a deduplicated `0/0/2/2`
union. Cycle-3 source `cf531d1`/`b7b4358`, evidence `3e3c0c7`/`f3f6f9d`, and
docs `bfdf05b` compose at exact behavior head `8bdc33d96bf88f5986c0e01b3979a2cef0427e82`,
tree `7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`. Its complete exact-1.94.1
local gate is green with 57 focused tests. Formal cycle 3 rejected exact
candidate `746e510c7d8eb93229996e74f91827f489e5bb31`, tree
`c49221efbea66c840b333f0de0161aa686aad52f`, with a deduplicated `0/0/3/2`
union. Cycle-4 core `e569514`/`4c8cff3`, native `53c05cd`/`1857a3f`, and
finding docs `b057958` compose at exact behavior head
`cb93bff35271e6dfc3f4c27ac7a72e621941845c`, tree
`fa402acb75c6d364c41db66f6b55595aa1d0e59a`. Its complete exact-1.94.1 local
gate is green with 171 named focused executions. Formal cycle 4 rejected exact
candidate `42ce6f0ee132a94037c1d99fc19c71c7e0b00bcb`, tree
`b761f7b93d535a1580910f43ff509c40aa07415b`, with track counts `0/0/0/1`,
`0/0/1/1`, and `0/0/1/0`, deduplicated to `0/0/2/1`. Cycle-5 evidence
`ad47fcb`/`bcce292`, source `80382d8`/`e0fd8e0`, and finding docs
`ba53f55`/`b870731` compose at exact behavior head
`b870731d25b81fb0dc643f99084a71d90c3ce7cf`, tree
`0b025f8e42e18006a72d89becf0e395d35c91a57`. Its complete exact-1.94.1 local
gate, including nine all-feature reference-host tests and one host-lifecycle
test, is green. Formal cycle 5 rejected exact candidate
`54b1aab5660e90096b95518bde4ebffb93f28fa6`, tree
`54586d2256c8a3d2289b92bc9bc842eed9ce4d07`: correctness/API and lifecycle/
platform were `0/0/0/0`; performance/resources and the deduplicated union were
`0/0/1/0`. Arbitrary retained activity-Waker clones can independently forward
concurrent blocking downstream callbacks while consuming one prompt slot.
Cycle 6 must add an activity-backed single-flight coalescing notifier, lossless
replay, stale-target close, retained capacity through callback/clone teardown,
and deterministic owned-future many-clone evidence. Finding docs
`7dee269`/`e20023c`, evidence `b007ada`/`4a929c4`, and source
`0488d71`/`707a794` compose at exact head `707a794230758374fa2dab6d65eaf27449c7c477`,
tree `1e60299e21f45079f4e8cf27468a28d1ab4fe227`. The shared notifier now
serializes callbacks, coalesces one lossless replay, closes stale delivery, and
retains capacity through callback/final-clone teardown. The two deterministic
many-clone regressions, 34 direct tests, one engine test, nine host tests, one
host-lifecycle test, and the complete pinned/extended local gate are green.
Formal cycle 6 rejected exact `85058a8aa88fab6912d9313f1ce71e2778cc937f`,
tree `fd3c5072c9473c7fe8767cc2692238eacb8a0f43`: correctness/API `0/0/0/1`,
lifecycle/platform `0/0/1/0`, performance/resources `0/0/1/0`, union
`0/0/1/1`. Reentrant replay can self-rearm indefinitely in one wake. Cycle-7
observation-aware bounded callback delivery, lossless observation-then-notice
replay, deterministic finite-budget evidence, opening-status consistency,
rejection docs `6128f03`/`1d354ff`, evidence `acca13c`/`b75fc54`, and source
`3d48ce8`/`fbb3f5c` compose at exact head `fbb3f5c`/`7cee96e`. Entry clears
state, bind observes, one later notice earns one replay, and close/panic clears
state without lock-held Waker work. Direct 35, host nine/lifecycle one, and the
complete pinned/extended gate were green. Formal cycle 7 rejected exact
`617672984fbb897f2efec63de6a05bb32db9a3db`, tree
`f2cd844449193b46cfa1473ae21edad68664157e`: correctness/API and performance/
resources were `0/0/0/0`, lifecycle/platform was `0/0/1/0`, and the union is
`0/0/1/0`. Replay B is selected before prior target A is dropped, so A's
destructor can panic and wedge `notifying` or reentrantly close/replace the lane
before stale B delivery. Cycle-8 docs `22d5702`/`3650dba`, evidence
`cf4abfd`/`5681bab`, and source `a1b3d23`/`d8075ff` compose at exact head
`d8075ff`/`fa32564`. A is destroyed under `catch_unwind` outside the lock while
the lane/activity remains retained; only then does arbitration select the
current replay. Destructor close/replacement wins, every callback/target-drop
panic clears lane flags, callback panic wins if both occur, and callback
concurrency stays at most one without lock-held foreign work. The rejected-base
B=0 wedge/B=1 stale-delivery tests now recover and suppress respectively while
retaining capacity. Direct 37, host nine/lifecycle one, and the complete pinned/
extended gate were green. Formal cycle 8 rejected exact
`e929b5ea7e3264c2b56066a416bc2a979a03b214`, tree
`cfadc42814688a29c4d512e5fd91c843423821d4`: correctness/API was `0/0/0/0`,
lifecycle/platform and performance/resources were each `0/0/1/0`, and the
union is `0/0/2/0`. A captured secondary panic payload may panic on destruction
and replace the promised primary; separately, synchronous re-poll/re-notify can
execute 257 callbacks for budget 256. Cycle 9 must suppress/forget the
secondary while proving primary marker/lane/capacity/fresh delivery, and cap an
activation at initial plus one replay while retaining residual pending work for
later activation. Cycle-9 docs `2faedc7`/`5296dcc`, evidence
`cf2e220`/`ee25455`, and source `527e10d`/`0279b8c` compose at exact head
`0279b8c`/`50b2423`. Explicit activation is initial plus at most one replay;
residual pending survives; close/panic clears; A drops before arbitration;
concurrency remains one; and capacity stays held. Dual panic forgets the opaque
secondary payload to preserve callback-primary precedence, while a lone target-
drop panic propagates. The marker and residual-work regressions pass with 39
direct tests, and the complete local gate is green. The analogous preexisting
terminal path is outside this bounded slice and is not claimed fixed. Formal
cycle 9 rejected exact `1eeab670a552bc15b5602319b0bb1ce27d2be497`, tree
`5c86e624cf3c0e6d521382c377a9ed9b0500ee5b`: correctness/API and lifecycle/
platform each reported the same `0/0/1/0` medium, performance/resources
reported `0/0/0/0`, and the deduplicated union is `0/0/1/0`. Once the
activation budget is spent, a legal wake after the replay poll can remain
pending without downstream
scheduling until unrelated explicit notify activity; the committed test
manually invokes `retained_wakers[2]`. A last self-wake or cancellation wake can
therefore leave the no-timeout host prompt pending indefinitely. Cycle 10 now
schedules every post-poll wake without unrelated activity while preserving the
existing safety invariants. Cycle-10 docs `216c3b4`/`895c9d4`, final evidence
`74a8497`/`5e46f56`, and source `b043364` compose at exact behavior head
`72e8e75ba2490d4dfa0f680d9dca0b4e10a0401a`, tree
`5405180e5b3b4b59c4d7e712f614bdbc958a9d75`. The `ActivityWake`
`Open`/`DeliveryResourceExhausted`/`Closed` lane advances short chains
autonomously, caps one activation at
exactly 256 callbacks, and converts a continuing chain into cancellation-first
terminal poll or the existing redacted nonretryable prompt-failed error. Close/
panic, A-drop, panic ordering, permit retention, and no foreign Waker work under
the mutex remain; no thread, queue, dependency, or public API is added. Direct
41 and the complete local gate are green. Formal cycle 10 rejected exact
`4ea1c1f5be3586ce9bee696b12c4120dc2a72018`, tree
`78e781ffd7b03aafdf295ae79f4090120971c248`: correctness/API and lifecycle/
platform each reported `0/0/1/0`, performance/resources reported `0/0/0/0`,
and the distinct union is `0/0/2/0`. Queue-only callback return makes each
self-waking `Pending` poll start a new notify-local counter at one, bypassing
the 256 cap. Suppressed/nonselected cleanup-panic payloads can separately
replace the primary or double-panic abort during unwind. Cycle 11 must retain a
prompt-lifetime budget and forget every suppressed payload, proving queue/
cancellation bounds and marker/no-abort/lane/capacity recovery. No cycle-11
source, evidence, or gate is claimed.
No cycle-11 review, integration, or delivery exists yet.
The exact local composition contains sixteen alphabetical tools: thirteen
workspace-backed tools share one original retained descriptor plus twelve
identity-preserving clones, while rootless `web_fetch` and Gateway-backed
`web_search` own no workspace descriptor. Slice 35 inserts rootless
`ask_user_question` first without another workspace
descriptor. Its injected `QuestionPrompter` owns interaction and no CLI UI is
selected implicitly. The delivered terminal Linux system executor remains the
only platform-default process implementation in these two slices.
Public `TerminalTool::open` and `open_with_limits` fail construction off Linux.
Private reference-host descriptor composition deliberately keeps `terminal` in
the non-Linux catalog; after strict preparation and permission, allowed execute
revalidates arguments and returns fixed unsupported before cwd lookup, guardian
or worker creation, or spawn. See [`terminal.md`](terminal.md), the
[`slice-34 ledger`](reviews/m03-terminal-review-01.md), the implemented
[`ask_user_question` contract](ask-user-question.md), and its
[`slice-35 ledger`](reviews/m03-ask-user-question-review-01.md).
Thirty-four bounded Milestone 03 slices are delivered; the local slice-35
composition does not change that count. The earlier slice-27 reviewed seal
`aac9e5f417bec1c00501bad2343955009d7ed96e`, tree
`633ddd44406e22f373962c6a2ec965eae4b9cbdb`, passed exact feature CI
`32874471757`, feature benchmark-evidence `32874471812`, main CI `32875016066`,
and main benchmark-evidence `32875015892`; both benchmark runs retain two
nonexpired exact-SHA artifacts. `main` was fast-forwarded without force from
`a56ff350c2aace1dc22cb14c269aee89d399cd8e`. The retained earlier lineage records
that
twenty-third-slice `rename_file` production and independent evidence are
composed; exact cycle-1 remediation `a3491cf`, tree `0b195bd`, passes the
complete replacement local gate. Tree-identical cycle-2 candidate `4f224a5`,
tree `cb75dca`, is green with zero findings in all three fresh tracks. First
seal `a03a57b` passed exact feature benchmark evidence; feature CI reproduced
an unrelated Linux session-lifecycle test deadlock. Test-only remediation
`2c771ed`, tree `5de94a6`, passes the complete replacement local gate without
changing production. Cycle-3 candidate `5cc1523`, tree `99b88ec`, was not green
because two tracks found an unpinned source-inode reuse race. Remediation
`4cbd46f`, tree `35f531e`, retains the source descriptor and passes the complete
replacement gate. Cycle-4 candidate `1337980`, tree `ab2bdc2`, is green with
zero findings in all three fresh tracks. Seal `7cb5ef9` passed exact feature
and main CI/benchmark delivery gates with two artifacts in each benchmark run.
Native `rename_file` is delivered as slice twenty-three.

The delivered twenty-seventh slice started from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` and pinned fx reference
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Its composition
inserts rootless `WebFetchTool` alphabetically in both production and explicitly
injected/custom construction paths to produce thirteen host tools. It owns no
workspace descriptor, so the descriptor-backed set remains exactly twelve
workspace tools using one original descriptor plus eleven clones. The
tool is non-WASM and cfg-gated by `web-fetch-http`, which is included by
`ai-gateway-http`; it adds no CLI state or command. Production and independent
focused evidence are composed and all seven exact host tests pass. Pre-review
gate record `0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
`5742e4084272120a4531e0d59f0199a5873f39d1`, passed the complete local gate.
Formal cycle 1 is **NOT GREEN** on exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`. The shared rootless thirteen-tool
production/custom composition itself remains unchanged. Its remediation passed the complete
replacement local gate. Formal cycle 2 is **NOT GREEN** on exact candidate
`6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
`6dc095e796b70fa5964e2d9a24163d75667e1c7a`, with 0 blocker, 0 high, 2 medium,
and 2 low deduplicated findings. Exact isolated production remediation
component `6b02c212deaf78da7dc1fd27e5f00f7fb588a50e`, tree
`490f628caa20449c3db96069b34356b0117b7ae4`, retains the rootless host shape and
implements the corrected native boundary below. Exact composed cycle-2
remediation precursor `1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passes the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. This gate record
makes no formal-review outcome, workflow, integration, or delivery claim;
formal candidates are identified only by exact-SHA review results. The full
record is in [`web-fetch.md`](web-fetch.md). Formal cycle 3 is **NOT GREEN** on
exact candidate `16f5afea28ee8a0102377634c2b447364fa3ee32`, tree
`0440e1eda3cad5ba1a4138bbd0808622de285420`. Correctness/API and network/HTTP
lifecycle each reported 0 blocker, 0 high, 1 medium, and 1 low finding;
performance/concurrency reported zero findings at every severity. The
deduplicated union is 0 blocker, 0 high, 2 medium, and 1 low: blocking per-query
entropy inside the total deadline, missing cancellation/deadline authority at
native pre-effect phase transitions, and a duplicated cancellation waiter
reported by both non-green tracks. The exact candidate is rejected. The exact
isolated production remediation component
`9abef298352ea3d9517543c384d9703b949cda75`, tree
`b1ad6f79a9de3414d87c9ff01b28e8c216e6b676`, changes only native
`web_fetch.rs`. It implements the construction-time 32-byte key, `AtomicU32`
and bounded SHA-256 query-ID derivation, carried before/after native-effect
deadline checks, and one cancellation owner. Exact isolated
independent-evidence commit `3da79a08eab706f6dd6cd4b1592eb0ffa97f61c6`, tree
`f690b9b377dedc32776a5fc7b76d4944774b354b`, is based on production and changes
only `web_fetch_http.rs`; its 13/13 focused checks prove exactly one
cancellation wake, a cancelled result, and pending owned-work drop/release
across bounded/raw seams without sleep or network. This remediation record
makes no
replacement-gate, formal-review, workflow,
integration, or delivery claim; formal candidates are identified only by
exact-SHA review results.
Formal cycle 4 is **NOT GREEN** on exact candidate
`af043dc860ab88941df1385543a92c3d9880beed`, tree
`095bac47e4db4001b9010b4f66b46202c620dfaa`. Correctness/API reported
0 blocker, 0 high, 1 medium, and 2 low findings; network/HTTP lifecycle was
green at 0/0/0/0; performance/concurrency reported 0 blocker, 0 high, 1
medium, and 0 low. The deduplicated union is 0 blocker, 0 high, 2 medium, and
2 low, and the exact candidate is rejected. The replacement contract requires
stable first-seen address deduplication before HTTP-client construction and a
configured connect timeout for truncated-DNS TCP replay, subordinate to
cancellation and any earlier overall deadline. It also corrects the maintained
custom-host contract: every production and explicitly injected/custom
composition path contains thirteen alphabetical tools, while exactly twelve
workspace-backed tools use one original retained descriptor plus eleven
clones. Exact isolated production remediation component
`9d793035422cd449c9160c7fccd62221382b5ac5`, tree
`87c48e4a7cf1a7b057adbcef40de5a62d0aa35d6`, changes only native
`web_fetch.rs` and implements the two network corrections. Exact isolated
independent-evidence commit `408e33ec07171988a8f78ee6175adac16532e966`, tree
`6172f1092561fb06316836f1b7f789db038a4a57`, changes only native
`web_fetch_http.rs` and adds a deterministic same-poll authority regression;
it makes no native-DNS proof. Exact composed code/evidence precursor
`d4cebe5f5d1fac00f239a260fa64853ce44cb3b5`, tree
`56a1d73538cf78c5f7c891498deb5bfef9c9e1b0`, contains both. This remediation
record makes no replacement-gate, formal-review outcome, candidate, workflow,
integration, or delivery claim; formal reviewer reports identify the exact
candidate they reviewed.
Formal cycle 5 is **NOT GREEN** on exact candidate
`81b963ad5a2033fb2295f7325a28fba6b66197d5`, tree
`f5ede2e70637f5cd8ab373c9dfc893189dd5775c`. Correctness/API reported
0 blocker, 0 high, 0 medium, and 1 low finding; network/HTTP lifecycle reported
0 blocker, 0 high, 1 medium, and 0 low; performance/concurrency reported
0 blocker, 0 high, 0 medium, and 1 low. The repeated timer-accounting low
deduplicates across correctness and performance, yielding 0 blocker, 0 high,
1 medium, and 1 low. The exact candidate is rejected. The medium finding is a
same-poll native DNS TCP-connect deadline defect; it changes neither reference-
host construction nor the thirteen-tool production/custom composition shape.
Exact isolated source remediation
`cde7d2ab2498375672c1ec6e124aff04a4020f26`, tree
`8e8cd69524b4a88f2cc3262ef6d6b2dadc4d1d64`, changes only native
`web_fetch.rs`; exact composed code precursor
`d4554a9e14b93a90b3e4f1ae58f210cb2ceb5be7` has the same tree. Reference-host
composition is byte-unchanged. This remediation record makes no replacement-
gate, formal-review outcome, candidate, workflow, integration, or delivery
claim.
The delivered count remains twenty-six and this is not a performance or
fx-equivalence claim.

Reference-host construction supplies no runtime and performs no web request.
Later production `web_fetch` polling requires a current host-owned Tokio
runtime with I/O and time enabled. No current handle produces fixed
`RuntimeRequired`; a current driverless runtime violates the documented
`# Panics` precondition and may terminate a release process.
The host supplies no resolver or entropy override. Native transport
construction synchronously snapshots the host's first UDP-configured system
nameserver and one random query-ID seed outside invocation timing. Each
admitted hostname invocation uses that nameserver and an atomic per-query
sequence rooted in the stored seed for bounded direct DNS socket work; it does
not reread resolver configuration or perform per-query entropy. A failed
nameserver or seed snapshot makes later hostname execution return the same
fixed, retryable unavailable result until a new transport is constructed.
Literal public IPs skip DNS and bypass both prerequisites. One outer
cancellation waiter and one outer machine-god invocation-deadline sleep are
reused for bounded permit, DNS, HTTP, and body waits. Each truncated A or AAAA
DNS TCP replay may additionally own one short-lived configured connect-timeout
sleep, for at most two sequential DNS replay sleeps per invocation;
Reqwest/Hyper may own bounded HTTP connection-attempt timers. The outer sleep
is allocated once; each DNS replay sleep is allocated once when that replay
begins. None resets or extends the outer absolute deadline. The native
transport checks cancellation and the same absolute deadline at pre-effect
boundaries between A, AAAA, TCP replay, HTTP dispatch, and body work.
The final synchronous boundary checks the token/deadline directly without a
second waiter. No workspace descriptor is used
for resolver configuration, entropy, or network execution.
The twenty-fourth, library-only `copy_file` slice is delivered. Cycle-3
candidate `99ecdb3`, tree `145b3be`, is green with zero findings in all three
fresh tracks. Seal `3bdd7cb` passed exact feature CI `32684856309`, feature
benchmark `32684856373`, main CI `32685192453`, and main benchmark
`32685192394`; each benchmark run retains exactly two nonexpired exact-SHA
artifacts.
The twenty-fifth `create_folder` behavior is composed from exact delivered base
`d1a5bc24112bcede8c2d12789e763a12cf44bd4a`. Exact frozen contract commit
`9fab189c9c1add76a38775d08f4342c6bcc7635b` passed all six jobs of CI
`32687614476`; benchmark workflow `32687614442` passed both jobs and retains
exactly two nonexpired exact-SHA artifacts. Candidate source composes eleven
alphabetical tools through the original retained descriptor plus ten clones.
Seven exact Rust 1.94.1 reference-host tests are green as part of the current
17-private/20-direct/6-engine/7-host/1-core-contract focused evidence. Cycle-2
candidate `6e1f885`, tree `ac57575`, is historically not green: correctness/API
and performance/concurrency are green with zero findings, while filesystem/
robustness reported two low evidence/documentation findings and zero production
defects. Exact remediation `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate, including the deterministic mixed-device private test and both
eleven-tool host constructors. Documentation record `9d0bacd`, tree `b5fb1c2`,
and tree-identical cycle-3 candidate `c1e572e` preserve that non-documentation
behavior. Cycle 3 is not green only for one low documentation-lineage finding;
filesystem and performance are green, and all three tracks found zero
production defects. Exact lineage remediation `12c11ba`, tree `b96575b`, passes
the complete replacement gate. Gate record `f6f6584` parents tree-identical
cycle-4 candidate `a78b693`, tree `2b913e8`. Correctness and performance are
green with zero findings; filesystem found zero production defects and one low
stale seal sentence, corrected under the user's seal-review exemption. Delivery
and `main` integration remained pending at that checkpoint. First feature CI
`32699750602` has all
four native Linux/macOS jobs green but is not green because Linux Quality
rejected a test-only mode conversion. Platform-native `RawMode` remediation is
composed at exact `1effcbb`, tree `b5eccb1`, and passes the complete replacement
gate. Tree-identical cycle-5 candidate `ff18a9a`, tree `f77b198`, is green with
zero findings in all three fresh tracks. Feature benchmark `32699750662` is
green with exactly two nonexpired exact-SHA
artifacts. Seal `e75578b` passed exact feature CI `32702785549`, feature
benchmark `32702785574`, main CI `32703303933`, and main benchmark
`32703303931`; both benchmark runs retain exactly two nonexpired exact-SHA
artifacts. The `create_folder` checkpoint host has exactly eleven tools.
Current execution
evidence includes all four native Linux/macOS jobs from feature CI
`32699750602`; cross-target Linux/FreeBSD test compilation and warnings-denied
Linux test-target Clippy are also green.

Historical delivery lineage: integrated contract for the twelfth bounded
Milestone 03 library slice,
with its workspace-tool composition extended by the delivered seventeenth and
eighteenth slices. Eighteen slices are delivered. The nineteenth bounded
`grep_files` candidate is **IN PROGRESS** from exact base
`f6aa458bb875d6cb26565adc878703fe140916d3`; its tree-identical integration
kickoff is `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`. Its production, independent
tests, and maintained documentation are parallel, non-overlapping components;
exact production `27eec2f` and initial test `6eaee93` components exist and
initially compose through `9057feb` and `44e33d7`. Reference-host fixture fix
`bdbb677` makes focused production/test composition green. Documentation
component `b04151a` produces fully composed behavior `42e4793`; lint fix and
exact local gates are green at `45ad91f`. All three first-cycle tracks are
**NOT GREEN** on exact `355a11a`. Remediation and exact replacement local gates
are green at final code/test precursor `275d263`. First replacement candidate
`ae87bf1` remains historically **NOT GREEN** across all three tracks. Second-fix
production and documentation compose through `ac5d772`, `d672210`, `7ad0863`,
and exact local-gate-green precursor `b498ba0`. Formal second replacement
candidate `5aeddc1` has correctness/API and filesystem/robustness **GREEN** with
zero findings and performance/concurrency **NOT GREEN** with one medium
allocation-amplification finding and two low documentation/evidence findings.
Third production remediation `8777825` composes at `ab1c133`; independent
regression `dcf57ad` composes at `d7526d4`; review-findings documentation
`44afb23` composes at `f08c5f2`; lint follow-up `1f13f9a` produces exact fully
composed local-gate precursor `a8f6179`. Exact Rust 1.94.1 formatting,
warnings-denied workspace Clippy, 598 non-documentation tests plus two doctests,
25 private native tests, 40 direct `grep_files` tests, four engine tests, and
diff checks are green. Exact a8f cross-target/dependency/link validation is
green; compatibility/release validation is green. Formal third-cycle candidate
`0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
**GREEN** with zero findings. Correctness/API and performance/concurrency are
**NOT GREEN** only for the same LOW documentation contract mismatch; reviewers
confirmed zero production defects. Isolated wording remediation
`993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
`f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
green, and its behavior tree remains `a8f6179` except for documentation. Formal
fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero findings
in all three fresh tracks: correctness/API, filesystem/robustness, and
performance/concurrency. Exact-SHA formatting, warnings-denied workspace
Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests, 25 private
tests, 40 direct `grep_files` tests, four engine tests, and the 58/420/270/0
documentation inventory are green. All historical findings are closed,
including the attempted-read-window storage wording. This documentation-only
seal is exempt from another adversarial review under the user's explicit
instruction. Documentation seal `0f48806310882caf3c668c72fe1b9d211cae744b`
is feature-green: CI run `32623585346` passed all six jobs and benchmark-
evidence run `32623585349` passed both jobs and artifacts, all for exact `0f`.
`main` was fast-forwarded without force from `f6ab594` to exact `0f`. Main CI
run `32623904784` is **GREEN** for exact `0f`: all six jobs and every step
passed without reruns. Main benchmark-evidence run `32623904800` is **GREEN**
on attempt 1 for exact `0f`: both jobs and every step passed, with two valid
non-expired exact-SHA artifacts retained. The `grep_files` slice is delivered;
the remaining native tools remain pending.
This final delivery record is documentation-only and exempt from adversarial
review; its own exact remote workflows are required after push and cannot be
self-recorded.
Eighteenth-slice
production, independent tests, documentation, and
composition are present. Initial local gates were green at `60070d8`, but the
first formal review at `1f5de6a` found a high matcher-work bound defect. Its fix,
regression, and replacement local gates are green at `4171a4a`; all three same-
SHA replacement reviews are green at `523df858`. Documentation seal and
integrated `main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed
exact feature CI `32610950593`, feature benchmark evidence `32610950594`, main
CI `32611208411`, and main benchmark evidence `32611208415`. The slice started
from base `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`. Seventeenth-slice
production `5c2d129` and independent tests `ca0091c` compose at `f228c06`,
where all 34 focused tests are green. Review hardening composes at `b69ec4b`,
bringing the focused total to 36 plus five private unit tests. The first formal
candidate was not green; three
replacement tracks are green on exact candidate `4193ecc`. Documentation seal
and integrated `main` SHA `60dd54f273afc7e62fb4b3cc1fb1a347d739998b`
passed exact feature CI run `32605071080` on successful retry attempt 2, feature
benchmark-evidence run `32605071063`, main CI run `32606050292`, and main
benchmark-evidence run `32606050294`; all four workflows report that exact seal
SHA. Benchmark success is delivery evidence only and makes no
product-performance claim. This documentation-only commit is the final delivery
record, is explicitly exempt from another adversarial review after behavior was
already green, and reports its own exact workflows at handoff.
The first formal sixteenth candidate is composed
through `dec98e0`, whose three review tracks were not green. Its replacement
source and test fixes are composed in exact behavior candidate
`3fa54635dab00ebba78b233c69fd39e04e9be57e`; all three replacement tracks are
green. First remote CI `32599591900` exposed the Linux removed-root gap; this
portable fix is exact behavior candidate `17f1884`, green under both executable
review tracks. Documentation seal `d3312d7` resolves its lineage finding,
passed exact feature CI `32600292770` and benchmark evidence `32600292779`, was
fast-forwarded without force to `main`, and passed exact main CI `32600567094`
and benchmark evidence `32600567090`.
This slice's production implementation, an
independently owned seven-test black-box suite, three fresh adversarial tracks,
and exact feature and `main` workflows are green. Its final delivery record is
integrated on `main` at `ac3984fb16dbab3adf86a949c7555ceca7c3e8df`; exact feature CI run
`32579779134`, feature benchmark-evidence run `32579779123`, main CI run
`32580066474`, and main benchmark-evidence run `32580066485` are green.
The separate schema-v3 configured credential-source extension is a thirteenth
slice whose production implementation, independent tests, local gates, and
all three fresh adversarial tracks are green on exact behavior SHA
`35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. Its final delivery record is
integrated on `main` at `f840576af241c58d1e55399e66ba92f7770cd50c`; exact
final-record feature CI run `32583585145`, feature benchmark-evidence run
`32583585148`, main CI run `32583871385`, and main benchmark-evidence run
`32583871368` are green. The integrated fourteenth slice adds safe
selected-root preparation and consuming constructors without changing the
integrated path-constructor contract below. Its production and 16 independently
owned focused tests are present and their focused gates are green. Initial
formal review found fixture-mode and macOS ACL issues; their fixes bring the
focused total to 16, including ALLOW-rejection and ordinary-HOME
DENY-compatibility regressions. All three tracks were green on exact behavior
SHA `f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`. The first seal exposed only
Linux strict-Clippy portability diagnostics; their source normalization is
present at `90d8f96`, with local macOS and Linux cross-target gates green. All
three final tracks are green on exact candidate `72cf64f6`. Replacement seal
`f08dbd9e` is green under exact feature CI `32589778343` and benchmark evidence
`32589778374`. Feature record `6f66b6e5` is green under exact feature CI
`32590128235` and benchmark evidence `32590128233`, and is green on `main` under
CI `32590429626` and benchmark evidence `32590429592`. This documentation-only
commit is the final delivery record. Milestone 03 remains `IN PROGRESS`. Full
lineage is recorded in the
[`native reference-host review`](reviews/m03-native-reference-host-review-01.md).

The `create_folder` candidate extends `NativeReferenceHost`, which composes the
existing validated native configuration,
AI Gateway provider and transport boundary, file session store, ask permission
adapter, with an eleventh confined workspace tool in one provider-neutral
`Engine`. Candidate membership is `copy_file`, `create_folder`, `delete_file`,
`edit_file`, `file_info`, `glob_files`, `grep_files`, `list_files`, `read_file`,
`rename_file`, and `write_file`; core exposes that catalog in deterministic
alphabetical order. The last delivered membership omits `create_folder`.
It is a library surface in
`machine-god-native`. The `machine-god-cli` crate and every existing CLI output
byte remain unchanged.

The composed `create_folder` behavior registers exactly eleven alphabetical
tools by inserting `create_folder` immediately after `copy_file`. Both
reference-host constructors transfer the same retained workspace identity
through one original descriptor plus ten identity-preserving clones. Cycle 2
was not green on two low evidence/documentation findings. Exact remediation
`f527293`, tree `40eef14`, passes the complete replacement local gate for the
17-private/20-direct/6-engine/7-host/1-core-contract inventory. Cycle-3
candidate `c1e572e`, tree `b5fb1c2`, is not green only for one low lineage-
record finding. Exact lineage remediation `12c11ba`, tree `b96575b`, passes the
complete replacement gate. Cycle-4 candidate `a78b693`, tree `2b913e8`, has
zero production findings; its sole low stale seal-record finding is corrected
in the exempt documentation seal. First feature CI `32699750602` then exposed
one Linux-only test Clippy failure; platform-native `RawMode` remediation is
composed at `1effcbb`, tree `b5eccb1`, and passes the complete replacement gate;
tree-identical cycle-5 candidate `ff18a9a`, tree `f77b198`, is green with zero
findings in all three fresh tracks. It is not the delivered or `main`-
integrated composition at that review checkpoint. Seal `e75578b` subsequently
passed exact feature/main CI and benchmark workflows, so this is now the
delivered composition.

The delivered twenty-sixth composition
inserts `open_file` immediately after `list_files`. One additional identity-
preserving descriptor clone lets exactly twelve alphabetical tools share the
same retained workspace identity through one original descriptor plus eleven
clones. `open_file` uses
dedicated `Capability::OpenFile { path }`, retains an approved existing regular-
file descriptor without following symlinks, and on Linux launches fixed
`/usr/bin/xdg-open` with
`/proc/<machine-god-parent-pid>/fd/<retained-fd>` as its sole argument. The
helper runs from `/` with null stdio and the trusted host environment; machine-
god performs no `PATH` lookup and accepts no model-selected process settings. A
trusted injected launcher seam is deterministic for tests and inert until
polled. Public construction is Linux-only; macOS public construction is
unsupported, its private retained-root host tool returns unsupported at
execution, and every other target is unsupported.

Exactly 32 global permits bound production system-launch workers; saturation is
retryable precommit unavailable with zero new worker/helper, and a permit stays
held through arbitrary Waker completion and worker return. The worker is
established before helper spawn. Worker-start or spawn failure is a retryable
precommit unavailable result. The final spawn and cancellation/drop
abort transitions share one serialized gate: abort-first guarantees no launch,
while successful spawn commits the effect. Postcommit cancellation, timeout, or
explicit future/drop cleanup terminates and reaps the direct helper without
claiming rollback. Before publication, cleanup suppresses waking, reaps the
helper, drops request/descriptor ownership, and synchronously joins. Normal
published cleanup joins. Inline or blocking arbitrary-Waker overlap may release
the handle to avoid self-join/cross-thread deadlock; after helper/request
cleanup, only callback/final bookkeeping remains and its permit bounds it. This
narrow docs-only amendment replaces the frozen absolute no-worker-detach clause
because it contradicted legal Waker behavior and is exempt from its own review.
Postcommit cancellation, nonzero or
signalled exit, timeout, or wait failure returns fixed redacted, nonretryable
result uncertainty when a tool-level result is observed; there is no postspawn
waiter-setup state. Exit zero establishes helper
acceptance only, not downstream consumption or display. Success is exactly
`{"path":"canonical/relative/path"}`. Formal cycle 3 rejected exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`, for two low findings: the
authoritative post-`try_wait` clock branch was not truly tested, and ordinary
no-Waker publication could detach a permit-bounded worker tail instead of
joining it. Exact cycle-4 candidate
`4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`, remediates both and passes the
complete replacement local gate. Cycle 4 is rejected because correctness/API
found one low maintained-documentation lineage drift; filesystem/process-
lifecycle and performance/concurrency are green with zero findings.
That remediation was composed into cycle-5 candidate
`4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. All three tracks found zero
production defects and the same low remaining current-lineage wording defect.
That correction is composed in exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh correctness/API,
filesystem/process-lifecycle, and performance/concurrency tracks are green with
zero findings at every severity. Exact feature workflows, delivery, and exact
`main` workflows are complete. Seal and integrated `main` SHA
`a02c28a6bc39f2981586f02cb76793c430c83a20`, tree
`03c751cffacee4808b057079dedb02cfc3f193cc`, passed feature CI `32738160229`
at 6/6 and feature benchmark `32738160725` at 2/2, retaining upstream artifact
`9524219365` and bootstrap artifact `9524052760`. Exact main CI `32738798417`
passed 6/6, and main benchmark `32738798415` passed 2/2 while retaining
upstream artifact `9524461989` and bootstrap artifact `9524298408`. The current
host has exactly twelve alphabetical tools using one retained descriptor plus
eleven identity-preserving clones. This makes no product-performance or fx-
equivalence claim. At that checkpoint, the final docs-only record was exempt
from adversarial review under the user's instruction; its own exact feature and
`main` workflows remained required and are reported below.

Final delivery-record SHA
`762d70df106d40e59b599e18b1ac5c62f678927d`, tree
`909eb320e05df4d56f5bcecf0e3655e6d761f622`, passed feature CI `32740668405`
at 6/6 and feature benchmark `32740667465` at 2/2, retaining upstream artifact
`9525188220` and bootstrap artifact `9525017236`. Main benchmark `32741322179`
passed 2/2 and retained upstream artifact `9525436660` and bootstrap artifact
`9525268460`. Main CI `32741322249` was not green: five of six jobs passed,
and Quality alone failed the exact test named by concatenating
`blocked_wake_releases_request_before_publication_and_holds_` with
`permit_until_worker_return` because the immediate `exit 0` fixture could
legitimately publish before the first poll installed `BlockingWake`. This is a
test-fixture synchronization defect, not a production behavior finding. Exact
local test-only remediation
`62c2a5349bc682079c2458ccebe9f9ea9578a3c1`, tree
`b38984441b6bb470ecb4b1c69bc9a3a9984f0bb0`, adds the existing
`before_first_wait` barrier and passed the normal native Linux arm64 exact test
100/100. Exact cycle-7 candidate
`ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
`f8a681db319f0a89e21f38e7f9f8c474c270452b`, received **GREEN** correctness/API
and filesystem/process-lifecycle reviews with zero findings. Performance/
concurrency was **NOT GREEN** with exactly two low findings and zero blocker,
high, or medium findings: the unconditional `before_first_wait` rendezvous
could hang when `Command::spawn` failed before the hook, as reproduced on
native Linux arm64 with `/tmp` mounted `noexec`; and maintained current/
operative documentation tails ended at the superseded cycle-6/handoff state.
The candidate is rejected. Exact test-only code remediation
`274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`, tree
`865e93423719cdb5655cb7dd22fd20f207717cbb`, changes the fixture to the existing
`before_spawn` barrier, reached before every spawn outcome, so Waker
registration deterministically precedes publication. The normal native Linux
arm64 exact test passed 100/100, and the `/tmp`-noexec spawn-failure case passed
1/1. Production source, public API, and manifests are unchanged. This docs
correction composes atop that commit; its SHA is pending. The full replacement
local gate and all three fresh cycle-8 tracks remain pending. This executable
test-only fix is not eligible for the documentation-only exemption. This makes
no product-performance or fx-equivalence claim.

Exact documentation-correction and cycle-8 candidate
`6cfc17407cb6fa05d7568cd4f074775fc76c0e25`, tree
`44aa7c2636f341e8d759ef18626d0565a5a7d05e`, passed the complete replacement
local gate. Cycle-8 correctness/API was **NOT GREEN** with exactly two low
findings and zero blocker, high, or medium; lifecycle and
performance/concurrency were green with zero findings. The first low found no
successful-helper witness in the normal blocked-Waker fixture, whose remaining
assertions also passed after a noexec spawn failure. The second found that the
operative documentation did not identify the known cycle-8 candidate. Exact
test-only remediation `a8415f2ac79bea979d27651174d21065c6c5d5d7`, tree
`7210b0a0bd719e8373a7bf15bfc7084d7eff0199`, factors the shared lifecycle
assertions, makes the successful helper write and verify an exact marker, and
adds a separate deterministic missing-helper spawn-failure case. Normal Linux
arm64 focused evidence is 202/202; the failure case under `/tmp` noexec is
100/100. Production source, public API, manifests, and workflows remain
unchanged. The documentation correction and exact cycle-9 candidate SHA/tree,
complete replacement local gate, and three fresh cycle-9 reviews remain
pending. This makes no product-performance or fx-equivalence claim.

Exact cycle-9 reviewed candidate
`964c59408bda1a3793978041432b84b808b474a6`, tree
`7e5306ad77ece822b4f0080c4d6a24f142635e04`, passes the complete replacement
local gate. All three fresh correctness/API, filesystem/process-lifecycle, and
performance/concurrency reviews are **GREEN** with zero blocker, high, medium,
or low findings. Rust 1.94.1 evidence includes Linux arm64 system 15/15, direct
12/12, engine 4/4, normal split cases 202/202, noexec failure 100/100,
warnings-denied Clippy, the full Rust workspace, two doctests, Python 130 with
eight expected macOS skips, pinned-fx compatibility, dependency policy/audit,
FreeBSD/WASI plus active Node 1/1, documentation checks, and five release CLI
smokes. The 319,152-byte release binary retains SHA-256
`4526cbab38ef595a40d30938579e30760e148d9b83241e8b12a7d3325dadfbda`.
Production source, public API, manifests, and workflows remain unchanged. The
subsequent documentation-only green seal names this reviewed candidate and is
exempt from another adversarial cycle under the user's instruction; its exact
SHA/tree and feature/main workflows remain pending. These are regression and
delivery checks, not a product-performance or fx-equivalence claim.

The delivered fifteenth slice adds a `NativeSessionLifecycle` owned by
this wrapper. It supplies durable by-ID create, resume, replay, and reset over
the exact file-store instance shared with the engine; the caller still supplies
the validated session ID and production native code supplies OS-random
incarnations. Production, fourteen independently owned focused tests, one
formal finding regression, and all three adversarial tracks are green on exact
candidate `e6a3804`. Feature record `dbba2c7` is green on the feature branch and
`main` under exact CI and benchmark workflows. Its normative behavior is in
[`native-session-lifecycle.md`](native-session-lifecycle.md).

The delivered sixteenth slice adds bounded IDs-only listing through that
same retained lifecycle and store. It returns at most 100 sorted unique IDs plus
`truncated`, processes/selects at most 1,024 non-dot entries plus one fetched
and name-inspected overflow witness, and accepts/decodes at most 64 MiB of
aggregate canonical record bytes plus one transient transfer byte to detect
concurrent growth. It adds no CLI behavior. Production and 13 initial
independent tests are composed; all three first formal tracks were not green.
The fixes are composed into exact behavior candidate `3fa5463`, with 18 focused
tests and all three replacement review tracks green. Portable behavior
`17f1884` and seal `d3312d7` passed exact feature and `main` delivery gates. Its
normative behavior is in
[`native-session-listing.md`](native-session-listing.md).

The delivered seventeenth slice adds `file_info` beside integrated `list_files` and
`read_file`. It prepares a distinct `FilesystemAccess::Metadata` capability and exact
normalized path, walks ancestors descriptor-relatively without following
symlinks, and inspects the final object with no-follow metadata without opening
it. Its bounded structured result and fixed redacted behavior are normative in
[`file-info.md`](file-info.md). It changes no constructor arguments, provider,
transport, permission, session, runtime, credential, or CLI authority. The
slice's production and independent tests compose at `f228c06`, where all 34
initial focused tests are green. Review hardening composes at `b69ec4b`, bringing
the focused total to 36 plus five private unit tests. Three replacement tracks
are green on exact candidate `4193ecc`; seal and integrated SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` is green under feature CI
`32605071080` on successful retry attempt 2, feature benchmark evidence
`32605071063`, main CI `32606050292`, and main benchmark evidence `32606050294`.
All four report the exact seal SHA.

The delivered eighteenth slice adds `glob_files` beside those three delivered tools.
It prepares the distinct `FilesystemAccess::EnumerateRecursive` capability at
the normalized selected subtree and exact explicit pattern/path/mode arguments.
Allowed execution performs a complete bounded descriptor-relative no-follow
scan and returns either the globally bytewise-smallest sorted match prefix or an
exact count. Its normative contract is [`glob-files.md`](glob-files.md). It
changes no constructor arguments, provider, transport, permission handler,
session, runtime, credential, root-selection, or CLI authority. Exact composed,
review, and delivery lineage is recorded in the
[`review record`](reviews/m03-glob-files-review-01.md).

The nineteenth candidate adds `grep_files` beside those four delivered tools.
It prepares the distinct `FilesystemAccess::SearchContent` capability at the
normalized selected file or subtree, performs a complete bounded literal
search through descriptor-relative no-follow regular-file-only traversal, and
reuses one scan-local, 8 KiB-window content buffer whose logical file view resets
between files and whose high-water storage cannot exceed 204,801 bytes. It
returns exact eligible-text statistics with bounded structured match, file, or
count results. Its frozen candidate contract is
[`grep-files.md`](grep-files.md); its green local evidence and remaining formal-
review and delivery gates are in the
[`review record`](reviews/m03-grep-files-review-01.md). It changes no constructor
arguments, provider, transport, permission handler, session, runtime,
credential, root-selection, or CLI authority.

## Slice-33 web-search composition

The frozen slice-33 extension reuses the production host's already validated
model, bearer-backed `Arc<dyn AiGatewayTransport>`, and canonical Gateway
network target to construct one `AiGatewayWebSearchTransport`. It registers a
local `WebSearchTool` over that injected transport in both production and
explicit custom-transport paths. Synchronous host construction makes no worker
request, polls no prompt, and starts no task, timer, DNS, or HTTP work.

The resulting candidate catalog is `copy_file`, `create_folder`, `delete_file`,
`edit_file`, `file_info`, `glob_files`, `grep_files`, `list_files`, `open_file`,
`read_file`, `rename_file`, `web_fetch`, `web_search`, `write_file`. Exactly
twelve tools retain the original workspace descriptor plus eleven identity-
preserving clones; neither network tool owns a workspace descriptor.
`web_search` preparation supplies core with the exact configured Gateway
`NetworkTarget`, so the existing `AskPermissionHandler` resolves policy before
the one private worker request can begin.

The production adapter is present only under the existing non-WASM
`ai-gateway-http` composition gate, and this reference host remains Linux/macOS-
only. It adds no constructor environment lookup, configuration field,
credential source, runtime, CLI ownership, provider-neutral core event, retry,
fallback, live-provider test, or performance/equivalence claim. The production,
independent-evidence, and documentation components are composed in the
slice-33 candidate and remain subject to its exact local gate, fresh adversarial
product review, and delivery workflow.

## Feature and platform boundary

The integrated composition API is exported only under this gate:

```text
all(
  feature = "ai-gateway-http",
  not(target_family = "wasm"),
  any(target_os = "linux", target_os = "macos")
)
```

The twelfth slice therefore supports Linux and macOS only, requires the
optional `ai-gateway-http` feature, and has no WebAssembly export. This gate
applies to both the production-HTTP and custom-transport constructors because
the composed host always contains the Linux/macOS descriptor-rooted tools and
file session store. It does not broaden the existing standalone portability of
core, the AI Gateway codec, custom transports, or the ask adapter.

The fifteenth slice's `NativeReferenceHost` integration retains this exact
gate. The standalone lifecycle API is separately exported on Linux and macOS
without requiring `ai-gateway-http`; it does not make the standalone core
engine or session-store trait depend on OS randomness, a native filesystem,
the HTTP feature, or a runtime.

The delivered sixteenth slice's standalone `list_sessions` method uses that same
Linux/macOS lifecycle gate without requiring `ai-gateway-http`. Listing through
the composed wrapper inherits this page's stricter gate.

The delivered standalone `FileInfoTool` is likewise supported on Linux and
macOS without requiring `ai-gateway-http`; other targets receive its fixed
unsupported-platform construction failure. Registration through
`NativeReferenceHost` inherits the composition gate above.

Delivered standalone `GlobFilesTool` has the same Linux/macOS platform boundary
without requiring `ai-gateway-http`; other targets receive its fixed
unsupported-platform construction failure. Registration through
`NativeReferenceHost` inherits the composition gate above.

Delivered standalone `GrepFilesTool` has the same Linux/macOS platform boundary
without requiring `ai-gateway-http`; other targets receive its fixed
unsupported-platform construction failure. Registration through
`NativeReferenceHost` inherits the composition gate above.

In-progress standalone `RenameFileTool` has the same Linux/macOS platform
boundary without requiring `ai-gateway-http`; other targets receive its fixed
unsupported-platform construction failure. Registration through
`NativeReferenceHost` inherits the composition gate above.

The public composition and observation surface is:

```rust,ignore
NativeReferenceHost::compose_ai_gateway_http(
    loaded_config: LoadedNativeConfig,
    credential_environment: AiGatewayCredentialEnvironment,
    workspace_root: &Path,
    session_root: &Path,
    permission_prompter: Arc<dyn PermissionPrompter>,
    question_prompter: Arc<dyn QuestionPrompter>,
    web_search_deadline: Arc<dyn WebSearchDeadline>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError>

NativeReferenceHost::compose_with_ai_gateway_transport(
    loaded_config: LoadedNativeConfig,
    transport: Arc<dyn AiGatewayTransport>,
    network_target: NetworkTarget,
    workspace_root: &Path,
    session_root: &Path,
    permission_prompter: Arc<dyn PermissionPrompter>,
    question_prompter: Arc<dyn QuestionPrompter>,
    web_search_deadline: Arc<dyn WebSearchDeadline>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError>

NativeReferenceHost::engine(&self) -> &Engine
NativeReferenceHost::into_engine(self) -> Engine
NativeReferenceHost::loaded_config(&self) -> &LoadedNativeConfig
NativeReferenceHost::credential_source(&self)
    -> Option<AiGatewayCredentialSource>
NativeReferenceHost::session_store(&self) -> &Arc<FileSessionStore>
NativeReferenceHost::session_lifecycle(&self) -> &NativeSessionLifecycle
```

The integrated fourteenth slice adds these consuming constructors:

```rust,ignore
NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots(
    loaded_config: LoadedNativeConfig,
    credential_environment: AiGatewayCredentialEnvironment,
    prepared_roots: PreparedNativeRoots,
    permission_prompter: Arc<dyn PermissionPrompter>,
    question_prompter: Arc<dyn QuestionPrompter>,
    web_search_deadline: Arc<dyn WebSearchDeadline>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError>

NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots(
    loaded_config: LoadedNativeConfig,
    transport: Arc<dyn AiGatewayTransport>,
    network_target: NetworkTarget,
    prepared_roots: PreparedNativeRoots,
    permission_prompter: Arc<dyn PermissionPrompter>,
    question_prompter: Arc<dyn QuestionPrompter>,
    web_search_deadline: Arc<dyn WebSearchDeadline>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError>
```

In all four current signatures the explicit `question_prompter` follows the
permission prompter and precedes the web-search deadline. The two prepared-root
methods consume retained roots prepared under the separate
[`native root-selection contract`](native-root-selection.md), rather than
reopening path arguments. Historical green behavior SHA `f1dc4751`, Linux lint
normalization `90d8f96`, candidate `72cf64f6`, replacement seal `f08dbd9e`, and
feature record `6f66b6e5` cover the pre-slice-35 root-selection work only. They
do not review or deliver the later `question_prompter` parameter or sixteen-tool
catalog. Slice-35 formal cycles 2, 3, and 4 are rejected; the exact cycle-4
candidate and findings are recorded above. Cycle-5 source and deterministic
race evidence are composed at `b870731d25b81fb0dc643f99084a71d90c3ce7cf`,
tree `0b025f8e42e18006a72d89becf0e395d35c91a57`, and the complete local gate is
green. Formal cycle 5 rejected exact `54b1aab`/`54586d2` with a deduplicated
`0/0/1/0` product resource/capacity finding. Cycle-6 docs, evidence, and source
compose at `707a794`/`1e60299`; both deterministic many-clone tests and the
complete local gate are green. Formal cycle 6 rejected exact
`85058a8`/`fd3c507` with a `0/0/1/1` union. Cycle-7 bounded replay, status
consistency, source, evidence, and complete local gate compose at
`fbb3f5c`/`7cee96e`. Formal cycle 7 rejected `6176729`/`f2cd844` with a
`0/0/1/0` union. Cycle-8 destruction-before-selection remediation, evidence,
and complete local gate compose at `d8075ff`/`fa32564`. Fresh reviews remain
pending, and formal cycle 8 rejected `e929b5e`/`cfadc42` with a `0/0/2/0`
union. Cycle-9 panic-primary and bounded-activation remediation/evidence plus
the complete local gate compose at `0279b8c`/`50b2423`. Formal cycle 9 rejected
`1eeab67`/`5c86e62` with a deduplicated `0/0/1/0` liveness medium; cycle-10
source/evidence and complete local gate now compose at
`72e8e75`/`5405180`. Formal cycle 10 rejected `4ea1c1f`/`78e781f` with two
distinct mediums and a `0/0/2/0` union; cycle-11 work does not exist. The
analogous preexisting terminal code is outside this slice.

Both integrated path constructors consume an already validated
`LoadedNativeConfig`; neither loads configuration nor reads the process
environment. The accepted selection is exactly permission mode `ask`, provider
`vercel_ai_gateway`, and transport `ai_gateway_http`. The thirteenth slice
additionally requires configured
`NativeCredentialSourceKind::Environment`. Any other future or otherwise
unsupported selection fails
closed before a root, credential, transport, provider, or engine is opened or
constructed.

## Exact production composition

`compose_ai_gateway_http` performs these synchronous construction stages in
this order:

1. validate the loaded permission, provider, transport, and credential-source
   selections;
2. open the existing absolute workspace once and retain that directory
   identity for the thirteen workspace-backed candidate tools;
3. open the existing absolute session root as `FileSessionStore`;
4. consume the injected `AiGatewayCredentialEnvironment` and discover one
   validated bearer token under its existing precedence rules;
5. move that bearer token into production `AiGatewayHttpTransport`;
6. construct descriptor-backed `TerminalTool`;
7. construct the dedicated web-search transport and `WebSearchTool` with the
   loaded configuration's projected model, canonical fixed production target,
   and explicit deadline authority;
8. construct `AiGatewayProvider`;
9. construct rootless `WebFetchTool`;
10. wrap the injected permission prompter in `AskPermissionHandler` and retain
   the separately injected question prompter in rootless
   `AskUserQuestionTool`; and
11. build `Engine` with exactly `ask_user_question`, `copy_file`,
   `create_folder`, `delete_file`,
   `edit_file`, `file_info`, `glob_files`, `grep_files`, `list_files`,
   `open_file`, `read_file`, `rename_file`, workspace-backed `terminal`,
   rootless `web_fetch`, Gateway-backed rootless `web_search`, and `write_file`,
   default `EngineLimits`, and the default `NoopEventSink`; core's catalog
   exposes those sixteen names in
   deterministic alphabetical order.

The prepared-root production constructor has the same order except that it
consumes the already prepared workspace/session authorities together in place
of the two path opens. The two custom-transport constructors omit credential
discovery and production HTTP construction. After selection validation, their
explicit-root variant opens workspace and session roots, while their
prepared-root variant consumes both retained authorities. Each then constructs
terminal, web-search transport/tool, provider, web-fetch, permission, question,
and engine components in that exact relative order. Constructing the default
question tool is infallible and adds no new build-error stage.

The non-secret workspace and session roots are therefore opened before
credential discovery and bearer-token handoff. A selection, workspace, or
session-store failure neither discovers nor hands a credential to the HTTP
transport. Credential discovery retains its exact precedence and fail-closed
validation contract, and production transport construction retains its pinned
endpoint and HTTP/TLS/status/cancellation policy.

The workspace is opened once with the existing Linux/macOS final-component
no-follow and authoritative directory checks. One retained descriptor remains
with one tool and twelve descriptor clones of the same opened directory object
feed the other twelve workspace-backed tools. The candidate engine registers
exactly the sixteen alphabetical tools listed above; rootless
`ask_user_question`, `web_fetch`, and Gateway-backed `web_search` receive no
workspace descriptor. It discovers or registers no other tool.
This shared retained identity prevents separate path opens from selecting
different workspace directory objects if the host path is replaced between
tool construction steps. It does not make the workspace a sandbox against the
host, change any tool's model-selected path rules, or freeze mounts beneath
the retained directory.

The session root is supplied separately from the workspace. It must already
exist and is opened through the existing `FileSessionStore::open` contract.
Composition does not compare the roots' opened identities or detect equality
or ancestor relationships. Selecting disjoint roots is a trusted-host
responsibility: if the session root equals or sits beneath the workspace,
workspace tools can reach session artifacts within their normal bounded path
rules after permission is granted. The composition does not derive either root
from `LoadedNativeConfig` or native status.

## Explicit custom-transport override

`compose_with_ai_gateway_transport` is a trusted authority override. It still
requires the same validated `ask` / `vercel_ai_gateway` / `ai_gateway_http`
selection, opens the same workspace and session-store authorities, constructs
the same provider, permission adapter, and rootless question tool, registers
the same sixteen candidate tools including workspace-backed `terminal`,
rootless `ask_user_question`, rootless `web_fetch`, and Gateway-backed
`web_search`, retains one original descriptor plus twelve clones for exactly
thirteen workspace-backed tools, and uses the same
default engine limits and no-op sink. It deliberately
performs no
credential discovery and does not construct `AiGatewayHttpTransport`.

The injected `Arc<dyn AiGatewayTransport>` owns whatever endpoint, network,
authentication, status, timeout, retry, runtime, and diagnostic policy it
implements. It must obey the existing `AiGatewayTransport` contract, including
returning only accepted response bytes or a redacted `ProviderError`. This path
is intended for trusted custom hosts and deterministic tests; it is not a way
to weaken the production transport's pinned policy while retaining a
production-transport claim.

The custom constructor also requires the exact canonical HTTP(S)
`NetworkTarget` contacted by that opaque transport. This is the target exposed
to core's web-search permission request; noncanonical or malformed targets fail
construction. Both production and custom constructors require an explicit
`Arc<dyn WebSearchDeadline>`. That fallible authority owns runtime/timer wakeup
policy and must report `RuntimeRequired` rather than panic when it cannot drive
the absolute deadline.

`credential_source()` returns `None` for this constructor. That value means
only that native credential discovery did not run. It does not assert that the
custom transport is unauthenticated or holds no secret.

## Synchronous construction and later polling

Both integrated path constructors are synchronous. They perform only selection
validation, bounded component construction, the documented root opens, and—on
the production path—discovery from the already injected credential snapshot
and HTTP client construction. They make no network request, poll no permission
prompt, load or save no session record, and create no file or directory. They
do not create a Tokio runtime, task, thread, timer, channel, retry, or other
background work.

The integrated path constructors do not select or create the workspace or
session root. They also do not call
`AiGatewayCredentialEnvironment::from_process`; a caller that wants process
discovery must take that explicit snapshot before composition.
Constructing `AskPermissionHandler` does not invoke the injected prompter.
Constructing `FileSessionStore` retains only its root and does not touch a
session record or lock sidecar.

The delivered fifteenth slice's construction only shares the retained
`FileSessionStore` with `NativeSessionLifecycle`; it performs no entropy read,
session load, save, reset, engine registration, or lock-sidecar operation.
Those effects are owned by a lifecycle future and remain inert until that
future is first polled.

The delivered sixteenth slice does not add construction effects. Creating its listing
future is also inert; only first poll can enumerate the retained root, validate
canonical records, acquire their per-ID locks, or create private lock sidecars.

The delivered seventeenth slice adds no construction effect beyond cloning and
retaining the already opened workspace descriptor for the third tool.
`file_info` preparation is effect-free, and creating its execution future is
inert; only first poll can traverse ancestor descriptors or inspect final
metadata. It creates no task, thread, file, directory, timer, or background
work.

The delivered eighteenth slice adds no construction effect beyond the third
descriptor clone and retention for the fourth tool. `glob_files` preparation is
effect-free, and creating its execution future is inert. Its first poll performs
the complete bounded synchronous scan after approval; it creates no task,
thread, file, directory, subprocess, timer, or background work.

The delivered nineteenth slice adds no construction effect beyond the fourth
descriptor clone and retention for the fifth tool. `grep_files` preparation is
effect-free, and creating its execution future is inert. Its first poll performs
the complete bounded synchronous content search after approval; it creates no
task, thread, file, directory, subprocess, timer, or background work.

Slices twenty through twenty-three add four identity-preserving descriptor
clones and retention for `write_file`, `edit_file`, `delete_file`, and
`rename_file`. `rename_file` preparation is effect-free and its execution
future is inert until first poll. After approval, that first poll performs the
bounded synchronous two-parent validation, one no-replace rename, postcommit
identity check, and parent synchronization documented in
[`rename-file.md`](rename-file.md); construction itself performs none of those
effects and starts no background work.

The delivered twenty-fourth slice adds one more identity-preserving descriptor
clone and retains it for `copy_file`. Its preparation is effect-free and its
execution future is inert until first poll. After approval, that poll performs
the confined, bounded synchronous source validation, binary-safe streaming
into a private destination-parent stage, one no-replace commit, postcommit
verification, and destination-parent synchronization documented in
[`copy-file.md`](copy-file.md). Construction performs none of those effects and
starts no task, thread, I/O, or background work. The slice changes no CLI byte
and makes no complete fx-equivalence or performance claim.

The composed twenty-fifth slice adds one more identity-preserving clone and
retains it for `create_folder`. Preparation remains effect-free and returns the
existing single-path `FilesystemAccess::Create` authority. Its inert future
performs the bounded synchronous no-follow recursive creation, fresh
postcommit rewalk, and bottom-up durability described in
[`create-folder.md`](create-folder.md) on first poll. Construction performs no
directory creation, lookup, permission normalization, task, thread, I/O, or
background work. The candidate catalog and clone counts are eleven and ten;
those are now the delivered `create_folder` base counts.

The delivered twenty-sixth slice adds one more identity-
preserving clone and no construction effect. Its launcher has
a trusted injected test seam; production approved execution alone may spawn
fixed `/usr/bin/xdg-open` on Linux. Other targets return unsupported without
spawn.
Exactly 32 global permits bound production system-launch workers, and each is
held through arbitrary Waker completion and worker return; saturation is
precommit unavailable with zero new worker/helper. The worker starts before the
helper. Spawn and cancellation/drop share one
serialized gate: abort-first guarantees zero launch, while successful spawn is
the commit boundary. Postcommit cancellation, timeout, or explicit future/drop
cleanup terminates and reaps the direct helper without claiming rollback.
Before publication, cleanup suppresses waking, reaps the helper, drops request/
descriptor ownership, and synchronously joins. Normal published cleanup joins.
Inline or blocking arbitrary-Waker overlap may release the handle to avoid
deadlock; only permit-bounded callback/final bookkeeping remains after helper/
request cleanup. The docs-only amendment replaces the frozen absolute no-
worker-detach rule and is exempt from its own review. Postcommit
cancellation and process or wait failures return
fixed redacted, nonretryable result uncertainty when a tool-level result is
observed. The delivered catalog
and clone counts are exactly twelve and eleven.
External paths, directories, URLs, a real macOS backend, CLI composition,
benchmarks, performance claims, and equivalence remain deferred. Formal cycle 3
rejected exact candidate `6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`, for the two low deadline-test and
no-Waker ordinary-join gaps. Exact cycle-4 candidate
`4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`, remediates both and passes the
complete replacement local gate, but cycle 4 is rejected because
correctness/API found one low maintained-documentation lineage drift;
filesystem/process-lifecycle and performance/concurrency are green with zero
findings. Cycle-5 candidate `4317ac61feb57b706b6a023d2b2518c10e140d69`,
tree `90750911b26dc4eed9e54e73c17c11a6c5a12423`, was rejected when all three
tracks found the same low remaining current-lineage wording defect and zero
production defects. That correction is composed in exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh tracks are green
with zero findings at every severity. Exact feature workflows, delivery, and
exact `main` workflows are complete on seal and integrated `main` SHA
`a02c28a6bc39f2981586f02cb76793c430c83a20` under the runs and artifacts
recorded above. The current host has exactly twelve alphabetical tools using
one retained descriptor plus eleven identity-preserving clones. This final
docs-only record is exempt from adversarial review under the user's instruction
but required its own exact feature and `main` workflows, which were still to be
reported at that checkpoint. Subsequent cycle-7 candidate
`ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
`f8a681db319f0a89e21f38e7f9f8c474c270452b`, is rejected with correctness/API
and lifecycle green at zero findings, but performance/concurrency not green
with the two low fixture-hang and maintained-current-lineage findings recorded
above. Exact test-only code remediation
`274f4e0f705f33ec2ea4bae60f5bd6bbe02e1f0f`, tree
`865e93423719cdb5655cb7dd22fd20f207717cbb`, uses the existing `before_spawn`
barrier. Its docs correction SHA, full replacement local gate, and all three
fresh cycle-8 tracks remain pending. Production source, public API, and
manifests are unchanged. This makes no product-performance or fx-equivalence
claim.

Subsequent exact cycle-8 candidate
`6cfc17407cb6fa05d7568cd4f074775fc76c0e25`, tree
`44aa7c2636f341e8d759ef18626d0565a5a7d05e`, passed the full local gate.
Correctness/API rejected it with exactly two low findings and zero blocker,
high, or medium: no successful-helper witness in the normal fixture, and stale
operative lineage that did not name this candidate. Lifecycle and
performance/concurrency were green at zero findings. Exact remediation
`a8415f2ac79bea979d27651174d21065c6c5d5d7`, tree
`7210b0a0bd719e8373a7bf15bfc7084d7eff0199`, adds distinct successful-marker
and deterministic spawn-failure tests around shared lifecycle assertions;
normal Linux arm64 evidence is 202/202 and the noexec failure case is 100/100.
Production source, public API, manifests, and workflows remain unchanged. The
documentation correction/cycle-9 candidate, full replacement gate, and three
fresh cycle-9 reviews remain pending. This makes no product-performance or
fx-equivalence claim.

Exact cycle-9 candidate `964c59408bda1a3793978041432b84b808b474a6`, tree
`7e5306ad77ece822b4f0080c4d6a24f142635e04`, passes the full replacement gate.
All three fresh correctness/API, filesystem/process-lifecycle, and
performance/concurrency reviews are green with zero findings at every
severity. Production source, public API, manifests, and workflows remain
unchanged. A documentation-only green seal naming that reviewed candidate is
exempt from another adversarial cycle; its exact SHA/tree and feature/main
workflows remain pending. This makes no product-performance or fx-equivalence
claim.

If the resulting engine later polls the production
`AiGatewayHttpTransport`, that work must run inside a live host-owned Tokio
runtime with I/O and time enabled. The runtime must remain driven while
requests, response streams, pooled connections, or asynchronous socket teardown
remain active. `NativeReferenceHost` supplies no runtime. Core, the codec, and
the explicit custom-transport boundary remain executor-neutral.

## Retained observation and secret boundary

`loaded_config()` returns the exact `LoadedNativeConfig` consumed by the
constructor. In particular, accepted file-backed schema-v1 and schema-v2 values
remain observable with `ConfigOrigin::File` and their respective
`schema_version()` values; the host does not relabel or migrate them. Their
in-memory projections supply credential source `environment`; v1 also projects
permission mode `ask`, provider `vercel_ai_gateway`, transport
`ai_gateway_http`, and model `zai/glm-5.2`. Built-in and file-backed schema-v3
origins and values are retained exactly.

The configured `NativeCredentialSourceKind::Environment` is an acquisition
kind, not this runtime observation. The production constructor retains only the
non-secret `AiGatewayCredentialSource` selected during discovery. Its stable metadata is
available as `Some(source)` through `credential_source()`. The custom transport
constructor returns `None`. `NativeReferenceHost` has no bearer-token getter,
and the credential-source observation exposes no secret value. The production
token remains encapsulated by `AiGatewayHttpTransport` under its existing
non-reflection and best-effort clearing contract.

`engine()` borrows the composed engine. `into_engine()` consumes the wrapper
and returns that engine; the retained configuration and source observation are
then no longer available through the wrapper. Host debugging is exactly
`NativeReferenceHost { .. }`: it exposes no configuration structure, model,
credential source, roots, component diagnostic, or transport detail.

The delivered fifteenth slice adds host observations for the retained
`FileSessionStore` and `NativeSessionLifecycle`. `session_store()` and
`session_lifecycle()` expose components backed by the same shared store
allocation that the engine received during construction. They do not reopen
the selected path, re-resolve status, or create a second store with potentially
different retained identity. Successful lifecycle replay deliberately returns
the bounded durable record contents to its trusted caller; lifecycle and host
debug output still reflects no record or identity data.

Successful listing likewise deliberately returns IDs to its trusted caller.
The `NativeSessionList` result and its derived `Debug` expose those IDs and
`truncated`; this does not change the separately redacted lifecycle and host
debug surfaces or lifecycle error `Display`/`Debug`.

This sharing is checked rather than assumed. Every standalone lifecycle
constructor rejects non-identical store `Arc` allocations with fixed redacted
`MismatchedSessionStore` / `mismatched_session_store` construction failure
before consulting the incarnation source or filesystem, even when both stores
were opened from the same path. Reference-host composition constructs one
concrete allocation and supplies it to both the engine and lifecycle. An
impossible internal mismatch maps to the host's existing redacted `Engine`
build stage.

## Fixed failures

`NativeReferenceHostBuildErrorKind` is non-exhaustive so callers must preserve
forward compatibility. The complete initial categories and display strings
are:

| Kind | Exact `Display` |
| --- | --- |
| `UnsupportedSelection` | `native reference-host selection is unsupported` |
| `WorkspaceRoot` | `native reference-host workspace root is unavailable` |
| `SessionStore` | `native reference-host session store is unavailable` |
| `Credential` | `native reference-host credential is unavailable` |
| `HttpTransport` | `native reference-host HTTP transport construction failed` |
| `Provider` | `native reference-host provider construction failed` |
| `Engine` | `native reference-host engine construction failed` |

Each failure retains only its kind. `Debug` is the fixed
`NativeReferenceHostBuildError { kind: ... }` structure, `Display` is the
corresponding table entry, and the error has no nested source. Component errors
are reduced at their boundary. No configuration value, model, credential
source or bytes, workspace or session path, prompt data, endpoint, provider or
transport diagnostic, operating-system text, or raw error number is retained
or reflected.

Construction order also fixes failure precedence. Selection fails first;
workspace failure precedes session-store failure; both precede production
credential and HTTP failures; and provider or engine failure occurs only after
the earlier components have been constructed. The custom path cannot return
`Credential` or `HttpTransport` because it exercises neither stage.

## Deferred scope and milestone boundary

The integrated path constructors and `FileSessionStore::open` still do not
select or create roots. The integrated fourteenth slice adds a separate
selection/preparation boundary and consuming constructors. Formal review was
green on exact behavior SHA `f1dc4751`; after the later Linux lint normalization
at `90d8f96`, all three final tracks are green on exact candidate `72cf64f6`.
Replacement seal `f08dbd9e`, feature record `6f66b6e5`, and exact `main`
workflows are green; this documentation-only commit is the final delivery
record.
Neither that root slice nor this integrated composition implements a concrete
terminal `PermissionPrompter`, allocates a session ID or
`SessionIncarnationId`, or adds create/list/resume/replay/reset session
lifecycle CLI commands. The delivered seventeenth slice adds only `file_info`,
the delivered eighteenth slice adds only `glob_files`, and the nineteenth
candidate adds only `grep_files`; none adds the other remaining native tools,
composes or runs the CLI, or changes any existing CLI byte. A reset under a
reused session ID still requires a new host-generated incarnation before reuse.

The delivered fifteenth slice fills the library-level by-ID create, resume,
durable-record replay, and reset sub-boundary. `NativeSessionLifecycle` uses the
exact store shared with the engine, allocates new incarnations from production
OS randomness, persists create before success, and resets by atomic
current-record replacement with a checked advancing revision. It does not add
session listing, session-ID generation, a UI/event replay, or any CLI command;
delivery is green through feature record `dbba2c7`; formal review is green on
`e6a3804`. The delivered sixteenth slice supplies bounded library-only
listing through the same lifecycle. It adds no rich summaries,
workspace/latest/cursor semantics, pagination, global snapshot, session-ID
generation, UI replay, or CLI command. Its behavior and limits are in
[`native-session-listing.md`](native-session-listing.md); production/test
composition is present through `dec98e0`, whose three first formal tracks were
not green. Exact behavior candidate
`3fa54635dab00ebba78b233c69fd39e04e9be57e` composes the replacement, and all
three replacement tracks are green. Portable behavior `17f1884` and seal
`d3312d7` passed exact feature and `main` delivery gates.

Deterministic end-to-end evidence through a freshly built release binary,
remaining native-tool and CLI ownership, compatibility promotion, and
product-performance claims remain open. Delivered `file_info` replacement
reviews are green on exact `4193ecc`; exact feature and `main` delivery is green
at `60dd54f273afc7e62fb4b3cc1fb1a347d739998b`. Its
initial 34 focused tests are present at `f228c06`, and finding hardening brings
the total to 36 plus five private unit tests at `b69ec4b`. The slice does not
alter the pinned fx inventory,
benchmark workloads, or workflows. Zig remains only the pinned upstream
benchmark build input; machine-god remains a Rust product.

The `glob_files` slice adds no benchmark workload or workflow change and
makes no compatibility or product-performance claim. Production, independent
tests, documentation, and composition are present. Its first review at
`1f5de6a` found a matcher-work bound defect; the fix, regression, and replacement
local gates are green at `4171a4a`. All three same-SHA replacement reviews are
green at `523df858`. Documentation seal and integrated `main` SHA
`35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed exact feature CI
`32610950593`, feature benchmark evidence `32610950594`, main CI `32611208411`,
and main benchmark evidence `32611208415`; the combined native-tool checklist
stays open because the other listed tools remain incomplete.

The nineteenth `grep_files` candidate likewise adds no benchmark workload or
workflow change and makes no compatibility or product-performance claim. Its
isolated production and initial independent-test components exist. The fixture
fix is green at `bdbb677`; documentation component `b04151a` produces first
fully composed behavior `42e4793`, and lint fix plus exact local gates are green
at `45ad91f`. All three first-cycle tracks are **NOT GREEN** on exact `355a11a`.
Remediation and exact replacement local gates are green at final code/test
precursor `275d263`. First replacement candidate `ae87bf1` is **NOT GREEN**
across all three tracks. Second-fix production and documentation compose through
`ac5d772`, `d672210`, `7ad0863`, and exact local-gate-green precursor
`b498ba0`. Formal second replacement candidate `5aeddc1` has correctness/API
and filesystem/robustness **GREEN** with zero findings and
performance/concurrency **NOT GREEN** with one medium allocation-amplification
finding and two low documentation/evidence findings. Third remediation composes
through `8777825`, `ab1c133`, `dcf57ad`, `d7526d4`, `44afb23`, `f08c5f2`, and
`1f13f9a` at exact fully composed local-gate precursor `a8f6179`. Its scan-local
content buffer reads through an 8 KiB window, grows only to a 204,801-byte high-
water ceiling, and logically resets for reuse between files; both dynamic-
programming branches route through injectable cancellation checks. Exact Rust
1.94.1 formatting, warnings-denied workspace Clippy, 598 non-documentation tests
plus two doctests, 25 private native tests, 40 direct tests, four engine tests,
cross-target/dependency/link validation, and diff checks are green.
Compatibility/release validation is green. Formal third-cycle candidate
`0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
**GREEN** with zero findings. Correctness/API and performance/concurrency are
**NOT GREEN** only for the same LOW documentation contract mismatch; reviewers
confirmed zero production defects. Isolated wording remediation
`993b618bf78d30f6a68f3b248b572e33e4de1126` composes at exact
`f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`; its documentation gates are
green, and its behavior tree remains `a8f6179` except for documentation. Formal
fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero findings
in all three fresh tracks: correctness/API, filesystem/robustness, and
performance/concurrency. Exact-SHA formatting, warnings-denied workspace
Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests, 25 private
tests, 40 direct `grep_files` tests, four engine tests, and the 58/420/270/0
documentation inventory are green. All historical findings are closed,
including the attempted-read-window storage wording. This documentation-only
seal is exempt from another adversarial review under the user's explicit
instruction. Documentation seal `0f48806310882caf3c668c72fe1b9d211cae744b`
is feature-green: CI run `32623585346` passed all six jobs and benchmark-
evidence run `32623585349` passed both jobs and artifacts, all for exact `0f`.
`main` was fast-forwarded without force from `f6ab594` to exact `0f`. Main CI
run `32623904784` is **GREEN** for exact `0f`: all six jobs and every step
passed without reruns. Main benchmark-evidence run `32623904800` is **GREEN**
on attempt 1 for exact `0f`: both jobs and every step passed, with two valid
non-expired exact-SHA artifacts retained. The `grep_files` slice is delivered;
the remaining native tools remain pending.
This final delivery record is documentation-only and exempt from adversarial
review; its own exact remote workflows are required after push and cannot be
self-recorded.
The combined native-tool checklist stays open.

The frozen reference-host composition checklist item is complete: the
implementation, independent tests, composed adversarial review, exact feature
gates, fast-forward integration, and exact `main` gates are green. The combined
credential-and-configuration item is also complete. The thirteenth slice's
three adversarial tracks are green on exact behavior SHA `35ce591e`, and exact
final-record feature and `main` workflows are green at integrated SHA
`f840576a`. The fourteenth slice supplies root selection/preparation, the
fifteenth supplies create/resume/replay/reset, and the delivered sixteenth
slice supplies bounded IDs-only listing. The combined root-and-session-lifecycle
item is complete: the replacement reviews and exact feature and `main` gates
are green. Milestone 03 remains in progress because remaining native tools,
top-level CLI/slash-command ownership, and composed release-binary end-to-end
evidence remain open.

## Slice 35 cycle-5 rootless-composition remediation

Slice 35 adds one explicit shared `QuestionPrompter` parameter to
each production and custom reference-host constructor. Construction stores the
prompter inside a rootless `AskUserQuestionTool`; it does not poll the prompter,
inspect a terminal, discover interactivity, open another root, or start work.
Selection, workspace, session-store, credential, HTTP, web-search, terminal,
provider, and engine failure ordering remains otherwise unchanged. An invalid
question concurrency setting, if an explicit-limits host path is later exposed,
must map to a fixed redacted composition stage rather than reflect prompt data.

The exact local tool order is `ask_user_question`, `copy_file`, `create_folder`,
`delete_file`, `edit_file`, `file_info`, `glob_files`, `grep_files`,
`list_files`, `open_file`, `read_file`, `rename_file`, `terminal`, `web_fetch`,
`web_search`, and `write_file`. Historical behavior head `a76818e`, tree
`f44def5`, passed its recorded local gate, but formal cycle 1 rejected exact
candidate `6c54ec3`, tree `bea9024`. Cycle-2 evidence and production corrections
compose with the finding docs at exact `c8718c6`, tree `c27463b`; its complete
local gate was green. Formal cycle 2 rejected exact candidate `910d7bc`, tree
`503a91f`, with a deduplicated `0/0/2/2` union. Cycle-3 production, evidence,
and docs compose at exact behavior head `8bdc33d96bf88f5986c0e01b3979a2cef0427e82`,
tree `7a342fc27d6b2d65dcbdcf547cfbdc8214e73702`; its replacement local gate is
green with 57 focused tests. Formal cycle 3 rejected exact candidate
`746e510c7d8eb93229996e74f91827f489e5bb31`, tree
`c49221efbea66c840b333f0de0161aa686aad52f`, with a deduplicated `0/0/3/2`
union. Cycle-4 core `e569514`/`4c8cff3`, native `53c05cd`/`1857a3f`, and
finding docs `b057958` compose at exact behavior head `cb93bff`, tree
`fa402acb`. Host composition is unchanged while capability inspection is total,
private answer ownership is fixed to four slots admitting zero through four
entries, and the active permit remains held through prompt/cancellation-Waker
teardown. Formal cycle 4 showed that the final cancellation-Waker destructor
could make cancellation observable only after the last check, while concurrent
cancellation could move a registered callback whose tail continued after outer
drop released that permit. Cycle 5 implements activity-backed
cancellation Waker clones and callbacks to retain the originating permit
through callback return, an equivalent cached registration, prompt/waiter/
cached-Waker teardown under activity, and a final cancellation recheck after
teardown before every direct return. The descriptor count stays thirteen:
the question tool and two web tools own no workspace descriptor. The no-authority
question call invokes neither the separately injected `PermissionPrompter` nor
permission events; this is a no-policy-authority disposition, while the
`QuestionPrompter` separately owns host interaction authority. A noninteractive
host must inject the fixed unavailable
prompter behavior; this slice does not inspect TTY state or add a CLI UI. The
contract, rejected cycle-4 outcome, and cycle-5 local-gate checkpoint are
[`ask-user-question.md`](ask-user-question.md) and
[`m03-ask-user-question-review-01.md`](reviews/m03-ask-user-question-review-01.md).
The complete historical exact-1.94.1 cycle-4 local gate is green at
`cb93bff`/`fa402acb`; exact candidate `42ce6f0`/`b761f7b` was nevertheless
rejected with the deduplicated `0/0/2/1` union. Deterministic cycle-5 race
evidence and source now compose at `b870731`/`0b025f8`, whose complete
exact-1.94.1 local gate is green. Formal cycle 5 rejected exact
`54b1aab`/`54586d2` because independently forwarding arbitrary retained Waker
clones can run concurrent blocking downstream callbacks behind one slot.
Cycle-6 single-flight coalescing notification, lossless replay, target close,
capacity ownership, and many-clone evidence compose at exact
`707a794`/`1e60299`; focused, required pinned, extended, and unchanged release-
smoke gates are green. Formal cycle 6 rejected exact `85058a8`/`fd3c507`
because replay may self-rearm synchronously without bound; its low finding also
requires consistent operative opening statuses. Cycle-7 observation-aware
source/evidence and complete gate are green at `fbb3f5c`/`7cee96e`; fresh
reviews later rejected exact `6176729`/`f2cd844` with a `0/0/1/0` union.
Cycle-8 prior-target destruction before replay selection, unwind recovery,
reentrant-close suppression, and the complete local gate compose at
`d8075ff`/`fa32564`. Reviews, remote workflows, integration, and delivery remain
pending, and formal cycle 8 rejected exact `e929b5e`/`cfadc42` with a
`0/0/2/0` union. Cycle-9 primary-panic preservation, constant per-activation
callback work, residual pending delivery, and the complete local gate compose
at `0279b8c`/`50b2423`. Formal cycle 9 rejected exact
`1eeab67`/`5c86e62` with a deduplicated `0/0/1/0` liveness medium; cycle-10
source/evidence and the complete local gate compose at
`72e8e75`/`5405180`. Formal cycle 10 rejected `4ea1c1f`/`78e781f` with two
distinct mediums and a `0/0/2/0` union; cycle-11 source, evidence, review,
integration, and delivery do not exist. Analogous preexisting terminal code is
out of scope and is not claimed fixed.
