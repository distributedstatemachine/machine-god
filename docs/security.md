# Security

The core has no ambient filesystem, process, environment, credential, or
network authority. Native capabilities are supplied explicitly by a host. The
first Milestone 03 native slice only snapshots config/state environment inputs
and reads final-path metadata for status. A second bounded native authority
loads configuration synchronously and read-only. A third, provider-neutral
slice adds capability-aware tool preflight without exercising native authority.
A fourth slice adds a bounded Unix-only `read_file` implementation behind that
preflight seam. A fifth adds bounded one-level Unix-only `list_files`
enumeration behind the same seam. A sixth adds a bounded AI Gateway request and
stream codec behind an injected host transport; the codec itself receives no
endpoint, credential, socket, TLS, status, retry, clock, or runtime authority.
A seventh integrated slice adds one optional native implementation of that
transport with a pinned production HTTPS endpoint and an explicitly injected
bearer token. An eighth integrated bounded slice adds a Unix file-backed
`SessionStore` beneath one explicit retained host directory descriptor. The
seventh slice's feature-branch review and exact remote runs are green and
recorded in the
[`native AI Gateway HTTP transport review`](reviews/m03-ai-gateway-http-review-01.md).
It is integrated on `main` at
`508b0adbbe4447a85bd08f47095ae16c089c05d5`; exact main CI run `32535790803`
and benchmark run `32535790824` are green. This is not a production-ready
claim.
The eighth slice's exact feature, documentation-seal, and `main` checks are
green, with evidence retained in the
[`native file session store review`](reviews/m03-session-store-review-01.md).
It is integrated on `main` at
`8f7b47db9580b14570bf9fb55763858f71a81271`; exact main CI run `32541315998`
and benchmark run `32541315997` are green.
A ninth integrated bounded slice defines the fail-closed native
`AskPermissionHandler` over an explicitly injected `PermissionPrompter`. It
does not contain a prompt UI or change CLI behavior. Its implementation and
black-box tests, three-track adversarial review, documentation seal, and exact
feature and `main` workflows are green. It is integrated on `main` at
`27e3f2b3ff170044732d9124ffb210beabcda206`; exact main CI run `32570197911`
and benchmark run `32570197870` are green. Permission mode remains `ask`; CLI
registration and a concrete prompt remain future work. The slice's exact
security boundary and review evidence are in
[`ask-permission.md`](ask-permission.md).
A tenth integrated bounded slice adds opt-in native credential discovery
behind the existing `ai-gateway-http` and non-WASM gate. It owns a separate
secret snapshot and does not add credential authority to core, native
configuration, or the CLI. It is integrated on `main` at
`ef6901d33c45f0b78b9ddf0042ad27b0ee1953c0`; exact main CI run `32573320962`
and benchmark run `32573320937` are green. Its exact boundary is in
[`ai-gateway-credentials.md`](ai-gateway-credentials.md).
An eleventh integrated bounded slice advances the built-in and current native
config schema to strict v2 while keeping exact strict v1 files read-compatible
without rewrite or migration. It is integrated on `main` at
`a10f24edde80a225f89e6c7068ec035cb70f80a8`; exact main CI run `32576876769`
and benchmark-evidence run `32576876780` are green. See
[`configuration.md`](configuration.md) and the
[`native host configuration review`](reviews/m03-native-host-config-review-01.md).
A twelfth bounded slice implements Linux/macOS-only library composition
behind the existing `ai-gateway-http` and non-WebAssembly gate. Production
implementation, independent black-box tests, three fresh adversarial tracks,
and exact feature and `main` workflows are green; its final delivery record is
`ac3984fb16dbab3adf86a949c7555ceca7c3e8df`, with exact feature CI run
`32579779134`, feature benchmark-evidence run `32579779123`, main CI run
`32580066474`, and main benchmark-evidence run `32580066485` green. Its
boundary is in [`native-reference-host.md`](native-reference-host.md).
A thirteenth bounded slice advances native configuration to strict v3 with
one required closed, non-secret `environment` credential-source acquisition
kind. Exact v1/v2 files remain readable and gain that projection only in
memory. Production implementation, independent tests, local gates, and all
three fresh adversarial tracks are green on exact behavior SHA
`35ce591e8ca6a8fef94485ff85d3e9c1397130a6`. It is integrated on `main` at
`8755757da0da07e33af48d57f46bd9ea490b5449`; exact main CI run `32582978232`
and benchmark-evidence run `32582978286` are green. Its final delivery record is
integrated on `main` at `f840576af241c58d1e55399e66ba92f7770cd50c`;
exact feature CI run `32583585145`, feature benchmark-evidence run
`32583585148`, main CI run `32583871385`, and main benchmark-evidence run
`32583871368` are green for that exact SHA.
The integrated fourteenth slice adds explicit Linux/macOS native root selection
and narrowly bounded safe preparation. It opens and retains the
workspace before state work, may create only a fixed descriptor-relative state
suffix under an existing selected base, validates rather than repairs existing
directories, rejects root equality or ancestry, rejects any unsafe macOS
extended ACL through the retained descriptor, and preserves credential
discovery after retained-root preparation. Production and 16 independently
owned focused tests are present, and focused gates are green. Formal review of
the initial exact candidate found and drove fixes for macOS extended ACLs and
ambient-umask-dependent fixtures. All three formal tracks were green on exact
behavior SHA `f1dc47517d5b2d6d37628be4eb2ab51871e20b5d`. The first seal exposed
Linux-only strict-Clippy diagnostics, normalized at `90d8f96`; local macOS and
Linux cross-target gates are green. All three final tracks are green on exact
candidate `72cf64f6`. Replacement seal `f08dbd9e` and feature record `6f66b6e5`
are feature-green; the latter is integrated on `main` under exact CI
`32590429626` and benchmark evidence `32590429592`. Its integrated security
boundary is in
[`native-root-selection.md`](native-root-selection.md).
The delivered fifteenth slice adds a by-ID native session lifecycle above the
exact file store shared with the composed engine. The caller supplies the
validated session ID; only production OS cryptographic randomness supplies new
incarnations. Create persists before success, replay returns a durable bounded
record rather than UI/events, and reset atomically changes incarnation without
a deletion gap. Production, fourteen independently owned focused tests, one
formal finding regression, and all three adversarial tracks are green on exact
candidate `e6a3804`. Feature record `dbba2c7` is green on the feature branch and
`main` under exact CI and benchmark workflows. Its
security boundary is in
[`native-session-lifecycle.md`](native-session-lifecycle.md).
The delivered sixteenth slice adds bounded IDs-only observation through that
same lifecycle. It recognizes only exact canonical record names, uses the
store's no-follow regular-file checks, existing per-ID lock protocol, strict
current schema and decoded-ID/digest validation, and fails the whole call on a
canonical corrupt candidate. It returns no more than 100 sorted unique IDs and
processes/selects at most 1,024 non-dot directory entries plus one fetched and
name-inspected overflow witness. It accepts/decodes at most 64 MiB of aggregate
canonical record bytes plus one transient transfer byte solely to detect
concurrent growth. All non-dot names within the scan consume budget. Production
and 13 initial independent tests are composed through first formal candidate
`dec98e0`, whose three first review tracks were not green. The fixes and 18-test
hardened suite are composed in exact behavior candidate `3fa5463`; all three
replacement review tracks are green. First remote CI `32599591900` exposed the
Linux removed-root gap now fixed by
exact portable behavior candidate `17f1884`. Its correctness/API and security
reviews are green; seal `d3312d7` resolves the documentation lineage finding
and passed exact feature and `main` delivery gates. Its security contract is in
[`native-session-listing.md`](native-session-listing.md).
A delivered seventeenth bounded slice adds Linux/macOS `file_info` under a distinct
`FilesystemAccess::Metadata` authorization kind. It shares one retained
workspace identity with `list_files` and `read_file`, follows no selected
symlink, and does not open the final component. Final symlinks therefore report
themselves, while FIFO, socket, device, and other special objects can be
classified without a blocking open. Production `5c2d129` and independent tests
`ca0091c` compose at `f228c06`, where all 34 initial focused tests are green.
Review hardening brings the focused total to 36 plus five private unit tests at
`b69ec4b`. Three replacement tracks are green on exact candidate `4193ecc`.
Documentation seal and integrated `main` SHA
`60dd54f273afc7e62fb4b3cc1fb1a347d739998b` passed exact feature CI run
`32605071080` on successful retry attempt 2, feature benchmark-evidence run
`32605071063`, main CI run `32606050292`, and main benchmark-evidence run
`32606050294`; all four workflows report that exact seal SHA. Benchmark success
is delivery evidence only and makes no product-performance claim. This
documentation-only commit is the final delivery record, is explicitly exempt
from another adversarial review after behavior was green, and reports its own
exact workflows at handoff.
Its security contract is in [`file-info.md`](file-info.md).

The delivered eighteenth bounded slice adds Linux/macOS `glob_files` under the
distinct `FilesystemAccess::EnumerateRecursive` authorization kind. Strict
effect-free preflight normalizes the selected subtree and an exact bytewise glob
pattern before policy. Execution reacquires and validates the retained
workspace identity, traverses iteratively with descriptor-relative no-follow
operations, follows no symlink, reads no content, includes hidden entries, and
fails without partial output if any fixed scan cap fires. The same complete
bounded traversal produces either a globally bytewise-smallest sorted match
prefix or an exact count. Production, independent tests, documentation,
composition, and initial local gates were green at `60070d8`; the first formal
review at `1f5de6a` found a high unmetered matcher-work defect. Its checked work-
budget fix, both-mode regression, and replacement local gates are green at
`4171a4a8811a98888b7e4e161281a1216564746f`; all three replacement tracks are
green on exact behavior SHA `523df858`. Documentation seal and integrated
`main` SHA `35c853605077f2ac700f4be1dd79eabd2ace4dd4` passed exact feature CI
`32610950593`, feature benchmark evidence `32610950594`, main CI `32611208411`,
and main benchmark evidence `32611208415`; benchmark success is delivery
evidence only and makes no product-performance claim. Its security contract is in
[`glob-files.md`](glob-files.md).

The nineteenth bounded slice adds Linux/macOS `grep_files` under distinct
`FilesystemAccess::SearchContent`. Exact base
`f6aa458bb875d6cb26565adc878703fe140916d3` and tree-identical kickoff
`f6ab594c928bead48b48ab080ac12a7ce9c0d3f4` precede parallel production,
independent-test, and documentation ownership. Exact production `27eec2f` and
initial independent-test `6eaee93` components exist and initially compose
through `9057feb` and `44e33d7`; reference-host fixture fix `bdbb677` makes
focused production/test composition green. Documentation component `b04151a`
produces fully composed behavior `42e4793`; lint fix and exact local gates are
green at `45ad91f`. All three first-cycle tracks are **NOT GREEN** on exact
`355a11a`. Remediation and exact replacement local gates are green at final
code/test precursor `275d263`. First replacement candidate `ae87bf1` is **NOT
GREEN** across all three tracks. Second-fix production and documentation compose
through `ac5d772`, `d672210`, `7ad0863`, and exact local-gate-green precursor
`b498ba0`. Formal second replacement candidate `5aeddc1` has correctness/API
and filesystem/robustness **GREEN** with zero findings and
performance/concurrency **NOT GREEN** with one medium allocation-amplification
finding and two low documentation/evidence findings. Third production
remediation `8777825` composes at `ab1c133`; independent regression `dcf57ad`
composes at `d7526d4`; review-findings documentation `44afb23` composes at
`f08c5f2`; lint follow-up `1f13f9a` produces exact fully composed local-gate
precursor `a8f6179`. Exact Rust 1.94.1 formatting, warnings-denied workspace
Clippy, 598 non-documentation tests plus two doctests, 25 private native tests,
40 direct `grep_files` tests, four engine tests, cross-target/dependency/link
validation, and diff checks are green. Compatibility/release validation is
green. Formal third-cycle candidate
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
self-recorded. Its strict preflight
prepares all eight canonical request fields and conservative search authority
at the selected file or subtree. Execution performs retained descriptor-
relative no-follow regular-file-only traversal, bounded eligible UTF-8 content
reads, metered linear literal matching, same-buffer context, and exact
structured results with explicit skip statistics. It adds no external path,
symlink-target, Git/ignore/subprocess, regex, CLI, performance, or equivalence
claim. Its security contract is in [`grep-files.md`](grep-files.md).

Status resolution recognizes only the `machine-god` namespace. Empty XDG
values fall back to `HOME`; a selected nonempty relative or non-Unicode root is
invalid and cannot be bypassed through a `HOME` fallback. Missing or empty
`HOME` is unavailable. Inspection uses `symlink_metadata`, so a final symlink is
classified as the wrong kind instead of followed. It does not canonicalize,
open or parse `config.json`, create locations, or write state. These constraints
make status a bounded observation surface, not configuration authority.

Status paths are user-visible and may disclose the selected config/state
location when the user explicitly runs the command. JSON output uses JSON
escaping, and human output uses the same JSON-string encoding for paths. C0/C1
controls, Unicode line/paragraph separators, and bidirectional-formatting
controls are emitted as escapes, so environment-derived paths cannot inject
terminal lines or visually reorder the surrounding status fields. Non-UTF-8
CLI arguments are rejected as invalid. Output errors use a fixed diagnostic
rather than reflecting path or OS-error text.

Configuration loading uses the same config-location selection. In the
thirteenth slice, missing and unavailable locations yield the explicit
strict schema-v3 built-in values: permission mode `ask`, provider
`vercel_ai_gateway`, transport `ai_gateway_http`, model `zai/glm-5.2`, and the
non-secret credential-source kind `environment`. Exact legacy schema-v1 and
schema-v2 objects map in memory to that acquisition kind while remaining
observable as versions `1` and `2`. The loader never rewrites or migrates them.
An invalid selected config-location environment value does not fall back.

On the supported Unix targets exercised by Milestone 03, a present path is
opened no-follow and nonblocking. A preliminary path-kind check is followed by
authoritative opened-descriptor regularity validation before any bytes are
read. Final symlinks and non-regular entries therefore fail closed without a
FIFO open becoming an unbounded wait. Hardened open semantics for non-Unix
targets remain deferred. The loader does not canonicalize, create, or write
anything; its no-follow guarantee is for the final component and is not a claim
that the complete ancestor path is frozen.

The raw configuration bound remains 64 KiB, with at most one additional byte
retained to detect overflow or growth. Accepted bytes must be valid UTF-8 and
then one strict schema. V3 has the exact five v2 fields plus required string
`credential_source: "environment"`. V2 has exactly five fields: integer
`schema_version: 2`; strings `permission_mode: "ask"`,
`provider: "vercel_ai_gateway"`, and `transport: "ai_gateway_http"`; and a
1–128-byte visible-ASCII model and rejects `credential_source` as unknown. V1
has exactly integer `schema_version: 1` and
string `permission_mode: "ask"`. Oversize files, invalid UTF-8, malformed JSON,
unknown or duplicate fields, missing or wrong fields, unsupported versions or
enum values, and invalid models are rejected.

The model validator is shared with the AI Gateway provider, preventing config
and provider acceptance from drifting. Config owns the bounded model string and
is cloneable but not copyable. Its debug output replaces the model with
`<redacted>`, and typed failures do not echo environment-derived paths,
configuration contents, model values, or operating-system error text.
Inaccessible paths and read errors are not converted into defaults.

Bearer credential bytes are fields in none of v1, v2, or v3 and never enter
config debug output. Provider, transport, and credential-source enums contain
only public declarative identity. `NativeCredentialSourceKind::Environment`
cannot carry a token or arbitrary variable name and grants no process authority
to the loader.
In particular, `NativeTransportKind::AiGatewayHttp` exists when the optional
concrete transport is disabled and on WebAssembly; it is not evidence that an
HTTP implementation or required runtime is available. Loading valid config
does not instantiate the provider or transport, create a Tokio runtime,
discover or attach a bearer token, or perform network I/O. The non-secret
acquisition kind may appear in config debug output; the model remains redacted.

The existing status path remains metadata-only and its CLI output is
byte-stable. Configuration mutation or migration, a concrete prompt UI and
modes beyond `ask`, token fields in configuration, CLI composition and
expansion, composed release-binary end-to-end host evidence, and compatibility
or performance claims remain open. At the historical pre-`create_folder`
checkpoint, the delivered library composition included the ten bounded
workspace tools documented below. `rename_file` and `copy_file` were delivered,
while `create_folder` implementation and eleven-tool composition were present
in candidate source. Cycle-2 candidate `6e1f885`, tree
`ac57575`, is historically not green: correctness/API and performance/
concurrency are green with zero findings, while filesystem/robustness reported
two low evidence/documentation findings and zero production defects. Exact
remediation `f52729379a4c2352cbb9817bcd19e8bb6e3b2b8f`, tree
`40eef148230a79e5d9700b5ca2bdfd0ace2f192c`, passes the complete replacement
local gate, including deterministic mixed-device identity-chain evidence.
Documentation record `9d0bacd`, tree `b5fb1c2`, and tree-identical cycle-3
candidate `c1e572e` preserve identical non-documentation behavior. Cycle 3 is
not green only for one low documentation-lineage finding; filesystem and
performance are green, and all tracks found zero production defects. Lineage
remediation `12c11ba`, tree `b96575b`, passes the complete replacement gate.
Gate record `f6f6584` parents tree-identical cycle-4 candidate `a78b693`, tree
`2b913e8`. Cycle 4 found zero production/filesystem defects; its sole low stale
seal-record finding is fixed in the exempt documentation seal. Delivery and
`main` integration remained pending at that checkpoint. First feature CI
`32699750602` has all native
Linux/macOS jobs green but is not green because Linux Quality rejected a test-
only mode conversion. Platform-native `RawMode` evidence remediation `1effcbb`,
tree `b5eccb1`, passes the complete replacement gate. Tree-identical cycle-5
candidate `ff18a9a`, tree `f77b198`, is green with zero findings in all three
fresh tracks. Seal `e75578b` passed exact feature CI `32702785549`, feature
benchmark `32702785574`, main CI `32703303933`, and main benchmark
`32703303931`; both benchmark runs retain exactly two nonexpired exact-SHA
artifacts. The `create_folder` checkpoint authority surface has eleven tools.
The current twenty-sixth `open_file` slice is delivered after a green formal
review cycle 6.
Its dedicated `Capability::OpenFile { path }` is narrower than arbitrary
process authority and covers one canonical workspace-confined existing regular
file. Linux execution rejects symlinks, retains the approved file descriptor,
and supplies only `/usr/bin/xdg-open` with
`/proc/<machine-god-parent-pid>/fd/<retained-fd>` as its sole argument. The
helper runs from `/` with null stdio and the trusted host environment; machine-
god performs no ambient `PATH` lookup and accepts no model-selected program,
arguments, environment, or working directory. A trusted injected launcher seam
provides deterministic tests and is inert until execution is polled. Public
construction is Linux-only; macOS public construction is unsupported, its
private retained-root host tool returns unsupported at execution, and all other
targets are unsupported.

At most 32 production system-launch workers are active; saturation is retryable
precommit unavailable with zero new worker/helper, and each permit remains held
through arbitrary Waker completion and worker return. The worker is established
before helper spawn. Worker-start failure and launcher unavailability are
retryable precommit failures. The final spawn and
cancellation/drop abort transitions share one serialized gate: abort-first
guarantees no launch, while successful spawn commits. Cancellation after that
boundary, timeout, or explicit future/drop cleanup terminates and reaps the
direct helper without claiming rollback. Before publication, cleanup suppresses
waking, reaps the helper, drops request/descriptor ownership, and synchronously
joins. Normal published cleanup joins too. Inline or blocking arbitrary-Waker
overlap may release the handle to avoid self-join/cross-thread deadlock; only
permit-bounded callback/final bookkeeping may outlive future drop after helper/
request cleanup. This narrow docs-only amendment replaces the frozen absolute
no-worker-detach clause because it contradicted legal Waker behavior and is
exempt from its own adversarial review under the owner's instruction.
Postcommit cancellation and nonzero or signalled exit, timeout, or wait failure
return fixed redacted, nonretryable result uncertainty when a tool-level result
is observed; worker creation is pre-spawn, so there is no postspawn waiter-setup
state. Exit zero
means only that the helper accepted the request, not that a desktop application
consumed or displayed the file. Success is exactly
`{"path":"canonical/relative/path"}`. Delivered source composes exactly twelve
alphabetical tools from one retained workspace descriptor plus eleven identity-
preserving clones. Formal cycle 3 rejected exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
`4a479b51ebdba49afb81a6827f1381d01ed75e52`, for two low findings: the
post-`try_wait` authoritative-clock branch was not truly tested, and ordinary
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
equivalence claim. This final docs-only record is exempt from adversarial
review under the user's instruction; its own exact feature and `main` workflows
remain required and will be reported at handoff.

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
`before_first_wait` barrier so Waker registration deterministically precedes
publication; the native Linux arm64 exact test passed 100/100. Production
source is unchanged. For this remediation, the full local gate, fresh three-
track adversarial review, seal, and exact feature and `main` workflows remain
pending. This test-only fix is not eligible for the documentation-only
exemption. This amendment makes no
product-performance or fx-equivalence claim.

The twelfth slice composes the existing library components only after an
already validated config value is supplied; the thirteenth slice adds validation
that its configured acquisition kind is `Environment` without changing loader
or CLI authority.

The last integrated `NativeReferenceHost` first rejects any loaded selection
other than `ask` / `vercel_ai_gateway` / `ai_gateway_http`. The thirteenth
slice also requires configured credential source `environment`. It then
opens the existing absolute workspace once and clones that retained descriptor
so exactly `copy_file`, `delete_file`, `edit_file`, `file_info`, `glob_files`,
`grep_files`, `list_files`, `read_file`, `rename_file`, and `write_file` share
one opened directory identity. One tool consumes the original and the other
nine consume
identity-preserving clones. This
prevents path replacement between separate tool-construction opens from giving
the tools different roots. The same trusted-host ancestor and subordinate-mount
limits as the individual tool contracts still apply. A separately supplied
session root is retained through `FileSessionStore`; neither root is discovered
from status or configuration, selected by model input, or created. Composition
does not compare opened root identities or reject equality or ancestry. The
trusted host must select disjoint roots. If the session root equals or sits
beneath the workspace, workspace tools can reach session artifacts under their
normal bounded path rules after permission is granted.

Candidate source extends both constructors with `create_folder` immediately
after `copy_file`, so eleven alphabetical tools share the original retained
descriptor plus ten identity-preserving clones. Cycle 2 was not green on two
low evidence/documentation findings. Exact remediation `f527293`, tree
`40eef14`, passes the complete replacement local gate. Cycle-3 candidate
`c1e572e`, tree `b5fb1c2`, is not green only for one low documentation-lineage
finding. Exact lineage remediation `12c11ba`, tree `b96575b`, passes the complete
replacement gate. Gate record `f6f6584` parents tree-identical cycle-4 candidate
`a78b693`, tree `2b913e8`. Cycle 4 found zero production/filesystem defects; its
sole low stale seal-record finding is fixed in the exempt documentation seal.
First feature CI `32699750602` then exposed one Linux-only test Clippy failure;
platform-native `RawMode` remediation `1effcbb`, tree `b5eccb1`, passes the
complete replacement gate. Tree-identical cycle-5 candidate `ff18a9a`, tree
`f77b198`, is green with zero findings in all three fresh tracks. Seal `e75578b`
passed exact feature/main CI and benchmark workflows; this is now the integrated
eleven-tool authority surface.

Delivered source inserts `open_file` immediately after `list_files` in
both constructors. Exactly twelve alphabetical tools share the original
retained workspace descriptor plus eleven identity-preserving clones. This
twelve-tool composition is delivered on `main`.

Production construction opens both non-secret roots before it consumes the
injected credential snapshot, discovers a bearer token, and hands that token to
`AiGatewayHttpTransport`. Selection, workspace, and session-store failures
therefore occur without discovering or handing off a credential. The wrapper
retains only non-secret selected-source metadata and has no secret getter. It
retains the exact loaded configuration, including file origin and observable
schema version `1` or `2` for an accepted legacy file, while the in-memory
projection supplies its provider, transport, model, and acquisition kind.
`NativeReferenceHost::credential_source()` remains a different runtime
observation: it reports the concrete selected OIDC-token or API-key source.

The custom-transport constructor is an explicit trusted authority override. It
performs no native credential discovery or production HTTP construction and
reports `credential_source() == None`. That observation does not prove that the
custom transport is unauthenticated or secret-free. The custom transport owns
endpoint, network, authentication, status, retry, runtime, and diagnostic
policy and must return only an accepted byte stream or an already redacted
provider error under the existing injected-transport contract.

Both constructors are synchronous. Besides bounded construction and
the root opens, they make no network request, poll no prompt, load or save no
session record, create no root or runtime, and start no task, thread, timer,
retry, or other background work. `AskPermissionHandler` retains but does not
invoke the injected prompter. Later production HTTP polling still requires a
live host-owned Tokio runtime with I/O and time enabled and driven through
teardown.

The delivered seventeenth slice changes only the workspace-tool bundle. Path and
prepared-root constructors distribute the original retained workspace
descriptor plus two clones across three tools, all with the already validated
identity. They supply exactly `file_info`, `list_files`,
and `read_file`; core exposes the catalog alphabetically. If a required clone
cannot be made, composition returns the
existing fixed redacted `WorkspaceRoot` stage before engine construction. Tool
construction performs no per-path metadata lookup; `file_info` work remains
inert until its execution future is polled after exact policy approval.

The delivered eighteenth slice changes only that workspace-tool bundle again. Path
and prepared-root constructors distribute the original retained descriptor plus
three clones across exactly `file_info`, `glob_files`, `list_files`, and
`read_file`, all under the same already validated identity; core exposes the
catalog alphabetically. A required clone failure remains the same fixed
redacted `WorkspaceRoot` stage before engine construction. `glob_files`
preflight remains effect-free, and creating its execution future remains inert
until the first poll after exact recursive-enumeration approval.

The nineteenth candidate changes only that workspace-tool bundle again. Path
and prepared-root constructors distribute the original retained descriptor plus
four clones across exactly `file_info`, `glob_files`, `grep_files`,
`list_files`, and `read_file`, all under the same already validated identity;
core exposes the catalog alphabetically. Clone failure remains the fixed
redacted `WorkspaceRoot` stage. `grep_files` preflight is effect-free, and its
execution future remains inert until first poll after exact `SearchContent`
approval. Composition adds no process, network, environment, credential,
session, runtime, or CLI authority.

Reference-host failures retain only a non-exhaustive fixed stage kind:
unsupported selection, workspace root, session store, credential, HTTP
transport, provider, or engine. Component errors, roots, config/model values,
credential bytes and source, endpoint data, prompt data, OS diagnostics, and
raw error numbers are discarded. Host debug output is fixed to
`NativeReferenceHost { .. }` and exposes no config structure or source. The
twelfth-slice behaviors are adversarially green and integrated on `main` under
exact green workflows. The schema-v3 and configured-source validation changes
are integrated through final record `f840576a`, with three fresh adversarial
tracks green on exact SHA `35ce591e` and exact final-record feature and `main`
workflows green.

The integrated fourteenth slice leaves the existing path constructors
unchanged and adds `NativeRootSelection` plus `PreparedNativeRoots`. Selection
is effect-free and consumes only an injected `NativeEnvironment` plus an
explicit absolute workspace path. Preparation opens the existing workspace
first, requires the selected `XDG_STATE_HOME` or fallback `HOME` base to exist,
and follows no
suffix symlink while it opens or creates only `machine-god` or
`.local/state/machine-god`. Newly created directories request `0700`. Existing
directories are never chmodded, chowned, replaced, or removed. The selected
base and existing intermediates must be owned by the effective UID with no
group/other write; the existing final root permits no group/other permission.
A macOS base or suffix must additionally have an empty ACL or only exact
flag-free `DENY DELETE` entries as read from its retained descriptor. This
narrowly accepts the protective `everyone deny delete` entry on ordinary
macOS homes. Any `ALLOW` or unknown tag, any entry flag or ACL-level flag, any
other or combined permission, malformed value, or read failure is unsafe. This
closes the gap where a directory can remain mode `0700` while granting or
inheriting authority for another principal.
A just-created fixed name is normalized descriptor-relatively to `0700` before
the permission-requiring reopen, then identity-checked, `fchmod`ed, and verified
at exact `0700`; its macOS ACL is then required to meet the same restrictive
rule. Existing ACLs are never stripped or repaired. The same-effective-UID
account is the remaining normalization trust boundary. The descriptor-bound
check uses the exact-pinned,
target-macOS-only `calcifer-macos-acl` 0.1.0 narrow safe API; its published
source, checksum, no-normal-dependency graph, dependency policy, and
vulnerability results are reviewed evidence for this slice. Opened workspace and
final state identities must be disjoint in both directions under device/inode
identity and descriptor-relative parent walking. Fixed errors and debug output
reflect no path, environment, identity, ownership, mode, or OS diagnostic.
Accepted state bases are rebuilt from lexical components before the no-follow
open, preventing a trailing slash or `/.` from moving a symlink out of the
final lookup position. A simultaneous create loser can fail closed and retry
while the winner normalizes an owner-bit-masked new directory; it never chmods
the `EEXIST` entry.
Prepared-root reference-host constructors consume the retained descriptors;
production credential discovery follows acceptance of those roots. Production
and focused tests cover this behavior. Formal adversarial review was
green on exact behavior SHA `f1dc4751`; after the post-review Linux lint
normalization at `90d8f96`, all three final tracks are green on exact candidate
`72cf64f6`. Replacement seal `f08dbd9e`, feature record `6f66b6e5`, and exact
`main` delivery evidence are green.

The integrated file-session slice does not consume those status-derived state
paths.
The host explicitly supplies one existing absolute root. On supported Linux and
macOS Unix targets, `FileSessionStore::open` opens the final component
no-follow, verifies the resulting descriptor is a directory, and retains it.
The store performs no environment lookup, root discovery or creation, session
listing, deletion, reset, or arbitrary child-path access. Root ownership,
permissions, quotas, ancestor resolution, and filesystem behavior remain in the
trusted host boundary. Hardened non-Unix support is deferred.

For each validated `SessionId`, lowercase SHA-256 of the ASCII domain separator
`machine-god:file-session:v1:` plus the ID's UTF-8 bytes selects fixed flat
`.json`, `.lock`, and `.tmp` names. This keeps raw IDs out of ordinary filenames
and prevents ID bytes from becoming path syntax, but it does not hide a
guessable ID or record contents and does not provide authentication or
confinement. Descriptor-relative operations under the retained root provide the
path boundary. Load also verifies the exact decoded record ID, so a misplaced
record or theoretical digest collision fails as corrupt state rather than being
merged.

The strict compact schema-v1 envelope is bounded to
`MAX_FILE_SESSION_BYTES` (`8_651_165`), enough for every record under default
`EngineLimits`. Loads retain at most one extra byte to detect exact
overflow, then reject invalid UTF-8, malformed or unknown structure, unsupported
versions, zero revisions, wrong IDs, and over-limit data. The record and
permanent lock are opened no-follow and authoritatively required by `fstat` to
be regular files; special files cannot turn a read into FIFO, device, socket,
or directory access. Missing loads create no artifact. Corrupt and nonregular
artifacts are preserved rather than repaired, replaced, unlinked, or migrated.
The store iteratively enforces core's default aggregate JSON bounds of 64
container levels and 65,536 nodes for direct callers before serialization and
after decode; the file-size ceiling remains a separate resource bound.

Every save is serialized under the byte cap before publication. A permanent
regular no-follow lock sidecar provides exclusive advisory coordination
for cooperating store instances and processes. It is retained so cooperating
processes do not split across recreated lock inodes. A process that ignores the
lock, or an actor able to remove or replace the sidecar, remains outside that
coordination guarantee. Under the exclusive lock, new and update saves compare
the exact stored revision, reject incarnation changes, and assign the next
revision with checked arithmetic. They write through an exclusively created,
no-follow, authoritatively regular `0600` temporary file, synchronize that file,
atomically rename it over the record in the retained directory, and synchronize
the directory.

Before rename, failure preserves the old authoritative record. After rename,
directory-sync failure is necessarily ambiguous: the new record may be visible
but not proven durable, so a caller must load and reconcile rather than blindly
retrying a possibly completed operation. Atomic publication is one complete old
or new record on supported local filesystems honoring the assumed Unix advisory
lock, rename, and sync semantics. It is not a cross-record transaction, a
defense against noncooperating writers, an NFS guarantee, or a promise that
every sudden-power-loss/full-system failure mode preserves the last acknowledged
version. The implementation requests `fsync`; on macOS it does not request
`F_FULLFSYNC` and does not claim a successful save reached physical media.
Record contents are plaintext; encryption, authentication, secure
erasure, key management, and migration remain deferred. Bounded IDs-only
listing is defined by the separate delivered sixteenth slice and is not part of
ordinary `SessionStore::load` or `save`. Reset is
defined only by the separate fifteenth slice and is not part of ordinary
`SessionStore::save`: it requires a new host-generated incarnation and a
reset-specific atomic current-record replacement.

Load and save futures are inert until first poll and spawn no task, thread,
timer, or runtime work. First poll performs bounded synchronous filesystem I/O,
advisory locking, file sync, and directory sync inline. Retained data and
successful transfer work are bounded. Advisory locking, byte transfers, and
`fsync` retry `EINTR`; other interrupted operations use the fixed error
taxonomy. Retry attempts and wall-clock duration are not bounded. Those calls
can block the polling executor thread, and dropping the future cannot interrupt
a synchronous call already in progress. Fixed errors never reflect session
IDs, hashes, roots, child paths,
record bytes, parser diagnostics, OS text, or raw error numbers. The complete
contract and fixed taxonomy are in
[`session-store.md`](session-store.md).

The delivered fifteenth slice gives `NativeReferenceHost` one
`NativeSessionLifecycle` backed by the exact `Arc<FileSessionStore>` already
given to its engine. Lifecycle and session-store observations do not reopen a
path or select another state root. Construction performs no entropy read or
session I/O. Create, resume, replay, and reset are futures with no effect before
first poll and detach no work; first-poll store operations retain the existing
bounded synchronous-I/O and advisory-lock behavior.

Exact shared-store identity is validated, not trusted. A lifecycle constructor
given a different concrete store allocation from the one configured in its
engine fails with fixed redacted `MismatchedSessionStore` before consulting the
incarnation source or filesystem. Reopening the same path is not equivalent:
it could retain a different directory object after replacement and would split
engine turns from lifecycle CAS. Reference-host construction shares one `Arc`
and maps an impossible internal mismatch to its fixed engine build stage.

Production create and reset use a fixed-size OS cryptographic-random draw with
at least 128 random bits per new incarnation. There is no clock, PID, counter,
session-ID hash, model value, or deterministic fallback. Tests may inject a
deterministic source behind the trusted native boundary. Entropy failure is
fail-closed, and reset never republishes the currently stored incarnation.

Create uses the store's absent-record CAS and cannot overwrite an existing
record. Resume accepts only one current-schema, bounds-valid record and refuses
to merge a different local incarnation. Replay returns the successful record
contents deliberately to its trusted caller but invokes no provider, tool,
permission handler, network, UI, or event sink. It is a point-in-time durable
snapshot, not a reconstruction of transient effects.

Reset refuses a locally live lifetime before record replacement and never
force-invalidates a local handle or active turn. Its preceding bounded load may
create the permanent fixed lock sidecar, and its incarnation source may already
have been consulted; neither changes the durable record. Under the per-ID store
lock, reset fences the exact old ID, incarnation, and revision and atomically
renames one empty current-schema replacement. The same ID is retained,
incarnation changes, revision advances with checked arithmetic, and the turn
allocator returns to `1`. Another process's old handle cannot be recalled, but
its later save fails the incarnation fence. Reset does not undo external
effects already dispatched by that process.

Missing operations, duplicate create, local live state, entropy failure, CAS
conflict, corrupt state, unavailable store, and engine/invariant failure remain
distinct fixed lifecycle categories. Diagnostics and debug output expose no
ID, incarnation, revision, record content, random bytes, path, parser or OS
text. An unavailable create/reset may follow a completed rename whose directory
sync failed; callers must resume or replay to reconcile and must not treat the
category as blanket permission to retry. The complete delivered rules are in
[`native-session-lifecycle.md`](native-session-lifecycle.md).

The delivered sixteenth slice's listing future is inert before poll and performs its
bounded synchronous enumeration, record validation, and advisory locking on the
first polling thread without detached work. It can create a private `0600`
permanent lock sidecar for a canonical record, but it cannot write, repair,
replace, delete, migrate, or quarantine record data. Noncanonical and unrelated
names are ignored only after consuming scan budget. A hostile or nonregular
derived lock entry for a present canonical record is corrupt; ordinary lock I/O
failure is unavailable.

The replacement enumeration path first acquires a fresh descriptor-relative
`.` descriptor, then validates the linked identity of that exact acquired
descriptor. On macOS an unlinked retained directory can otherwise reopen `.`;
checking the retained descriptor before the fresh acquisition leaves a liveness
time-of-check/time-of-use gap and is not the replacement contract. A stable
completed rename preserves identity and remains supported. Removal before
acquisition or before the acquired-identity check is unavailable. Concurrent
rename or removal may conservatively yield unavailable or an observation of the
exact acquired identity; it never redirects to a replacement and is not a
global snapshot.

There is no root-wide lock or multi-record snapshot. Each canonical candidate
has an independent point-in-time validation, so concurrent changes to other IDs
can be reflected at different instants. A candidate that disappears before its
locked read may be omitted and may leave its permanent private lock sidecar.
Canonical filenames are sorted before validation; only a fired raw scan cap
makes candidate selection filesystem-iteration-dependent. `truncated`
discloses only that a fixed budget prevented complete observation; it is not a
continuation capability, does not prove another valid ID exists, and does not
promise the globally first ID or semantic subset. Returned IDs are intentionally
disclosed to the trusted caller, and `NativeSessionList`'s derived `Debug`
deliberately exposes the same IDs and `truncated`. Only lifecycle error
`Display` and `Debug` are redacted; they retain no ID, digest, filename, path,
record content, schema detail, parser or OS diagnostic.

Listing receives no live-session registry, incarnation source, provider,
permission, prompt, tool, network, workspace, configuration, environment, or
runtime authority. Because the current schema has no authoritative summary,
workspace, timestamp, latest-order, or index fields, the candidate invents none
from file metadata, messages, metadata maps, or directory order. Full bounds
and non-features are in
[`native-session-listing.md`](native-session-listing.md).

The ask-handler slice adds no ambient authority to core or native. The host
must inject either an owned `PermissionPrompter` or an explicitly shared
`Arc<dyn PermissionPrompter>`. The adapter never reads terminal input, writes
terminal output, inspects environment or configuration, accesses a file or
process, contacts a network, selects an executor, or persists a grant. Such
authority and its security controls belong behind the injected prompter.

For the engine path, core bounds the prepared capability and arguments and
constructs the complete auditable request before the adapter is called. The
adapter forwards that owned request exactly once and does not clone, mutate,
serialize, truncate, revalidate, or traverse it. This preserves the identity,
session incarnation, turn, capability, critical-risk hint, and fixed reason
that core already exposed through `PermissionRequested`; a presentation layer
must not silently authorize a shortened or reconstructed request.

Only structured allow-once, allow-turn, allow-session, and deny values cross
back into the adapter. The first three map to the matching core scopes but are
not cached by either the adapter or core. Deny maps to the fixed host-facing
reason `permission denied`. Prompt infrastructure failure carries no source
data in the zero-data `PermissionPromptError` and maps only to the fixed
`permission_prompt_failed` / `permission prompt failed` core error. It cannot
be treated as approval or ordinary denial.

Authorization is inert until polled and the adapter detaches no work. Dropping
an unpolled future never calls the prompter; dropping a pending future drops the
underlying prompt future. There is no separate cancellation token or revocation
message at this boundary. A prompter must therefore retain prompt work in its
returned future or perform its own drop cleanup; detaching an approval request
that can outlive that future violates the contract. Full polling, redaction,
and deferred-scope requirements are in
[`ask-permission.md`](ask-permission.md).

Tool preflight closes the representation gap between a model's raw JSON call
and the operation presented to permission policy. The source-compatible default
retains the existing raw `Capability::Tool` and arguments. A tool may instead
prepare a normalized `Capability` and replacement arguments, and an allowed
execution receives exactly those prepared arguments. Preparation is an
effect-free validation and normalization step implemented by trusted host code.
It must be deterministic, synchronous, bounded, and nonblocking, and it must not
open a path, start a process, contact a network destination, mutate state, or
exercise any other unapproved authority. Core checks cancellation immediately
before and after preparation returns but cannot interrupt it in flight.

Core applies its resource bounds before policy or execution. Prepared arguments
retain the exact configured tool-argument byte limit and their JSON depth and
node bounds. Capability depth and node traversal covers only JSON values
embedded in `Capability::Tool` or `Capability::Custom`; every capability variant
is separately serialized as a whole under one total byte cap of
`max_tool_argument_bytes + 1024`. The fixed 1 KiB is headroom within that total
cap, not a separately metered envelope or payload allowance. An invalid or
oversized prepared value fails before either boundary. A tool-reported
preparation error is reduced to a fixed generic durable tool error, does not
consult policy, and causes no tool effect. Policy still sees a fresh request
with the existing critical risk, fixed reason, and deterministic permission ID;
core still does not cache positive grant scopes.

The prepared arguments are not a second grant of authority. A trusted tool may
use them only for effects contained by the exact prepared capability that
policy allowed. Native filesystem, process, and network implementations must not
reinterpret them into a broader path, command, or destination. This slice
supplies that authorization seam. Its first native consumer is `read_file`. The
host injects one absolute workspace root, and Unix construction opens and
retains the final root directory without following a final symlink. Before that
open it rebuilds the host root from lexical path components, removing redundant
separators and `.` components throughout while preserving `..`, without
canonicalizing ancestors or resolving symlinks. The resulting removal of
terminal separators and terminal `.` components ensures that `/root-link/` and
`/root-link/.` cannot bypass final-component no-follow; the equivalent forms of
a real directory remain accepted. Provider input is exactly an object with one
UTF-8 `path` string bounded to 4,096 bytes. Effect-free preflight performs only
strict decoding and lexical normalization; it removes `.` components,
collapses repeated separators, and rejects `/`-rooted paths, `..`, forbidden
control or line/paragraph-separator or bidirectional-formatting characters, or
an empty normalized path before policy. On supported Unix, backslash and
Windows-looking prefixes are ordinary confined filename bytes, not path
separators or prefixes.
Policy receives
`FilesystemAccess::Read` for exactly the normalized workspace-relative path
that execution receives.

Allowed execution starts from the retained root descriptor and opens every path
component descriptor-relatively with no-follow semantics. It retains stable
ancestor descriptors during traversal, opens nonblocking, and authoritatively
checks that the final descriptor is a regular file before reading. A symlink in
any model-selected component, a directory, FIFO, socket, device, or other
non-regular final entry fails closed. Path replacement after a component is
opened cannot redirect later lookup through a different ancestor; replacement
of the final pathname after open cannot change which descriptor is read.
Replacement by another ordinary file before its open may change the bytes at
the same authorized path, so this is path authorization, not an immutable file
identity or snapshot guarantee.

The reader retains at most 8 KiB plus one byte to detect overflow or growth and
never truncates a successful result. It accepts only valid UTF-8 and returns
exactly `{content}`; successful content is intentionally model-visible and
therefore also present in the durable tool result and observer event. Failures
use the fixed constructor and tool-error tables in
[`read-file.md`](read-file.md). Operating-system error numbers may select among
those fixed categories, but no OS text, workspace path, requested path, or file
bytes enter their display, debug, code, or message. Only non-cancellation
preparation and execution errors become core's generic durable tool-error
result. A direct `Tool::execute` observes cancellation as the fixed
`read_file_cancelled` error; when the engine owns the shared token, engine
cancellation wins, terminates the turn, and leaves the unknown placeholder
rather than writing a generic tool error. Cancellation is checked before
traversal, between bounded reads, and after content validation. It closes
retained per-call descriptors and buffers but cannot interrupt an individual
open, metadata, or read syscall already in flight.

`list_files` uses the same authorization and confinement boundary for
`FilesystemAccess::Enumerate`. Strict effect-free preflight accepts only `{}` or
a sole string `path`, defaults omission to `.`, applies the 4,096-byte bound and
the same absolute, parent, control, and bidirectional-character rejection, and
gives policy and execution the exact same normalized path. Backslash and space
are literal Unix filename characters. No preflight filesystem effect occurs.

Allowed enumeration opens every selected component relative to the retained
root with directory and no-follow requirements. It never follows a selected or
entry symlink and never opens a child, resolves an entry target, reads content,
recurses, applies ignore rules, accesses an external path, or discovers a
workspace. It skips only `.` and `..`. Entry kinds come directly from the
directory entry type; an unknown type is `other`. Every observed visible name,
including a truncation witness, must be safe valid UTF-8 or the call fails with
a fixed redacted error.

Enumeration retains no more than 100 entries and 16 KiB of aggregate raw name
bytes. It reads the first extra visible entry needed to prove truncation and
then stops, sorting only what it retained. This limits retained memory and
syscall work after a bound is crossed, but a truncated selection can reflect
filesystem iteration order. It is not a whole-directory ordering or snapshot
claim. At the independent maximums, JSON escaping and fixed entry syntax remain
within 44,130 serialized bytes including core's fixed `ToolOutput` envelope,
below the default 64 KiB result limit.

Failures use the fixed constructor and tool-error tables in
[`list-files.md`](list-files.md). No root, requested path, entry name,
operating-system text, or error number enters the fixed public diagnostic.
Cancellation is checked before traversal, between component opens, before
enumeration, between entry reads, and after result validation and sorting. It
cannot preempt one open or directory-read syscall already in flight and spawns
no detached work.

Delivered `file_info` uses the same explicit retained workspace boundary for a
new, narrower `FilesystemAccess::Metadata` operation. That distinct enum value
does not imply content `Read`, directory `Enumerate`, mutation, target-following,
or external-path authority. Strict effect-free preflight accepts exactly a
required sole string `path`, applies the same 4,096-byte lexical normalization
and forbidden-character checks as `read_file`, and supplies the identical
normalized string to policy and allowed execution. Invalid input reaches
neither policy nor filesystem authority. Nonempty current-directory forms
normalize to `.` so the retained root itself is an explicit operation.

Allowed metadata execution acquires a fresh `.` descriptor from the retained
root, then validates that exact acquired descriptor's linked identity. Linux
rejects zero link count. macOS requires a descriptor-relative parent/name
lookup to match its device, inode, and directory type. It then opens only
ancestor directories descriptor-relatively with directory, no-follow, close-on-exec,
and nonblocking requirements. It performs one final descriptor-relative
no-follow metadata operation without opening the final component. An ancestor
symlink or non-directory therefore fails closed. A final symlink reports link
metadata, not target metadata; FIFO, socket, device, and other special objects
report `other` without an open that could block, read, or trigger device
behavior. The operation never reads content, enumerates children, follows a
target, or mutates an object. An explicit `.` operation obtains its result with
one `fstat` of the acquired root after linked-identity validation.

Checked conversion rejects a negative metadata size, an invalid nanosecond
value, or a value outside the public integer representation. Unix seconds are
signed and pre-epoch timestamps remain valid. Extension is derived only from
the already bounded normalized basename and only for regular files: leading-
dot-only and trailing-dot names yield `null`, and only the nonempty suffix after
the last non-leading dot is returned. No MIME sniffing or target/content lookup
occurs. The returned path and extension remain below 17 KiB even under
worst-case JSON escaping; core's configured lower result bound still applies.

The retained descriptor continues to identify the originally opened workspace
after its host path is renamed or replaced. A stable rename preserves the fresh
descriptor's identity; removal before acquisition or linked-identity validation
is unavailable. An already opened ancestor cannot be redirected by later path
replacement. A component replaced before its lookup may be observed. All
returned metadata fields come from one final no-follow `statat`, or one final
`fstat` for `.`, but there is no preflight-time, content, symlink-target, or
continued-existence snapshot. Removal after successful metadata acquisition
does not make the captured result unsafe to return.

The exact fixed errors in [`file-info.md`](file-info.md) retain no root,
requested path, extension, metadata value, operating-system text, or raw error
number. The future performs no work until first poll, checks cancellation before
fresh-root acquisition, after acquisition, around each ancestor open, before
and after final metadata, and immediately before return, and detaches nothing.
It cannot preempt one open or metadata syscall
already in flight. Dropping before poll is effect-free; dropping later closes
per-call descriptors and discards any unreturned result.

Delivered `glob_files` uses the same retained workspace boundary for the new
`FilesystemAccess::EnumerateRecursive` operation. This is separate from one-
level `Enumerate` and does not imply content `Read`, `Metadata`, mutation,
symlink-target, or external-path authority. Strict effect-free preflight accepts
only `{pattern:string,path?:string,mode?:"matches"|"count"}`, independently
bounds requested and normalized path/pattern forms to 4,096 UTF-8 bytes, and
prepares the exact normalized subtree plus explicit pattern/path/mode
arguments. Pattern and mode attenuate results; policy still authorizes recursive
observation throughout the selected subtree.

The pattern grammar is deliberately small and bytewise. Only `/` separates
components; repeated separators and exact `.` segments normalize away, while
absolute, parent, empty-normalized, control, line/paragraph-separator, and
bidirectional-formatting input rejects before policy. Backslash, brackets, and
braces are literal. `?` consumes one UTF-8 byte, `*` consumes zero or more bytes
without crossing `/`, and only an exact `**` segment consumes zero or more
components. Slash-free patterns match basenames recursively; slashful patterns
match candidate paths relative to the selected search root.

Allowed execution first performs the same fresh-`.` linked-root validation as
`file_info`, then opens the selected root and child directories descriptor-
relatively with directory, no-follow, close-on-exec, and nonblocking
requirements. Iterative traversal fully reads, validates, and bytewise sorts
each directory before processing. Hidden entries are included. Only regular
files and final symlinks are match candidates; directories are traversal-only;
specials are ignored; and no link is descended through or target/content read.
Already opened ancestors cannot be redirected. A replacement before an entry's
own lookup may be observed, and a `NOENT` race after enumeration may omit it;
other traversal failures fail the complete call. There is no multi-entry
snapshot.

Both modes complete or fail without partial output under 100,000 visited non-
dot entries, 16 MiB aggregate raw entry-name bytes, directory traversal depth
256, 8,388,608 aggregate matcher-work steps, and 4,096-byte full workspace-
relative candidate paths. Matcher steps meter slashful path splitting,
pattern/DP-state visits, and inner component-byte matching. A directory at
depth 256 is scanned and its candidate children are eligible; a child-directory
open at depth 257 is `scan_limit`. Matches mode emits the longest globally
bytewise-sorted prefix under 100 paths and 16 KiB aggregate raw path bytes,
without backfilling after the first byte-cap omission, and sets `truncated`
exactly when a match is omitted. Count mode returns the exact count. Invalid
entry names, scan caps, I/O, and cancellation use the fixed redacted taxonomy in
[`glob-files.md`](glob-files.md).

The execution future is inert before first poll and performs bounded
synchronous work on that poll. It checks cancellation around root liveness,
selected component opens, each directory read, entry classification and child-
directory opens, match accounting, and return. It cannot preempt one syscall
already in flight and starts no detached work.

Delivered `grep_files` uses the same retained workspace boundary for distinct
`FilesystemAccess::SearchContent`. It conservatively authorizes bounded entry-
name observation and regular-file content inspection at the selected regular
file or beneath the selected directory. It does not imply or inherit `Read`,
`Metadata`, `Enumerate`, or `EnumerateRecursive`, and it grants no mutation,
external-path, or symlink-target authority. Strict effect-free preflight
accepts exactly the eight fields in [`grep-files.md`](grep-files.md), bounds and
normalizes them without I/O, makes all defaults explicit, and gives policy and
execution the same selected path and canonical arguments.

Allowed execution first performs fresh-root linked-identity validation. Every
selected ancestor, selected file/directory, traversed directory, and candidate
regular file is opened descriptor-relatively with no-follow, close-on-exec,
nonblocking, and authoritative type requirements. Entry names are fully read,
validated as confined safe UTF-8, and bytewise sorted. Every complete
descendant path is checked before allocation, entry-kind handling, or include
matching. Hidden names are included. Symlinks are never followed or searched,
even when their targets are inside the workspace. Stable special objects are
skipped without open; a raced nonblocking special open is authoritatively
rejected before read. The optional include is compiled once per call, its
complete parse/match work is metered, and it cannot prune traversal.
Selected-file filtering follows no-follow stat classification and precedes
content open. Fixed literal pattern-table work is charged before selected-root
resolution. Slashful selected-file rejection consumes one charged cancellation-
checked include decision. An excluded selected file consumes that fixed literal
and include work but no candidate, content-byte, or per-file matching work; an
included file opens and is revalidated before those latter budgets. Slashful
candidate splitting checks cancellation at least every 1,024 candidate bytes,
and both recursive and non-recursive dynamic-programming branches route through
the scan-local injectable cancellation checker.

Regular content is accepted only after the complete observed file is no more
than 204,800 bytes, valid UTF-8, and NUL-free. Initial or sentinel-observed
oversize and invalid-text files are excluded and disclosed through aggregate
statistics; an unrelated open/metadata/read failure fails the entire call.
Apparent matches are not retained before whole-file eligibility is known.
Aggregate content bytes, candidate count, traversal/name/depth, include
compile/match work, and literal matcher work all have checked exact caps. The
literal engine is worst-case linear and folds ASCII only when requested,
preventing pattern-length multiplication from becoming unmetered work.

One content buffer is local to one scan. Initialized storage is acquired or
grown before reads in attempted-read windows of at most 8 KiB, with a high-water
length no greater than the 204,801-byte file-plus-witness ceiling. The buffer
logically resets between files. Only logical length, the visible slice, and
checked content-byte accounting advance by bytes actually read. Concurrent and
reentrant scans do not share the buffer. Reset prevents stale bytes from
entering the next logical file; actual per-file and aggregate overflow
witnesses remain retained in the checked content budget.

Matching and requested context derive from one validated logical file view of
that reusable buffer. No path-based context reopen can redirect content or
observe a second file identity. Retained output strings own their bytes before
the buffer is reset. UTF-8-
safe bounded excerpts contain the complete first match, and bounded context
records distinguish line clipping from omitted requested context. Aggregate
path/text and complete serialized-output caps limit retained and escaped data.
Both list modes return deterministic bounded pages only after the complete scan,
with exact eligible-text totals, reusable `next_offset` values under the
67,108,864 offset bound, list-completeness `truncated`,
and explicit context truncation. Count mode has exact eligible matching-line
and matching-file totals and no pagination fields. A fired scan/work cap is a
fixed failure, never a partial success.

The retained/opened descriptors resist path replacement, but the operation is
not a filesystem or content snapshot. A candidate replaced before its own open
may be observed; one that vanishes in the documented `NOENT` window may be
omitted. Once opened, pathname replacement cannot redirect its descriptor.
Concurrent file writes can affect the bounded bytes read. The overflow witness
and aggregate read budget prevent growth from extending work without bound.

The future is inert before first poll and detaches nothing. Cancellation is
checked around root liveness, every directory/file authority operation and
bounded read, at fixed intervals through include compilation, matching, and line
indexing, before every serialization-trimming attempt, and immediately before
return. Recursive and non-recursive include matching use the same injectable
checker. Cancellation cannot preempt one syscall already in flight. Drop closes
every owned descriptor and the one scan-local content buffer.
Fixed errors retain no path, pattern, include, entry name, file byte, match,
metadata, OS diagnostic, or errno. Successful paths, excerpts, context, and
counts are intentionally sensitive model-visible durable data rather than
redacted diagnostics.

Delivered `rename_file` uses a separate typed
`Capability::FilesystemRename { old_path, new_path }` so policy observes both
canonical endpoints of the proposed move. Strict effect-free preflight accepts
only the two required strings, rejects broader or same-canonical-path input,
and gives policy and execution the exact same pair. Allowed Linux/macOS
execution remains rooted in the retained workspace descriptor, reacquires and
validates the linked root, walks both existing parents descriptor-relatively
without following symlinks, and requires an existing regular-file source and
absent destination through both validation passes.

The irreversible boundary is exactly one no-replace `renameat_with` call after
the final cancellation check. It is never retried, including after `EINTR`;
there is no overwrite, parent creation, external path, content read, staging,
or copy/delete fallback, and a directory, symlink, or special source is never
accepted by validation. The documented final source-replacement race can still
move any replacement entry before returning ambiguity. After success, later
cancellation is ignored while the destination's device/inode/type is compared
with the validated source and each unique parent is synchronized under a
16-call cumulative `fsync` cap. Distinct source and destination parents are
both attempted in that order even if the first sync fails; one shared parent
is synchronized once.

A successful rename followed by failed identity verification or durability,
and every `EINTR`, is returned as fixed nonretryable commit ambiguity because
the move may already be durable. `NOREPLACE` prevents destination replacement,
but portable Linux/macOS rename has no source-inode compare-and-swap: a final
different entry installed at the source name can be moved. Postcommit identity
checking prevents false success but cannot undo that move. Fixed diagnostics
retain no paths, entries, metadata, OS text, or errno. The complete normative
boundary is [`rename-file.md`](rename-file.md).

The delivered `copy_file` slice similarly uses typed
`Capability::FilesystemCopy { source, destination }`, so policy observes both
canonical endpoints of the proposed copy. Strict effect-free preflight accepts
only the two required strings, rejects same-canonical-path input, and binds
policy and execution to the same pair. Approved Linux/macOS execution confines
both endpoints to the retained root, rejects directory, symlink, special, and
external endpoints, requires an absent destination, and leaves the source
unchanged. It streams no more than 16 MiB through one 64 KiB buffer under a
4,096-call I/O budget while computing SHA-256; it never allocates the whole
source.

The destination is first a private mode-`0600` stage in the destination parent.
The implementation verifies stable source identity and metadata, stage
identity, digest, ordinary mode, and the documented ACL boundary before a
single `NOREPLACE` commit. After commit it verifies the destination and
synchronizes the destination parent within the bounded postcommit budget.
Cancellation and failure before commit clean up the stage where its identity is
still proven; interruption or failure at and after the commit boundary can be
reported as fixed nonretryable ambiguity. Documented moved-parent,
source-replacement, and same-UID stage races remain explicit rather than being
presented as sandbox guarantees. The full authority, confidentiality,
durability, diagnostic, and race boundary is
[`copy-file.md`](copy-file.md). Its exact feature and `main` delivery gates are
green; this is not a complete fx-equivalence or performance claim.

The composed `create_folder` behavior uses the existing single-path
`Capability::Filesystem { access: Create, path }`. Strict effect-free preflight
binds policy and execution to the same canonical confined workspace-relative
path. Approved Linux/macOS execution rejects every selected symlink and
non-directory ancestor, uses only retained descriptor-relative authority, and
makes at most one `mkdirat` attempt per missing component. An existing final
directory is idempotent success; an existing final non-directory fails.

Each creation requests mode `0755`, but host umask and inherited ACLs
deliberately determine effective permissions. The tool performs no post-create
chmod or ACL normalization, avoiding a broader replacement-race mutation
surface. A hostile umask may make a new intermediate unopenable. The first
successful or uncertain creation is the irreversible boundary; no creation is
retried, cancellation is ignored after that point, and no partial prefix is
removed. Fresh postcommit public-path verification and bottom-up best-effort
durability are capped at 257 sites, 16 calls per site, and 4,112 total sync
calls. Failed verification, moved retained parents, uncertain `mkdirat`, or
durability failure returns fixed nonretryable ambiguity without claiming
rollback. The behavior and local-gate evidence boundary is
[`create-folder.md`](create-folder.md).

The delivered, cycle-6-review-green `open_file` slice
adds no read result and no arbitrary process selection. Strict effect-free
preflight binds policy and execution to one canonical confined path through
dedicated `Capability::OpenFile`. Linux execution no-follow opens and retains
only an
existing regular file before calling fixed `/usr/bin/xdg-open`; the sole
`/proc/<machine-god-parent-pid>/fd/<retained-fd>` argument keeps the helper bound
to the retained identity while the parent owns the descriptor. The helper runs
from `/` with null stdio and the trusted host environment. A trusted injected
launcher seam supports deterministic tests without exposing launcher selection
to the model.

Exactly 32 process-global permits bound production system-launch workers;
saturation is precommit unavailable with zero new worker/helper, and a permit
remains held through arbitrary Waker completion and worker return. The worker
exists before helper spawn. Spawn and cancellation/drop share one
serialized gate: abort-first has zero launch effect and successful spawn commits
the effect. Postcommit cancellation, timeout, or explicit future/drop cleanup
terminates and reaps the direct helper without claiming rollback. Before
publication, cleanup suppresses waking, reaps the helper, drops request/
descriptor ownership, and synchronously joins. Normal published cleanup joins.
Inline or blocking Waker overlap may release the handle to avoid deadlock; only
permit-bounded callback/final bookkeeping may outlive drop after helper/request
cleanup. The amended docs-only rule replaces the impossible frozen absolute
no-worker-detach clause and is exempt from its own review. Postcommit
cancellation, nonzero or
signalled exit, timeout,
and wait failure return fixed redacted, nonretryable result uncertainty when a
tool-level result is observed. There is no postspawn waiter-setup state. Exit
zero reports helper acceptance only, not downstream
application consumption or display. Success is exactly
`{"path":"canonical/relative/path"}`. External paths, directories, URLs, a
real macOS launcher, CLI changes, benchmark changes, performance claims, and
fx-equivalence remain deferred. The product is Rust; Zig is only the pinned
upstream benchmark input. Formal cycle 3 rejected exact candidate
`6815843ac2c8d7731ca6554e5a84772351def850`, tree
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
but requires its own exact feature and `main` workflows, to be reported at
handoff. This makes no product-performance or fx-equivalence claim.

Delivered `create_folder` execution evidence is native macOS plus
Linux/FreeBSD cross-target test compilation, Linux library Clippy, and WASI
compilation/active unsupported
behavior. It does not establish native Linux execution, which remains pending
exact feature CI. Deterministic mixed-device identity traversal covers a
changed-`st_dev` chain without privileged real-mount operations; it is not
subordinate-mount sandbox proof.

These tools provide descriptor-rooted confinement of model-selected path
components, not a claim that an untrusted host is sandboxed. The host's
resolution of ancestor components leading to an injected root path and mount
points visible beneath a retained directory are trusted inputs. Hardened
non-Linux/macOS workspace construction and traversal remain deferred for
`list_files`, delivered `file_info`, delivered `glob_files`, and delivered
`grep_files`; `read_file`
retains its separate
supported-Unix boundary. The normative surfaces are
[`read-file.md`](read-file.md), [`list-files.md`](list-files.md),
[`file-info.md`](file-info.md), [`glob-files.md`](glob-files.md), and
[`grep-files.md`](grep-files.md), with mutation contracts in
[`write-file.md`](write-file.md), [`edit-file.md`](edit-file.md),
[`delete-file.md`](delete-file.md), [`rename-file.md`](rename-file.md), and
[`copy-file.md`](copy-file.md), with the composed pending-delivery behavior in
[`create-folder.md`](create-folder.md).

The injected-transport AI Gateway provider preserves network authority at an
explicit trusted-host boundary. `AiGatewayProvider` accepts only an owned body,
fixed protocol/model/session metadata and a transport object. It cannot select
a URL, resolve DNS, open a socket, configure a proxy, negotiate TLS, read a
credential, interpret an HTTP status, follow a redirect or schedule a retry.
The host's `AiGatewayTransport` owns each of those decisions and must return
only the byte stream of an accepted response or a redacted `ProviderError`.

Consequently, SSRF controls belong where the endpoint is selected and
resolved, not in this codec. A production transport must constrain schemes,
origins, redirects, proxy routing and resolved addresses according to its host
policy, including DNS rebinding and link-local/private address considerations.
Secret controls also remain there: credentials must be attached outside the
request body and fixed codec headers, must not enter debug or error text, and
must be scoped to the selected origin. Authentication, rate-limit, availability
and other status classification must occur before returning a successful byte
stream. The host likewise decides whether an operation is safe to retry; the
codec calls the transport at most once, and exactly once only after a valid
request future is polled through startup. It never retries a possibly delivered
model request.

The optional native `ai-gateway-http` transport is one intentionally narrower
host policy. Production requests can target only
`https://ai-gateway.vercel.sh/v3/ai/language-model`. The alternate
`AiGatewayHttpEndpoint::loopback_http` constructor is explicitly for
deterministic tests and accepts plaintext HTTP only for a numeric IPv4 address
in `127.0.0.0/8` or IPv6 `::1`, with an explicit port and absolute path. It
requires that port to be nonzero and rejects user information, queries,
fragments, hostnames and alternate or encoded IP forms; it does not turn
arbitrary private networks or non-loopback addresses into a production
configuration surface. Endpoint text is bounded to
2,048 ASCII bytes. Redirects and proxies are disabled, so
the bearer credential cannot be redirected or proxy-routed by this client.

The trusted host must pass the bearer value directly through
`AiGatewayBearerToken::new`. The transport itself performs no environment,
file, keychain, interactive, CLI or broader configuration lookup. The value
must be a
1–4,096-byte RFC 6750 bearer `b64token`: one or more ASCII letters, digits,
`-`, `.`, `_`, `~`, `+`, or `/`, followed only by optional trailing `=`
padding. It is attached only as the `Authorization: Bearer` header. Its display
surface does not reveal the value, its debug representation and all transport
diagnostics are redacted, and no dependency text is forwarded. This is a
non-reflection guarantee, not a secure-memory-erasure claim; the injecting host
remains responsible for credential acquisition, lifetime, rotation and origin
scope.

The tenth slice supplies one separate optional acquisition path. An owned
snapshot contains only `VERCEL_OIDC_TOKEN` and `AI_GATEWAY_API_KEY`; a nonempty
OIDC token has precedence, an exactly empty value is absent, and any selected
nonempty invalid value fails closed without fallback. Non-Unicode selection is
distinct from malformed or oversized Unicode bearer input. Results move the
existing bearer type without cloning, and errors retain no value, source, OS
diagnostic, or validator text. Debug and display surfaces are non-reflecting.
At most two validated 4 KiB tokens are retained before selection, but process
lookup may materialize a larger OS value before rejection. Drop clearing is
best-effort and does not cover environment storage, allocator history, HTTP
header copies, or other dependency internals. The adapter does not persist,
rotate, log, print, transmit, or configure a credential by itself.

The Reqwest client disables redirects, proxy use, automatic response
decompression, cookies, retry and referer generation and fixes HTTP/1. A
semaphore bounds active requests, defaulting to 16 with a validated range of
1–64; its permit remains owned by the response stream. Same-endpoint idle
connections may be reused. A connection attempt defaults to 30 seconds and is
bounded above by 5 minutes. The complete request/response stream defaults to 10 minutes and
is bounded above by 1 hour; both durations must be positive and the connect
timeout cannot exceed the total. The total deadline begins before semaphore
acquisition and covers capacity waiting. Dependency frames are split into
public chunks of at most 64 KiB by default; a host may choose 1 byte through 1
MiB. This public chunk cap does not bound Hyper's internal frame/read
allocation, and the codec still applies its
independent chunk, record, undecoded-buffer, total-byte and record-count limits.
The fixed POST request carries the codec headers and JSON body plus the injected
authorization header, fixed `Accept: text/event-stream`, and fixed
`Accept-Encoding: identity`. No response header, error body or dependency
diagnostic is reflected through the provider error.

Only HTTP 200 is accepted as a stream. Status handling is closed and exact:
401/403 are non-retryable authentication errors; 429 is retryable rate
limiting; 408, 425 and every 5xx status are retryable unavailability; all other
4xx statuses are non-retryable invalid requests; and 3xx or any remaining
non-200 status is a non-retryable protocol error. A generic network failure is
conservatively retryable transport failure, while a recognized TLS failure is
non-retryable transport failure. The mapping uses only the status or fixed
failure class and never consumes an error body for diagnosis. Application,
status, and backoff retries are disabled. Hyper may recover a stale reused
connection only before writing request bytes; at most one peer-visible request
is dispatched, and a possibly delivered request is never replayed.

TLS uses Reqwest's Rustls backend with the pinned `webpki-root-certs` dataset;
default Reqwest features and native-root loading are disabled. The repository
explicitly permits that certificate dataset's `CDLA-Permissive-2.0` license.
This makes trust deterministic
and avoids silently inheriting a machine-wide trust store, at the deliberate
cost that enterprise interception roots and private certificate authorities in
the operating-system store are not trusted. Updating trust follows the pinned
Rust dependency rather than an immediate OS trust-store update. The feature is
unavailable and its exports are cfg-gated on WebAssembly targets.

The same core cancellation token reaches the Reqwest work. Cancellation before
dispatch prevents a request; cancellation while active-request capacity, the
upload, response head, or body is pending wakes the wrapper and drops the owned
in-flight operation and semaphore permit.
Dropping a transport future or returned byte stream performs the same ownership
teardown. Machine-god owns no internal runtime and detaches no producer task,
retry, or timer. Reqwest/Hyper owns connection-dispatch tasks on the host
runtime. The concrete transport must be polled inside a live host-owned Tokio
runtime with I/O and time enabled, and that runtime must remain driven through
asynchronous socket teardown. No active runtime handle produces a fixed
redacted failure. A handle without I/O or time violates the API precondition;
Tokio may panic and the abort-on-panic release profile may terminate the host.
Cancellation
and drop bound owned work but do not promise that the operating system, remote
peer or dependency can prove whether already-transmitted bytes were delivered.
The normative transport contract is
[`ai-gateway-http.md`](ai-gateway-http.md).

Machine-god supplies no fx referer, title or user agent and therefore does not
misrepresent the caller's identity. The codec supplies no authorization, team
or endpoint metadata. The optional HTTP transport adds exactly the explicitly
injected authorization value plus its fixed accept headers; a custom host
transport may add its own values only under that host's identity and disclosure
policy. The model, session ID and complete encoded prompt are necessarily
disclosed to the selected transport. Tool descriptions,
schemas, arguments and serialized results may be part of that prompt and must
be treated as sensitive model input.

Untrusted response bytes are accepted only through independent request, chunk,
record, undecoded-buffer, total-response, record-count, message, tool,
streamed-tool-input, final-tool-call and per-call argument limits, plus an
aggregate request JSON-node limit and a fresh node limit for each decoded
response or serialized-argument JSON tree. Strict UTF-8, JSON and
supported-event schema validation rejects malformed, conflicting, duplicate,
provider-executed, incomplete and post-finish state. A final identity can
replace a differing provisional identity only through one unambiguous
same-name, structurally equal explicit-input match. Canonical lookup normalizes
signed floating zero and invalid provisional JSON is retained only as a fixed
marker. An authoritative exact-ID final may replace that marker or unfinished
input, after which bounded delta/end records for its tombstone are ignored.
Bounded blank, comment, non-data and unknown-event records are no-ops.
Partial tool inputs are never emitted. `[DONE]` or EOF without one valid finish
is a failure, so truncation cannot be reported as a normal stop. Provider error
events and a unified `error` finish are redacted protocol failures rather than
model-visible text.

An empty successful response chunk is rejected. A nonempty chunk that produces
no event consumes at most one source item in a poll before the codec schedules
another poll and yields. A malicious always-ready source therefore cannot hold
an executor thread by returning an unlimited sequence of comments, ignored
fields, unknown events, or partial records in one poll.

Construction, request, decoder, bound and cancellation errors use fixed
categories without reflecting prompts, tool values, response records, model
bytes, endpoint data, credentials or transport-controlled diagnostics. Provider
and request debug output is similarly structural. A supplied transport owns the
sanitization of its own error kind, code, message and retryability before the
codec receives it.

Cancellation is checked around request construction and transport startup and
between response chunks, records and yielded events. The provider registers a
cancellation wakeup rather than waiting for a new hostile or stalled response
chunk. The transport receives the same cancellation token and is responsible
for waking and tearing down its pending I/O. When cancellation and a terminal
provider result become ready in the same poll, cancellation wins. Dropping
provider work drops its owned future, byte stream, buffers and partial tool
state. Ready decoder outcomes first deregister their cancellation waiter, so an
inactive poller's waker is neither retained nor spuriously woken later. All
request-side JSON, including ignored metadata, is depth/node checked
before the owned request leaves its guard. An early-rejected, cancelled or
dropped guarded request drains those values with an iterative child stack,
preventing deep hostile input from triggering recursive JSON teardown. Accepted
values are within the safe depth ceiling. No codec task, timer, thread or retry
is detached. A blocking or cancellation-insensitive injected transport violates
its host-side contract and cannot be repaired by the codec. The exact boundary
is documented in [`ai-gateway.md`](ai-gateway.md).

The threat model must cover workspace escape, symlink races, command injection,
permission confusion, SSRF, secret exposure, corrupted state, denial of service,
and cancellation or shutdown races.

Every logical session lifetime has a validated, host-generated
`SessionIncarnationId` persisted in its `SessionRecord`. Core has no clock or
randomness and never derives a fallback. Reusing, resetting, or rewinding a
`SessionId` requires a new globally unique incarnation; stores reject changing
the incarnation of an existing record, and an engine rejects a different
incarnation while that session ID is live. Permission request IDs use the v2
SHA-256 preimage over length-delimited session ID, incarnation ID, turn ID, and
ordinal. This prevents an ID-keyed permission cache from replaying an allow into
a fresh logical lifetime whose turn numbering and tool-call ordinal restarted.
`ToolContext` and `EngineEvent` carry the same incarnation so tool idempotency
keys and event-sink deduplication or audit keys cannot collide across resets
whose session, turn, call, and event sequence identifiers repeat.

Event sinks are untrusted diagnostic producers. A sink failure crosses the
public API only as `event_sink_failed` / `event sink failed`; the original code
and message are dropped, including when cancellation races a ready sink error.
Engine debug formatting likewise never calls `ModelProvider::name` or includes
provider-controlled text; it exposes only fixed structural fields.

Benchmark CI obtains Zig only from the official Zig 0.16.0 HTTPS archive and
verifies SHA-256 digest
`70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
before extracting into a fresh fixed directory under `RUNNER_TEMP`. The workflow
then fails unless the installed executable reports version `0.16.0`. This keeps
the upstream-reference compiler outside the Rust product's dependency and
authority surfaces while binding its CI bytes without a third-party setup
action.
