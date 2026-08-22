# Native reference-host composition

Status: integrated contract for the twelfth bounded Milestone 03 library slice.
Fourteen slices are now integrated. This slice's production implementation, an
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
adapter, and two confined read-only tools into one provider-neutral `Engine`.
It is a library surface in `machine-god-native`. The `machine-god-cli` crate and
every existing CLI output byte remain unchanged.

The composed fifteenth-slice candidate adds a `NativeSessionLifecycle` owned by
this wrapper. It supplies durable by-ID create, resume, replay, and reset over
the exact file-store instance shared with the engine; the caller still supplies
the validated session ID and production native code supplies OS-random
incarnations. Production and fourteen independently owned focused tests are
green; formal review and exact delivery remain pending. Its normative behavior is in
[`native-session-lifecycle.md`](native-session-lifecycle.md).

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

The fifteenth candidate retains this exact gate. It does not make the
standalone core engine or session-store trait depend on OS randomness, a native
filesystem, the HTTP feature, or a runtime.

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
   identity for the two tools;
3. open the existing absolute session root as `FileSessionStore`;
4. consume the injected `AiGatewayCredentialEnvironment` and discover one
   validated bearer token under its existing precedence rules;
5. move that bearer token into production `AiGatewayHttpTransport`;
6. construct `AiGatewayProvider` with the loaded configuration's projected
   model;
7. wrap the injected prompter in `AskPermissionHandler`; and
8. build `Engine` with exactly `list_files` and `read_file`, default
   `EngineLimits`, and the default `NoopEventSink`.

The non-secret workspace and session roots are therefore opened before
credential discovery and bearer-token handoff. A selection, workspace, or
session-store failure neither discovers nor hands a credential to the HTTP
transport. Credential discovery retains its exact precedence and fail-closed
validation contract, and production transport construction retains its pinned
endpoint and HTTP/TLS/status/cancellation policy.

The workspace is opened once with the existing Linux/macOS final-component
no-follow and authoritative directory checks. One retained descriptor remains
with one tool and a descriptor clone of the same opened directory object feeds
the other. The composed engine registers exactly the existing one-level
`list_files` and bounded UTF-8 `read_file`; it discovers or registers no other
tool. This shared retained identity prevents separate path opens from selecting
different workspace directory objects if the host path is replaced between
tool construction steps. It does not make the workspace a sandbox against the
host, change either tool's model-selected path rules, or freeze mounts beneath
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
the same provider and permission adapter, registers the same two tools, and
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

The composed fifteenth candidate's construction only shares the retained
`FileSessionStore` with `NativeSessionLifecycle`; it performs no entropy read,
session load, save, reset, engine registration, or lock-sidecar operation.
Those effects are owned by a lifecycle future and remain inert until that
future is first polled.

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

The composed fifteenth candidate adds host observations for the retained
`FileSessionStore` and `NativeSessionLifecycle`. `session_store()` and
`session_lifecycle()` expose components backed by the same shared store
allocation that the engine received during construction. They do not reopen
the selected path, re-resolve status, or create a second store with potentially
different retained identity. Successful lifecycle replay deliberately returns
the bounded durable record contents to its trusted caller; lifecycle and host
debug output still reflects no record or identity data.

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
Neither that root slice nor this integrated
composition implements a concrete terminal `PermissionPrompter`, allocates a
session ID or `SessionIncarnationId`, or adds create/list/resume/replay/reset
session lifecycle commands. It does not add the remaining native tools, compose
or run the CLI, or change any existing CLI byte. A reset under a reused session
ID still requires a new host-generated incarnation before reuse.

The composed fifteenth candidate fills the library-level by-ID create, resume,
durable-record replay, and reset sub-boundary. `NativeSessionLifecycle` uses the
exact store shared with the engine, allocates new incarnations from production
OS randomness, persists create before success, and resets by atomic
current-record replacement with a checked advancing revision. It does not add
session listing, session-ID generation, a UI/event replay, or any CLI command;
formal review and delivery gates remain pending.

Deterministic end-to-end evidence through a freshly built release binary,
remaining CLI ownership, compatibility promotion, and product-performance
claims remain open. The slice does not alter the pinned fx inventory,
benchmark workloads, or workflows. Zig remains only the pinned upstream
benchmark build input; machine-god remains a Rust product.

The frozen reference-host composition checklist item is complete: the
implementation, independent tests, composed adversarial review, exact feature
gates, fast-forward integration, and exact `main` gates are green. The combined
credential-and-configuration item is also complete. The thirteenth slice's
three adversarial tracks are green on exact behavior SHA `35ce591e`, and exact
final-record feature and `main` workflows are green at integrated SHA
`f840576a`. The combined root-and-session-lifecycle item remains unchecked:
delivery of the root sub-boundary leaves create, list,
resume, replay, reset, and reset/new-incarnation behavior open in the integrated
baseline. The fifteenth candidate covers create/resume/replay/reset but leaves
native listing open, so the combined item remains unchecked. Milestone 03
remains in progress.
