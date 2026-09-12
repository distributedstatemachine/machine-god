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
family, ticket allocation and valid time partition. The owner serializes policy
settlement with successful conditional publication; `finish` itself publishes
nothing and cannot approve a stale asynchronous candidate.

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

Tool and prompt list changes invalidate only their respective catalog family.
Resource list changes invalidate both resource and template catalogs and all
cached resource reads. A resource update is accepted only for an exact subscribed
URI and conservatively invalidates all resource-read results, matching the pin;
it does not invalidate either list catalog. The read-cache owner receives one
bounded generation and clears its results before acknowledging that generation;
later notifications remain pending. Subscription handoff invalidates all four
catalog families and read results without creating refresh or relaunch work.

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
