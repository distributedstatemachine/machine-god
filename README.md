# machine-god

`machine-god` is an experimental, embeddable coding-agent engine written in Rust.

The engine is the primary product. The command-line application is its native
reference host. Development status, architecture, compatibility, security, and
performance evidence live in [`docs/`](docs/README.md).

Milestones 01 and 02 are complete, and Milestone 03 is in progress with
twenty-six delivered bounded slices. A proposed twenty-seventh `web_fetch`
slice is **IN PROGRESS** from exact base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e`. Production, independently owned
direct/engine/transport evidence, and rootless reference-host wiring are now
composed locally; the complete local gate, three fresh reviews, remote
workflows, integration, and delivery remain pending. The cfg-gated non-WASM
candidate adds a thirteenth host tool without changing the twelve descriptor-
backed workspace tools or their original-plus-eleven-clone ownership. Its
bounded public-HTTPS,
exact `Capability::Network`, DNS-pinning, no-redirect, untrusted-output, and
explicit-deferral boundary is frozen in the
[`web_fetch` candidate contract](docs/web-fetch.md). This changes neither the
delivered count nor M03 status and makes no product-performance or fx-
equivalence claim. The twenty-fourth, library-only
`copy_file` slice is green under three fresh same-SHA adversarial reviews and
exact feature and `main` delivery gates. The twenty-fifth, library-only
`create_folder` behavior is composed from that delivered base. Cycle-2
candidate `6e1f885`, tree `ac57575`, is historically not green: correctness/API
and performance/concurrency are green, while filesystem/robustness reported two
low evidence/documentation findings and zero production defects. Exact
remediation `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
Rust 1.94.1 local gate with the 17-private-test remediation inventory and
eleven-tool host wiring. Documentation record `9d0bacd`, tree `b5fb1c2`, and
tree-identical cycle-3 candidate `c1e572e` retain that non-documentation
behavior. Cycle 3 is not green: filesystem/robustness and performance/
concurrency are green with zero findings, while correctness/API reported one
low documentation-lineage finding and zero production defects. Exact lineage
remediation `12c11ba`, tree `b96575b`, passes the complete replacement gate.
Gate record `f6f6584` is the parent of tree-identical cycle-4 candidate
`a78b693`, tree `2b913e8`. Correctness/API and performance/concurrency are green
with zero findings; filesystem/robustness found zero production defects and one
low stale documentation-seal sentence, corrected under the user's explicit
seal-review exemption. Delivery remained pending, and the delivered count
therefore remains twenty-four at that historical checkpoint. Linux/macOS
native jobs in first feature CI `32699750602` are green, but that
workflow is not green because Linux Quality exposed a test-only `RawMode`
Clippy mismatch. Exact portable evidence remediation `1effcbb`, tree
`b5eccb1`, passes the complete replacement gate, including Linux test-target
warnings-denied Clippy. Cycle-5 candidate `ff18a9a`, tree `f77b198`, is green
with zero findings in all three fresh tracks. Seal `e75578b` passed exact
feature CI `32702785549`, feature benchmark `32702785574`, main CI
`32703303933`, and main benchmark `32703303931`. Both benchmark runs retain
exactly two nonexpired exact-SHA artifacts. Native `create_folder` is delivered
as slice twenty-five; at that checkpoint, the delivered host had eleven tools.
The twenty-sixth, library-only native `open_file` slice is delivered after a
green sixth formal review cycle. Its
dedicated provider-neutral
`Capability::OpenFile { path }` authorizes one strict canonical,
workspace-confined existing regular file. Linux execution retains the selected
file descriptor through the launch lifecycle and gives exactly
`/usr/bin/xdg-open` a `/proc/<parent-pid>/fd/<retained-fd>` target, fixed `/`
working directory, and null stdio. Machine-god selects neither a shell nor a
program from ambient `PATH`; the trusted `xdg-open` and desktop-dispatch
boundary may consult inherited host environment and configuration.
The exported Linux `OpenFileLauncher` seam accepts the same descriptor-bound
request. Its contract requires inert future construction and cancellation-aware
ownership. At most 32 production system launches are active; saturation is
precommit unavailable with no new worker/helper, and the permit is retained
through callback completion and worker return. Spawn and cancellation/drop
share one serialized gate: abort-first
guarantees zero launch, while a successful spawn commits an external effect
that cannot be rolled back. Cancellation, timeout, or explicit drop then kills
and reaps the direct helper. Prepublication cleanup suppresses waking, drops the
request/descriptor, and joins; normal postpublication cleanup also joins. An
overlapping inline or blocking arbitrary Waker cannot safely be joined, so the
handle is released after helper/request cleanup and only globally permit-bounded
callback/final bookkeeping may outlive future drop. This narrow docs-only
amendment replaces the frozen absolute no-worker-detach clause because legal
Waker behavior made it contradictory, and is exempt from its own adversarial
review under the owner's instruction. The delivered Linux/macOS reference host
registers twelve
alphabetical tools from one original workspace descriptor plus eleven clones;
macOS retains
the catalog entry but `open_file` execution is unsupported before lookup or
spawn. External paths, directories, URLs, a real macOS launcher, CLI changes,
benchmark changes, product-performance claims, and fx-equivalence remain
deferred. The
first formal review cycle rejected exact candidate `79e65c1`, tree `481fd7c`,
for cancellation-ordering, spawn-gate, reentrant-waker, and evidence-contract
findings. Cycle 2 rejected exact candidate
`027ba3367eb0853fec828ed0900398c7b7458e71`, tree
`9002e8f137d5ed2352cd620db6145da2339cdb2c`, with the complete resource,
deadline, lifecycle, invariant, proc-test, fake-launcher, and wait-seam finding
record in the review ledger. Cycle 3 rejected exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`. Correctness/API was green with
zero findings. Performance/concurrency reported one low because its deadline
test stopped at the pre-probe guard instead of exercising the authoritative
post-`try_wait` clock. Filesystem/process-lifecycle reported one low because a
no-Waker publication gap could detach a normal tail instead of joining it,
although the tail remained permit-bounded and the helper, request, and retained
descriptor were already cleaned. The cycle had zero blocker, high, or medium
findings, zero other findings, and no production resource escape. Replacement
remediation publishes `notification_complete` atomically when no Waker was
registered and adds deterministic after-wait-probe deadline and no-Waker
normal-join regressions. Exact cycle-4 candidate
`4632162f8d3f323fce65263ec92f0802d9416121`, tree
`ab1ecebe1680813614db3682f505e5de0fc31cfc`, passed the complete replacement
local gate. Filesystem/process-lifecycle and performance/concurrency were green
with zero findings; correctness/API found no production defect and one low
maintained-documentation lineage drift. Cycle 4 is therefore not green. That
documentation remediation was composed into exact cycle-5 candidate
`4317ac61feb57b706b6a023d2b2518c10e140d69`, tree
`90750911b26dc4eed9e54e73c17c11a6c5a12423`. All three tracks rejected that
candidate after reporting the same low current-lineage wording defect and zero
production findings. That correction is composed in exact cycle-6 candidate
`b8fd0c2061e2bbd20704d9e9e0c49f6d8a89f9d6`, tree
`07243b366f90366135ccbb1f8e146c71f7224f40`. All three fresh correctness/API,
filesystem/process-lifecycle, and performance/concurrency tracks are **GREEN**
with zero findings at every severity. Native Linux arm64 Rust 1.94.1 reruns
passed system 14/14, direct 12/12, engine 4/4, warnings-denied Clippy, and the
repeated performance matrix 70/70. Seal and integrated `main` SHA
`a02c28a6bc39f2981586f02cb76793c430c83a20`, tree
`03c751cffacee4808b057079dedb02cfc3f193cc`, passed feature CI `32738160229`
at 6/6 and feature benchmark `32738160725` at 2/2, retaining upstream artifact
`9524219365` and bootstrap artifact `9524052760`. It passed main CI
`32738798417` at 6/6 and main benchmark `32738798415` at 2/2, retaining
upstream artifact `9524461989` and bootstrap artifact `9524298408`. Native
`open_file` is delivered as slice twenty-six, and the current host has exactly
twelve alphabetical tools backed by one retained descriptor plus eleven
identity-preserving clones. This makes no product-performance or fx-equivalence
claim. At that checkpoint, the final docs-only record was exempt from
adversarial review under the user's instruction; its own exact feature and
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

The repository includes the provider-neutral streaming engine, its bounded
durable tool loop, a deterministic testkit, and read-only native
configuration/status discovery and loading, plus capability-aware tool
preflight before permission
policy. It also includes bounded Unix-only `read_file` and one-level
`list_files` library capabilities rooted in host-injected workspaces. The
seventeenth delivered slice adds bounded no-follow `file_info` metadata
inspection under a distinct exact authorization kind. Its local gates, all
three replacement adversarial tracks, and exact feature and `main` delivery
workflows are green. The
repository also includes a bounded AI Gateway codec over an injected host
transport. An optional,
native-only HTTPS transport for that codec is the seventh integrated bounded
slice. It uses one pinned production endpoint and an explicitly injected,
redacted bearer token.
An eighth bounded slice implements a Unix file-backed session store under an
explicit host-opened root. Its exact feature, documentation-seal, and `main`
checks are green; it is integrated at
`8f7b47db9580b14570bf9fb55763858f71a81271`. Provider/CLI wiring, a concrete
prompt UI, remaining session-lifecycle delivery and hardening, the remaining
native tools, and compatibility work remain planned. A ninth bounded slice
defines an executor-neutral, fail-closed native `AskPermissionHandler` over an
explicitly injected prompter. It is integrated on `main` at
`27e3f2b3ff170044732d9124ffb210beabcda206`; exact main CI run `32570197911`
and benchmark run `32570197870` are green. It has no CLI or terminal authority.
See its [contract](docs/ask-permission.md).
The tenth integrated bounded slice adds opt-in native discovery of a validated
AI Gateway bearer credential from an owned, redacted environment snapshot. It
is integrated on `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`; exact main CI run `32573320962`
and benchmark run `32573320937` are green. It does not change configuration
or CLI behavior; see its
[contract](docs/ai-gateway-credentials.md).
The eleventh integrated bounded slice advances the built-in native
configuration and current file schema to strict v2 with fixed declarative AI
Gateway provider, HTTP transport, and `zai/glm-5.2` model defaults. The exact
strict two-field v1 file remains read-compatible without rewrite or migration
and remains observable as schema v1 after its in-memory projection. Credentials
are in neither schema, and the new fields do not compose a provider, HTTP client,
runtime, token, network request, or CLI path. The feature implementation,
black-box tests, documentation, three adversarial tracks, and exact feature and
`main` gates are green. It is integrated on `main` at
`a10f24edde80a225f89e6c7068ec035cb70f80a8`; exact main CI run `32576876769`
and benchmark-evidence run `32576876780` are green. See the
[native configuration contract](docs/configuration.md) and
[review record](docs/reviews/m03-native-host-config-review-01.md).

A twelfth bounded library slice implements `NativeReferenceHost` for Linux
and macOS behind the optional `ai-gateway-http` and non-WebAssembly gate. It
composes an already validated native configuration with the AI Gateway
provider, either production HTTP from an injected credential snapshot or a
trusted injected custom transport, one shared retained workspace feeding the
current `file_info`, `glob_files`, `grep_files`, `list_files`, and `read_file`
candidate tool catalog under that same
retained identity, the existing file session store, and the
ask handler over an injected prompter. Construction remains synchronous and
creates no root, runtime, network request, prompt operation, session record, or
background work. Its final delivery record is integrated on `main` at
`ac3984fb16dbab3adf86a949c7555ceca7c3e8df`; exact feature CI run
`32579779134`, feature benchmark-evidence run `32579779123`, main CI run
`32580066474`, and main benchmark-evidence run `32580066485` are green. The CLI
is byte-unchanged and remains thin. See the
[composition contract](docs/native-reference-host.md) and
[review record](docs/reviews/m03-native-reference-host-review-01.md).

A thirteenth bounded slice advances the strict current native
configuration to schema v3 by adding the required non-secret
`credential_source: "environment"` selection. Exact v1 and v2 files remain
strictly readable without rewrite and project the same acquisition kind only
in memory. The loader receives no token or process authority; the production
reference host still consumes an explicitly injected credential snapshot, and
its runtime observation still reports the concrete selected OIDC-token or
API-key source. Production implementation, independent tests, focused and
required local gates, and all three fresh adversarial tracks are green on exact
behavior SHA `35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. It is integrated on
`main` through final delivery record
`f840576af241c58d1e55399e66ba92f7770cd50c`; exact final-record feature CI
run `32583585145`, feature benchmark-evidence run `32583585148`, main CI run
`32583871385`, and main benchmark-evidence run `32583871368` are green. See the
[configuration contract](docs/configuration.md) and
[review record](docs/reviews/m03-configured-credential-source-review-01.md).

The integrated fourteenth bounded slice adds explicit
Linux/macOS native root selection and safe preparation. It selects an existing
absolute workspace plus a state location from an injected native-environment
snapshot, opens and retains the workspace first, and may create only a fixed
descriptor-relative state suffix with private new-directory modes. It does not
change schema v3, configuration bytes, the CLI, status, or session lifecycle.
Production and 16 independently owned focused tests are present, and focused
root-selection and prepared-composition gates are green. Initial formal review
found fixture-mode and macOS ACL issues; those fixes and their ALLOW-rejection
and ordinary-HOME DENY-compatibility regressions are composed. All three formal
tracks were green together on exact behavior SHA
`f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`. The first documentation seal then
exposed three Linux-only strict-Clippy diagnostics in feature CI run
`32588948956`; its native Linux/macOS tests and benchmark run `32588948975`
were green. Portable lint normalization is present at `90d8f96`, and local
macOS plus Linux cross-target gates are green. All three final adversarial
tracks are green on exact candidate SHA
`72cf64f63e0dfa30bc1ee21d8aca16550e819c21`. Replacement documentation seal
`f08dbd9eb2da81848b8eefb2d218006a64575835` is green under exact feature CI run
`32589778343` and benchmark-evidence run `32589778374`. Feature-evidence record
`6f66b6e5972e78ba0f0ccae06b899158d99bc864` is green under exact feature CI
`32590128235` and benchmark evidence `32590128233`; it is fast-forwarded without
force to `main` and green there under exact CI `32590429626` and benchmark
evidence `32590429592`. This documentation-only commit is the final delivery
record; its exact workflows are reported at handoff. See the integrated
[native root-selection contract](docs/native-root-selection.md) and
[review record](docs/reviews/m03-native-root-selection-review-01.md).

The delivered fifteenth slice implements a Linux/macOS native by-ID
session lifecycle over the exact `FileSessionStore` shared with the composed
engine. The caller supplies a validated session ID; the native host uses OS
randomness for each new incarnation. Create durably publishes an empty record,
resume returns the engine-canonical current lifetime, replay returns a bounded
durable `SessionRecord` snapshot rather than UI events, and reset atomically
publishes an empty new incarnation while advancing the durable revision. It
does not add session listing or any CLI behavior. Production, fourteen
independently owned focused tests, and one formal finding regression are green;
all three adversarial tracks are green together on exact candidate `e6a3804`.
Feature record `dbba2c7` is green under feature CI `32594562796`, feature
benchmark evidence `32594562785`, `main` CI `32594846484`, and `main` benchmark
evidence `32594846476`. This documentation-only commit is the final record; its
workflows are reported at handoff. See the integrated
[native session-lifecycle contract](docs/native-session-lifecycle.md).

The delivered sixteenth slice adds bounded Linux/macOS library-only session
listing. `NativeSessionLifecycle::list_sessions` returns at most 100 sorted,
unique validated IDs plus a truncation flag while processing at most 1,024
non-dot entries plus one fetched/name-inspected overflow witness and accepting/
decoding at most 64 MiB of aggregate canonical record bytes plus one transient
transfer byte used only to detect concurrent growth. Canonical corruption fails
closed; unrelated names are ignored but count against the scan budget.
The result is neither a multi-record snapshot nor a pagination or summary
surface. Production, documentation, and 13 initial independent tests are
composed from base `9ada4b5` through first formal candidate `dec98e0`. All three
first review tracks were not green. Isolated fix `4b8d8b0` and test hardening
`446b495` are composed into exact behavior candidate `3fa5463` with the
corrected documentation; its 18 focused tests, required local gates, and all
three replacement review tracks are green. First remote CI run `32599591900`
exposed a Linux removed-root
liveness gap. Exact portable-fix candidate `17f1884` applies the descriptor
check and is green under both executable review tracks. Documentation seal
`d3312d7` resolves the lineage finding, passed exact feature CI `32600292770`
and benchmark evidence `32600292779`, was fast-forwarded without force to
`main`, and passed exact main CI `32600567094` and benchmark evidence
`32600567090`. It adds no
CLI behavior and makes no fx equivalence or performance claim. See the
[native session-listing contract](docs/native-session-listing.md) and
[review record](docs/reviews/m03-native-session-listing-review-01.md).

The delivered seventeenth bounded slice adds Linux/macOS library-only
`file_info`.
Strict effect-free preflight accepts only a required 4,096-byte-bounded
workspace-relative path, prepares `FilesystemAccess::Metadata`, and gives
policy and execution the exact same normalized path. Allowed execution walks
ancestors descriptor-relatively without following symlinks, then inspects the
final component with no-follow metadata without opening it. The exact bounded
result reports normalized path, fixed kind, checked size, signed Unix modified
time, and a nullable lexical regular-file extension. Final symlinks report
themselves; FIFO, socket, device, and other special objects are classified
without being opened. Reference-host composition grows from two to exactly
three workspace tools: `file_info`, `list_files`, and `read_file`. Core exposes
that catalog in deterministic alphabetical order. Production is present at
isolated SHA `5c2d129`; independent tests are present at isolated SHA `ca0091c`
and compose with production at `f228c06`, where the initial 34 focused tests are
green. Review hardening composes at `b69ec4b`, bringing the independently owned
focused suite to 36 green tests plus five private unit tests. Required local
gates and all three replacement adversarial tracks are green on exact candidate
`4193ecc`. Documentation seal and integrated `main` SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` is green under exact feature CI
run `32605071080` (successful retry attempt 2), feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four report that exact seal SHA. The benchmark successes are
delivery evidence only and make no product-performance claim. The overall
native-tool checklist stays open. This documentation-only commit is the final
delivery record, is explicitly exempt from another adversarial review after the
behavior was already green, and reports its own exact workflows at handoff. See
the [`file_info` contract](docs/file-info.md) and
[review record](docs/reviews/m03-file-info-review-01.md).

The eighteenth delivered bounded slice adds Linux/macOS library-only `glob_files`.
Strict effect-free preflight accepts a required glob pattern plus optional
search root and `matches`/`count` mode, prepares the distinct
`FilesystemAccess::EnumerateRecursive` capability at that normalized subtree,
and makes defaults explicit to allowed execution. Descriptor-relative,
no-follow iterative traversal includes hidden entries, never descends through
symlinks, reads no content, and fails without partial output if its entry,
name-byte, depth, candidate-path, or 8,388,608-step aggregate matcher-work cap
fires. Match output is the globally
bytewise-smallest sorted prefix under exact 100-path and 16 KiB aggregate path-
byte caps; count mode completes the same bounded scan and is exact. The
slice extends the composed host catalog to `file_info`, `glob_files`,
`list_files`, and `read_file`, with one retained workspace identity distributed
as the original descriptor plus three clones. Production, independent tests,
and documentation are composed. The first formal review at `1f5de6a` found a
high unmetered matcher-work defect; the checked 8,388,608-step fix, independent
both-mode regression, and all replacement local gates are green at exact
code-and-test head `4171a4a8811a98888b7e4e161281a1216564746f`. All three
replacement adversarial tracks are green on exact behavior SHA `523df858`.
Documentation seal and integrated `main` SHA
`35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed exact feature CI run
`32610950593`, feature benchmark-evidence run `32610950594`, main CI run
`32611208411`, and main benchmark-evidence run `32611208415`; all four report
that exact seal SHA. Benchmark success is delivery evidence only and makes no
product-performance claim. This documentation-only commit is the final
`glob_files` delivery record, is explicitly exempt from another adversarial
review after behavior was green, and reports its own exact workflows at
handoff. Final documentation record
`f6aa458bb875d6cb26565adc878703fe140916d3` passed exact feature CI
`32611623653` and feature benchmark evidence `32611623655`. GitHub did not
materialize workflows for its first `main` event, so tree-identical
non-behavior successor `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` passed
feature CI `32612424382` and feature benchmark evidence `32612424383`, was
fast-forwarded to `main`, and passed exact main CI `32612662260` and main
benchmark evidence `32612662203`. Neither docs-only record reopened behavior
review. See the
[`glob_files` contract](docs/glob-files.md) or
[review record](docs/reviews/m03-glob-files-review-01.md). The reviewed behavior changes
no CLI byte, benchmark workload, compatibility status, or performance claim,
and the combined native-tool checklist remains open.

The delivered nineteenth bounded slice adds Linux/macOS library-only `grep_files`
from exact base `f6aa458bb875d6cb26565adc878703fe140916d3`, with tree-identical
integration kickoff `f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`. Production,
independent tests, and maintained documentation are parallel, non-overlapping
components. Exact production `27eec2f` and initial independent-test `6eaee93`
components exist and initially compose through `9057feb` and `44e33d7`;
reference-host fixture fix `bdbb677` makes focused production/test composition
green. Documentation component `b04151a` produces first fully composed behavior
candidate `42e4793`; lint fix and exact local gates are green at `45ad91f`.
All three first-cycle formal tracks are **NOT GREEN** on exact candidate
`355a11a`. Remediation and exact replacement local gates are green at final
code/test precursor `275d263`. First replacement candidate `ae87bf1` is **NOT
GREEN** with one low correctness ordering finding, one low filesystem evidence-
wording finding, and medium-plus-low performance/cancellation findings.
Second production remediation `ac5d772` composes at `d672210`; second
documentation remediation `7ad0863` produces fully composed, exact local-gate-
green precursor `b498ba0`. `ae87bf1` remains historically **NOT GREEN**.
Formal second replacement candidate `5aeddc1` has correctness/API and
filesystem/robustness **GREEN** with zero findings; performance/concurrency is
**NOT GREEN** with one medium allocation-amplification finding and two low
documentation/evidence findings. Third production remediation `8777825`
composes at `ab1c133`; independent regression `dcf57ad` composes at `d7526d4`;
review-findings documentation `44afb23` composes at `f08c5f2`; lint follow-up
`1f13f9a` produces exact fully composed local-gate precursor `a8f6179`. Exact
Rust 1.94.1 formatting, warnings-denied workspace Clippy, 598 non-documentation
tests plus two doctests, 25 private native tests, 40 direct `grep_files` tests,
four engine tests, and diff checks are green on that precursor. Exact a8f
cross-target/dependency/link and compatibility/release validators are green.
Formal third-cycle candidate
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
Strict effect-free preflight accepts exactly `pattern`, `path`, `include`,
`case_insensitive`, `mode`, `head_limit`,
`offset`, and `context_lines`, makes every default explicit, and prepares
`FilesystemAccess::SearchContent` at the normalized selected file or subtree.
The remediation contract requires fixed literal pattern-table work before root
resolution, one fully metered include compilation per call, full descendant-
path validation before allocation or filtering, reusable 64 MiB-bounded
continuation offsets, and selected-file filtering before content open. Slashful
selected-file rejection is charged and cancellation-checked; slashful candidate
splitting and both dynamic-programming branches retain fixed cancellation
checks. Allowed execution performs a bounded linear literal search with
optional ASCII case folding over eligible UTF-8 no-follow regular files reached
through the retained descriptor. One scan-local content buffer reads through an
8 KiB window, grows only to a 204,801-byte high-water ceiling, and logically
resets for reuse between files while preserving per-file and aggregate overflow
witnesses. Recursive and non-recursive include matching use injectable
cancellation checks with deterministic coverage. The tool reports exact
matching and eligible-text statistics in bounded `matches`,
`files_with_matches`, or `count` results, with same-buffer context and fixed
redacted errors. Candidate composition extends
the host to exactly five alphabetical tools—`file_info`, `glob_files`,
`grep_files`, `list_files`, and `read_file`—using the original retained
workspace descriptor plus four clones. It adds no CLI byte, benchmark workload,
compatibility status, performance claim, or fx-equivalence claim, and the
combined native-tool checklist stays open. See the
[`grep_files` contract](docs/grep-files.md) and
[review plan](docs/reviews/m03-grep-files-review-01.md).

The delivered twentieth through twenty-fourth slices add `write_file`,
`edit_file`, `delete_file`, `rename_file`, and `copy_file`. The twenty-fourth
slice extends the library host to exactly ten alphabetical tools by adding
`copy_file`, using the original retained workspace descriptor plus nine
identity-preserving clones. Its typed `FilesystemCopy` capability exposes both
canonical endpoints to policy. Approved execution confines both paths beneath
that retained root and streams at most 16 MiB through one 64 KiB buffer into a
private destination-parent stage before a single no-replace commit, bounded
postcommit verification, and destination-parent synchronization. It does not
overwrite, create parents, accept directory or symlink endpoints, allocate the
whole source, or broaden CLI authority. Seal `3bdd7cb` passed exact feature and
`main` CI/benchmark delivery gates with two artifacts in each benchmark run.
This description makes no complete fx-equivalence or performance claim. See
the [`copy_file` contract](docs/copy-file.md).

The composed twenty-fifth `create_folder` behavior accepts one strict canonical
confined workspace-relative path and recursively creates missing directory
components. It uses existing provider-neutral `FilesystemAccess::Create`
authority, requests mode `0755` while honoring host umask and ACL inheritance,
never follows symlinks or normalizes permissions afterward, never retries a
`mkdirat`, and never rolls back a created prefix. The first successful or
uncertain creation is the commit boundary; postcommit verification and
bottom-up durability are explicitly bounded. Candidate source adds
`create_folder` after `copy_file` for eleven alphabetical tools backed by the
original retained descriptor plus ten clones. The exact frozen contract commit
`9fab189c9c1add76a38775d08f4342c6bcc7635b` is green under all six jobs of CI
`32687614476`; benchmark workflow `32687614442` passed both jobs and retains
exactly two nonexpired exact-SHA artifacts. Those runs cover the contract only.
Cycle-2 candidate `6e1f885`, tree `ac57575`, is historically not green only for
two low filesystem-evidence/documentation findings; the other two tracks are
green and all tracks found zero production defects. Exact remediation
`f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate. Documentation record `9d0bacd`, tree `b5fb1c2`, and tree-identical
cycle-3 candidate `c1e572e` retain identical non-documentation behavior. Cycle
3 is not green only for one low lineage-record finding; the other two tracks
are green and all three found zero production defects. Exact lineage remediation
`12c11ba`, tree `b96575b`, passes the complete replacement gate. Gate record
`f6f6584` parents tree-identical cycle-4 candidate `a78b693`, tree `2b913e8`.
Cycle 4 found zero production defects; its sole low stale seal-record finding is
fixed in the exempt documentation seal. First feature benchmark `32699750662`
is green with exactly two nonexpired exact-SHA artifacts, but feature CI
`32699750602` is not green because Linux Clippy rejected a test-only
`u32::from(Mode::bits())`. The trace now stores platform-native `RawMode`; a
complete replacement gate is green at `1effcbb`, tree `b5eccb1`, and fresh
cycle-5 candidate `ff18a9a`, tree `f77b198`, is green with zero findings in all
three tracks. Seal `e75578b` passed exact feature CI `32702785549`, feature
benchmark `32702785574`, main CI `32703303933`, and main benchmark
`32703303931`; both benchmark runs retain exactly two nonexpired exact-SHA
artifacts. This is delivery and `main` integration, but not product-performance
or fx-equivalence evidence. See the
[`create_folder` behavior contract](docs/create-folder.md).

The project is not yet production-ready. See the exact
[CLI contract](docs/cli.md),
[`read_file` contract](docs/read-file.md),
[`list_files` contract](docs/list-files.md),
[`file_info` contract](docs/file-info.md),
[`glob_files` contract](docs/glob-files.md),
[`grep_files` contract](docs/grep-files.md),
[`write_file` contract](docs/write-file.md),
[`edit_file` contract](docs/edit-file.md),
[`delete_file` contract](docs/delete-file.md),
[`rename_file` contract](docs/rename-file.md),
[`copy_file` contract](docs/copy-file.md),
[`create_folder` behavior contract](docs/create-folder.md),
[`open_file` contract](docs/open-file.md),
[`web_fetch` candidate contract](docs/web-fetch.md), and
[AI Gateway codec](docs/ai-gateway.md) plus
[native HTTP transport](docs/ai-gateway-http.md) and
[credential discovery](docs/ai-gateway-credentials.md) contracts, and the
normative [native file session store](docs/session-store.md), the integrated
[native reference-host composition](docs/native-reference-host.md), and the
integrated [configured credential source](docs/configuration.md), and the
integrated [native root-selection boundary](docs/native-root-selection.md).
The [native session lifecycle](docs/native-session-lifecycle.md) is integrated;
its bounded [session-listing extension](docs/native-session-listing.md) is a
delivered and green under exact feature and `main` workflow evidence.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

The Rust project is licensed under Apache-2.0. It is inspired by
[`vercel-labs/fx`](https://github.com/vercel-labs/fx), whose pinned comparison
revision is recorded in `benchmarks/upstream.lock`. Zig is pinned only to build
that upstream benchmark reference; it is not a machine-god product language or
runtime dependency.
