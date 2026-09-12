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
`Required` permits optional-server failures and is the controller's startup and
reload policy. `AllSelected` remains available to callers explicitly requiring
every selected server to succeed. Required-server failure,
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
any admitted same-modern HTTP retry retains that original attempt budget under
the outer deadline. Eager tools discovery uses the negotiation attempt's remaining
deadline. There is no legacy negotiation or protocol fallback.
Stdio `restart_limit` bounds retries of complete connection plus eager tools
startup; observed prior cleanup must settle before another full startup attempt.
Negotiation failures are terminal and do not relaunch an unsupported server.
`build_configured` has no aggregate startup deadline: each serial server and full
restart keeps its configured attempt budget. `build` additionally bounds all
attempts by its explicitly supplied outer deadline. Both retain cancellation.
Peer lifetime is separately selected as `McpPeerLifetime::OwnerControlled` or
`Until`; an explicit expiry constrains startup and every later HTTP or stdio
operation, including queued writes. Request completion does not end an
owner-controlled peer's lifetime. Inter-attempt cleanup has a fresh finite
30-second housekeeping deadline, constrained by any selected outer deadline.
No application call is replayed by this mechanism.

Each runtime candidate carries that server's configured operation timeout; there
is no silently competing global timeout default. Host, configuration, network and
OAuth generations are retained as a shared immutable cancellation-guard allocation
across every executable binding from that server. At most eight are admitted.
Queue admission observes them, and the guarded submission runtime rechecks them at
the final proof-bearing write boundary. A one-build operation token is not reused
as a post-publication lifetime token.

## Profile activation controller

`NativeMcpController` shares the host's actual management service and its exact
store identity, runtime, owned worker scope, reserved tool names and captured
startup authorities. Construction and unpolled operations are inert. Profile
loads and exact-source revalidation run on that worker scope; network startup is
caller-polled asynchronously, without a second executor or worker `block_on`.

`start_configured`, `reload_configured` and `activate_deferred_configured` select
the no-aggregate-cap startup mode. Their explicit-deadline counterparts remain
available. Profile load, exact-source validation and prior-generation drain each
receive a fresh 30-second housekeeping window, constrained by a selected outer
deadline or peer expiry. A timed-out profile worker keeps its generation
reservation until actual completion; timeout does not release cleanup custody.

`deadline_after` explicitly observes this controller's selected monotonic clock
and checked-adds a positive caller-selected duration. Zero is rejected before
reading the clock; overflow is a bounded error. Callers invoke it on the first
operation poll, never during inert host/future construction. It also works after
close so finalization can select a fresh cleanup window without ambient time.

Initial `All` publishes all available enabled servers subject to required-server
readiness. `AskStartup` publishes required peers only. Its first explicit
`activate_deferred` call coalesces optional discovery against the retained exact
profile snapshot. Successful optional peers append once without replacing the
required peers, registrations or existing turn pins. Empty optional sets are a
no-op. Deferred outcomes are cached per generation, including failures; a later
reload is needed to retry. An individual waiting caller's cancellation does not
cancel the shared deferred loader, whose original optional deadline and owner
still apply. Configured mode retains the same one-loader and cached-outcome rules.

Interactive startup failure leaves management available, including the route to
explicit authentication. Every newly admitted model prompt or continuation checks
the controller's exact active publication before native tool/provider projection.
Missing publication, disabled required servers, expired/closed required peers and
retired authority reject that prompt; rejection never automatically requeues it.
Peer operation occupancy is not a readiness failure. A successful later reload
permits a newly submitted prompt; failed reload preserves the old active readiness.
One-shot `AskStartup` still fails before conversation work on required failure.
Signal cancellation and owner closure still stop interactive startup. The latest
loaded source, including failed discovery, is retained as one already-budgeted
generation for historical management observations, never as live peer authority.

`reload` reads a fresh profile and attempts all enabled servers. Full replacement
requires every required server to succeed. Optional-server failures permit a
degraded replacement and remain visible in its startup receipt, matching pinned
fx behavior, including the full reload after explicit authentication. Exact source
validation precedes publication; the runtime compare-and-swap checks the original
publication witness. Predicted
candidate checkpoints are captured before publication, never inferred from a
later runtime read. Failure leaves the previous configuration lifetime token and
active publication intact; independently revoked credential authority is not
restored by retaining that publication. These checks do not reserve the profile
against later external filesystem edits. Captured environment, network and authentication
selections are reused explicitly; they are not recaptured from ambient state.

Only one mutation is admitted. A positive bound of at most eight generations
counts pending, active, retired and outstanding returned outcomes; retain four
for an ordinary host. Reservations precede profile I/O and remain charged while
abandoned store workers or returned receipt owners survive. Startup cleanup
custody is installed before its first effect. Runtime completion observations
also have a fixed retained bound, including observations returned before a
subsequent reload fails.

`close` is an irrevocable cutoff, not proof of child reaping or socket completion.
It cancels generation and operation owners outside controller locks and retains
abandoned jobs. `settle` uses a separate cleanup token and deadline to drive those
jobs, drain runtime peers and observe startup peers and profile workers. A failed
or abandoned settlement retains custody for another explicit attempt. Successful
publication wins a concurrent caller cancellation; a close observed during its
state commit is reported without resurrecting active state. Local cleanup never
claims HTTP session deletion, remote revocation or reversal of application work.
The host remains responsible for its shared worker scope's final shutdown/join.

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

Successfully assembled authenticated peers retain their exact issued OAuth lease.
`authentication_refresh_due` observes those original leases, their selected clocks
and profile/generation cutoffs without reading files, loading another saved
configuration or refreshing a token. It requests refresh within the 60-second
skew, including expired tokens; invalid profile/generation authority remains an
error rather than anonymous fallback. Every retained authority is checked, so an
earlier due lease cannot hide a later revoked one. The controller owns selection
and publication of a fresh generation; old headers are never renewed in place.

The concrete lease is installed before HTTP discovery and remains attached to
every subsequent connection, proof-bearing writer, control writer and response
reader. Its original authentication clock supplies the blocked-I/O expiry timer;
access-token checks run before and after I/O and before each write/flush, including
wall-clock expiry. This fence is independent of the finite request deadline and
the selected peer lifetime. Authentication and HTTP clocks need not be identical,
and an absolute lease deadline is not compared with an unrelated injected clock.
Feature/tool guards are retained alongside authentication, not replaced by it.

At most one retained lease per configured server, and at most 64 in total, is
allowed per startup service. A second build selecting a still-retained name fails
before authentication/DNS/discovery effects. Completed fresh candidates are pruned
after actual peer cleanup. Expired or revoked records remain bounded tombstones
until their startup generation is dropped, so closing an expired peer cannot
erase the need to refresh. Failed acquisition never registers a lease; a rejected
candidate that has already expired retains the same conservative bounded record.
Normal required startup followed by deferred optional discovery uses distinct
names. Reload uses a new startup service with its own immutable profile selection.

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
