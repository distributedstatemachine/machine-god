# Native reference-host composition

Status: **IN PROGRESS** for the proposed `web_fetch` candidate; `open_file`
remains delivered.
The delivered composition contains exactly twelve alphabetical workspace tools,
and twenty-six bounded Milestone 03 slices are delivered;
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

The proposed twenty-seventh slice starts from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e` and pinned fx reference
`b1774fbf6c7602b503026f96f6e960e946c692ef`. The local candidate composition
inserts rootless `WebFetchTool` alphabetically to produce thirteen host tools.
It owns no workspace descriptor, so the descriptor-backed set remains exactly
twelve workspace tools using one original descriptor plus eleven clones. The
candidate is non-WASM and cfg-gated by `web-fetch-http`, which is included by
`ai-gateway-http`; it adds no CLI state or command. Production and independent
focused evidence are composed and all seven exact host tests pass. Pre-review
gate record `0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
`5742e4084272120a4531e0d59f0199a5873f39d1`, passed the complete local gate.
Formal cycle 1 is **NOT GREEN** on exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`. The rootless thirteen-tool host
composition itself remains unchanged. Isolated production remediation
component `0c8c76935a6e3ca392e58b2aa9c375f88221f41f`, tree
`d96c13c853424325a688631dfea25c504bb62250`, and evidence tip
`c3dc6a00da22738b6840fc2bc66840dc735eee6f`, tree
`558140e5ac31f6f8f2cd7d15064681b53e7fd39b`, exist. Documentation composition,
the complete replacement gate, three fresh same-SHA reviews, remote workflows,
integration, and delivery remain pending under
[`web-fetch.md`](web-fetch.md).
The delivered count remains twenty-six and this is not a performance or
fx-equivalence claim.

Reference-host construction supplies no runtime and performs no web request.
Later production `web_fetch` polling requires a current host-owned Tokio
runtime with I/O and time enabled. No current handle produces fixed
`RuntimeRequired`; a current driverless runtime violates the documented
`# Panics` precondition and may terminate a release process.
The host supplies no resolver override: each admitted hostname invocation reads
the host's first UDP-configured system nameserver, then owns its bounded direct
DNS socket work. Literal public IPs skip DNS. No workspace descriptor is used
for resolver configuration or network execution.
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
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError>

NativeReferenceHost::compose_with_ai_gateway_transport(
    loaded_config: LoadedNativeConfig,
    transport: Arc<dyn AiGatewayTransport>,
    workspace_root: &Path,
    session_root: &Path,
    permission_prompter: Arc<dyn PermissionPrompter>,
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
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError>

NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots(
    loaded_config: LoadedNativeConfig,
    transport: Arc<dyn AiGatewayTransport>,
    prepared_roots: PreparedNativeRoots,
    permission_prompter: Arc<dyn PermissionPrompter>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError>
```

These methods consume retained roots prepared under the separate
[`native root-selection contract`](native-root-selection.md), rather than
reopening path arguments. Production and independent focused tests are present;
formal adversarial review was green on exact behavior SHA `f1dc4751`; after the
Linux lint normalization at `90d8f96`, all three final tracks are green on exact
candidate `72cf64f6`. Replacement seal `f08dbd9e` and feature record `6f66b6e5`
are green, and the additions are integrated on `main` under exact green
workflows.

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
   identity for the twelve candidate tools;
3. open the existing absolute session root as `FileSessionStore`;
4. consume the injected `AiGatewayCredentialEnvironment` and discover one
   validated bearer token under its existing precedence rules;
5. move that bearer token into production `AiGatewayHttpTransport`;
6. construct `AiGatewayProvider` with the loaded configuration's projected
   model;
7. wrap the injected prompter in `AskPermissionHandler`; and
8. build `Engine` with exactly `copy_file`, `create_folder`, `delete_file`,
   `edit_file`, `file_info`, `glob_files`, `grep_files`, `list_files`,
   `open_file`, `read_file`, `rename_file`, and `write_file`, default
   `EngineLimits`, and the default `NoopEventSink`;
   core's catalog exposes those names in deterministic alphabetical order.

The non-secret workspace and session roots are therefore opened before
credential discovery and bearer-token handoff. A selection, workspace, or
session-store failure neither discovers nor hands a credential to the HTTP
transport. Credential discovery retains its exact precedence and fail-closed
validation contract, and production transport construction retains its pinned
endpoint and HTTP/TLS/status/cancellation policy.

The workspace is opened once with the existing Linux/macOS final-component
no-follow and authoritative directory checks. One retained descriptor remains
with one tool and eleven descriptor clones of the same opened directory object
feed the others. The candidate engine registers exactly the twelve alphabetical
tools listed above and discovers or registers no other tool. This shared retained
identity prevents separate path opens from selecting
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
the same provider and permission adapter, registers the same twelve candidate
tools, and uses the same default engine limits and no-op sink. It deliberately
performs no
credential discovery and does not construct `AiGatewayHttpTransport`.

The injected `Arc<dyn AiGatewayTransport>` owns whatever endpoint, network,
authentication, status, timeout, retry, runtime, and diagnostic policy it
implements. It must obey the existing `AiGatewayTransport` contract, including
returning only accepted response bytes or a redacted `ProviderError`. This path
is intended for trusted custom hosts and deterministic tests; it is not a way
to weaken the production transport's pinned policy while retaining a
production-transport claim.

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
