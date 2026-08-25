# Architecture

`machine-god-core` contains provider-neutral contracts and orchestration without
ambient operating-system authority. `machine-god-native` provides explicit native
capabilities. `machine-god-cli` composes them. `machine-god-testkit` provides
deterministic test doubles.

The testkit's concrete boundary doubles are documented in
[`testkit.md`](testkit.md). They are executor-neutral like core: finite scripts,
cancel-driven pending provider/tool work, and permanently pending observer or
policy steps support precise manual polling without clocks. Each double owns a
single mutex per related script-and-record state so concurrent call ordering is
linearizable and inspection returns a consistent snapshot. The in-memory store
executes comparison and mutation inside that same critical section, making its
optimistic revision behavior atomic rather than merely convenient for
single-threaded tests.

The milestone-02 public contracts are documented in
[`core-api.md`](core-api.md). Public interfaces keep model access, storage,
tools, permission policy, and event delivery behind object-safe traits. Core
uses standard futures and `futures-core::Stream`; it does not select or require
an async executor.

Milestone 03 has twenty-six delivered bounded slices. The proposed
twenty-seventh `web_fetch` slice is **IN PROGRESS** from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e`. Production and independently owned
direct, engine, production-boundary, core-contract, and host evidence are
composed locally. Pre-review gate record
`0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
`5742e4084272120a4531e0d59f0199a5873f39d1`, passed the complete local Rust
1.94.1, integrity, dependency, baseline portability, WASI, and release-binary
gate. Formal cycle 1 is **NOT GREEN** on exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`. Isolated production remediation
component `0c8c76935a6e3ca392e58b2aa9c375f88221f41f`, tree
`d96c13c853424325a688631dfea25c504bb62250`, and evidence tip
`c3dc6a00da22738b6840fc2bc66840dc735eee6f`, tree
`558140e5ac31f6f8f2cd7d15064681b53e7fd39b`, exist. Documentation composition,
the complete replacement gate, and three fresh same-SHA reviews remain pending.
Native Linux HTTP evidence remains an exact-CI requirement
because the macOS cross-host lacks the target C sysroot. Feature and `main`
workflows, integration, and delivery remain pending. Its non-WASM
`web-fetch-http` feature adds one rootless `WebFetchTool`;
`ai-gateway-http` includes that feature. Core retains provider-neutral
`Capability::Network` and orchestration, while native owns
URL/DNS/Reqwest/Tokio effects. Construction remains runtime-independent;
polling production requires a current host-owned Tokio runtime with I/O and
time enabled. No current handle is a typed `RuntimeRequired` failure, while a
current driverless runtime violates the documented `# Panics` precondition and
may terminate a release process. One outer bounded wrapper owns the active-call
permit, cancellation future, and one reused deadline sleep through rendering
and its final boundary. Native DNS reads a system-configured UDP resolver after
admission and uses only invocation-owned Tokio A/AAAA sockets with bounded TCP
fallback; it creates no libc lookup, cache, retry, resolver thread, or spawned
task. A process-wide cached Rustls configuration supplies pinned roots and
HTTP/1.1 ALPN to each fresh pinned Reqwest client without reparsing the roots.
The candidate host has thirteen alphabetical tools, but its descriptor-backed
workspace set remains twelve tools using one original descriptor plus eleven
clones. The frozen boundary is
[`web-fetch.md`](web-fetch.md); it adds no CLI, cache, artifact, compatibility,
or product-performance claim. The twenty-third,
library-only native `rename_file`, has composed production and independent
evidence; exact cycle-1 remediation `a3491cf`, tree `0b195bd`, passes the
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
The twenty-fourth, library-only `copy_file` slice is delivered. Cycle-3
candidate `99ecdb3`, tree `145b3be`, is green with zero findings in all three
fresh tracks. Seal `3bdd7cb` passed exact feature CI `32684856309`, feature
benchmark `32684856373`, main CI `32685192453`, and main benchmark
`32685192394`; each benchmark run retains exactly two nonexpired exact-SHA
artifacts.
The twenty-fifth, library-only native `create_folder` slice is **DELIVERED**
from delivered base `d1a5bc2`. Exact frozen contract commit
`9fab189c9c1add76a38775d08f4342c6bcc7635b` passed all six jobs of CI
`32687614476`; benchmark workflow `32687614442` passed both jobs and retains
exactly two nonexpired exact-SHA artifacts. Candidate source composes strict
single-path `FilesystemAccess::Create` authority, recursive no-follow directory
creation, a first-creation commit boundary, bounded bottom-up durability, and
eleven-tool host registration. Cycle-2 candidate `6e1f885`, tree `ac57575`, is
historically not green: correctness/API and performance/concurrency are green
with zero findings, while filesystem/robustness reported two low evidence/
documentation findings and zero production defects. Exact remediation
`f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate. Documentation record `9d0bacd`, tree `b5fb1c2`, and tree-identical
cycle-3 candidate `c1e572e` preserve identical non-documentation behavior.
Cycle 3 is not green only for one low documentation-lineage finding; filesystem
and performance are green, and all tracks found zero production defects. Exact
lineage remediation `12c11ba`, tree `b96575b`, passes the complete replacement
gate. Gate record `f6f6584` parents tree-identical cycle-4 candidate `a78b693`,
tree `2b913e8`. Correctness and performance are green with zero findings;
filesystem found zero production defects and one low stale documentation-seal
sentence, corrected under the user's seal-review exemption. The delivered host
stayed at ten tools at that checkpoint. First feature CI `32699750602` has all
four native Linux/macOS jobs green but is not green because Linux Quality
rejected a test-only `RawMode` conversion. Exact portable trace remediation
`1effcbb`, tree
`b5eccb1`, passes the complete replacement gate. Tree-identical cycle-5
candidate `ff18a9a`, tree `f77b198`, is green with zero findings in all three
fresh tracks. Seal `e75578b` passed exact feature CI `32702785549`, feature
benchmark `32702785574`, main CI `32703303933`, and main benchmark
`32703303931`; both benchmark runs retain exactly two nonexpired exact-SHA
artifacts. At that checkpoint, the delivered host had eleven tools and made no
product-performance claim.
The twenty-sixth, library-only native `open_file` slice is delivered after a
green formal cycle 6. It adds dedicated provider-neutral
`Capability::OpenFile { path }`
for one strict canonical, workspace-confined existing regular file rather than
treating default-application launch as filesystem read or accepting a model-
selected process. Linux execution opens the path beneath retained directory
descriptors without following selected symlinks and retains the final regular-
file descriptor through the launcher lifecycle. The production launcher uses
exactly `/usr/bin/xdg-open` and the parent-owned
`/proc/<parent-pid>/fd/<retained-fd>` target, with fixed `/` working directory,
null stdio, and a 30-second timeout decision. Machine-god performs no shell or
`PATH` lookup; the trusted helper and desktop dispatch may consult inherited
host environment and configuration. Linux also exports a trusted injected
launcher seam whose returned future must be inert until polled. At most 32
production system-launch workers exist; saturation is precommit unavailable
with zero new worker/helper, and each permit remains owned through arbitrary
Waker completion and worker return. The final spawn attempt and
cancellation/drop abort transition share one serialized gate:
abort-first guarantees zero launch, while successful spawn commits an effect
that cannot be rolled back. Postspawn cancellation makes core drop the execution
future; cancellation, timeout, or explicit drop kills and reaps the direct
helper. Prepublication cleanup suppresses waking, drops request/descriptor
ownership, and joins; normal postpublication cleanup joins too. Inline or
blocking arbitrary-Waker overlap may release the `JoinHandle` to avoid self-
join/cross-thread deadlock after all helper/request ownership is gone; only
permit-bounded callback/final bookkeeping may outlive future drop. This docs-
only amendment replaces the frozen absolute no-worker-detach invariant because
it contradicted legal Waker behavior and is exempt from its own adversarial
review under the owner's instruction. Formal cycle 3 rejected exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`, for two low findings: the
post-`try_wait` authoritative-clock branch was not truly tested, and ordinary
no-Waker publication could detach a permit-bounded worker tail instead of
joining it. Exact candidate `4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`, remediates both and passes the
complete replacement local gate. Cycle 4 is nevertheless not green:
filesystem/process-lifecycle and performance/concurrency are green with zero
findings, while correctness/API reported one low maintained-documentation
lineage drift and no production defect. That remediation was composed into
cycle-5 candidate `4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. All three tracks found zero
production defects and the same low remaining current-lineage wording defect.
That correction is composed in exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh correctness/API,
filesystem/process-lifecycle, and performance/concurrency tracks are green with
zero findings at every severity. Seal and integrated `main` SHA
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

The first formal sixteenth candidate is composed through `dec98e0`, whose three
review tracks were not green. Its source and test fixes are composed in exact
behavior candidate
`3fa54635dab00ebba78b233c69fd39e04e9be57e`; all three replacement tracks are
green. First remote CI `32599591900` exposed the Linux removed-root gap; this
portable fix is exact behavior candidate `17f1884`, green under both executable
review tracks. Documentation seal `d3312d7` resolves its lineage finding,
passed exact feature CI `32600292770` and benchmark evidence `32600292779`, was
fast-forwarded without force to `main`, and passed exact main CI `32600567094`
and benchmark evidence `32600567090`.
The twelfth slice's
production implementation, independent black-box tests, three fresh adversarial
tracks, and exact feature and `main` workflows are green. It is integrated on
`main` through final delivery record
`ac3984fb16dbab3adf86a949c7555ceca7c3e8df`; exact feature CI run
`32579779134`, feature benchmark-evidence run `32579779123`, main CI run
`32580066474`, and main benchmark-evidence run `32580066485` are green.
The first two are native-host slices;
the third extends the authority-free core tool contract, the fourth and fifth
use that contract for bounded executable native capabilities, and the sixth
provides a bounded Gateway codec over an injected host byte transport. The
seventh supplies one optional native HTTP implementation of that transport.
The eighth slice supplies a bounded Unix file implementation of core's
session-store boundary under an explicitly opened host root. Its exact feature,
documentation-seal, and `main` checks are green, with evidence retained in the
[`native file session store review`](reviews/m03-session-store-review-01.md);
it is integrated on `main` at
`8f7b47db9580b14570bf9fb55763858f71a81271`. Exact main CI run `32541315998`
and benchmark run `32541315997` are green.
The ninth slice supplies an executor-neutral, fail-closed native
`AskPermissionHandler` over an explicitly injected `PermissionPrompter`. It is
implemented, reviewed, and integrated on `main` at
`27e3f2b3ff170044732d9124ffb210beabcda206`. Exact main CI run `32570197911`
and benchmark run `32570197870` are green; the full lineage is recorded in the
[`ask permission handler review`](reviews/m03-ask-permission-review-01.md).
Its fixed contract is in [`ask-permission.md`](ask-permission.md).
The tenth slice adds a separate, opt-in native credential snapshot. It
selects a nonempty `VERCEL_OIDC_TOKEN` before a nonempty `AI_GATEWAY_API_KEY`,
validates the selected value through the existing bounded bearer-token type,
and moves that token without cloning into an explicit result. It does not
change core, add configuration credential fields, alter the transport, or
change CLI behavior. It is
integrated on `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`; exact main CI run `32573320962`
and benchmark run `32573320937` are green. Its fixed contract and evidence are
in [`ai-gateway-credentials.md`](ai-gateway-credentials.md).
The eleventh slice advances native configuration to a strict current
schema v2 while retaining strict v1 reads without file rewrite or migration.
It is a declarative library surface only, integrated on `main` at
`a10f24edde80a225f89e6c7068ec035cb70f80a8`. Exact main CI run `32576876769`
and benchmark-evidence run `32576876780` are green. Its contract and delivery
lineage are in [`configuration.md`](configuration.md)
and the
[`native host configuration review`](reviews/m03-native-host-config-review-01.md).
The twelfth slice implements a Linux/macOS-only `NativeReferenceHost` behind
the existing `ai-gateway-http` and non-WebAssembly gate. It composes an
already validated config selection with the provider, either production HTTP
from an injected credential snapshot or a trusted custom transport, one shared
retained workspace feeding delivered `file_info`, `list_files`, and `read_file`
under that same identity, the existing file store under
a separate session root, and the ask adapter over an injected prompter. It uses
default engine limits and the no-op event sink. The CLI
remains byte-unchanged and thin. The contract and lineage are
in [`native-reference-host.md`](native-reference-host.md) and its
[`review record`](reviews/m03-native-reference-host-review-01.md). It is
integrated at the exact green `main` SHA and runs recorded above.
The thirteenth slice advances current native configuration to strict v3 by
adding the closed non-secret `environment` credential-source acquisition kind.
It retains exact strict v1/v2 reads and their observable versions, projecting
that acquisition kind only in memory. Production implementation, independent
tests, focused and required local gates, and all three fresh adversarial tracks
are green on exact behavior SHA `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`.
It is integrated on `main` at `8755757da0da07e33af48d57f46bd9ea490b5449`;
exact main CI run `32582978232` and benchmark-evidence run `32582978286` are
green. Its final delivery record is integrated on `main` at
`f840576af241c58d1e55399e66ba92f7770cd50c`; exact feature CI run
`32583585145`, feature benchmark-evidence run `32583585148`, main CI run
`32583871385`, and main benchmark-evidence run `32583871368` are green for
that exact final-record SHA.
The integrated fourteenth slice adds explicit Linux/macOS native
root selection and safe preparation. It derives state location from an
injected environment snapshot, retains an explicit existing workspace, may
create only a fixed descriptor-relative state suffix under an existing selected
base, accepts only empty or exact protective deny-delete macOS ACLs through the
retained directory descriptor, and rejects equality or ancestry of the retained
roots. New
reference-host constructors consume those descriptors before production
credential discovery. Production and 16 independently owned focused tests are
present, and their focused gates are green. Formal review findings for macOS
extended ACLs and ambient-umask-dependent fixtures are fixed; all three formal
tracks were green on exact behavior SHA `f1dc4751`. The first seal exposed
Linux-only strict-Clippy diagnostics, normalized at `90d8f96`; local macOS and
Linux cross-target gates are green. All three final tracks are green on exact
candidate `72cf64f6`. Replacement seal `f08dbd9e` and feature record `6f66b6e5`
are feature-green; the latter is integrated on `main` under exact CI
`32590429626` and benchmark evidence `32590429592`. Its integrated contract is
in [`native-root-selection.md`](native-root-selection.md).
The delivered fifteenth slice adds a Linux/macOS
`NativeSessionLifecycle` owned by `NativeReferenceHost`. The host, engine, and
lifecycle share one `Arc<FileSessionStore>` and therefore one retained state
root. Caller-supplied IDs drive durable create, resume, current-schema record
replay, and atomic reset; production OS randomness supplies new incarnations.
No session listing or CLI path is added. Production, fourteen independently
owned focused tests, one formal finding regression, and all three adversarial
tracks are green on exact candidate `e6a3804`. Feature record `dbba2c7` is green
on the feature branch and `main` under exact CI and benchmark workflows;
the contract is in
[`native-session-lifecycle.md`](native-session-lifecycle.md).
The delivered sixteenth slice extends that same lifecycle with bounded Linux/macOS
session listing. It enumerates the retained store root and returns only up to
100 sorted unique validated IDs plus `truncated`, bounded by 1,024 processed or
selected non-dot directory entries plus one fetched/name-inspected overflow
witness and 64 MiB of accepted/decoded aggregate canonical record bytes plus one
transient transfer byte to detect concurrent growth. Every non-dot entry within
the scan consumes budget. Canonical candidates use the existing per-ID lock,
no-follow regular-file access, strict current schema, and filename/decoded-ID
digest validation. There is no multi-record snapshot, live-registry lookup,
rich summary, workspace/latest/cursor semantic, CLI path, or fx-equivalence
claim. Production and 13 initial independent tests are composed through
`dec98e0`. All three first formal tracks were not green. Isolated fix `4b8d8b0`
and test hardening `446b495` are composed in exact behavior candidate
`3fa5463`, with 18 focused tests and all three replacement tracks green.
Portable behavior `17f1884` and seal `d3312d7` passed exact feature and `main`
delivery gates.
The contract is in
[`native-session-listing.md`](native-session-listing.md).
The delivered seventeenth slice adds bounded Linux/macOS `file_info` metadata
inspection under a distinct `FilesystemAccess::Metadata` kind. It uses the
same retained workspace authority and lexical path confinement as `read_file`,
walks ancestors descriptor-relatively and no-follow, and inspects the final
component with one no-follow metadata operation without opening it. The exact
bounded result reports normalized path, fixed kind, checked size, signed Unix
modified time, and a nullable lexical regular-file extension. It therefore
reports final symlinks rather than their targets and classifies special files
without opening them. The composed host supplies exactly
`file_info`, `list_files`, and `read_file`; core exposes the catalog in
deterministic alphabetical order. Production `5c2d129` and independent tests
`ca0091c` compose at `f228c06`, where all 34 initial focused tests are green.
Review hardening brings the focused total to 36 plus five private unit tests at
`b69ec4b`. Three replacement tracks are green on exact candidate `4193ecc`.
Documentation seal and integrated `main` SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` passed exact feature CI run
`32605071080` on successful retry attempt 2, feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four workflows report that exact seal SHA. Benchmark success
is evidence only and supports no product-performance claim. This
documentation-only commit is the final delivery record, is explicitly exempt
from another adversarial review after the behavior was already green, and
reports its own exact workflows at handoff. The contract and review record are in
[`file-info.md`](file-info.md) and
[`m03-file-info-review-01.md`](reviews/m03-file-info-review-01.md).
The delivered eighteenth slice adds bounded Linux/macOS recursive `glob_files`
enumeration under the distinct `FilesystemAccess::EnumerateRecursive` kind.
Its strict effect-free preflight normalizes one required glob pattern, an
optional selected search root, and an optional exact result mode, then supplies
policy and execution the same normalized subtree and exact explicit arguments.
Execution reacquires the retained workspace under the `file_info` liveness
rule, traverses descriptor-relatively and no-follow, fully validates and
bytewise sorts each directory, and returns either the globally smallest bounded
sorted match prefix or an exact count after a complete bounded scan. The
slice extends the host catalog to `file_info`, `glob_files`, `list_files`,
and `read_file`, distributing the original retained descriptor plus three
clones of one workspace identity. Production, independent-test, documentation,
composed-behavior, and initial local-gate lineage is green through `60070d8`
from base `bbe8ce4cd4b0b131b7670171c2e9ea5d0ffee2da`. The first review at
`1f5de6a` found a high matcher-work bound defect; its fix, regression, public-
bound assertion, and replacement local gates are green at exact `4171a4a`. All
three replacement tracks are green on exact behavior SHA `523df858`; exact
seal and delivery gates are green at `35c8536`. Its contract and review record are
[`glob-files.md`](glob-files.md) and
[`m03-glob-files-review-01.md`](reviews/m03-glob-files-review-01.md).
The delivered nineteenth slice adds bounded Linux/macOS `grep_files` content search
under distinct `FilesystemAccess::SearchContent`. Exact base
`f6aa458bb875d6cb26565adc878703fe140916d3` and tree-identical integration
kickoff `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` precede parallel production,
independent-test, and documentation ownership. Exact production `27eec2f` and
initial independent-test `6eaee93` components exist and initially compose
through `9057feb` and `44e33d7`; reference-host fixture fix `bdbb677` makes
focused production/test composition green. Documentation component `b04151a`
produces first fully composed behavior candidate `42e4793`; lint fix and exact
local gates are green at `45ad91f`. All three first-cycle formal tracks are
**NOT GREEN** on exact candidate `355a11a`. Remediation and exact replacement
local gates are green at final code/test precursor `275d263`. First replacement
candidate `ae87bf1` is **NOT GREEN** across all three tracks; production and
documentation corrections compose through `ac5d772`, `d672210`, `7ad0863`, and
fully composed local-gate-green precursor `b498ba0`. `ae87bf1` remains
historically **NOT GREEN**. Formal second replacement candidate `5aeddc1` has
correctness/API and filesystem/robustness **GREEN** with zero findings;
performance/concurrency is **NOT GREEN** with one medium repeated-buffer-
allocation amplification finding and two low documentation/evidence findings.
Third production remediation `8777825` composes at `ab1c133`; independent
regression `dcf57ad` composes at `d7526d4`; review-findings documentation
`44afb23` composes at `f08c5f2`; lint follow-up `1f13f9a` produces exact fully
composed local-gate precursor `a8f6179`. Exact Rust 1.94.1 formatting,
warnings-denied workspace Clippy, 598 non-documentation tests plus two doctests,
25 private native tests, 40 direct `grep_files` tests, four engine tests, and
diff checks are green. Exact a8f cross-target/dependency/link and compatibility/
release validators are green. Formal third-cycle candidate
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
Strict effect-free preflight accepts all eight pinned field names and prepares
their explicit defaults. Allowed execution accepts a selected regular file or
directory, applies fresh-root liveness and complete descriptor-relative no-
follow traversal, searches only bounded eligible UTF-8 regular files with a
linear literal matcher and ASCII-only folding. One scan-local content buffer
reads through an 8 KiB window, grows only to a 204,801-byte high-water ceiling,
and logically resets between files without exposing stale bytes; per-file and
aggregate overflow witnesses remain charged. Recursive and non-recursive
include matching route through injectable cancellation checks. Execution
returns one of three exact structured result modes with disclosed skip/scan
totals. Same-buffer context,
aggregate traversal/content/matcher/output budgets, fixed redacted errors, and
cancellation/race semantics are normative in
[`grep-files.md`](grep-files.md). The composed host candidate has exactly five
alphabetical tools, distributing one retained workspace descriptor plus four
clones to `file_info`, `glob_files`, `grep_files`, `list_files`, and
`read_file`. The candidate adds no CLI, regex, symlink-target, Git/ignore/
subprocess, external-path, benchmark, compatibility, or performance claim.
Its review record is
[`m03-grep-files-review-01.md`](reviews/m03-grep-files-review-01.md).
The seventh slice's exact feature-branch evidence is retained in the
[`native AI Gateway HTTP transport review`](reviews/m03-ai-gateway-http-review-01.md);
it is integrated on `main` at
`508b0adbbe4447a85bd08f47095ae16c089c05d5`. Exact main CI run `32535790803`
and benchmark run `32535790824` are green.
For config and status, `machine-god-native` snapshots only `XDG_CONFIG_HOME`,
`XDG_STATE_HOME`, and `HOME`, resolves namespaced config and state paths, and
inspects their final metadata for status. A separate
synchronous native authority can load the resolved config file read-only.
`machine-god-cli` remains a thin formatter for status, does not invoke the
loader, and owns no product state. The exact surfaces are documented in
[`cli.md`](cli.md) and [`configuration.md`](configuration.md). The separate
[`read_file` contract](read-file.md),
[`list_files` contract](list-files.md), delivered
[`file_info` contract](file-info.md), delivered eighteenth
[`glob_files` slice](glob-files.md), and nineteenth
[`grep_files` candidate](grep-files.md) do not change either CLI surface. The
separate [`AI Gateway provider contract`](ai-gateway.md) also remains a library
surface and does not change CLI bytes. The same is true of the normative
[`native file session store`](session-store.md). The ask-handler slice is
also a library surface and does not change CLI bytes or supply a concrete
terminal prompt. Credential discovery is another separate
library surface; bearer credentials are fields in no supported configuration
schema, and discovery by itself does not add CLI composition. The twelfth
slice's production library constructor consumes an explicitly
injected credential snapshot; it does not give the config loader or CLI ambient
credential authority.

```text
process environment -> resolved native paths
 config path --+-> final metadata -----------> status -> CLI text/JSON
               +-> bounded read-only loader -> strict v1/v2/v3 config data
 state path ------> final metadata -----------> status -> CLI text/JSON

host-selected absolute workspace -> retained directory authority
 model {path}  -> lexical preflight -> Read policy      -> read_file
 model {path?} -> lexical preflight -> Enumerate policy -> one-level list_files
 model {path}  -> lexical preflight -> Metadata policy  -> no-follow file_info
 model {pattern,path?,mode?}
               -> lexical preflight -> EnumerateRecursive policy
               -> bounded recursive glob_files matches/count

host-selected endpoint/auth/status/retry transport
       -> injected byte stream -> AI Gateway codec -> ModelEvent stream -> core

host-injected bearer token -> optional bounded native HTTP transport
                           -> the same injected AI Gateway codec boundary

config credential_source: environment
 + explicitly owned VERCEL_OIDC_TOKEN / AI_GATEWAY_API_KEY snapshot
 -> bounded native credential discovery -> explicit bearer token -> transport

host-selected existing absolute state root -> retained directory descriptor
 SessionId -> domain-separated SHA-256 v1 name -> bounded file SessionStore
           -> NativeSessionLifecycle -> create / resume / record replay / reset
 retained root -> bounded canonical scan -> sorted unique IDs + truncated

core-owned bounded PermissionRequest -> AskPermissionHandler
                                     -> injected PermissionPrompter
                                     -> structured allow/deny decision

validated LoadedNativeConfig + existing workspace/session roots
 + injected PermissionPrompter
 + credential snapshot -------------> production AI Gateway HTTP
 |                                    -> NativeReferenceHost -> Engine
 + trusted custom transport override -> credential_source: None

injected NativeEnvironment + explicit absolute workspace
 -> NativeRootSelection -> PreparedNativeRoots
 -> retained disjoint workspace/state descriptors -> prepared-root host constructor
```

The config and state roots resolve independently. A nonempty XDG root wins and
must be absolute Unicode; an invalid selected root fails that location without
trying `HOME`. An empty XDG value falls back to a nonempty absolute-Unicode
`HOME`. Missing or empty `HOME` makes a needed fallback unavailable. The only
paths produced are `<config-root>/machine-god/config.json` and
`<state-root>/machine-god`, with `.config` and `.local/state` inserted for the
respective `HOME` fallbacks.

The fourteenth slice reuses only the state-selection precedence, not status
inspection. Selection remains effect-free and rebuilds the accepted state base
from lexical components so trailing separator or `.` decoration cannot bypass
the final-component no-follow open. Preparation requires the selected
`XDG_STATE_HOME` or fallback `HOME` base to exist, opens the workspace first,
and walks or creates only `machine-god` or `.local/state/machine-god` relative
to the retained state-base descriptor with no-follow directory operations. A
new fixed name is normalized descriptor-relatively to `0700` beneath its
validated parent before the permission-requiring reopen, then identity-checked,
`fchmod`ed, and verified at exact `0700`; the base and existing
intermediates require effective-UID ownership and no group/other write, while
the existing final root permits no group/other permission. Existing directories
are never chmodded or repaired. The final state and workspace roots remain
retained and must be neither equal nor ancestors of one another by
descriptor-identity and parent traversal. Native status still only
inspects final-path metadata and creates nothing.

Status inspection remains deliberately shallower than configuration loading. It
uses `symlink_metadata` on the final path, reports
missing/inaccessible/wrong-kind states, and treats a final symlink as
wrong-kind. It does not open, read, or parse the config file. Permission mode is
fixed to `ask`; the CLI does not construct an engine, register `read_file`,
`list_files`, delivered `file_info`, delivered `glob_files`, or candidate
`grep_files`, or prompt for
permission. The CLI
serializes paths as JSON strings
even in human status so path contents do not become terminal controls. Bare
invocation keeps the bootstrap identity contract. Help, version, status, and
argument errors remain byte-stable presentation behavior, not an engine-owned
command model.

The synchronous loader resolves only the config location. In the thirteenth
slice, an unavailable location or missing file yields the explicit built-in
schema-v3 object
`{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2","credential_source":"environment"}`.
Invalid selected environment input and all other load failures fail closed. A
present file must be at most 64 KiB of valid UTF-8. It must be that exact
six-field v3 shape, with any 1–128-byte visible-ASCII model, the exact strict
five-field v2 shape without `credential_source`, or the exact strict legacy v1
shape `{"schema_version":1,"permission_mode":"ask"}`. Unknown, duplicate,
missing, wrong-type, or unsupported fields and values are rejected in each
schema; in particular, v2 rejects `credential_source` as unknown.

Accepted v1 and v2 files map only in memory to credential source `environment`;
v1 also projects provider `vercel_ai_gateway`, transport `ai_gateway_http`, and
model `zai/glm-5.2`. Their observable `schema_version()` values stay `1` and
`2`. Loading does not migrate or rewrite either file.
`NativeConfig` exposes `schema_version`, `permission_mode`, `provider`,
`transport`, `model`, and `credential_source` getters and owns the bounded model
string, so it and `LoadedNativeConfig` are cloneable but not copyable. Debug
output redacts the model and may show the non-secret acquisition kind.
`CONFIG_SCHEMA_VERSION` is `3`, `AI_GATEWAY_DEFAULT_MODEL` is `zai/glm-5.2`,
and `AI_GATEWAY_MAX_MODEL_BYTES` is `128`. The config model and AI Gateway
provider use one visible-ASCII model validator.

The closed `NativeProviderKind::VercelAiGateway` and
`NativeTransportKind::AiGatewayHttp` values expose the stable names
`vercel_ai_gateway` and `ai_gateway_http`. These values are declarative. In
particular, the HTTP enum is present when the optional concrete
`ai-gateway-http` feature is disabled and on WebAssembly, where that transport
is unavailable; valid config does not prove a usable transport implementation.
`NativeCredentialSourceKind::Environment` likewise exposes only the stable
non-secret name `environment`; it cannot carry a token or arbitrary environment
name and does not make the loader read process state.

On the supported Unix targets exercised by Milestone 03, the loader opens the
final path no-follow and nonblocking. A preliminary path-kind check is followed
by authoritative opened-descriptor regularity validation. The loader retains at
most the 64 KiB cap plus one byte and never writes, creates, or canonicalizes.
Hardened open semantics for non-Unix targets remain deferred. Typed diagnostics
distinguish failure classes without reflecting selected paths, file contents,
model values, or operating-system error text. Credential bytes remain in no
schema. The separate twelfth slice consumes this already loaded value without
changing the loader: file-backed v1 and v2 values remain observable as their
original versions while projected values drive composition. The production
constructor validates configured `Environment` before using the already
injected credential snapshot. Its runtime `credential_source()` remains the
concrete selected OIDC-token or API-key source; the custom-transport override
skips discovery and reports `None`. Configuration mutation or migration,
permission modes beyond `ask`, a concrete prompt UI, runtime ownership, token
fields in config, CLI composition, the native tools beyond bounded `read_file`,
`list_files`, delivered `file_info`, delivered `glob_files`, and candidate
`grep_files` library
capabilities, session
migration/encryption, CLI expansion, and composed release-binary end-to-end
evidence remain open. The by-ID lifecycle, session-listing extension, and
`file_info` are delivered; the broader native-tool inventory remains open.

```text
                        machine-god-core
 host ---------------------------------------------------------------+
  |                                                                  |
  +-> ModelProvider ----+                                             |
  +-> SessionStore -----+-> Engine -> Session -> Turn event stream    |
  +-> PermissionHandler +                  |                          |
  +-> Tool(s) ----------+                  +-> TurnHandle/cancellation|
  +-> EventSink (optional observer)                                   |
                                                                     |
 native filesystem / process / network authority remains outside ----+
```

An engine requires explicit provider, store, and permission components. Event
observation may use the authority-free no-op sink. Validated IDs, explicit
durable session-incarnation IDs, structured component errors, optimistic session
revisions, monotonic event sequences, one-live-turn session leases, and
idempotent cancellation form the initial cross-component invariants.

The twelfth slice composes one exact selection of those existing
boundaries. Both synchronous constructors first validate permission mode `ask`,
provider `vercel_ai_gateway`, and transport `ai_gateway_http`. They then open
one workspace directory once and clone its retained descriptor so exactly the
`list_files` and `read_file` tools share one opened directory identity. A
separately supplied existing session root is opened through `FileSessionStore`. The
injected `PermissionPrompter` is wrapped by `AskPermissionHandler`, and
`AiGatewayProvider` receives the loaded config model and either the production
or custom transport. `EngineBuilder` receives no explicit limits or event sink,
so the documented defaults and `NoopEventSink` apply.

The thirteenth slice extends that first validation stage to require
`NativeCredentialSourceKind::Environment`. It does not change constructor
arguments: the production path still consumes the host-injected snapshot, and
the custom transport remains an explicit trusted authority override.

The delivered seventeenth slice extends the composed workspace bundle from two to
exactly three descriptors of one retained directory identity: the original and
two clones. Both path and prepared-root constructors supply exactly `file_info`,
`list_files`, and `read_file`; core exposes that catalog alphabetically. Its metadata policy and
execution receive the same normalized path. This slice does not change
provider, permission, session-store, credential, transport, runtime, or CLI
authority. Production is present at `5c2d129` / composed `1d93a65`;
independent tests compose at `f228c06`, where all 34 initial focused tests are
green. Review hardening brings the focused total to 36 plus five private unit
tests at `b69ec4b`. All three replacement tracks are green on exact candidate
`4193ecc`. Seal and integrated SHA `60dd54f273afc7e62fb4b3cc1fb1a347d739998b`
is green under exact feature CI `32605071080` on successful retry attempt 2,
feature benchmark evidence `32605071063`, main CI `32606050292`, and main
benchmark evidence `32606050294`; all four report the exact seal SHA.

The delivered eighteenth slice extends the same workspace bundle from three to four
descriptors of one retained identity: the original and three clones. Both path
and prepared-root constructors supply exactly `file_info`, `glob_files`,
`list_files`, and `read_file`; core exposes that catalog alphabetically. Its
recursive-enumeration policy and execution receive the same normalized selected
subtree, while exact prepared pattern/path/mode arguments attenuate only the
output. No provider, permission handler, session-store, credential, transport,
runtime, constructor argument, root-selection, or CLI authority changes.
Production, independent tests, documentation, and composition are present. The
first review at `1f5de6a` found a matcher-work bound defect; its fix, regression,
and replacement local gates are green at `4171a4a`. All three same-SHA
replacement formal tracks are green at `523df858`. Documentation seal and
integrated `main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed
exact feature CI `32610950593`, feature benchmark evidence `32610950594`, main
CI `32611208411`, and main benchmark evidence `32611208415`; all four report
that exact seal SHA.

The nineteenth candidate extends that workspace bundle from four to exactly
five descriptors of one retained identity: the original plus four clones.
Both path and prepared-root constructors supply exactly `file_info`,
`glob_files`, `grep_files`, `list_files`, and `read_file`; core exposes that
catalog alphabetically. `grep_files` preflight and execution agree on the exact
normalized `SearchContent` selected path and all eight canonical request
values. Literal/include/mode/pagination/context values attenuate output but do
not substitute for subtree content-search authority. No provider, permission-
handler, session-store, credential, transport, runtime, constructor argument,
root-selection, or CLI authority changes. Exact base and kickoff are
`f6aa458` and tree-identical `f6ab594`. Exact isolated components and focused
production/test composition are named above; documentation component `b04151a`
produces fully composed behavior `42e4793`, with lint fix and local gates green
at `45ad91f`. All three first-cycle tracks are **NOT GREEN** on exact
`355a11a`. Remediation and exact replacement local gates are green at final
code/test precursor `275d263`. First replacement candidate `ae87bf1` is **NOT
GREEN** across all three tracks. Second-fix production and documentation compose
through `ac5d772`, `d672210`, `7ad0863`, and exact local-gate-green precursor
`b498ba0`. Formal second replacement candidate `5aeddc1` has correctness/API
and filesystem/robustness **GREEN** with zero findings and
performance/concurrency **NOT GREEN** with one medium allocation-amplification
finding and two low documentation/evidence findings. Third remediation composes
through `8777825`, `ab1c133`, `dcf57ad`, `d7526d4`, `44afb23`, `f08c5f2`, and
`1f13f9a` at exact fully composed local-gate precursor `a8f6179`. Exact Rust
1.94.1 formatting, warnings-denied workspace Clippy, 598 non-documentation tests
plus two doctests, 25 private native tests, 40 direct tests, four engine tests,
and diff checks are green. Exact a8f cross-target/dependency/link and
compatibility/release validators are green. Formal third-cycle candidate
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

Slices twenty through twenty-four extend the same workspace bundle with
delivered `write_file`, `edit_file`, `delete_file`, `rename_file`, and
`copy_file`. The last delivered host registers exactly ten
tools in alphabetical order: `copy_file`, `delete_file`, `edit_file`,
`file_info`, `glob_files`, `grep_files`, `list_files`, `read_file`,
`rename_file`, and `write_file`. One tool consumes the originally retained
workspace descriptor and the other nine receive
identity-preserving clones; both path and prepared-root constructors therefore
give every tool the same opened workspace identity. These later slices do not
change provider, permission-handler, session-store, credential, transport,
runtime, root-selection, or CLI authority.

The delivered twenty-fifth slice inserts `create_folder` immediately after
`copy_file`, producing eleven alphabetical tools from the original retained
descriptor plus ten identity-preserving clones. Preparation uses the
existing provider-neutral `Capability::Filesystem { access: Create, path }`
with the exact canonical path that approved execution receives. The complete
contract and local-gate evidence are in
[`create-folder.md`](create-folder.md).

The delivered twenty-sixth slice inserts `open_file` after `list_files`,
producing twelve alphabetical tools from the same retained workspace identity:
one tool consumes the original retained descriptor and the other eleven receive
identity-preserving clones. Its strict path-only preflight prepares dedicated
`Capability::OpenFile { path }`; approved Linux execution opens and retains one
existing regular-file descriptor without following a model-selected symlink.
The fixed system launcher receives only `/usr/bin/xdg-open` and the parent-owned
`/proc/<parent-pid>/fd/<retained-fd>` target, with fixed `/` working directory,
null stdio, and no shell or `PATH` lookup by machine-god. The same twelve-tool
catalog composes on macOS, where active `open_file` execution returns
unsupported before filesystem lookup, worker creation, or helper spawn. This is
the delivered twelve-tool/eleven-clone host composition.

The composition does not compare the two roots for equality or ancestry. The
trusted host must keep them disjoint; otherwise the bounded workspace tools can
reach session artifacts beneath the workspace after permission is granted.

The integrated fourteenth slice leaves those existing path constructors
unchanged and adds a stricter prepared-root path.
`NativeRootSelection::from_environment`
derives the fixed state path from injected values without I/O.
`PreparedNativeRoots::prepare` opens the workspace first, requires the selected
state base to exist, retains no-follow descriptors while walking or creating
only the fixed suffix, validates existing state-directory ownership and modes
without repair, accepts only empty or exact flag-free deny-delete macOS ACLs
and rejects every other ACL or descriptor ACL-read failure (including on a
newly created suffix after normalization), and rejects
retained-root equality or ancestry by device/inode identity and
descriptor-relative parent walking. New
`NativeReferenceHost` constructors consume that prepared value and transfer its
workspace identity to the registered workspace tools and its state-root identity
to `FileSessionStore` without reopening either path. The delivered seventeenth
slice extends that transfer to three tools; the delivered eighteenth slice
extends it to four; the nineteenth extends it to five; slices twenty through
twenty-three extend it to the delivered nine-tool bundle; and the delivered
twenty-fourth slice extends it to ten tools using the original descriptor
plus nine clones. Config
selection is validated by the composing constructor, and production credential
discovery remains after the already prepared retained roots are accepted.
The composed `create_folder` source does not alter these delivered counts; it
extends both constructors together to eleven tools and ten clones without
reopening the root. Cycle 2 was not green on two low evidence/documentation
findings. Exact remediation `f527293`, tree `40eef14`, passes the complete
replacement local gate. Cycle-3 candidate `c1e572e`, tree `b5fb1c2`, is not
green only for one low documentation-lineage finding. Exact lineage remediation
`12c11ba`, tree `b96575b`, passes the complete replacement gate. Gate record
`f6f6584` parents tree-identical cycle-4 candidate `a78b693`, tree `2b913e8`;
cycle 4 found zero production defects and its sole low stale seal-record finding
is corrected in the exempt documentation seal. First feature CI `32699750602`
then exposed one Linux-only test Clippy failure; platform-native `RawMode`
remediation `1effcbb`, tree `b5eccb1`, passes the complete replacement gate and
tree-identical cycle-5 candidate `ff18a9a`, tree `f77b198`, is green with zero
findings in all three fresh tracks.
Production and focused tests for this behavior are present. Formal adversarial
review was green on
exact behavior SHA `f1dc4751`; after the post-review Linux lint normalization at
`90d8f96`, all three final tracks are green on exact candidate `72cf64f6`.
Replacement seal `f08dbd9e`, feature record `6f66b6e5`, and exact `main`
delivery evidence are green.

The delivered `open_file` slice extends both constructors together to twelve
tools and eleven clones without reopening the workspace root.

On the production path, both non-secret roots open before the consumed
credential snapshot is discovered and its token moves into
`AiGatewayHttpTransport`. On the custom path, the injected transport is a
trusted authority override: no credential discovery or production HTTP
construction runs, and the retained credential-source observation is `None`.
That absence says nothing about authority or secrets encapsulated by the custom
transport. The production path instead retains only `Some` selected-source
metadata and exposes no bearer-token getter.

Construction opens roots and creates bounded component values synchronously but
makes no network request, prompts no user, touches no session record, creates
no root or runtime, and starts no background task. Production HTTP work later
must be polled on a live host-owned Tokio runtime with I/O and time enabled.
The wrapper retains the exact `LoadedNativeConfig`, including file-backed v1
and v2 origins and observable versions while using their projected provider,
transport, model, and credential-source values. Its fixed debug form exposes no
config structure or source. Every nested construction error is reduced to one
fixed redacted stage in the non-exhaustive reference-host error taxonomy. The
twelfth-slice composition behaviors are adversarially green and integrated on
`main`; the schema-v3 extension and credential-source validation are integrated
through final record `f840576a`, with all three fresh adversarial tracks green
on exact behavior SHA `35ce591e` and exact final-record feature and `main`
workflows green. Root preparation was adversarially green on exact behavior SHA
`f1dc4751`; after Linux lint normalization at `90d8f96`, all three final tracks
are green on exact candidate `72cf64f6`. Replacement seal `f08dbd9e` and feature
record `6f66b6e5` are feature-green, and the latter is integrated under exact
green `main` workflows.

The eighth slice is `machine-god-native::FileSessionStore`. On supported
Linux and macOS Unix targets, its host supplies one existing absolute root. The
constructor opens the final root no-follow, verifies a directory, and retains
that descriptor; it performs no environment discovery or root creation. Flat
record, permanent advisory-lock, and temporary names are derived by lowercase
SHA-256 over the fixed ASCII domain separator
`machine-god:file-session:v1:` followed by the `SessionId` UTF-8 bytes. The hash
keeps raw IDs out of filenames but is neither content secrecy nor confinement;
descriptor-relative access beneath the trusted host root provides the latter.

Records use strict schema-v1 compact JSON envelopes and are limited by
`MAX_FILE_SESSION_BYTES` (`8_651_165`), enough for every record satisfying
default `EngineLimits`. Loads retain only that cap plus one overflow witness,
open no-follow and nonblocking, and require an authoritative post-open regular
file check before decode. They verify the requested record ID and return
`None` for an absent record without creating artifacts. Corrupt, oversized,
wrong-ID, symlink, and other nonregular state fails closed and remains in place.

Saves serialize within the cap and perform new/update compare-and-swap under a
permanent per-session regular no-follow advisory lock. They preserve the
incarnation, assign revisions with checked arithmetic, write an exclusively
created `0600` temporary regular file, synchronize it, rename it atomically
over the record in the same retained directory, and then synchronize the
directory. The lock coordinates cooperating processes only. A directory-sync
failure after rename is ambiguous because the new record may already be
visible; the caller must load and reconcile. The atomicity claim is per record
on filesystems honoring the assumed Unix lock, rename, and sync semantics, not
a multi-record, NFS, hostile-writer, or complete sudden-power-loss guarantee.

Store futures are effect-free until first poll and detach no background work.
The first poll performs bounded synchronous serialization, I/O, advisory-lock
acquisition, and sync work inline and can block the executor thread. Full
format, polling, error, trust, and deferred-scope details are in
[`session-store.md`](session-store.md).

The delivered fifteenth slice adds `NativeSessionLifecycle` above that exact store
without changing core's provider-neutral `SessionStore` trait. The engine,
lifecycle, and `NativeReferenceHost::session_store()` observation share the
same store allocation and retained directory descriptor. Lifecycle
construction performs no entropy or persistence effect. Each create, resume,
replay, or reset future is inert until first poll and inherits the store's
bounded synchronous polling behavior.

Every lifecycle constructor verifies exact shared `Arc` identity between its
concrete `FileSessionStore` and the store configured in its engine. Two stores
opened from the same path are still distinct authority objects and are rejected
as `MismatchedSessionStore` before the incarnation source or filesystem is
consulted. Reference-host composition wires one allocation by construction and
maps an impossible internal mismatch to its existing redacted engine stage.

Create takes a caller-supplied validated `SessionId`, obtains a production
incarnation from a fixed-size OS cryptographic-random draw, and atomically
persists the empty current record before returning a live session. Resume loads
and validates the current record and converges on the engine-canonical local
state for the same incarnation. Replay returns an owned durable
`SessionRecord` snapshot; it does not register a live session or reconstruct
events, UI state, transport chunks, permission decisions, or external effects.

Reset rejects a locally live incompatible lifetime before record replacement,
then atomically replaces one validated current record under its exact
ID/incarnation/revision fence. The preceding load may create the fixed lock
sidecar and the bounded incarnation source may already have been consulted.
The session ID is unchanged, the incarnation is new, the durable revision
advances with checked arithmetic, and `next_turn_sequence`, messages, and
metadata become `1`, empty, and empty. The file is never deliberately missing
between lifetimes. Cross-process old handles are not revoked, but the new
incarnation fences their later saves. A post-rename directory-sync failure is
ambiguous and requires resume/replay reconciliation rather than blind reset
retry. Complete concurrency, entropy, resource, and redaction rules are in
[`native-session-lifecycle.md`](native-session-lifecycle.md). Formal review and
delivery are green through behavior candidate `e6a3804` and feature record
`dbba2c7`.

The delivered sixteenth slice adds one independent bounded observation
future to that lifecycle. It processes/selects at most 1,024 non-dot entries
plus one fetched/name-inspected overflow witness, and accepts/decodes at most
64 MiB of aggregate canonical record bytes plus one transient transfer byte to
detect concurrent growth. It validates exact candidates with the store's same
per-ID lock and current-schema rules and returns at most 100 sorted unique IDs.
Canonical filenames are sorted before validation; only a fired raw scan
cap makes candidate selection filesystem-iteration-dependent. It has no
directory-wide lock or multi-record snapshot: per-candidate observations may
occur at different instants, and a candidate that vanishes before its locked
read may be omitted while leaving a private lock sidecar. A nonregular derived
lock for a present canonical record is corrupt; ordinary lock I/O is
unavailable. `truncated` reports incomplete bounded observation only, not a next
page or globally first ID prefix. The replacement macOS path acquires its fresh
`.` descriptor first and then validates that exact acquired linked identity.
A stable completed rename remains valid; removal before acquisition or checking
is unavailable; and concurrent rename/removal may conservatively fail or
observe the acquired identity without a global snapshot. `NativeSessionList`
and its `Debug` deliberately expose IDs; only lifecycle error `Display` and
`Debug` are ID/path/content-redacted. An unpolled
future is inert; first poll performs bounded synchronous filesystem and lock
work and detaches nothing. Full bounds, corruption behavior, authority limits,
and deliberate non-features are in
[`native-session-listing.md`](native-session-listing.md). Production and the 13
initial independent tests are composed through `dec98e0`; all three first
formal tracks were not green. The fixes and 18-test hardened suite are composed
in exact behavior candidate `3fa5463`; all three replacement tracks are green.
Portable behavior `17f1884` and seal `d3312d7` passed exact feature and `main`
delivery gates.

The ninth slice is `machine_god_native::AskPermissionHandler`. It adapts
core's existing provider-neutral `PermissionHandler` to an explicitly injected,
object-safe `PermissionPrompter`. `new` accepts an owned concrete prompter;
`shared_prompter` accepts an `Arc<dyn PermissionPrompter>`. The native adapter
selects no executor and owns no terminal, UI, environment, filesystem, process,
network, configuration, or persistence authority.

For an engine-driven authorization, core first constructs and bounds the full
auditable `PermissionRequest` and emits `PermissionRequested`. The adapter then
forwards its owned request by value without cloning, mutation, serialization,
truncation, revalidation, or traversal. The injected prompter returns one of
four structured outcomes: allow once, allow for the turn, allow for the
session, or deny. The adapter maps those values to the corresponding core grant
scope or the fixed reason `permission denied`. These grant scopes are reported
decisions only; neither core nor the adapter caches them for later requests.

Prompt failure is fail-closed. The zero-data `PermissionPromptError` cannot
carry host diagnostics, and the adapter returns only
`permission_prompt_failed` / `permission prompt failed`. Construction of the
authorization future is inert. Its first poll invokes the injected prompter
exactly once; dropping a pending authorization drops the prompt future. The
adapter detaches nothing and has no separate cancellation signal, so a
conforming prompter must keep its work owned by that future or clean it up on
drop. A concrete prompt UI, CLI composition, grant persistence, and permission
modes beyond `ask` remain outside this slice. The complete contract and
delivery evidence are in [`ask-permission.md`](ask-permission.md).

The tenth slice is a separate `ai-gateway-http`-gated credential adapter.
`AiGatewayCredentialEnvironment::new` accepts owned injected `OsString`
values; `from_process` snapshots only `VERCEL_OIDC_TOKEN` and
`AI_GATEWAY_API_KEY`. Discovery consumes the snapshot, treats only an exactly
empty value as absent, and selects a nonempty OIDC token before a nonempty API
key. A selected non-Unicode value is an invalid-environment error. A selected
Unicode value moves through `AiGatewayBearerToken::new`, so the existing exact
1–4,096-byte RFC 6750 syntax remains the sole validator. Invalid higher
precedence fails closed rather than falling through.

The snapshot can retain at most two validated 4 KiB values; the selected token
moves into `DiscoveredAiGatewayCredential`, and the unused token is dropped.
Snapshot, result, token, and error formatting do not reflect credentials.
Process lookup may materialize a complete OS value before application
validation, and clearing remains best-effort rather than a locked-memory or
complete zeroization claim. The exact contract and delivery evidence are in
[`ai-gateway-credentials.md`](ai-gateway-credentials.md).

The first concrete provider remains on the native side of core's explicit
boundary but owns no network effect. `AiGatewayProvider` encodes the supported
`ModelRequest` projection and decodes pinned protocol `0.0.1`, language-model
specification `4` data-stream bytes. An `AiGatewayTransport` supplied by the
host receives the owned body, fixed protocol/model/session headers and the turn
cancellation token, then returns an executor-neutral byte stream. Endpoint
selection, HTTP, DNS, proxies, TLS, authentication, status validation, timeout
and retry policy all remain outside the codec:

```text
trusted host authority
 endpoint + credentials + network/status/retry policy
                       |
                       v
             AiGatewayTransport
                       |
               bounded byte stream
                       v
              AiGatewayProvider
       request projection + stream codec
                       |
                       v
 machine-god-core ModelProvider / ModelEventStream
```

Provider construction fixes a nonempty default model, injected transport and
independent resource limits. A request-level model may override that default.
The body contains only `prompt`, `tools`, `toolChoice`, and optional
`maxOutputTokens`; temperature and inference metadata have no wire projection,
are structurally validated where applicable, and are then ignored and omitted.
Fixed metadata carries content type, protocol/specification versions, model,
streaming mode and the same core session ID for both session and affinity.
Machine-god adds no endpoint, authorization, referer, title, or user-agent and
makes at most one transport call without codec-side retry: exactly one only
after a valid request future is polled through startup.

The transcript projection accepts system/user text, assistant text and complete
tool calls, and complete tool results whose name is resolved from the
immediately preceding assistant calls. JSON blocks and structurally invalid
tool histories fail before transport. Response chunks may split delimiters,
UTF-8 and JSON arbitrarily. The codec recognizes the pinned single-`data: ` line
record shape while bounded blank, comment, non-data and unknown-event records
are no-ops. It incrementally reconstructs local tool inputs and yields only
complete text, reasoning, tool-call, usage and stop events. Malformed known
schemas, conflicting, duplicate, provider-executed, incomplete, post-finish and
over-limit input fails closed. `[DONE]` and EOF are not finish proof; exactly
one valid finish produces exactly one stop. A final call whose ID differs from
its provisional stream identity reconciles only through one unique ended input
with the same tool name and structurally equal explicit JSON input. The bounded
canonical index normalizes signed floating zero. An authoritative exact-ID
final can replace invalid or unfinished provisional input; a tombstone safely
absorbs later bounded delta/end records for that finalized provisional ID.

Both startup and response parsing are poll-driven. Cancellation is checked
around encoding and transport startup and between chunks, records and yielded
events, and wins when a terminal result becomes ready in the same poll. The
codec registers a cancellation wakeup, and the transport receives the same
token so it can wake while its future or byte stream is pending. Empty chunks
fail, while a nonempty no-event chunk consumes at most one unit of source work
per poll before scheduling another poll and yielding. Ready stream outcomes
deregister the codec's cancellation waiter; only a pending poll retains it.
Drop owns and destroys the in-flight transport future/stream and partial decode
state. Guarded request
JSON is iteratively drained on unpolled, cancelled, and rejected paths, so depth
rejection does not cause recursive teardown; accepted JSON is first proven to
be within the safe depth ceiling. No provider task, timer, thread or retry is
detached. The normative projection, limits and redacted failure behavior are in
[`ai-gateway.md`](ai-gateway.md).

The optional `ai-gateway-http` Cargo feature provides a native, Tokio-hosted
`AiGatewayHttpTransport` implementation without changing the codec or core.
Construction remains effect-free, while the concrete transport future and
stream require a host-owned Tokio runtime with I/O and time enabled. That
runtime must remain driven through asynchronous connection teardown. Core, the
codec, and custom transports remain executor-neutral.
Its production endpoint is the fixed
`https://ai-gateway.vercel.sh/v3/ai/language-model`; the host must inject a
1–4,096-byte RFC 6750 bearer `b64token`. The only alternate endpoint API is an
explicit test-only plaintext constructor restricted to numeric IPv4 loopback
in `127.0.0.0/8` or IPv6 `::1`, with an explicit port and absolute path and no
userinfo, query, fragment or alternate IP spelling. The port must be nonzero.
Arbitrary production URL selection and ambient credential lookup are therefore
absent. Alongside the codec metadata, the transport adds only that authorization
value, `Accept: text/event-stream` and `Accept-Encoding: identity`, apart from
required HTTP framing headers.

The transport owns one Reqwest client configured with pinned WebPKI roots,
proxies and redirects disabled, no response decompression or cookie engine, no
application/status retry, and HTTP/1 only. Hyper may recover a stale reused
connection only before writing request bytes; it never replays a possibly
peer-visible request. The transport defaults to at most 16 active requests, a
30-second connection timeout and a 10-minute total request/stream timeout.
Validated custom limits allow 1–64 active requests, a
positive connection timeout no longer than 5 minutes, and a positive total
timeout no longer than 1 hour and no shorter than the connection timeout.
The total deadline starts before bounded-capacity acquisition and includes that
wait. Same-endpoint idle connections may be reused. Dependency frames are split
to public chunks of 64 KiB by default, with a validated configurable ceiling no
larger than 1 MiB; this does not bound Hyper's internal frame allocation. The
codec independently enforces its own per-chunk,
record, buffer and aggregate response limits.

Only status 200 yields a byte stream. Fixed redacted provider errors classify
401/403 as non-retryable authentication, 429 as retryable rate limiting,
408/425/5xx as retryable unavailability, other 4xx as non-retryable invalid
requests, and 3xx or every other non-200 response as a non-retryable protocol
failure. Neither response error bodies and headers nor Reqwest/Hyper/Tokio/Rustls
diagnostic text cross this boundary. Cancellation is observed before dispatch
and throughout pending capacity, upload, response-head and response-body work.
Dropping or cancelling the future or stream drops the owned in-flight
request/body and active-request permit. Machine-god creates no internal runtime
or producer task; Reqwest/Hyper owns connection-dispatch tasks on the host
runtime, so that runtime must keep advancing through socket teardown. No retry
or timer is detached by machine-god. The complete feature, API,
platform and security contract is in
[`ai-gateway-http.md`](ai-gateway-http.md).

The multi-round turn loop is an executor-neutral future polled inline by the
`Turn` stream. A one-event acknowledgement gate connects it to observer
delivery: orchestration cannot advance past a nonterminal event until the sink
accepts that event and the caller receives it. No task, channel, timer, or
runtime-specific primitive is required. Provider, store, policy, and tool
futures stay owned by that orchestration frame, so cancellation or drop tears
down the in-flight phase without detached work. Immutable tool specifications
are cached in deterministic name order when the engine is built and cloned into
each provider request.

Durability divides each loop into explicit phases:

```text
user+turn reservation -> model
model tool calls -> atomic assistant + N unknown placeholders commit
                 -> effect-free tool preflight -> bounds -> permission
                 -> allowed tool -> exact in-place result replacement -> model
model final answer -> assistant commit -> terminal events
```

Tool preflight is a provider-neutral transformation before policy. The
source-compatible `Tool::prepare` default returns the provider's original JSON
arguments with the existing raw `Capability::Tool`. A tool may instead return a
`PreparedToolCall` containing a normalized capability and replacement arguments.
Preparation is synchronous trusted-host code and must be deterministic,
bounded, nonblocking, and free of external effects. Core checks cancellation
immediately before and after the call; it cannot interrupt preparation in
flight. Core validates the prepared arguments at the exact configured tool
argument-byte limit, including their JSON depth and node bounds. For a prepared
capability, depth and node traversal covers only JSON values embedded in its
`Tool` or `Custom` variant. Every variant is also serialized as a whole under
one total byte cap of the configured argument limit plus 1 KiB. That fixed 1
KiB is headroom within the total capability cap, not a separately metered
envelope. Core then presents the prepared capability to policy and passes
exactly the prepared arguments to `Tool::execute` only after policy allows the
request.

The trusted tool must ensure those arguments can drive only effects contained
by the exact capability that policy authorized. Filesystem, process, and network
implementations must not reinterpret normalized arguments into a broader path,
command, or destination. This obligation keeps authorization and execution
about the same normalized operation without giving core semantic knowledge of
tool JSON or ambient operating-system authority.

The native `read_file` tool is constructed with one explicit absolute workspace
root. On supported Unix targets construction opens the final root directory
without following a final symlink and retains that directory authority. Pure
preflight performs no filesystem lookup: it strictly decodes one path string,
bounds it to 4,096 UTF-8 bytes, and lexically normalizes or rejects its
components. The resulting `Capability::Filesystem(Read)` path and prepared
execution path are the same normalized workspace-relative string.

After policy allows that capability, execution walks from the retained root
with descriptor-relative opens. Every component is opened no-follow, each
ancestor descriptor remains stable for the next lookup, and the final opened
descriptor is authoritatively required to be a regular file. Nonblocking open
flags keep a substituted special file from hanging the traversal. The reader
retains at most 8 KiB plus one byte, rejects invalid UTF-8 rather than encoding
arbitrary bytes, and returns exactly a JSON object containing `content` on
success. Cancellation is checked before traversal, between bounded reads, and
after validation; it cannot preempt one operating-system call already in
flight. No background task is detached.

The native `list_files` tool is likewise rooted in an explicitly supplied
absolute host path whose opened directory descriptor is retained. Its
effect-free preflight accepts only `{}` or an object whose sole field is a
string `path`; omission selects `.`. The 4,096-byte lexical path
rules match the confined Unix spelling rules of `read_file`, including rejection
of absolute paths, parent components, controls, and bidirectional-formatting
characters. Backslash and space remain literal Unix filename characters. The
prepared `Capability::Filesystem(Enumerate)` path and execution argument are the
same normalized workspace-relative string.

Allowed execution walks to exactly one selected directory with
descriptor-relative directory and no-follow requirements. It enumerates only
that directory's immediate entries and obtains `file`, `directory`, `symlink`,
or `other` solely from each entry's reported type; an unknown type is `other`.
It does not recurse, open children, read content, apply ignore rules, resolve
symlink targets, inspect external paths, or discover a workspace. Only `.` and
`..` are skipped. Every returned name is safe valid UTF-8.

The tool retains at most 100 entries and 16 KiB of aggregate raw entry-name
bytes, then reads the first extra visible entry needed to establish truncation.
It sorts only the retained subset. A truncated subset can therefore depend on
filesystem iteration order, and the result makes no whole-directory ordering or
snapshot claim. The structured output is exactly `{path, entries:
[{name, kind}], truncated}`. Its conservative maximum serialized size is 44,101
bytes for the structured content and 44,130 bytes including core's fixed
`ToolOutput` envelope, below the default 64 KiB per-result limit. The normative
behavior, fixed redacted errors, and cancellation boundaries are in
[`list-files.md`](list-files.md).

The delivered native `file_info` tool uses one required path and the same
effect-free 4,096-byte lexical confinement as `read_file`. Successful preflight
produces `Capability::Filesystem(Metadata)` and prepared execution arguments
containing exactly the same normalized path; nonempty current-directory forms
normalize to `.`. Execution first acquires a fresh `.` descriptor and validates
that exact acquired linked identity under the platform-specific Linux/macOS
rules. It opens only ancestor directories descriptor-relatively with no-follow
requirements, then performs no-follow metadata lookup on the final component
without opening it. `.` uses final `fstat` after validation. Final symlinks
therefore report their own size and modification time, while FIFO, socket,
device, and unknown objects are safely classified as `other`.

The exact result is `{path, kind, size_bytes, modified: {unix_seconds,
nanoseconds}, extension}`. Size is checked nonnegative, Unix seconds remain
signed, and nanoseconds must be in `0..=999_999_999`. Extension is non-null only
for a regular file with a non-leading basename dot and nonempty last suffix;
`.bashrc` and `foo.` produce `null`, `.config.json` produces `json`, and
`archive.tar.gz` produces `gz`. One final `statat`, or `fstat` for `.`, supplies
all returned metadata fields, but no snapshot exists from preflight time and
continued existence is not promised. Path and extension remain jointly bounded
below 17 KiB after worst-case JSON escaping. Fixed redacted errors, root rename/removal
semantics, cancellation boundaries, and deferred scope are normative in
[`file-info.md`](file-info.md).

The delivered eighteenth native `glob_files` slice uses strict effect-free
`{pattern:string,path?:string,mode?:"matches"|"count"}` preflight. Requested
and normalized path and pattern forms are each bounded to 4,096 UTF-8 bytes.
Path confinement matches `file_info`; pattern normalization uses `/`, removes
repeated separators and exact `.` segments, rejects absolute/parent/empty or
forbidden forms, and treats backslash, brackets, and braces literally. Matching
is over UTF-8 bytes: `?` consumes one byte, `*` stays within one component, and
only an exact `**` segment spans zero or more components. Slash-free patterns
match basenames recursively; slashful patterns match candidate paths relative
to the selected search root.

Successful preflight produces `Capability::Filesystem(EnumerateRecursive)` at
the normalized selected subtree and exact prepared arguments with path and mode
defaults explicit. The pattern and mode restrict returned data but do not
broaden authority; recursive `EnumerateRecursive` remains distinct from one-
level `Enumerate`. Allowed execution applies `file_info` fresh-root liveness,
then fully reads, validates, and bytewise sorts every traversed directory before
iterative descriptor-relative no-follow processing. Hidden entries are
included. Regular files and final symlinks are candidates, directories are
traversal-only, specials are ignored, and neither symlinks nor content are read.

Both modes either complete or fail without partial output under 100,000 visited
non-dot entries, 16 MiB aggregate entry-name bytes, directory traversal depth
256, an 8,388,608-step aggregate matcher-work budget, and a 4,096-byte full
workspace-relative candidate-path bound. Matcher steps cover slashful candidate
splitting, pattern/DP cell visits, and component-byte matching. The selected
root is directory depth 0; directories through depth 256 are scanned, their
regular/symlink children remain eligible, and attempting to open a directory at
depth 257 is `scan_limit`. A `NOENT` race may omit an entry; other failures are
fixed and redacted. Stable-tree results are deterministic, but concurrent scans
are not snapshots.

Matches mode returns the longest globally bytewise-sorted prefix under 100
paths and 16 KiB aggregate raw path bytes; it omits the first nonfitting path
and every later path rather than backfilling, and `truncated` is true exactly
when an observed match was omitted. Count mode reports the exact count. The
complete matcher, result, scan, error, cancellation, compatibility, and deferred
contracts are in [`glob-files.md`](glob-files.md).

The delivered nineteenth native `grep_files` slice uses strict effect-free preflight
for required literal `pattern` and optional `path`, `include`,
`case_insensitive`, `mode`, `head_limit`, `offset`, and `context_lines`.
Prepared arguments contain all eight canonical values with explicit defaults,
and policy receives a distinct filesystem capability with
`FilesystemAccess::SearchContent` at the same normalized selected path. That
path can be a regular file or directory after approval. `SearchContent` owns
bounded enumeration plus content
inspection and is not inferred from `Read`, `Metadata`, `Enumerate`, or
`EnumerateRecursive`.

Allowed execution applies fresh-root liveness and opens all selected/traversed
objects descriptor-relatively without following symlinks. Directory entries
are completely validated and bytewise sorted; traversal includes hidden names.
Every full descendant path must pass its bound before allocation, entry-kind
handling, or include matching. The optional include is compiled once, and its
complete parse and matching work shares one aggregate counter. Fixed literal
pattern-table work is charged before selected-root resolution. Selected-file
filtering follows no-follow stat classification and precedes content open. A
slashful selected-file rejection is charged and cancellation-checked; an
excluded selected file consumes fixed pattern-table/include work but no
candidate, content-byte, or per-file matching work. An included selected file
opens and is revalidated before those latter budgets.
Stable specials are skipped; a raced nonblocking special open is authoritatively
rejected without read or link following. Content matching is
literal and worst-case linear, with exact bytes or ASCII-only folding, and
returns one record per matching line. Eligible files are complete observed
NUL-free valid UTF-8 no larger than 204,800 bytes; explicit aggregate counters
report oversized and non-text exclusions while other read failures fail the
call.

The complete scan is bounded by 100,000 entries, 16 MiB entry names, 10,000
candidates, 64 MiB content, 8,388,608 include compile/match steps, 268,435,456
content steps, depth 256, and 4,096-byte paths. List results retain no more than
head 100 and accept offsets through 67,108,864 so every emitted continuation is
reusable, and retain at most 8 KiB paths, 8 KiB excerpt/context text, and a
48 KiB serialized `ToolOutput`.
Match/context excerpts are UTF-8-safe and derive from one buffer; pagination
uses exact totals, `next_offset`, top-level list incompleteness, and distinct
per-record context truncation. A fired scan/work cap fails without partial
output. Line indexing checks cancellation at fixed byte intervals and each
serialized-size trimming attempt begins with a check. Slashful candidate
splitting checks at most every 1,024 candidate bytes, and both recursive and
non-recursive dynamic-programming branches remain cancellation-checked. Exact
shapes, public
constants, errors, cancellation and race semantics are in
[`grep-files.md`](grep-files.md).

The twenty-third slice adds `Capability::FilesystemRename { old_path,
new_path }` rather than trying to encode a two-endpoint move as single-path
`FilesystemAccess`. Strict effect-free preflight canonicalizes both required
paths and gives policy and execution the same pair. Allowed Linux/macOS
execution reacquires and validates the linked retained root, walks both
existing parents descriptor-relatively without following symlinks, validates
an existing regular-file source and absent destination twice, and performs
exactly one `renameat_with(..., RenameFlags::NOREPLACE)` after the final
cancellation check. It never retries that call, including after `EINTR`, and
never creates a parent, overwrites a destination, reads content, stages a file,
or falls back to copy-and-delete.

After a successful call, later cancellation is ignored while execution checks
the destination's original device/inode/type and performs bounded parent
durability work. A same-parent move syncs once; a cross-parent move attempts
source then destination even if the first fails, with at most 16 cumulative
`fsync` calls per unique parent including interrupted calls. Success returns
only the canonical old and new paths. A successful syscall followed by failed
verification or sync, and every `EINTR`, is a fixed nonretryable ambiguous
outcome. `NOREPLACE` closes destination replacement but portable rename offers
no source-inode compare-and-swap, so a final source replacement can be moved;
postcommit identity checking prevents false success but cannot roll it back.
The full limits, fixed redacted errors, cancellation points, and race boundary
are normative in [`rename-file.md`](rename-file.md).

The delivered twenty-fourth slice adds
`Capability::FilesystemCopy { source, destination }`, serialized as
`filesystem_copy`, rather than reducing a two-endpoint operation to one
`FilesystemAccess` path. Effect-free strict preflight canonicalizes the two
required paths and gives policy and execution the same pair. Approved
Linux/macOS execution reacquires the retained root, walks both existing parents
without following symlinks, accepts only a stable regular-file source and an
absent destination, and streams at most 16 MiB through one 64 KiB buffer while
computing SHA-256. It stages a private file in the destination parent, verifies
the source, stage identity, digest, mode, and ACL boundary, then makes one
`NOREPLACE` commit. Bounded postcommit destination verification and
destination-parent synchronization complete success; the source is never
removed or modified.

The operation creates no parent, overwrites no destination, accepts no
directory, symlink, special file, or external path, and never allocates the
whole source. Its 4,096-call I/O budget, interrupted-call bounds, fixed
redacted errors, cancellation boundary, and same-UID race limitations are
normative in [`copy-file.md`](copy-file.md). Exact feature and `main` delivery
gates are green on seal `3bdd7cb`; this makes no complete fx-equivalence or
performance claim.

The composed twenty-fifth slice reuses the single-path provider-neutral
`Capability::Filesystem { access: Create, path }`. Effect-free strict preflight
accepts only one required `path`, applies the mutation-path canonicalization and
exact 4,096-byte/256-component/65,536-byte argument bounds, and gives policy
and execution the same confined path. Core already owns the stable `Create`
access variant; native code owns all recursive filesystem semantics.

Approved Linux/macOS execution walks from the retained root without
following symlinks, accepts an existing final directory as idempotent success,
and rejects every selected symlink, non-directory ancestor, external path, and
existing final non-directory. It may make at most one `mkdirat` call per
missing component and 256 total, each requesting mode `0755`. Host umask and
ACL inheritance are effective policy: no chmod or ACL rewriting follows
creation. A hostile umask may make a new intermediate unopenable and leave a
partial prefix under an ambiguous result.

The first successful or uncertain `mkdirat` commits. No `mkdirat` is retried,
including after `EINTR`; later cancellation is ignored and no created prefix is
removed. Postcommit work retains the first-created-parent-through-suffix
descriptor chain, freshly rewalks the public path, and always attempts bottom-
up synchronization despite earlier verification or sync failure. The public
bound is 257 sync sites, 16 calls per site, and 4,112 total calls. No effect
means no sync. Path-only success, fixed redacted ambiguity, concurrent entry and
moved-parent limitations, and the no-sandbox boundary are normative in
[`create-folder.md`](create-folder.md). Current execution evidence is native
macOS plus Linux/FreeBSD cross-target test compilation, Linux library Clippy,
and WASI compilation/active unsupported behavior. Native Linux execution
remains pending exact feature CI. Cycle 2 remains historically not green; exact
remediation `f527293`, tree `40eef14`, passes the complete replacement local
gate. Cycle-3 candidate `c1e572e`, tree `b5fb1c2`, is not green only for one low
documentation-lineage finding. Exact lineage remediation `12c11ba`, tree
`b96575b`, passes the complete replacement gate. Cycle-4 candidate `a78b693`,
tree `2b913e8`, has zero production findings; its sole low stale seal-record
finding is fixed in the exempt documentation seal. First feature CI
`32699750602` then exposed one Linux-only test Clippy failure. Platform-native
`RawMode` remediation `1effcbb`, tree `b5eccb1`, passes the complete replacement
gate. Tree-identical cycle-5 candidate `ff18a9a`, tree `f77b198`, is green with
zero findings in all three fresh tracks; delivery, performance, and fx-
equivalence claims remain pending.

The delivered `open_file` boundary is distinct from content read
and arbitrary process execution. Core policy receives exactly
`Capability::OpenFile { path: "canonical/path" }`; native Linux code owns
retained-descriptor validation and the default-application launch. Its exported
trusted launcher request carries the exact approved path, proc path, and owned
target descriptor, while the launch trait requires an inert future and complete
helper cleanup on drop. Exactly 32 global system-launch permits bound worker
creation and remain held through callback completion and worker return; a
saturated attempt is precommit unavailable with zero new worker/helper. The
final spawn and cancellation/drop transitions share one serialized gate:
abort-first guarantees zero launch, while successful spawn
commits an effect that cannot be revoked. Postspawn cancellation invokes the
existing engine drop path. Cancellation, timeout, or explicit drop kills and
reaps the direct helper but cannot prove whether the default application already
received the file. Before publication, cleanup suppresses waking, reaps the
helper, drops the request/descriptor, and synchronously joins. Normal published
completion joins too. An inline or blocking arbitrary Waker may force handle
release to avoid self-join/cross-thread deadlock; only permit-bounded callback/
final bookkeeping remains after helper/request cleanup. This narrowly amended
lifecycle replaces the frozen absolute no-worker-detach clause and is exempt
from its own adversarial review under the owner's instruction. External paths,
directories, URLs, macOS real launch, CLI behavior,
benchmark work,
performance claims, and fx-equivalence are deferred. The product remains Rust;
Zig remains solely the pinned upstream benchmark build input. Formal cycle 3
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
recorded above. The final docs-only record is exempt from adversarial review
under the user's instruction but requires its own exact feature and `main`
workflows, which were still to be reported at that checkpoint. Subsequent
cycle-7 candidate `ea59490c28cc5edd339b3d48bffa39df37634f37`, tree
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

The retained roots confine model-selected components, but they are not sandboxes
against the hosts that selected a workspace path. Resolution of a root path's
ancestors and mount points beneath a retained root belong to that trusted host
boundary. Hardened construction and traversal beyond Linux and macOS remain
deferred for `list_files`, delivered `file_info`, delivered `glob_files`, and
delivered `grep_files`;
`read_file` retains its separately documented supported-Unix boundary.

A preparation error consults no permission handler and starts no tool. It
replaces the already-durable unknown placeholder with the same fixed generic
tool-error result used for an execution error, then permits the next model round
to recover. Permission request identity, critical risk, fixed reason, event
ordering, cancellation precedence, and the absence of core-side grant caching
are unchanged. The cancellation claim covers the immediate checks around the
synchronous preflight; preparation itself must not block because it cannot be
cancelled while running.

The transcript prefix is the optimistic-merge boundary. Allocator and metadata
changes may advance across a retry while messages remain identical; any message
change is divergence and fails closed. This preserves external allocator work
without guessing how concurrent conversation suffixes should be ordered.
Prompt, inference-option, session-metadata, tool-catalog, and complete-transcript
message/serialized-byte limits are checked before their corresponding build,
store, or model boundary. Every committed call therefore has exactly one result
message even if cancellation, an infrastructure error, or a process interruption
prevents its real result from replacing the conservative placeholder. The next
model round is not requested until all placeholders in the current round have
been replaced.

Every reachable `serde_json::Value` in these boundaries also passes an iterative
container-depth and node-count walk before recursive serialization or deep
cloning. A scalar root is depth zero and a root container is depth one; every
root, scalar, and container counts as one node. One node budget aggregates the
entire tool-schema catalog, all inference metadata, or all record metadata and
message JSON. Provider arguments and tool results are independently bounded.
The walk stores only the iterator for each active ancestor, never all pending
siblings, making its auxiliary memory proportional to configured depth and
stopping at node limit plus one. Provider tool arguments are rejected before
policy or execution. A tool result is checked after the effect but before
serialization and durable replacement; an over-depth or over-node result leaves
the precommitted unknown placeholder and terminates without replay.

The configurable depth is subordinate to the public hard safety ceiling
`MAX_SAFE_JSON_DEPTH` (64). Engine construction rejects a larger configuration
before catalog traversal or any provider, store, or policy call and never
clamps it. Iterative validation alone cannot make an accepted 50,000-level tree
safe for subsequent recursive serialization, cloning, retention, extension
components, and ordinary destruction, so this is a structural invariant rather
than an operator-tunable resource budget.

Owned rejection paths replace each hostile `Value` with `Null` and consume the
original through an iterative child-iterator stack before the surrounding
object is dropped. This applies to builder abandonment, duplicate replacement
and build errors; dropped unpolled prompts; direct and conflict record loads;
mutation candidates; yielded provider calls; and tool output after an effect.
Cancellation-aware poll boundaries drain a just-ready yielded provider event,
conflict-loaded record, or tool output before honoring cancellation.
Normally yielded provider events are guarded before model-event counting, so a
limit or counter failure also drains their arguments iteratively.
Reclamation is O(actual nodes) time with O(actual depth) auxiliary memory and
does not leak rejected trees. An item still queued inside a provider's stream
has not crossed the core ownership boundary, so stack-safe destruction of that
internal queue remains the provider's responsibility.

Canonical session state stores its record behind an immutable `Arc`. Reservation
and transcript mutation capture only that cheap identity and persistence bit
under the session mutex, then serialize and deep-clone outside the critical
section. Immediately before starting a store compare-and-save, core reacquires
the mutex and requires the exact record identity and persistence bit to match;
otherwise it retries from the new snapshot. A state change after that recheck is
still caught by the durable revision CAS. Reconciliation similarly performs
whole-record comparisons outside the mutex and uses pointer identity to recheck
before a constant-time state update.

A live turn owns the session lease and provider cancellation signal as one
lifecycle unit. Destruction signals cancellation before removing waiters and
releasing the lease; terminal completion has already released that unit, so its
later destructor is cleanup-only. Out-of-band observer or delivery-state failure
before a terminal provider outcome follows the same cancel-before-release rule,
preventing dropped streams from orphaning retained provider work.
Terminal establishment is the cancellation precedence boundary. A final model
stop remains preterminal through its assistant-message save so cancellation can
wake and release a turn blocked on persistence. If eager save construction
requests cancellation and returns a future whose first poll succeeds, or that
first poll itself requests cancellation and succeeds, durable success is
authoritative: core checked cancellation immediately before construction,
always gives the new future that one poll, and then runs reconciliation and
terminal establishment synchronously before the workflow yields to the outer
turn. A pending future receives cancellation prechecks on every later poll.
After the successful boundary, the stop
retains its pending observer delivery and final reason even if cancellation
races afterward. Provider failures and missing stops establish their terminal
outcome when accepted because they have no final assistant commit.
Provider startup, stream, persistence, policy, tool, and observer delivery polls
include a post-poll cancellation observation before their result is
interpreted. Cancellation observed there establishes the terminal outcome first
while the turn remains preterminal, except for that narrow ready successful
final-save boundary; pending or ready-error final saves remain cancellable. Only
a locally synthesized cancellation bypasses observer delivery; a
provider-originated cancelled stop follows normal durability and observer
ordering.
Cancellation treats wakers as user-controlled callback objects: cloning happens
before locking, registry mutation only moves values, and superseded, removed, or
drained wakers are dropped or invoked after unlocking.

Each `Engine` owns a weak session-state registry keyed by `SessionId`. All
create/load races inside that engine converge on one in-memory record and active
turn flag only if the persisted `SessionIncarnationId` also matches. A collision
between the same live session ID and a different incarnation fails rather than
merging logical lifetimes; a live turn itself keeps the state alive if its
originating session handle is dropped. This is an in-process coordination
boundary, not a distributed lease. Registry access uses one requested-ID
`BTreeMap` lookup rather than scanning all live sessions. The
last owner removes its weak entry during state destruction only when pointer
identity still matches, so dead keys are reclaimed without an old destructor
removing a concurrently installed replacement. Registry lookup holds the
entries mutex only through weak-reference upgrade or
new-state insertion. Existing-state identity validation runs after unlocking
while the upgraded strong reference preserves lifetime. An incarnation conflict
can thus drop the last state owner and reenter registration cleanup without
self-deadlocking on the entries mutex. Independent engines and
processes coordinate durable turn-number allocation through the session store's
optimistic revision contract. Loaded records reconcile strictly and
monotonically: corrupt sequences, stale revisions, and equal-revision divergence
are protocol errors, and completion of an older in-flight save cannot replace a
newer canonical record. Successful-save reconciliation also rejects divergent
records at the same revision. Session stores preserve a host-generated globally
unique incarnation for the entire logical record lifetime and reject a save
that changes it. Reset, rewind, or reuse of a session ID requires the host to
rotate the incarnation; core neither guesses legacy values nor acquires
randomness or clock authority. Model requests, permission requests, tool
contexts, and engine events carry that incarnation. The permission-request v2
digest binds it alongside session, turn, and ordinal identity to prevent an
ID-cached allow from crossing a reset; tool idempotency and event-sink
deduplication can use the same durable lifetime identity. Intrinsic load
validation precedes registry
publication, preventing a concurrent handle from retaining invalid persisted
state even when the originating load returns an error. Revision zero is an
in-memory unsaved sentinel only; persisted loads and conflict reloads require a
positive optimistic-concurrency revision. Missing conflict reloads may change
persistence status only with an exact snapshot comparison under the session-state
lock, while record revisions stay monotonic independently of that status flag.
The durable turn allocator is monotonic independently of both fields: a higher
revision cannot authorize a lower `next_turn_sequence`. Higher revisions may
advance conversation messages and metadata only after passing that allocator
guard, while equal revisions require whole-record identity.

Diagnostic formatting is also an authority boundary. `Engine::fmt` emits only
fixed structural state (`has_provider` and tool count); it never invokes the
provider's `name` method or copies provider-controlled text.
