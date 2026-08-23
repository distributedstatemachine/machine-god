# Native reference-host composition

Status: integrated contract for the twelfth bounded Milestone 03 library slice,
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
exact local gates are green at `45ad91f`. Review, seal, delivery, and exact-
workflow evidence remain **PENDING**. Eighteenth-slice
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

`NativeReferenceHost` composes the existing validated native configuration,
AI Gateway provider and transport boundary, file session store, ask permission
adapter, and—in the nineteenth candidate—five confined read-only tools into one
provider-neutral `Engine`. Their exact membership is `file_info`, `glob_files`,
`grep_files`, `list_files`, and `read_file`, and core exposes that catalog in
deterministic alphabetical order. It is a library surface in
`machine-god-native`. The `machine-god-cli` crate and every existing CLI output
byte remain unchanged.

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
returns exact eligible-text statistics with bounded structured match, file, or
count results. Its frozen candidate contract is
[`grep-files.md`](grep-files.md), and its pending evidence requirements are in
the [`review record`](reviews/m03-grep-files-review-01.md). It changes no
constructor arguments, provider, transport, permission handler, session,
runtime, credential, root-selection, or CLI authority.

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

Candidate standalone `GrepFilesTool` has the same Linux/macOS platform boundary
without requiring `ai-gateway-http`; other targets receive its fixed
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
   identity for the five tools;
3. open the existing absolute session root as `FileSessionStore`;
4. consume the injected `AiGatewayCredentialEnvironment` and discover one
   validated bearer token under its existing precedence rules;
5. move that bearer token into production `AiGatewayHttpTransport`;
6. construct `AiGatewayProvider` with the loaded configuration's projected
   model;
7. wrap the injected prompter in `AskPermissionHandler`; and
8. build `Engine` with exactly `file_info`, `glob_files`, `grep_files`,
   `list_files`, and `read_file`, default `EngineLimits`, and the default
   `NoopEventSink`; core's catalog exposes those names in deterministic
   alphabetical order.

The non-secret workspace and session roots are therefore opened before
credential discovery and bearer-token handoff. A selection, workspace, or
session-store failure neither discovers nor hands a credential to the HTTP
transport. Credential discovery retains its exact precedence and fail-closed
validation contract, and production transport construction retains its pinned
endpoint and HTTP/TLS/status/cancellation policy.

The workspace is opened once with the existing Linux/macOS final-component
no-follow and authoritative directory checks. One retained descriptor remains
with one tool and four descriptor clones of the same opened directory object
feed the others. The candidate composed engine registers exactly the existing
one-level `list_files`, bounded UTF-8 `read_file`, no-follow metadata
`file_info`, bounded recursive `glob_files`, and bounded literal `grep_files`;
it discovers or registers no other tool. This shared retained
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
the same provider and permission adapter, registers the same five tools, and
uses the same default engine limits and no-op sink. It deliberately performs no
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

The nineteenth candidate adds no construction effect beyond the fourth
descriptor clone and retention for the fifth tool. `grep_files` preparation is
effect-free, and creating its execution future is inert. Its first poll performs
the complete bounded synchronous content search after approval; it creates no
task, thread, file, directory, subprocess, timer, or background work.

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
at `45ad91f`. Adversarial reviews, seal, and feature and `main` workflows remain **PENDING**.
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
are green. Milestone 03 remains in
progress because the other remaining native tools, top-level CLI/slash-command
ownership, and composed release-binary end-to-end evidence remain open.
