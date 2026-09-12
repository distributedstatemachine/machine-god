# Owned MCP stdio peer

`machine_god_native::mcp::peer` drives actual modern stdio discovery,
response correlation and raw catalog loading over the [owned connection](mcp-stdio.md).
It is not a second wire codec or an executable catalog publication service.

The caller explicitly supplies a factory for the same selected server/configuration,
the host worker scope, lifecycle cancellation, asynchronous timer, overall
deadline and discovery subdeadline. Construction and unpolled startup perform no
launch or timer work. One startup invokes the factory at most once: the peer
never switches versions or relaunches after discovery failure. Timers remain inert
before polling and retain ownership of abandoned timer work; no detached tasks or
timer threads are created. Native monotonic `Instant` observations bound
controllable asynchronous waits.

Startup drives the modern [negotiation machine](mcp-runtime.md) with exact integer
IDs and modern metadata. Successful discovery must be complete, advertise the
modern version and contain a capabilities object. Known capability objects and
optional boolean flags are validated before readiness. Missing or malformed
capabilities cannot become successful readiness. Initialization and initialized
notifications are not supported. Sampling, roots and elicitation support are not
advertised by this peer.

Discovery deadline, clean EOF, incomplete framing, protocol errors and offers of
older versions are terminal. No timeout snapshot or close evidence grants
downgrade/relaunch authority. Failed or abandoned startup closes its connection;
the host's retained workers and observed completion still own cleanup, including
deferred reap. The startup composer must settle prior ownership before a separate
configured complete-startup retry; that policy is not a peer negotiation retry.

## Requests and bounded routing

One mutable request lane owns at most one response expectation. Integer IDs
increase monotonically across the complete peer lifetime, without reconnects;
exhaustion rejects without wrapping. A tool ID is reserved before native proof
preparation. The submitted non-clone proof-bearing request must carry that exact
ID and belong to an explicitly registered runtime allocation. Discarded unsent
reservations consume their IDs permanently. No request is automatically replayed.
Successful writing is not remote execution success; returned responses remain
untrusted data requiring method-specific result handling.

Production preparation uses `reserve_tool` and moves its non-clone reservation
into `McpToolRequest::with_reservation`. At most 64 independent unsent requests
retain weak peer slots, allowing preparation across turns without holding the
serialized exchange lane across permission. Catalog controls may run between
these unsent requests. Dropping an unprepared request, unpolled preparation, denied admission
or unsubmitted claimed value makes the unsent slot available again, without I/O
or reusing its ID. A call must retain that exact allocation; matching the number
alone is insufficient. Dropping a stale reservation cannot clear a replacement.
The manual `reserve_tool_id` interface remains single-exclusive and cannot mix
with live owned reservations. `discard_tool_id` clears only that manual slot and
does not accept or discard another request's lease. Already attempted calls retain the existing
no-replay and connection-cleanup rules.

`call_frame` preserves the correlated response's original JSON bytes for
method-specific result admission; `call` remains the envelope-only convenience API.

The separate typed [`feature`](mcp-feature-runtime.md) method allocates its own
IDs and executes all seven resource/prompt actions using native-selected live
authority. Each pagination page is guarded. The raw discovery allowlist remains
unchanged and cannot carry read/get/completion or tool calls. Immutable launch
wire bounds must cover the selected feature codec bounds before any send;
`wire_limits` exposes those bounds without granting execution authority.

The peer polls stdout while a request write is still pending, preventing ordinary
bidirectional pipe backpressure from stalling correlation. Unsupported server
requests receive the fixed method-not-found response, with at most seven queued
replies beside the primary write. This lane cannot send arbitrary successful
server replies, feature requests or continuation responses. Notifications remain
explicit data, not permission, subscription or continuation authority. At most
64 notifications and 1 MiB of their original JSON bytes are retained; accounting
is conservatively released only when that queue fully drains. A request admits
at most 256 incoming envelopes, preventing a notification/request flood from
creating unbounded work. Existing per-frame JSON bounds remain in force.

`next_notification` also drives the same worker-owned receiving lane while idle.
Dropping this observation or reaching its observation deadline preserves a healthy
peer, partially received NDJSON and complete queued notifications. Completed
notifications are retained before the postread deadline check, so a boundary
timeout never loses an admitted event or publishes it as a late success.
Unsupported requests still receive only the fixed exact-ID method-not-found reply.
Up to seven pending reply futures remain peer-owned across idle abandonment, with
their original finite write deadlines and receipt outcomes; later observers cannot
recreate them or renew those deadlines. A later consequential exchange first
settles inherited replies while continuing to consume bounded stdout, before its
primary write may begin. An unpolled exchange does not take that ownership.
Idle observations share the existing 256-envelope operation budget and notification
queue limits. Owner cancellation/expiry, malformed input, EOF, foreign responses
and failed reply writes retire the connection; these failures are not idle timeouts.

Null, stale, foreign and duplicate response IDs close the request lane; stdio
never uses HTTP's null-ID discovery exception. Cancellation, deadline, failure
or abandonment of a polled exchange closes the underlying connection rather
than permitting ambiguous prefix replay. The peer exposes completion without
extending process authority. Hosts bound the number of peers, selected runtime
allocations and unpolled caller futures separately.

`catalog` drives every page through the raw [catalog assembler](mcp-pagination.md)
using original response bytes, preserving numeric/schema spelling and unknown
metadata. A caller-supplied monotonic timestamp origin keeps cache timestamps
comparable across refreshes and connections. Exact cursors, item identities,
budgets and cache hints remain assembler-owned. Resource/prompt discovery requires
the corresponding admitted capability; tools retain the pin's ordinary listing
behavior. Full descriptor/schema admission, atomic executable publication, runtime
permissions, callback authority, refresh subscriptions and CLI activation remain
native runtime composition responsibilities.

Subscription control construction uses the admitted typed filter selection and a
nonnegative integer request ID, with modern client metadata and the existing
bounded control frame size. Empty filters are rejected. This constructor does not
widen the public raw discovery method allowlist or establish acknowledgement.
Same-peer executable replacement can stage a bounded runtime whitelist while
holding its lock without mutation. Committing swaps the exact table infallibly
and returns the previous allocations for disposal outside publication locks;
dropping the staged guard leaves the old table unchanged.

The caller-polled subscription lane allocates listen IDs from the peer's ordinary
monotonic allocator and settles the exact typed write receipt before returning.
It does not claim acknowledgement: the owner feeds queued envelopes into the
shared catalog-refresh policy before readiness. Polling uses the existing bounded
idle receiver; timeout or dropped observation retains the active ID, partial NDJSON,
complete notifications and original pending unsupported-reply receipts. Ordinary
catalog/feature/tool exchanges continue to route those notifications while matching
only their own response IDs. An exact active listen final ends the listener; an
invalid final reports a listener failure without closing an otherwise healthy
shared stdio peer. Already retained notifications are observed before that terminal
status, including when an ordinary exchange consumed the final. Unrelated response
IDs remain fatal.

Cancellation writes the fixed exact-ID notification and does not require a final
server response. At most 64 cancelled IDs remain to consume one late final each;
64 retained IDs still permit one active listener, while another cancellation must
wait for retirement capacity. Abandoned or failed cancellation writes retain the
ordinary close-on-ambiguous-write rule. No listener request is replayed or renewed,
and no detached reader, process or executor is created by this lane.

Modern behavior is informed by fx `b1774fbf6c7602b503026f96f6e960e946c692ef`,
especially `mcp_runtime.zig` request metadata, capability parsing and modern
stdio discovery, and `protocol_negotiation.zig`. Strict duplicate-free wire
admission and explicit authority/cleanup boundaries are intentional native
constraints. Feature delivery and acceptance status belong only in the plan.
