# Native MCP runtime publication contracts

## Shared immutable executable bindings

`McpSubmissionRuntimeBinding::shared` retains shared immutable configuration and
authentication allocations and a cheap clone of the admitted schema. A catalog
with many tools does not need a separate copy of server configuration, resolved
authentication identity or schema source for each executable binding.

Both the existing copying constructor and the shared constructor enforce the same
finite logical identity bound before acquiring new storage. Shared storage does
not relax identity equality or permission evidence: exact original schema bytes,
configuration bytes, authentication bytes, server, remote tool and exposed tool
remain bound to the retained runtime allocation. Debug output remains redacted.
Publication composition must additionally bound aggregate distinct allocations;
sharing is not permission to retain an unbounded number of tools or generations.

The native runtime owner supports deferred retirement for atomic catalog swaps.
Deferral marks the old binding invalid immediately without invoking a waker.
The caller completes the returned retirement only after releasing its publication
locks. It must mark every old executable binding before exposing replacements.
Completion wakes observers outside those locks; no proof destructor, peer closure
or user interaction is performed by marking a binding invalid. Ordinary runtime
replacement and retirement preserve their existing invalidate-then-wake behavior.

## Exact HTTP head ownership

Typed HTTP request projection retains its immutable projected head allocation
through copied request, prepared submission, permission admission and final
one-shot claim. The native runtime can obtain that exact head without rebuilding
it from mutable connection configuration, current credentials, or model arguments.
The actual permission-bound wire bytes remain unchanged.

This read-only head accessor grants no destination or execution authority. The
existing concrete permission proof, runtime allocation checks, peer-minted request
lease and final plaintext writer checks remain mandatory. Legacy raw preparation
does not gain a typed projected head or access to a dynamic runtime route.

Typed requests also retain the exact constructor-selected `McpToolCallOptions`
through preparation and final claim. Progress IDs, negotiated protocol and actual
form/URL responder advertisements are not reconstructed from server result data.
Raw preparation does not gain these typed options or a dynamic execution route.

## Atomic publication and per-turn routing

`NativeMcpRuntime` requires explicit native contexts, monotonic clock, executor,
execution policy and finite limits. Construction performs no discovery, launch,
network operation or clock observation. Startup supplies complete configured
server candidates containing already-negotiated concrete owned peers and admitted
descriptor catalogs. Configuration/authentication bytes are native identity data,
not authority acquired from metadata or tool arguments.

Private candidate construction validates every server/version, stable exposed
name and collision policy, modern HTTP header eligibility, full exact-number
ToolSpec projection and aggregate retained storage before publication. It shares
server configuration/authentication and schema allocations. Limits cap 64 servers,
131,072 tools and a conservative 256 MiB retained candidate charge, including
captured executable specifications and search metadata; callers may lower caps.
Publication charges active and retired generations together against that byte
budget. Conservative retired-generation charges remain until the retired queue
fully drains, preventing repeated reloads from multiplying retained bindings.
Native peer transport buffers retain their separately bounded transport budgets;
unpublished candidates remain explicitly caller-owned rather than a hidden queue.
Every publication owns one captured registration per tool, reused on reselection.

Publication swaps the complete all-server candidate. Every old executable binding
is marked invalid before the new catalog becomes visible. Wakers, peer cancellation
and old owners are released after the publication lock. A foreign candidate or
exhausted retirement budget leaves the previous usable publication unchanged.
Every server's retained generation guards are checked again under the publication
lock, before retiring old bindings. Revocation after preparation rejects the
replacement without changing the active runtime, including feature-only servers
with no executable tools. These checks observe tokens, not an injected callback.

Each candidate retains its explicitly selected monotonic catalog timestamp origin.
`catalog_epoch(server)` exposes that origin without reading a clock or deciding
freshness. Published server ownership preserves it, so relative fetched/expiry
milliseconds need not be compared against an unrelated origin during refresh.

Catalog and permission routes require the actual registered native turn, not just
matching textual IDs or an `mcp_` prefix. Discovery does not pin an empty catalog.
Once ready, each exact live turn pins one publication weakly. Reload invalidates
old routes; a new turn must select replacements because core registrations cannot
silently change identity mid-turn. Closed/cancelled pins are pruned even if a
caller retains their registry. Captured tool registrations have weak peer routes
and cannot keep old peer generations alive independently.

## Permission, execution and bounded ownership

The runtime implements the existing exact-name MCP permission authority. Native
composite routing still sends actual registered builtins to their builtin
preparer. The runtime reserves a peer-minted request lease, binds original exact
arguments/schema and immutable options, and retains the exact projected HTTP head.
It does not grant permission or weaken Ask/Auto/Yolo, saved rules or schema review.

Per-peer acquisition is serialized, bounded to 64 pending acquisitions and races
caller, exact turn, retired route and explicit deadline cancellation. Permission
preparation releases the peer lane before prompting/review. Up to 64 independent
unsent leases may remain owned by prepared requests; denied/dropped preparations
are reclaimed on the next reservation without timers, drop callbacks or replay.

On the first polled tool-execution future, the runtime claims the exact ready
native submission before delegating to the required `NativeMcpToolExecutor`.
The owned call retains original arguments, options, context, live route and the
non-clone proof-bearing submission. Dropping that call releases an unsent proof
and lease even if the executor never exchanges. Construction is inert; no network
operation occurs before `first_exchange` is polled. That method consumes the one
initial attempt on its first poll, preserves proof/cancellation through queued
acquisition and actual peer exchange, and returns original correlated response
bytes with request/protocol provenance. Failure or cancellation cannot replay it.

The executor owns real result admission, explicit input-consent interaction and
durable argument/result archive policy. There is no default placeholder executor
or successful raw-result fallback. It must not strongly retain this runtime or
an engine that owns it. `execute_for_turn` preserves the entire `ToolExecution`,
including durable projections; direct `execute` consumes its complete output
without cloning. Explicit input/output bounds and input persistence hooks follow
core's existing archive contract and never normalize original JSON. Form/URL
advertisements assert an actual supplied responder, and default to false.

Raw response data, `revalidate`, and context IDs never authorize another request.
There is deliberately no resend or continuation API: a continuation requires a
separately typed predecessor transition, real consent and fresh native proof.
Host startup, CLI activation and result/consent composition remain separate
native responsibilities rather than being inferred from publication.

Retired servers remain in bounded native cleanup ownership (normally 64) until
the host polls `drain_retired`. Cancellation preserves undrained ownership and
previous completion receipts. Returned receipts distinguish local closure from
worker/reap completion; closure alone is not proof of complete cleanup or remote
HTTP session deletion. Runtime closure also invalidates the active generation.
