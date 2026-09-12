# Owned modern MCP HTTP peers

`mcp::http_peer` composes the [HTTP/TLS connector](mcp-http.md),
[modern negotiation](mcp-runtime.md), [SSE decoder](mcp-sse.md) and
[raw catalog assembler](mcp-pagination.md). Only modern Streamable HTTP is
supported. Deprecated HTTP+SSE endpoint discovery, dated initialization,
session IDs, initialized notifications, listener GETs, resume cursors and
session DELETE are not implemented.

## Authority and ownership

Hosts inject resolved endpoint/address authority, trust anchors, resolved
headers, monotonic clock/timer, lifetime policy and cancellation. Peers perform
no ambient DNS, environment, credential or clock discovery. Constructors and
unpolled requests acquire no network resources.

`McpPeerLifetime::OwnerControlled` retains peer ownership until close or
cancellation; `Until(Instant)` adds an exact host-selected expiry. Startup and
each application request still have independent finite deadlines. The lifetime
policy never promotes an ordinary response body beyond its original request
deadline. Modern subscriptions select their distinct bounded lifetime below.

There is one serialized application lane. All socket work is caller-polled;
there is no listener task, GET connection or background reconnect. Dropping a
polled application operation closes the peer. Completed responses release their
socket owners, while separately retained completion observations preserve
cleanup evidence.

## Modern subscriptions

`start_subscription` sends exactly one modern `subscriptions/listen` POST using
the shared typed filters and a fresh peer-reserved ID. It validates an SSE
response head and retains the actual response socket, body decoder, partial
line and buffered tail. It does not return subscription readiness: the runtime
installs that exact ID and filters in `McpCatalogRefresh`, then drives and admits
the matching acknowledgement before relying on notification coverage. No raw
notification creates authority, selects filters or publishes a catalog.

The finite startup deadline covers connection, request submission and response
head admission. Only that typed subscription body receives a distinct lifetime:
`u32::MAX` milliseconds from actual start, constrained by the original selected
peer expiry. It retains the same selected clock, owner cancellation and issued
authentication lease through every body read and envelope decode. A lease is
never refreshed or replaced inside the stream. Ordinary application bodies keep
their original deadlines unchanged.

`poll_subscription` drives at most one admitted envelope. Its caller-supplied
deadline bounds that observation, not the listener lifetime. Timeout or an
abandoned poll preserves the exact pending read; no POST is repeated and no
partial parser state is discarded. `active_subscription` distinguishes a quiet
or timed-out active listener from a closed one. Application exchanges can use
their separate socket while the listener retains a partial read. Work remains
caller-driven, with no detached reader or automatic reconnect.

Only notifications are returned for shared policy admission. A terminal response
must match the original ID, modern `resultType: complete` and identical
`result._meta` subscription ID. It closes that listener. Malformed input,
unsupported server requests, foreign/error responses, incomplete EOF and stream
budget exhaustion report failure and close only the listener; owner/authentication
retirement closes the peer. `close_subscription` releases only its retained local
socket and never writes cancellation, DELETE, GET or replacement requests.

Subscription frames have the pin's independent 64 KiB bound and at most 8,192
JSON nodes. SSE lines, buffered chunks and partial events remain bounded, with
1,024 events and 64 MiB raw SSE bytes per retained stream; connector body/wire
budgets also remain cumulative. Observation timeouts and operation completion
never reset those quotas. A pending event held across a deadline consumes one
additional bounded frame slot. The same eight-exchange cleanup bound includes
the retained subscription, and peer close drops it before signalling completion.

## Discovery and response streams

Modern discovery admits the shared negotiation machine's single bounded
HTTP 400 / -32022 same-modern retry. Both attempts share the original deadline.
HTTP 404/405, malformed success, redirects, authentication and I/O errors cannot
trigger fallback, initialization or consequential POST replay. Modern responses
are stateless: any session header is rejected rather than retained as authority.

A POST response has either an admitted JSON body or a bounded SSE data stream.
Raw JSON bytes remain beside routing envelopes, preserving exact numbers.
SSE event names, IDs and retry fields are ignored. Only a correlated response
completes the request; intervening notifications are queued as untrusted data.
Server requests fail as unsupported modern protocol input and cannot trigger
consent, arbitrary replies or another exchange.

Clean EOF without a correlated response, partial SSE EOF, malformed JSON or
foreign response IDs fail the operation and release its socket. No stream
condition authorizes GET resumption or POST replay.

## Submissions and features

Application IDs are reserved once before permission preparation and never
reused. `call` consumes an exact native submission and immutable projected
head, checking runtime allocation, reservation, endpoint and every selected base
header. Only modern method/name/parameter projection fields may be added.
The connector retains exact frozen bytes and the original proof through the
final plaintext writer above TLS and response reads. Cancellation never sends a
second request or claims remote effects were undone.

`reserve_tool` transfers a non-clone reservation through typed preparation.
Up to 64 independent unsent reservations permit cross-turn preparation while
the wire remains serialized. Abandonment releases only its own slot, never
another request's reservation. The manual `reserve_tool_id` API is exclusive
and cannot mix with live owned reservations. Discarding a manual reservation
does not reuse its ID.

Typed catalogs return raw candidates, not executable publication. The separate
[feature API](mcp-feature-runtime.md) executes seven native-selected actions
with fixed method headers, exact IDs, original command/turn guards and complete
bounded responses. Each write, flush and response read observes the selected
authority. Operation-scoped response limits reset when the operation settles or
drops.

`take_notification` removes already queued data without any network effect.
Notification processing, catalog refresh, consent and continuation authority
remain native runtime responsibilities.

## Bounds and cleanup

IDs are positive signed-64-bit integers and fail on exhaustion. Limits are
2,048 runtime allocations, eight live exchange observations, 64 queued events
totaling 1 MiB, 4,096 admitted events per operation, and 1,024 events per response
stream. Ordinary JSON frames are at most 8 MiB. Typed feature operations may
select up to 16 MiB and 262,144 nodes without widening notification bounds.
Connector/SSE byte, line and depth budgets apply independently; one stream's
default body budget is 64 MiB. Queue retention does not reset between operations.
Authentication challenges retain at most eight fields totaling 16 KiB.

`close` performs local cleanup only and cannot issue DELETE or other network
work. Completion observations wait for local retirement and release of every
retained socket owner, including abandoned futures; they neither drive work nor
prove remote revocation. Debug/display redact endpoint, header, challenge and
payload contents.

## Observed configured startup

`connect_observed` reports the inert peer's cleanup observation on first poll,
outside locks and before network effects. Rejection, callback unwind, failed
discovery, cancellation and abandoned futures retire that same owner.
An optional `first_attempt_deadline` preserves time already spent on credential
refresh and DNS; it can only shorten the configured discovery budget.
The returned attempt deadline preserves the remaining initial catalog budget.

`connect_configured_observed` omits a separate aggregate startup deadline,
but retains finite configured discovery and request deadlines. Positive
configured durations are bounded by `u32::MAX` milliseconds with checked
`Instant` addition, while public standalone connector constructors and ordinary
peer `connect` retain the 24-hour exchange ceiling. No timeout is silently
clamped, extended by retry or converted into indefinite response ownership.

The [implementation plan](implementation-plan.md) is the sole live gate ledger.
