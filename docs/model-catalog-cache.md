# Native model catalog cache

`NativeModelCatalogCache` retains the bounded rich catalog from
`AiGatewayModelCatalogProvider::list_model_details`, including exact advertised
reasoning and fast controls. Observations share an immutable `Arc` rather than
copying model identifiers. Missing entries do not manufacture capabilities.
During loading, an observation may still hold the prior catalog for retention;
request-time capability resolution must await `wait_while_loading` and use a
ready observation rather than treating that retained value as a completed load.

`load(now_ms, cancellation)` initiates a fetch when idle. Successful ready
catalogs have no TTL. Failed caches retry after 1,000 ms regardless of failure
retryability; ready caches retaining a nonempty catalog retry only when their
last failure is retryable. `refresh` explicitly bypasses that admission policy.
Both join an already-loading fetch instead of duplicating transport work.

The timestamp is supplied by the host from one monotonic clock, at attempt
admission (first polling), not completion. The host should promptly poll the
constructed future. Cooldown uses checked subtraction, so a backward timestamp
does not permit retry and arithmetic overflow never wraps into an elapsed
interval. Dropping an unfinished owner restores the complete prior observation,
including its attempt time; it does not fabricate a completed fetch failure.

An empty refresh preserves a prior nonempty catalog and clears its last failure.
A failed refresh preserves a prior nonempty catalog and records only the fixed
provider failure kind and retryability. Without a nonempty catalog, failures
enter the failed phase; an initial empty success enters ready. The provider's
existing transport, parsing and 30-second total-deadline bounds are unchanged.

All futures are inert before first poll. No task or thread is spawned. The host
owns and polls the initiating load future; dropping it drops the provider future,
restores the prior state and wakes waiters. `wait_while_loading` never initiates a
fetch. Dropping or cancelling a joining caller only unregisters that caller;
it does not cancel the owner. At most 64 loading waiters can register per cache;
further callers receive the data-free `WaiterLimit` error. Repeated polling uses
one slot and dropping/completing a waiter releases it. Provider polling and
waker callbacks run outside the cache mutex.

A cache is bound to one immutable provider/access configuration. A host changing
credentials or access must select a different cache; it must not reuse an old
authenticated cache under a different identity. Snapshots retain the provider's
actual access provenance. Debug output omits model IDs and provider contents.

The retry, no-TTL, retained refresh and cancellable-loading behavior follows
`src/core/app/model_cache_runtime.zig` (`beginLoad`, `resolveForRequest`,
`preloadThreadMain`, `markFailed`) at pinned fx revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Rust uses explicitly owned asynchronous
futures in place of the upstream worker thread. See also
[model preferences](model-preferences.md) and [model CLI](models-cli.md).
