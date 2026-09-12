# Owned MCP HTTP peers

`mcp::http_peer` composes the [HTTP/TLS connector](mcp-http.md),
[negotiation](mcp-runtime.md), [SSE decoder](mcp-sse.md) and
[raw catalog assembler](mcp-pagination.md). Compatibility follows fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`, especially `streamable_http.zig`,
`legacy_streamable_http.zig`, `legacy_http_sse.zig` and `mcp_runtime.zig`.

## Authority and ownership

Hosts inject the configured transport family, resolved endpoint/address
authority, trust anchors, resolved headers, monotonic clock/timer, lifetime
policy and cancellation. `McpPeerLifetime::OwnerControlled` retains the peer
until explicit close or cancellation; `Until(Instant)` additionally enforces an
exact host-selected expiry. No artificial session-expiry timestamp is inferred
from startup or a completed operation. Unpolled futures perform no acquisition. Peers do
not read ambient DNS, environment, credentials or time; the connector's existing
convenience constructor retains system-clock behavior while peers select its
explicit clock constructor.

There is one serialized application lane and at most one persistent listener.
Callers poll requests or `next_notification` to drive it; no task is spawned.
Pending listener reads survive request selection without being dropped and
recreated. Dropping a polled application operation closes the peer and local listener.
An idle `next_notification` observation can instead be dropped or time out without
retiring a live peer or losing its pending read/parser state. A pending listener
GET reconnect, including its retry delay and partial acquisition, stays owned by
the peer and retains its original finite acquisition deadline; a later observer
cannot restart that GET or extend that deadline. Each observation retains the
existing event/reconnect limits. Cancellation, owner expiry, malformed input and
failed reconnect acquisition still retire it. A completed event at the
deadline boundary is retained without publishing a late successful observation.

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
The separate [`feature`](mcp-feature-runtime.md) method executes all seven typed
feature actions with native-selected command/turn authority, exact internally
minted IDs and fixed method headers. It returns complete admitted data, not
permission, continuation or publication authority. Each write/flush and retained
connect/read lifetime observes the selected guards, including legacy GET resume.
Notifications/progress remain bounded untrusted envelopes. Modern server
requests fail as at the pin; legacy request-stream server requests receive a
fixed unsupported reply, not consent or continuation authority. Successful
elicitation/continuation replies remain a separate runtime boundary.

## Bounds and cleanup

Peer lifetime is explicitly host-selected, independently of the connector's
per-exchange ceiling; each operation still has a finite deadline. IDs use
positive signed-64-bit integers and fail on exhaustion. Limits are 2,048 runtime
allocations, eight live exchange observations, one listener, 64 queued events
totaling 1 MiB, and 4,096 admitted events per caller operation. Each response stream
processes at most 1,024 events; ordinary JSON frames are at most 8 MiB. Typed
feature operations explicitly select up to 16 MiB and 262,144 nodes, restoring
ordinary limits when the operation settles or drops. Persistent legacy SSE
readers allow the existing 16 MiB line/data ceiling for later feature responses,
without widening independent notification or per-operation admission budgets. Connector/SSE
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

## Observed configured startup

`connect_observed` adds a synchronous fallible completion observer and configured
startup duration to the explicit outer deadline. The observer receives the inert
peer's completion on poll, outside locks and before network effects. Rejection,
callback unwind, cancellation, failed initialization and abandoned futures retire
the same observed owner. There is no detached observer or listener task. Hosts
retain observations in a bounded ledger and prune only completed owners.

An optional `first_attempt_deadline` carries the initial budget already consumed
by caller-owned credential refresh and DNS. It can only shorten the first
configured attempt; an expired first budget rejects before observation/effects.
It does not shorten the fresh legacy fallback timeout. Omitting it starts the
initial budget when the peer future is polled.

The returned `(peer, attempt_deadline)` preserves the remaining initial tools
catalog budget. Modern discovery and its admitted retry share one deadline;
same-family legacy fallback gets a fresh configured timeout, bounded by the outer
deadline and selected peer lifetime. Legacy initialization/version exchanges and
initialized notification share that legacy attempt. Explicit deprecated SSE
endpoint discovery and initialization share one attempt. Errors do not add
automatic full-startup or application retries.

Configured timeouts admit positive durations through `u32::MAX` milliseconds,
with checked `Instant` addition. Observed peers select a separate configured
connector policy admitting that same maximum for each finite exchange; existing
public connector constructors and the original peer `connect` retain their
24-hour exchange ceiling. No configured timeout is silently clamped. Persistent
GET acquisition and initial endpoint discovery use finite startup deadlines;
only their validated read-only bodies then retain the selected peer lifetime.
They do not pass an indefinite or oversized deadline into the connector, reset
stream bounds, reconnect solely because startup time elapsed, or replay POSTs.

`connect_configured_observed` selects the same bounded configured attempt
policy without imposing an additional overall startup deadline. Its initial
auth/DNS deadline can only shorten that first attempt; admitted fallback gets
its own fresh configured budget, still constrained by any explicit peer expiry.
The existing `connect_observed` retains its caller-selected outer deadline.
Completion remains local cleanup
evidence, not session revocation, permission or remote cancellation proof.
