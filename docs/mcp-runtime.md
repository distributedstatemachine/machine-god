# Native MCP protocol boundary

`machine_god_native::mcp::protocol` owns effect-free wire admission and startup
protocol selection. It does not open a transport, publish a catalog, authenticate
a server, grant execution permission or retry an application operation. Native
transport adapters must separately own connection generations, deadlines,
cancellation, aggregate queues, request submission and cleanup.

Compatibility follows fx `b1774fbf6c7602b503026f96f6e960e946c692ef`, especially
`src/core/mcp/protocol_negotiation.zig`, `mcp_runtime.zig`,
`streamable_http.zig` and `legacy_streamable_http.zig`. Complete-feature delivery
status belongs only in the implementation plan.

## Wire admission

`NdjsonDecoder` accepts incremental chunks and returns at most one line per
`push`, together with the exact consumed-byte count. Callers resubmit the
unconsumed tail. Empty input is not EOF. Empty lines and trailing carriage
returns follow pinned stdio behavior; JSON and UTF-8 validation occur only after
a complete frame. `finish` rejects an unterminated frame. Oversize input or
completion closes the decoder; later input cannot revive it.

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
and null-ID initialize or ordinary tool responses are not accepted. Generic
envelope admission is not validation of a method's complete result, capabilities,
schema, authentication or authority.

## Startup negotiation

Configured transport families remain distinct; version fallback never changes
Streamable HTTP into deprecated HTTP+SSE.

| Configured family | Modern version | Legacy versions |
| --- | --- | --- |
| Stdio | `2026-07-28` | `2025-11-25`, `2025-06-18`, `2024-11-05` |
| Streamable HTTP | `2026-07-28` | `2025-11-25`, `2025-06-18`, `2025-03-26` |
| Deprecated HTTP+SSE | none | `2024-11-05` |

`Negotiation` emits explicit discovery, initialization, old-connection restart,
ready or failure actions. A version decision does not itself create or retire
a connection. Stdio legacy restart progresses monotonically downward; the
transport must finish ownership of the old attempt before the requested restart.
Legacy readiness still requires `notifications/initialized` at the transport.

Well-formed ordinary stdio discovery errors start the newest legacy attempt;
stdio `-32021` remains a terminal modern failure. Explicit discovery timeout or
clean connection-close evidence may select the oldest stdio legacy version,
but only while the overall operation remains live. Cancellation, overall
deadline expiry, malformed success, and partial framing are not that evidence.

HTTP fallback follows its separate response/status rules. A supported-version
error carrying the exact admitted modern-version signal permits at most one
modern retry. The corresponding ordinary successful discovery payload is not
retry evidence. HTTP discovery `-32021` follows the pinned ordinary-error
fallback path, unlike stdio. All supplied supported-version entries must have
the admitted shape; malformed metadata is not silently ignored.

Only legacy `2025-03-26` Streamable HTTP omits the protocol header, and only
legacy `2025-11-25` allows the pinned empty priming/poll-close behavior.
Deprecated SSE endpoint discovery and connection lifecycle remain separate
transport responsibilities.

## Runtime integration obligations

Transport startup must convert only validated observations into negotiation
events. Ready protocol selection still requires bounded capability/schema
admission, atomic catalog construction and live-generation checks. Selected
tools use the ordinary preparation and permission pipeline described in
[MCP selection](mcp-select-tool.md). Resources and prompts retain the exact
identity and untrusted-result rules in [MCP features](mcp-features.md).

No negotiation method accepts an ordinary tool call for replay. A transport
must retain final permission proof through asynchronous queue waits and request
submission, and must not automatically resubmit a partially written or
ambiguously completed consequential operation. Session shutdown, reload,
authentication changes and cancellation require explicit native ownership;
protocol data cannot grant or reconstruct that ownership.
