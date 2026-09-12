# MCP catalog refresh policy

`machine_god_native::mcp::catalog_refresh` is a public, effect-free native policy
component. It decides freshness and admits modern subscription notifications;
it does not run refreshes or listeners, connect or relaunch peers, publish
descriptors, obtain permission, or replay requests. Actual runtime drivers,
publication, subscription transport and cleanup remain separately composed
callers. The module currently shares the Linux/macOS availability of the admitted
peer-capability type used to select filters.

## Time and bounded refresh ownership

One policy belongs to one exact server/configuration/authentication partition.
Its opaque `McpRefreshGeneration` is only an allocation identity, never executable
authority or a runtime publication checkpoint. The owner supplies the original
catalog epoch and monotonic elapsed milliseconds on every decision. Changing
the runtime partition requires new policy identity and independent runtime
publication validation, even when server names or timestamps happen to match.

The policy retains four fixed family records, not catalogs or descriptors.
`McpDescriptorCatalog` already supplies admitted fetch/expiry metadata; the policy
does not parse TTL again. Modern missing TTL, explicit zero and negative TTL all
expire immediately. Positive TTL is fresh strictly before the earliest admitted
page expiry, not at that boundary. A zero TTL means refresh on demand, not a busy
timer or automatic startup loop. Public cache scope never permits cross-owner or
cross-generation reuse.

`begin` distinguishes a cache hit, an existing in-flight refresh, retry backoff
and a new consuming ticket. Missing families cannot claim a retained snapshot.
Existing same-partition data can remain available during refresh or failure, but
the owner must independently revalidate current executable authority before use.
Failures preserve old metadata and use the pinned 100 ms doubling backoff capped
at five seconds. Successful `finish` resets backoff and accepts only the original
family, ticket allocation and valid time partition. `validate_replacement`
borrows the ticket and applies the same checks without mutation. The owner keeps
policy access serialized across that prevalidation, conditional runtime
publication and `finish` with the same inputs. Callbacks and cleanup remain
outside global runtime publication locks. Neither method publishes anything,
approves a stale asynchronous candidate or promises rollback of a failed commit;
`finish` remains fallible and fail-closed.

A non-clone ticket retains one tiny atomic in-flight marker. Dropping or
cancelling an unfinished ticket only clears that marker: no callback, lock,
clock read, transport action or task runs in its destructor. A later `begin` can
retry instead of remaining permanently busy. The owning operation must still
retain its exclusive transport lane, cancellation and cleanup custody through
any actual exchange; releasing a policy marker is not transport completion or
permission to replay a partially submitted request.

Clock regression and invalidation/retry arithmetic exhaustion fail closed until
the owner replaces the policy. Already admitted absolute catalog expiry retains
the assembler's saturating representation; this policy never wraps counters or
retry deadlines. At most one active ticket per family exists. Notifications
coalesce into fixed counters, not a work queue. A finish clears only the counter
captured when its ticket began, so notifications received during refresh remain
pending. Resource and template families settle independently.

## Modern subscription admission

`McpSubscriptionFilters` derives list-change filters from admitted peer
capabilities and resource subscriptions from explicit owner-selected URIs.
It serializes only the `notifications` object for the modern
`subscriptions/listen` method; no raw request, ID reservation or writer is
constructed. Filters retain at most 64 unique nonempty exact URIs and 64 KiB of
aggregate URI text, checked before copying. These finite native retention limits
are independent of the actual transport's encoded request and response bounds.

The owner installs the exact peer-reserved nonnegative listen ID. IDs must
increase within the policy allocation. `observe` borrows an already admitted
`RpcEnvelope`; it does not parse JSON again or retain incoming payloads. Modern
invalidation requires the exact policy generation, subscription ID in
`params._meta`, an acknowledged subscription and an enabled filter. An
acknowledgement must contain exactly the requested filter set, including ordered,
byte-exact resource URIs. Unsupported acknowledgement filters request owned
listener closure, not fallback. Stale IDs, duplicate acknowledgements, unrelated
messages, non-notification envelopes and unselected resources cannot invalidate
catalogs. Correlated server cancellation requests closure of only that listener.
Listener end and unsupported/cancelled admission expire all catalog and result
epochs immediately. `end_subscription` accepts only the currently selected ID;
repeated or late termination cannot invalidate a replacement listener. Independent
resource-read and prompt epochs let result caches compare their exact partition
without clearing another owner's observations.

Tool and prompt list changes invalidate only their respective catalog family.
Resource list changes invalidate both resource and template catalogs and all
cached resource reads. A resource update is accepted only for an exact subscribed
URI and conservatively invalidates all resource-read results, matching the pin;
it does not invalidate either list catalog. The read-cache owner receives one
bounded generation and clears its results before acknowledging that generation;
later notifications remain pending. Subscription handoff invalidates all four
catalog families and read results without creating refresh or relaunch work.

`validate_subscription_response` checks only a listen request's terminal success:
both the response ID and complete-result subscription metadata must match the
original nonnegative integer ID. It borrows an admitted envelope, does not decode
JSON again, and cannot substitute for the separate acknowledgement or create
subscription authority. Transports retain their own cancellation and late-response
ownership rules.

## Native composition

Startup installs advertised modern list-change subscriptions after eager tools
discovery and requires the exact acknowledgement before readiness. The native
runtime retains that state with the original peer, catalog epoch and configuration
partition. Caller-driven polling drains at most 64 ready notifications before
catalog admission; a larger ready backlog rejects that attempt instead of claiming
a fresh cache hit. Partial transport reads remain peer-owned across observations.

Ordinary tool calls and consuming continuation rounds settle already queued
notifications before and after their exchange under the same original caller,
peer lane, deadline and exact server policy. Progress is consumed; selected
subscription invalidations update only that partition's policy. This settlement
does not poll new socket data, refresh tools, switch an existing turn pin or
replay a request. Listener cancellation retains its exact owned close path.

Already negotiated direct candidates start their advertised modern subscription
on first catalog demand. Startup and demand both require exact filter ACKs, not
merely a successful POST. The entire demand is selected against its original
command/turn authority and deadline. Dropping an ACK wait retains the exact ID,
filters and partial peer-owned read; a later authorized demand resumes that read
under its own bounds without replaying the listen request. Unsupported ACKs and
correlated cancellation disable automatic restart for that partition. Ordinary
listener completion expires snapshots and permits a subsequent demand to start a
new ID; failed reads do not replay any application request. Peer retirement still
owns socket/process cleanup, and abandoned stdio writes fail the peer closed.

An unrestricted admitted resource read expands the exact watched-URI union before
result-cache lookup. Already admitted notifications, including cancellations from
ordinary response streams, drain before selecting that handoff. Existing selected
URIs survive expansion. The full next set
is checked against 64 URIs and 64 KiB before the current listener is disturbed;
duplicate demand does not restart it. Handoff expires snapshots before closing the
old listener or performing replacement I/O, and requires the replacement ACK.
HTTP closes the owned stream; stdio sends exact cancellation and retains its
bounded late-response IDs. Capacity failure cannot bypass the 64 retired-ID bound.
No cache mutex is held across listen, cancellation, polling or owner callbacks.

Before a new turn is pinned or a human feature selects its publication, due tools
refresh on the same serialized peer. Exact unchanged descriptors update freshness
without rebinding or retaining duplicate payloads. Changed descriptors use a
conditional publication transaction, including the controller's exact publication
witness, without reopening peers or replaying older requests. Existing pinned
operations never silently select the replacement generation.

Lazy resource/template/prompt discovery for dependent feature actions uses the
same partition's TTL and failure backoff. A shared extra cache budget is capped
at 16 MiB and further limited by the selected runtime retained-byte ceiling.
Cache pressure preserves old metadata and backoff while the current operation may
use its separately bounded fresh response. Cache retention never prolongs command,
turn, configuration, authentication or peer authority. Direct list commands still
perform their explicit requested exchange.

## Complete read/get result retention

Resource reads and prompt gets retain only complete admitted data, never input
requests, errors, request reservations or continuation authority. The per-peer
cache holds at most 64 combined FIFO entries and shares the lazy catalog cache's
16 MiB/runtime-selected byte ceiling. Optional keys are bounded to 128 KiB;
larger legal descriptors remain usable without caching. Keys include the exact
admitted descriptor, selected identity and sorted, byte-exact string arguments.
Even public scope remains within the same peer/configuration/authentication
partition. Replaced peers cannot inherit the old cache.

Each use admits the current descriptor and revalidates the original command or
turn. TTL starts at response receipt, not cache insertion, hits, or return from
human consent. Missing/zero TTL is immediately expired. Expired results are not
served on request failure. Read invalidation clears all resource entries; prompt
list invalidation clears prompt entries. Listener termination/handoff invalidates
both. A notification or newer fetch during human input prevents the old result
from replacing current cache state. Clock regression and counter exhaustion fail
closed. Cache pressure only skips retention of the separately bounded response.

Prompt-get TTL caching is a native extension: the pin caches resource reads but
gets prompts directly. This does not claim literal upstream cache behavior or a
measured performance improvement. No legacy restart or retry path is introduced.

## Pinned behavior

Behavior is based on fx `b1774fbf6c7602b503026f96f6e960e946c692ef`:

- `src/core/mcp/feature_cache.zig`: earliest expiry, modern missing-TTL zero,
  demand refresh, same-partition stale serving, retry backoff and subscription
  identity/acknowledgement classification.
- `src/core/mcp/tool_subscription.zig`: modern `subscriptions/listen`, exact
  acknowledgement filters, selected resource URIs and invalidation coalescing.
- `src/core/mcp/mcp_runtime.zig`: demand-driven tool/feature refresh; required
  Ask startup and one-time deferred activation; resource-read subscription
  expansion; all-read invalidation and subscription-handoff expiry.

The pin implements modern subscriptions, including resource-specific selection;
they are not deprecated HTTP+SSE endpoint listeners. Required startup discovers
and loads tools before readiness. Optional Ask servers remain dormant until
explicit MCP demand or authority capture, and feature catalogs load on demand.
TTL does not independently activate dormant servers. Deprecated transports,
legacy notification admission and downgrade/restart behavior are not included.
