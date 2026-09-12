# Native MCP protocol boundary

`machine_god_native::mcp::protocol` owns effect-free wire admission and startup
protocol selection. It does not open a transport, publish a catalog, authenticate
a server, grant execution permission or retry an application operation. Native
transport adapters must separately own connection generations, deadlines,
cancellation, aggregate queues, request submission and cleanup.

Modern behavior is informed by fx `b1774fbf6c7602b503026f96f6e960e946c692ef`,
especially `src/core/mcp/protocol_negotiation.zig`, `mcp_runtime.zig` and
`streamable_http.zig`. Older versions and deprecated transports are intentionally
unsupported. Complete-feature delivery status belongs only in the implementation plan.

## Wire admission

`NdjsonDecoder` accepts incremental chunks and returns at most one line per
`push`, together with the exact consumed-byte count. Callers resubmit the
unconsumed tail. Empty input is not EOF. Empty lines and trailing carriage
returns follow pinned stdio behavior; JSON and UTF-8 validation occur only after
a complete frame. `finish` rejects an unterminated frame. Calling `finish` or
exceeding the frame budget closes the decoder; later input cannot revive it.

Scanning stops within the remaining frame budget plus one byte. Partial-frame
allocation grows geometrically within its cap, avoiding repeated full copies
for tiny chunks. A completed frame transfers its storage to the caller, which
must independently bound the number and retained capacity of queued frames.

Default wire limits are 8 MiB per frame, JSON depth 64, and 65,536 JSON values
plus object keys. Caller-selected limits must be positive and cannot exceed
16 MiB, depth 64 or 262,144 nodes. Frame bytes include a trailing CR but exclude
LF. JSON admission charges depth and node budgets while decoding, rejects
duplicate keys including escaped aliases, and rejects trailing JSON. These
stricter duplicate and resource checks are intentional differences from the
producer's permissive JSON-object handling.

`RpcEnvelope` retains a validated JSON-RPC 2.0 request, notification, success or
error response. Conflicting discriminants and malformed IDs are rejected.
String IDs remain distinct from integer IDs; integers use the signed 64-bit
domain and reject floating/exponent spellings. Outgoing integer allocation must
fail or retire the connection at exhaustion, never wrap and reuse an ID.
Errors and debug output omit message content and credentials.

Response correlation requires the exact expected ID. Null-ID errors are allowed
only when the caller explicitly selects the HTTP `server/discover` exception,
including its single admitted modern retry. Null success, stale non-null IDs,
and null-ID ordinary tool responses are not accepted. Generic
envelope admission is not validation of a method's complete result, capabilities,
schema, authentication or authority.

## Startup negotiation

Only stdio and Streamable HTTP are admitted, both using `2026-07-28`.
The configuration codec rejects `type: "sse"`; configuration aliases such as
`local`/`stdio` and `env`/`environment` remain part of the current grammar.

`Negotiation` emits discovery, ready or failure actions. Successful discovery
must be complete, carry a capabilities object and include the modern version
in an all-string supported-version list. Other offered versions do not select an
older mode. Initialization, initialized notifications and downgrade/restart
actions do not exist. Errors, EOF, timeouts and HTTP 404/405 cannot authorize
another protocol or deprecated transport.

Streamable HTTP permits one same-modern discovery retry only for HTTP 400 with
code `-32022`, exact `requested: "2026-07-28"` and an all-string `supported`
array containing that version. The retry retains transport-owned finite
deadlines and exact new-ID correlation. Repeated evidence, an ordinary successful
payload with HTTP 400, or evidence under HTTP 200 does not authorize this retry.
Stdio has no discovery retry. Neither path can retry an application operation.

Modern HTTP includes the protocol header and still supports bounded
[SSE response framing](mcp-sse.md). Event IDs do not authorize reconnect, resume
or replay. Protocol selection remains separate from capability, schema and
execution admission.

## Runtime integration obligations

### HTTP endpoint syntax and origin policy

On Linux/macOS, `mcp::endpoint::McpEndpoint` uses the established `url` parser
for complete endpoint syntax and request-target construction. It is immutable
data, not network or credential authority. Configured URLs are bounded to 4 KiB
before parsing and canonical retained URLs to 16 KiB. Deprecated SSE endpoint
events are not an alternate destination-selection path.

HTTPS is supported; HTTP requires an explicit port and exactly `localhost`,
`127.0.0.1` or `[::1]` in the submitted authority. Numeric and expanded loopback
aliases do not acquire plaintext admission through URL normalization. Explicit
default ports remain admitted even when the parser removes them from canonical
spelling. User information (including empty `@`), fragments, whitespace,
backslashes and malformed percent escapes are rejected. Canonical host/IDNA,
path and default-port normalization follow `url`; same-origin comparison uses
the parsed scheme, host and effective port, never a string-prefix test.

These checks follow the modern `streamable_http.zig` endpoint policy, with
explicit native input limits and
stricter rejection of silent parser repairs. Query strings remain potentially
secret: debug/errors redact them. URL parsing does not resolve DNS, follow
redirects, authorize OAuth or release header credentials. The owned connector
must still enforce those separate boundaries and final submission proof.

[Resolved remote headers](mcp-headers.md) preserve explicitly captured byte
values and pinned OAuth/bearer precedence. Their private identity bytes are
separate from endpoint, configuration and connection-generation admission.

### Transport and runtime ownership

Linux/macOS MCP runtime and explicitly supplied browser-launcher ownership are
available without default features or the AI Gateway HTTP feature. The shared
native URL launcher retains its selected executable, environment and host worker
scope; exposing it does not select HTTP transport or acquire network authority.
MCP HTTP transport remains separately feature-gated, and unsupported platforms
do not gain native launcher support through this composition.

The [owned stdio connection](mcp-stdio.md) supplies bounded bidirectional process
I/O, proof-bearing submission and host-collected cleanup. Negotiation and catalog
publication remain runtime responsibilities, separate from connection readiness.

Transport startup must convert only validated observations into negotiation
events. Ready protocol selection still requires bounded capability/schema
admission, atomic catalog construction and live-generation checks. Selected
tools use the ordinary preparation and permission pipeline described in
[MCP selection](mcp-select-tool.md). Resources and prompts retain the exact
identity and untrusted-result rules in [MCP features](mcp-features.md).
[Catalog page assembly](mcp-pagination.md) provides a complete raw candidate,
with exact cursor correlation, identity uniqueness and earliest-page expiry;
schema admission and live-generation publication remain separate.

No negotiation method accepts an ordinary tool call for replay. A transport
must retain final permission proof through asynchronous queue waits and request
submission, and must not automatically resubmit a partially written or
ambiguously completed consequential operation. Session shutdown, reload,
authentication changes and cancellation require explicit native ownership;
protocol data cannot grant or reconstruct that ownership.

The [one-shot submission boundary](mcp-submission.md) retains a concrete native
proof through direct stdio writes and exact, pre-admitted HTTP plaintext writes
above TLS. Owned HTTP networking/response handling, control/feature authority
and production permission-preparer routing remain separate responsibilities.
The [profile store](mcp-persistence.md) owns configuration publication separately
from runtime activation; callers must preserve save and reload outcomes as
independent facts.

### Lazy Ask discovery and exact turn pins

The actual reference host binds its controller once to the exact native runtime
before engine publication. The runtime retains only a weak reverse link; lookup
data cannot construct a controller, infer configuration or extend host ownership.
Before a model's first search, selection or feature operation pins a publication,
the runtime validates the registered native turn and exact submission registry,
then awaits the controller's configured deferred Ask discovery. Concurrent
waiters share the controller-owned discovery; caller cancellation or turn
retirement ends that wait without cancelling another turn's discovery. The
original context, registry and caller cancellation are revalidated after the wait.

An existing turn pin bypasses discovery and never changes to a newer view.
All-mode and completed deferred discovery are controller no-ops. Ordinary
optional-server failures preserve the usable required publication; global
controller failures, stale authority and owner closure propagate as errors.
No runtime lock is held across discovery, and already-selected tool execution
continues through its original native route and permission proof.

### Authentication refresh before new operations

Prompt admission, the first unpinned model MCP operation and new human feature
commands observe retained authentication leases before selecting a publication.
Within the credential refresh window they await one coalesced, caller-polled
controller job. Cancelling a waiter does not cancel another observer's job;
controller close and configured finite stage budgets still bound its effects.
Existing turn pins and input-required continuations never switch generations or
replay their original requests through this path.

Refresh rebuilds the active configuration snapshot, preserving required-only Ask
startup until deferred discovery has succeeded. It validates that exact saved
source before network/credential effects and again before atomic publication.
Saving configuration alone cannot activate new peers: changed source requires an
explicit reload. Revoked profile or credential authority is an error, not an
anonymous fallback. A failed refresh leaves the old publication selected, but a
credential replacement may already have revoked its authentication; retaining a
publication does not promise that its old credentials remain usable.
