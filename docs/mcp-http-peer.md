# Owned MCP HTTP peers

`mcp::http_peer` composes the [HTTP/TLS connector](mcp-http.md),
[negotiation](mcp-runtime.md), [SSE decoder](mcp-sse.md) and
[raw catalog assembler](mcp-pagination.md). Compatibility follows fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`, especially `streamable_http.zig`,
`legacy_streamable_http.zig`, `legacy_http_sse.zig` and `mcp_runtime.zig`.

## Authority and ownership

Hosts inject the configured transport family, resolved endpoint/address
authority, trust anchors, resolved headers, monotonic clock/timer, lifetime
deadline and cancellation. Unpolled futures perform no acquisition. Peers do
not read ambient DNS, environment, credentials or time; the connector's existing
convenience constructor retains system-clock behavior while peers select its
explicit clock constructor.

There is one serialized application lane and at most one persistent listener.
Callers poll requests or `next_notification` to drive it; no task is spawned.
Pending listener reads survive request selection without being dropped and
recreated. Dropping a polled operation closes the peer and local listener.

## Protocol lifecycle

Modern HTTP is stateless. Discovery, its one admitted retry and same-family
legacy fallback use the shared finite negotiation machine. Malformed success,
redirects, authentication or I/O errors never authorize fallback or POST replay.
Raw JSON bytes are retained beside parsed routing envelopes. Modern SSE ignores
IDs and retry fields, as does the pinned consumer.

Legacy Streamable HTTP captures an optional session ID only from correlated,
validated initialization. IDs are 1–1,024 visible ASCII bytes; duplicate headers
or later session replacement fail. Initialization sends no premature protocol
or session header. Later headers follow the selected version, including the
`2025-03-26` omission. Readiness includes a 202 initialized-notification response.
404 on an owned session fails the operation, without reinitializing or replaying.
Initialization cannot resume before its selected protocol/session is admitted.

Explicit deprecated HTTP+SSE begins with GET endpoint discovery. Same-origin
resolution reuses the exact admitted addresses; foreign or duplicate endpoint
events fail. POST requires 202 acknowledgement and an exact response on the
retained GET, including responses arriving before acknowledgement.

Legacy Streamable HTTP permits a separately started notification GET; 405
reports unsupported listening. Clean listener EOF can reopen the admitted
endpoint with its validated cursor. Legacy POST SSE resumes using GET only after
clean parser EOF and a nonempty cursor, or the pinned `2025-11-25` empty-ID
priming condition. Malformed/partial EOF does not resume. Retry hints use the
injected timer and original deadline. Cursor changes follow complete event
admission; modern hints cannot authorize resumption.
Hint accounting restarts for each reopened stream, matching the pin: a resumed
POST stream must provide fresh resume evidence before another GET is permitted.

## Submissions and events

Runtime allocations are explicitly admitted. Application IDs are reserved once
before permission preparation and never reused. `call` consumes an exact
`McpSubmission` and immutable projected head, checking allocation, reserved ID,
endpoint and every selected base header. Only modern method/name/parameter
projection fields may be added. The connector compares every frozen request
byte and retains native proof at the final plaintext writer above TLS. Tool
cancellation remains observed while a separate legacy GET carries the response.
An observed legacy cancellation/deadline after submission permits one best-effort
cancelled notification within 100 ms while the peer owner remains live. It does
not prove remote cancellation. Abandoned futures spawn no notification worker.

`reserve_tool` returns a non-clone reservation for the typed request to own
through permission and submission. Up to 64 independent unsent reservations
allow cross-turn preparation while the actual wire exchange remains serialized;
catalog controls can run between unsent requests. Its weak peer slot becomes reclaimable when
an unsent request is dropped or denied. The final call checks allocation identity
as well as its wire ID, so a stale owner cannot release another call's slot and
a raw request with the same number cannot consume a leased reservation.
Destruction performs no callbacks or network work. The manual `reserve_tool_id`
API remains single-exclusive and cannot mix with live owned reservations.
Explicit discard clears only the manual slot; neither form reuses consumed IDs.

Peer operations retain an exclusive owned lane across listener reconnect delays.
Their futures can enter the native tool's `Send` execution path without requiring
the peer's owned reader future to be `Sync`. This changes no retry, deadline or
wire-serialization policy and creates no detached task.

Typed catalog controls return raw candidates, not executable publication.
Notifications/progress remain bounded untrusted envelopes. Modern server
requests fail as at the pin; legacy request-stream server requests receive a
fixed unsupported reply, not consent or continuation authority. Successful
elicitation/continuation replies remain a separate runtime boundary.

## Bounds and cleanup

Peer lifetime is explicitly host-selected, independently of the connector's
24-hour maximum for one exchange; each operation still has a finite deadline. IDs use
positive signed-64-bit integers and fail on exhaustion. Limits are 2,048 runtime
allocations, eight live exchange observations, one listener, 64 queued events
totaling 1 MiB, and 4,096 admitted events per caller operation. Each response stream
processes at most 1,024 events; JSON frames are at most 8 MiB. Connector/SSE
byte, line and depth limits apply independently; one stream has a 64 MiB body
budget. Requests allow at most eight GET resumptions and listeners 32 reconnects
per caller operation, bounding reopened streams without exhausting a healthy
peer's lifetime through prior completed work. Queue retention does not reset
between operations. Retry delays are
capped at 60 seconds and cursors at 4 KiB. Authentication challenges retain at
most eight fields totaling 16 KiB for separately authorized handling.

`close` releases local ownership, without remote-revocation claims. Explicit
`shutdown` attempts one bounded session DELETE and reports confirmed, unsupported,
ambiguous, not-attempted or not-needed independently of local completion. Once
`close` completes, later shutdown never reopens its ownership ledger or sends
new bytes; a remaining session reports not-attempted. Completion observers
wait for retirement and release of all retained socket owners, including
abandoned futures; they do not drive work or prove remote effects were revoked.
Debug/display omit endpoints, IDs, headers, challenges and payloads.

DNS composition, authentication effects, executable publication, permission and
consent/continuation routing, and CLI activation retain native runtime ownership.
The [implementation plan](implementation-plan.md) is the sole live gate ledger.
