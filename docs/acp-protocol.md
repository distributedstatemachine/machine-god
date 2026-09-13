# ACP wire boundary

`machine_god_native::acp::protocol` provides effect-free modern ACP v1
JSON-RPC framing, exact JSON envelopes and outbound correlation. The native ACP
driver owns session state, permission and continuation custody, transport I/O,
output backpressure and shutdown. Correlation labels grant no authority.

The only supported initialization version is integer `1`. There is no older
version negotiation or deprecated SSE transport. Modern MCP Streamable HTTP
response SSE framing is a separate transport concern and remains supported.

## Envelope and resource contract

- Each message is one UTF-8 JSON object terminated by a newline. CRLF is
  accepted as JSON trailing whitespace. Batches, blank lines, duplicate object
  keys (including escaped-equivalent keys), missing JSON-RPC `2.0` and ambiguous
  request/response envelopes are rejected.
- IDs are signed 64-bit integers or strings up to 1,024 UTF-8 bytes. Numeric IDs
  cannot use fractional or exponent notation. Null IDs occur only on error
  responses. Methods are nonempty, control-free strings up to 256 bytes. Params,
  when present, are objects or arrays. Unknown members are inert extensions and
  pay the same admission budgets as known members.
- The frame ceiling is 8 MiB excluding the newline. JSON admission applies a
  64-container depth limit, 65,536 value/object-key token limit and conservative
  32 MiB retention charge (twice source bytes plus 256 bytes per token) before
  allocating the decoded tree. Syntax and UTF-8 validation then use core's exact,
  duplicate-rejecting codec. Arbitrary-precision payload numbers retain their
  original tokens, including negative zero and exponent spelling.
- Incremental decoding retains at most one bounded incomplete frame. It emits
  one oversize error, drains that frame through its newline and resumes framing.
  There is no internal decoded-message queue. EOF rejects unfinished frames,
  even syntactically complete JSON lacking its delimiter, and reclaims partial
  bytes. The driver must still finalize its native owners at EOF.
- Encoding checks constructed envelope shapes, raw payload bytes and JSON
  depth/node limits before recursive serialization, writes through a bounded
  output buffer, then applies the same exact lexical budget. It does not clone
  payload trees. Encoded output
  includes the final newline; callers must separately bound queued frames.
- Debug and error diagnostics omit IDs, methods, request/response payloads and
  remote error text. Explicit wire serialization naturally contains those data.

## Correlation ownership

One connection-scoped `AcpPendingRequests` table admits at most 32 outbound
requests. Monotonic host string IDs are not reused after completion,
cancellation or table clearing; counter exhaustion rejects admission. The
driver must retain this table for the connection lifetime.

Each entry carries non-authoritative session-incarnation, turn, operation and
round labels. Completion requires the full exact scope and leaves an entry
untouched on mismatch. Duplicate or invalidated replies cannot settle another
request. Session/turn invalidation and connection clearing return removed IDs
and labels so the driver can settle its separately held native waiters. Merely
dropping this metadata table is not native cancellation or finalization.

These primitives do not advertise client capabilities, grant filesystem or
terminal authority, persist injected MCP configuration, or implement legacy
session import. The complete native ACP feature composes those ownership
boundaries under the [implementation plan](implementation-plan.md).

`acp::client_requests::NativeAcpClientRequests` keeps the connection-lifetime
correlation table together with the actual native inbox and client URL endpoint.
Its explicitly injected permission-context registry supplies the exact live
authorizing call; a tool name, request-ID conversion or observed event order is
not a substitute. The native inbox displays one request at a time, and this
owner retains only that view. It returns at most one bounded encoded frame per
poll, which the I/O driver must retain in its empty bounded output slot until
written. Repeated polls do not retransmit unanswered requests.

Unknown and duplicate reply IDs cannot settle a native waiter. Invalid or remote
error responses cancel the exact pending prompt. Native waiter abandonment wakes
the connection and releases its stale correlation without blocking the next
prompt. Session reactivation invalidates even accepted-but-unconsumed replies
and never reuses outbound RPC identifiers. EOF and output failure close prompt
admission; they do not replace the session driver's owned native settlement.
Modern `elicitation/complete` is encoded only from the registered operation's
actual completion notice, not from answer acceptance.
