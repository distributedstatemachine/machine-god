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
policy never promotes a response body beyond its original request deadline.

There is one serialized application lane. All socket work is caller-polled;
there is no listener task, GET connection or background reconnect. Dropping a
polled application operation closes the peer. Completed responses release their
socket owners, while separately retained completion observations preserve
cleanup evidence.

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
