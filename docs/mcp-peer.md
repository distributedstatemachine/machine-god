# Owned MCP stdio peer

`machine_god_native::mcp::peer` drives actual stdio discovery, initialization,
response correlation and raw catalog loading over the [owned connection](mcp-stdio.md).
It is not a second wire codec or an executable catalog publication service.

The caller explicitly supplies a factory authorized to reopen the same selected
server/configuration, the host worker scope, lifecycle cancellation, asynchronous
timer, overall deadline and discovery subdeadline. Construction and unpolled
startup perform no launch or timer work. Factories must not switch server identity
between attempts. Timers must remain inert before polling and retain ownership
of any abandoned timer work; the peer creates no detached tasks or timer threads.
Native monotonic `Instant` observations bound controllable asynchronous waits.

Startup drives the existing [negotiation machine](mcp-runtime.md) with exact
integer IDs, modern metadata and pinned legacy initialization. Modern successful
discovery must be complete and advertise supported versions. Known capability
objects and optional boolean flags are validated before selecting fallback or
readiness. Missing legacy capabilities mean none; malformed capabilities do not
become a successful downgrade. Legacy readiness includes acknowledged submission
of `notifications/initialized`. Sampling, roots and elicitation support are not
advertised by this peer.

Every admitted restart closes and awaits positive completion of the old process
connection, including deferred reap, before invoking the factory again. An
owned host worker observes cleanup; cancellation or overall expiry may abandon
that observation but cannot detach cleanup or authorize a restart. Old-generation
notifications are discarded when the replacement connection starts.

Discovery subdeadline expiry requests a transport-owned bounded snapshot only
after the discovery write and unsupported replies have completed. The worker
stops write admission, checks cancellation, consumes any retained read tail and
at most four 16 KiB nonblocking reads, with at most 16 interrupted-read retries.
Any complete queued frame, partial frame, active write, flood or exhausted budget
rejects timeout fallback. A would-block observation with an empty decoder is
recorded as `DiscoveryTimeoutQuiescent`, distinctly from observed clean EOF.
This is a defined snapshot cutoff: later bytes do not become retrospectively
observed bytes. It does not claim EOF. Only settled quiescence or clean EOF while
overall control remains live permits the pinned oldest-legacy discovery fallback.
Initialize fallback still requires the negotiation machine's admitted response
or clean-EOF evidence; initialize timeout is terminal.

## Requests and bounded routing

One mutable request lane owns at most one response expectation. Integer IDs
increase monotonically across the complete peer lifetime, including restarts;
exhaustion rejects without wrapping. A tool ID is reserved before native proof
preparation. The submitted non-clone proof-bearing request must carry that exact
ID and belong to an explicitly registered runtime allocation. Discarded unsent
reservations consume their IDs permanently. No request is automatically replayed.
Successful writing is not remote execution success; returned responses remain
untrusted data requiring method-specific result handling.

Production preparation uses `reserve_tool` and moves its non-clone reservation
into `McpToolRequest::with_reservation`. The peer retains only a weak allocation
observer. Dropping an unprepared request, unpolled preparation, denied admission
or unsubmitted claimed value makes the unsent slot available again, without I/O
or reusing its ID. A call must retain that exact allocation; matching the number
alone is insufficient. Dropping a stale reservation cannot clear a replacement.
The older `reserve_tool_id` interface remains explicitly manually discarded and
does not accept another peer's lease. Already attempted calls retain the existing
no-replay and connection-cleanup rules.

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

Compatibility follows fx `b1774fbf6c7602b503026f96f6e960e946c692ef`, especially
`mcp_runtime.zig` request metadata, capability parsing and stdio startup, and
`protocol_negotiation.zig`. The native bounded snapshot, strict duplicate-free
wire admission and explicit authority/cleanup boundaries are intentional native
constraints. Feature delivery and acceptance status belong only in the plan.
