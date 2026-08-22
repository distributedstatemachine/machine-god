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

Milestone 03 has eleven integrated bounded slices and a twelfth bounded
candidate whose normative documentation is present but whose implementation,
tests, composition, review, remote gates, and delivery are pending.
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
The twelfth candidate defines a Linux/macOS-only `NativeReferenceHost` behind
the existing `ai-gateway-http` and non-WebAssembly gate. It would compose an
already validated config selection with the provider, either production HTTP
from an injected credential snapshot or a trusted custom transport, one shared
retained workspace feeding exactly `list_files` and `read_file`, the existing
file store under a separate session root, and the ask adapter over an injected
prompter. It uses default engine limits and the no-op event sink. The CLI
remains byte-unchanged and thin. The candidate contract and pending lineage are
in [`native-reference-host.md`](native-reference-host.md) and its
[`candidate review`](reviews/m03-native-reference-host-review-01.md).
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
[`read_file` contract](read-file.md) and
[`list_files` contract](list-files.md) do not change either CLI surface. The
separate [`AI Gateway provider contract`](ai-gateway.md) also remains a library
surface and does not change CLI bytes. The same is true of the normative
[`native file session store`](session-store.md). The ask-handler slice is
also a library surface and does not change CLI bytes or supply a concrete
terminal prompt. Credential discovery is another separate
library surface; credentials are fields in neither supported configuration
schema, and discovery by itself does not add CLI composition. The twelfth
candidate's production library constructor would consume an explicitly
injected credential snapshot; it does not give the config loader or CLI ambient
credential authority.

```text
process environment -> resolved native paths
 config path --+-> final metadata -----------> status -> CLI text/JSON
               +-> bounded read-only loader -> strict v1/v2 config data
 state path ------> final metadata -----------> status -> CLI text/JSON

host-selected absolute workspace -> retained directory authority
 model {path}  -> lexical preflight -> Read policy      -> read_file
 model {path?} -> lexical preflight -> Enumerate policy -> one-level list_files

host-selected endpoint/auth/status/retry transport
       -> injected byte stream -> AI Gateway codec -> ModelEvent stream -> core

host-injected bearer token -> optional bounded native HTTP transport
                           -> the same injected AI Gateway codec boundary

owned VERCEL_OIDC_TOKEN / AI_GATEWAY_API_KEY snapshot
 -> bounded native credential discovery -> explicit bearer token -> transport

host-selected existing absolute state root -> retained directory descriptor
 SessionId -> domain-separated SHA-256 v1 name -> bounded file SessionStore

core-owned bounded PermissionRequest -> AskPermissionHandler
                                     -> injected PermissionPrompter
                                     -> structured allow/deny decision

validated LoadedNativeConfig + existing workspace/session roots
 + injected PermissionPrompter
 + credential snapshot -------------> production AI Gateway HTTP
 |                                    -> NativeReferenceHost -> Engine
 + trusted custom transport override -> credential_source: None
```

The config and state roots resolve independently. A nonempty XDG root wins and
must be absolute Unicode; an invalid selected root fails that location without
trying `HOME`. An empty XDG value falls back to a nonempty absolute-Unicode
`HOME`. Missing or empty `HOME` makes a needed fallback unavailable. The only
paths produced are `<config-root>/machine-god/config.json` and
`<state-root>/machine-god`, with `.config` and `.local/state` inserted for the
respective `HOME` fallbacks.

Status inspection remains deliberately shallower than configuration loading. It
uses `symlink_metadata` on the final path, reports
missing/inaccessible/wrong-kind states, and treats a final symlink as
wrong-kind. It does not open, read, or parse the config file. Permission mode is
fixed to `ask`; the CLI does not construct an engine, register `read_file` or
`list_files`, or prompt for permission. The CLI serializes paths as JSON strings
even in human status so path contents do not become terminal controls. Bare
invocation keeps the bootstrap identity contract. Help, version, status, and
argument errors remain byte-stable presentation behavior, not an engine-owned
command model.

The synchronous loader resolves only the config location. In the eleventh
slice, an unavailable location or missing file yields the explicit built-in
schema-v2 object
`{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"zai/glm-5.2"}`.
Invalid selected environment input and all other load failures fail closed. A
present file must be at most 64 KiB of valid UTF-8 and must be either that exact
five-field v2 shape, with any 1–128-byte visible-ASCII model, or the exact strict
legacy v1 shape
`{"schema_version":1,"permission_mode":"ask"}`. Unknown, duplicate, missing,
wrong-type, or unsupported fields and values are rejected in each schema.

An accepted v1 file maps only in memory to provider `vercel_ai_gateway`,
transport `ai_gateway_http`, and model `zai/glm-5.2`; its observable
`schema_version()` stays `1`. Loading does not migrate or rewrite the file.
`NativeConfig` exposes `schema_version`, `permission_mode`, `provider`,
`transport`, and `model` getters and owns the bounded model string, so it and
`LoadedNativeConfig` are cloneable but not copyable. Debug output redacts the
model. `CONFIG_SCHEMA_VERSION` is `2`, `AI_GATEWAY_DEFAULT_MODEL` is
`zai/glm-5.2`, and `AI_GATEWAY_MAX_MODEL_BYTES` is `128`. The config model and
AI Gateway provider use one visible-ASCII model validator.

The closed `NativeProviderKind::VercelAiGateway` and
`NativeTransportKind::AiGatewayHttp` values expose the stable names
`vercel_ai_gateway` and `ai_gateway_http`. These values are declarative. In
particular, the HTTP enum is present when the optional concrete
`ai-gateway-http` feature is disabled and on WebAssembly, where that transport
is unavailable; valid config does not prove a usable transport implementation.

On the supported Unix targets exercised by Milestone 03, the loader opens the
final path no-follow and nonblocking. A preliminary path-kind check is followed
by authoritative opened-descriptor regularity validation. The loader retains at
most the 64 KiB cap plus one byte and never writes, creates, or canonicalizes.
Hardened open semantics for non-Unix targets remain deferred. Typed diagnostics
distinguish failure classes without reflecting selected paths, file contents,
model values, or operating-system error text. Credentials are deliberately in
neither schema. The separate twelfth candidate would consume this already
loaded value without changing the loader: a file-backed v1 remains observable
as version `1` while its fixed projected values drive composition. Configuration
mutation or migration, permission modes beyond `ask`, a concrete prompt UI,
runtime ownership, token fields in config, CLI composition, executable native
tools other than the bounded `read_file` and `list_files` library capabilities,
session migration/encryption/reset/listing, required-root lifecycle, CLI
expansion, and composed release-binary end-to-end evidence remain open.

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

The twelfth candidate packages one exact selection of those existing
boundaries. Both synchronous constructors first validate permission mode `ask`,
provider `vercel_ai_gateway`, and transport `ai_gateway_http`. They then open
one workspace directory once and clone its retained descriptor so exactly the
`list_files` and `read_file` tools share one opened directory identity. A
separate existing session root is opened through `FileSessionStore`. The
injected `PermissionPrompter` is wrapped by `AskPermissionHandler`, and
`AiGatewayProvider` receives the loaded config model and either the production
or custom transport. `EngineBuilder` receives no explicit limits or event sink,
so the documented defaults and `NoopEventSink` apply.

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
The wrapper retains the exact `LoadedNativeConfig`, including a file-backed v1
origin and observable version `1` while using its projected provider,
transport, and model values. Its fixed debug form exposes no config structure
or source. Every nested construction error is reduced to one fixed redacted
stage in the non-exhaustive reference-host error taxonomy. These are candidate
requirements, not evidence of implementation or delivery on this branch.

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

The retained roots confine model-selected components, but they are not sandboxes
against the hosts that selected a workspace path. Resolution of a root path's
ancestors and mount points beneath a retained root belong to that trusted host
boundary. Hardened construction and traversal on non-Unix targets remain
deferred.

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
