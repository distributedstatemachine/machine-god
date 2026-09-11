# Native MCP startup candidates

`mcp::startup::NativeMcpStartup` constructs actual stdio and HTTP peers from one
immutable native configuration snapshot. It returns an owned, unpublished batch;
it never publishes individual servers, replaces a runtime, opens a browser, or
executes application tools. The host selects worker ownership, clocks, complete
captured environment, stdio launch authority, network/TLS authority, authentication
source, configuration generation, peer lifetime and aggregate byte budget.
Construction and creation of an unpolled build perform no ambient acquisition.
The caller also supplies `catalog_epoch`, a shared monotonic timestamp origin.
All catalog pages use that same origin and carry it into server candidates;
timestamps are not silently rebased to each fetch. A future origin is rejected
on the first build poll, including an empty configuration, before transport work.

## Phase and acceptance

Selection follows the pinned fx startup admission rules:

| Phase | Enabled required | Enabled optional | Disabled |
| --- | --- | --- | --- |
| `All` | Connect | Connect | Record disabled |
| `AskStartup` | Connect | Record deferred | Record disabled |
| `AskDeferred` | Record deferred | Connect | Record disabled |

Servers are attempted in configuration order. A disabled required server remains
a required-readiness failure. Each receipt distinguishes not attempted, disabled,
deferred, ready and failed, with a separate cleanup observation. Deferred batches
cannot prove required readiness or prepare a replacement runtime; their owned
servers can only be transferred to a separately authorized additive publication
path. They must not replace an existing required-server set.

`NativeMcpStartupBatch::prepare` performs private runtime admission, not publication.
`Required` permits explicitly tolerated optional-server failures. `AllSelected`
rejects any selected-server failure and is suitable for retaining an old runtime
when reload cannot construct its complete replacement. Required-server failure,
global cancellation/deadline and aggregate admission failure reject either policy.
The caller owns the eventual atomic publish operation. Rejection or abandonment
drops only the new candidates and retains their cleanup observations.

## Actual transports and catalogs

Stdio uses the selected [immutable launch factory](mcp-stdio-startup.md), the owned
worker scope and observed peer negotiation. HTTP uses exact endpoint admission
through the selected [network authority](mcp-network.md), selected TLS trust and
the concrete [HTTP peer](mcp-http-peer.md). No resolver, process environment,
working directory, proxy or trust source is acquired implicitly by this builder.

Only an advertised tools catalog is eagerly fetched, including every correlated
page and exact descriptor/schema admission. Resource, resource-template and prompt
catalogs remain absent until an explicitly owned lazy feature request; their
absence does not disable a usable tools server. Peers advertising only features
remain ready with no eager tool catalog. Typed subscription activation is a
separate runtime responsibility, not an arbitrary raw startup request.

Every configured startup timeout retains its full positive `u32` millisecond
domain. HTTP authentication refresh and DNS consume the initial attempt's budget;
modern-to-legacy fallback gets its own configured attempt budget under the outer
deadline. Eager tools discovery uses the selected negotiation attempt's remaining
deadline. Legacy version retry timing stays with the peer's pinned protocol rules.
Stdio `restart_limit` bounds retries of complete connection plus eager tools
startup; observed prior cleanup must settle before another full startup attempt.
The outer build deadline and explicit peer lifetime always bound these attempts.
No application call is replayed by this mechanism.

Each runtime candidate carries that server's configured operation timeout; there
is no silently competing global timeout default. Host, configuration, network and
OAuth generations are retained as a shared immutable cancellation-guard allocation
across every executable binding from that server. At most eight are admitted.
Queue admission observes them, and the guarded submission runtime rechecks them at
the final proof-bearing write boundary. A one-build operation token is not reused
as a post-publication lifetime token.

## Explicit authentication

Each selected remote server uses `Configured`, an exact `Lease`, or a selected
`Stored` credential service. `authentication_config` is a pure identity helper for
startup and explicit auth commands: it includes exact endpoint/client selection
and resolved static, environment and additional headers with a stable empty OAuth
marker. A changing access token or dormant configured bearer value does not change
that identity. Endpoint or header-selection changes reject an old lease.

Stored credentials may perform an owned token refresh. Only an actual missing
credential entry permits fallback to configured bearer authentication; corrupt,
revoked, mismatched or unusable stored credentials fail the server. Active OAuth
takes precedence over dormant bearer environment selection. Discovery/token URLs
are independently admitted by the auth service; resource headers are never copied
to the token endpoint. This path has no browser authority and cannot start consent.

## Bounded ownership and cleanup

One pending build or returned batch is allowed per startup service. Candidate
configuration, authentication and descriptor data are charged to an explicit
positive budget capped at 256 MiB, with normal component limits retained.
Observers are registered before child/socket effects. Up to four unsettled peer
observations per server and two complete configured-server sets are retained;
completed observations are pruned. Excess ownership is rejected before acquisition.

Dropping a polled build cancels its in-progress owned attempt, closes candidate
peers and releases the build permit. `cleanup_observations` retains completion
evidence even if no batch was returned. A receipt's completion means owned local
cleanup has settled, not remote revocation or reversal of an application effect.
Waits use the injected clock and cancellation signals, without detached tasks.

Focused fixtures exercise real local HTTP discovery, paginated tools, stored
refresh, exact credential identity, pending-response cancellation and dropped-build
socket completion. Pure fixtures cover inertness, phase/readiness policy,
candidate rejection and bounded cleanup admission. Stdio process runtime checks
belong to the canonical fresh-helper gate in the implementation plan.
