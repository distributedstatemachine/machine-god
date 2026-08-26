# Optional native AI Gateway HTTP transport

This page is the normative contract for the seventh integrated bounded
Milestone 03 slice. It adds a Reqwest-backed `AiGatewayHttpTransport` as one
optional native implementation of the injected byte-transport boundary in the
[`AiGatewayProvider` codec](ai-gateway.md). The exact feature-branch review and
remote evidence are recorded in the
[`native AI Gateway HTTP transport review`](reviews/m03-ai-gateway-http-review-01.md).
The documentation-seal commit and eventual `main` SHA retain their exact-run
delivery gates. Milestone 03 remains in progress, and this is not a
production-ready claim.

The transport implements `AiGatewayTransport` with standard futures and
streams, but its concrete startup future and response stream must be polled
inside a live Tokio runtime with I/O and time enabled. A current-thread runtime
is sufficient. The host owns that runtime, which must outlive the transport's
requests, streams, and pooled connections and remain driven while they are
active or being torn down. Construction performs no network effect and requires
no runtime. Core, the codec, and custom injected transports remain
executor-neutral and retain their existing authority and resource boundaries.
The separate integrated [`native configuration schema v2`](configuration.md)
declares the stable `ai_gateway_http` transport kind only; parsing that value
does not construct this transport or its required runtime.

## Feature and public API

The generation implementation is opt-in through the `machine-god-native`
Cargo feature `ai-gateway-http`. That compatibility feature includes both the
narrower `ai-gateway-model-catalog-http` feature and `web-fetch-http`,
preserving its delivered generation/reference-host behavior. The generation
transport exports remain `ai-gateway-http`-only. Both HTTP features and all of
their concrete exports are cfg-gated off on WebAssembly; there is no WASM HTTP
implementation. The base crate and custom injected transports do not require
Reqwest or Tokio.

`NativeTransportKind::AiGatewayHttp` and its stable `ai_gateway_http` name are
not cfg-gated with this optional implementation. They remain valid declarative
configuration data in no-default-feature and WebAssembly builds, where
`AiGatewayHttpTransport` is absent. The enum therefore does not promise that a
configured transport is available or usable on the current build and target.

The feature exposes these construction surfaces:

- `AiGatewayBearerToken::new` accepts an explicitly supplied token;
- `AiGatewayHttpEndpoint::default` selects the sole production endpoint;
- `AiGatewayHttpEndpoint::loopback_http` constructs the explicitly test-only
  plaintext endpoint;
- `AiGatewayHttpLimits::default` selects the fixed default time, connection and
  response-chunk limits;
- `AiGatewayHttpLimits::new(connect_timeout: Duration, request_timeout: Duration,
  max_active_requests: usize, max_response_chunk_bytes: usize)` validates explicit
  values; `connect_timeout`, `request_timeout`, `max_active_requests`, and
  `max_response_chunk_bytes` are read-only accessors; and
- `AiGatewayHttpTransport::new` uses the production endpoint and default
  limits, while `with_endpoint_and_limits` accepts an already validated
  endpoint and limits.

The separate [`native credential discovery`](ai-gateway-credentials.md) and
shared bearer/error surface are available under either native HTTP feature.
They can supply the explicit `AiGatewayBearerToken`. Discovery is not part of
either transport and does not run during transport construction or a request.

Construction returns fixed `AiGatewayHttpConfigError` categories:
`InvalidBearerToken`, `InvalidEndpoint`, `InvalidLimits`, or
`ClientInitialization`. Display and debug text do not reflect the supplied
token, endpoint, dependency diagnostic, operating-system diagnostic or other
constructor input. Endpoint and transport debug output reveal only structural
kind and limit data; URI and authorization values remain redacted.

Repository-local consumers enable the transport explicitly:

```toml
[dependencies]
machine-god-native = { path = "crates/machine-god-native", features = ["ai-gateway-http"] }
```

The trusted host passes the credential as data. The example deliberately has
no secret literal and performs no ambient lookup:

```rust,ignore
use std::error::Error;
use std::sync::Arc;

use machine_god_native::{
    AiGatewayBearerToken, AiGatewayHttpTransport, AiGatewayProvider,
};

fn provider_from_host_secret(
    bearer_from_trusted_host: String,
) -> Result<AiGatewayProvider, Box<dyn Error>> {
    let token = AiGatewayBearerToken::new(bearer_from_trusted_host)?;
    let transport = Arc::new(AiGatewayHttpTransport::new(token)?);
    Ok(AiGatewayProvider::new("provider/model", transport)?)
}
```

This transport API does not read an environment variable, configuration file,
keychain, credential helper, command-line flag or prompt. The separate
integrated credential adapter may read only its two documented environment
names when a host explicitly selects process discovery. Credential rotation,
lifetime and authorization remain trusted-host responsibilities.

## Endpoint and credential confinement

The production endpoint is pinned to the public constant
`AI_GATEWAY_HTTP_DEFAULT_ENDPOINT`:

```text
https://ai-gateway.vercel.sh/v3/ai/language-model
```

There is no arbitrary production endpoint constructor. Production uses HTTPS
with the exact scheme, host, port and path above.

`AiGatewayHttpEndpoint::loopback_http` is the only alternate endpoint surface.
It exists exclusively for deterministic tests, accepts an endpoint string no
larger than 2,048 ASCII bytes, requires plaintext `http`, an explicit nonzero
port and an absolute path, and accepts only canonical numeric dotted-decimal
IPv4 loopback in `127.0.0.0/8` or the canonical bracketed IPv6 loopback host
`[::1]`.
User information, a query and a fragment are rejected. Names such as
`localhost`, non-loopback destinations, alternate or encoded IP spellings,
HTTPS alternates and non-HTTP schemes fail construction. This is a local-test
escape hatch, not a general endpoint configuration feature; a process able to
bind the selected loopback port is inside that test trust boundary.

`AiGatewayBearerToken::new` accepts a 1–4,096-byte RFC 6750 bearer `b64token`:
one or more ASCII letters, digits, `-`, `.`, `_`, `~`, `+`, or `/`, followed
only by optional trailing `=` padding. The token is retained only so the
transport can attach `Authorization: Bearer <token>` to the constructed
request. Its debug representation is always redacted, it exposes no revealing
display interface, and its bytes never enter transport error codes or messages.
This is not a memory-zeroization or locked-memory claim.

## Exact HTTP exchange

For each codec request, the transport issues one POST to its constructed
endpoint. It preserves the codec's validated content type,
protocol/specification, model, streaming, session and affinity metadata and
adds the bearer authorization header, fixed `Accept: text/event-stream`, and
fixed `Accept-Encoding: identity`. The Reqwest/HTTP stack may add required
wire-framing headers such as `Host` or `Content-Length`; machine-god adds no
team, endpoint-selection, fx referer, fx title or fx user-agent identity.

The request body is exactly the body supplied by `AiGatewayProvider`. The
transport does not inspect or rewrite prompt data and does not compress the
request. The codec's independent 12 MiB encoded-body cap applies before this
boundary. A direct caller that violates the transport request/header contract
receives a fixed redacted failure; it cannot replace the configured
authorization value.

Only status 200 returns an `AiGatewayByteStream`. Status is classified before
any body is exposed:

| HTTP status | `ProviderErrorKind` | Retryable |
| --- | --- | --- |
| `200` | accepted byte stream | not applicable |
| `401`, `403` | `Authentication` | no |
| `429` | `RateLimited` | yes |
| `408`, `425`, `500`–`599` | `Unavailable` | yes |
| every other `400`–`499` | `InvalidRequest` | no |
| `300`–`399` | `Protocol` | no |
| every other non-`200` status | `Protocol` | no |

The mapping uses only the status. It never reads, parses, retains or reflects a
non-200 body for diagnosis. Response headers, reason text, endpoint text,
credential bytes and dependency-controlled diagnostics likewise never enter a
public error or debug representation. A generic network failure is a fixed,
conservatively retryable `Transport` error. A recognized TLS failure is a fixed
non-retryable `Transport` error. Application, status, and backoff retries are
disabled. Hyper may reconnect and redispatch only when a reused idle connection
fails before any request byte is written. At most one peer-visible request is
dispatched, and no request is replayed after any byte may have reached the peer.

## Fixed client policy and resource bounds

Reqwest 0.13.4 has default features disabled and enables only its Rustls
support. The client uses Rustls with the pinned
`webpki-root-certs` dataset and an explicit closed policy:

| Concern | Default | Validated contract |
| --- | --- | --- |
| production endpoint | pinned | one pinned HTTPS origin and path |
| alternate endpoint | none | explicit numeric-loopback plaintext test endpoint only |
| proxy | disabled | no ambient or explicit proxy use |
| redirects | disabled | every 3xx reaches the fixed status mapper |
| response decompression | disabled | no automatic decoding |
| cookies | disabled | no cookie persistence or replay |
| retry/backoff | disabled | no replay after any request byte; stale pooled pre-write recovery is allowed |
| `Expect: 100-continue` | disabled | no automatic expectation handshake |
| active requests | 16 | 1–64; same-endpoint idle connection reuse is allowed |
| connect timeout | 30 seconds | greater than zero and at most 5 minutes |
| total request/stream timeout | 10 minutes | greater than zero and at most 1 hour |
| response chunk | 64 KiB | 1 byte through 1 MiB |

The connect timeout must not exceed the request timeout. Every violation is the
same fixed `InvalidLimits` construction category. The total deadline begins
before active-request capacity is acquired and covers capacity waiting, upload,
response-head wait, and response-body streaming; it is not reset by each chunk.
A semaphore permit is retained until the returned stream finishes or is
dropped. The configured request and time bounds are client policy rather than a
retry budget.

The transport splits dependency-provided `Bytes` frames before exposing chunks
larger than its configured limit and retains at most one upstream frame while
that frame is drained. This is a hard bound on each public chunk, not on
Reqwest/Hyper's internal read or frame allocation. The codec independently
rejects chunks larger than its own configured maximum and retains its separate record,
undecoded-buffer, total-response, record-count and JSON limits. Unused capacity
at either layer does not enlarge the other layer's budget. The codec owns the
request-body, overall-response and semantic event bounds; this transport does
not advertise a broader body allowance.

TLS trust comes from the dependency's bundled WebPKI roots, not the operating
system's native root store. This is deterministic and avoids silently trusting
machine-installed authorities. The tradeoff is that enterprise interception
roots and private authorities installed only in a native store are rejected,
and OS trust-store changes do not take effect until the pinned Rust trust
dependency is updated. No API in this slice injects another root or disables
certificate or hostname verification.

## Cancellation and drop

The codec passes the turn's `CancellationToken` into the transport. The
transport checks it before dispatch and observes it while request upload,
response-head acquisition, active-request-capacity waiting and response-body reads
are pending. Cancellation wakes the returned future or stream; it does not
depend on another network byte arriving. If cancellation becomes ready in the
same poll as a transport result, the fixed cancelled provider error wins.

Dropping the startup future before dispatch sends nothing. Dropping it after
dispatch, cancelling it, or dropping/cancelling the returned byte stream drops
the owned in-flight Reqwest future or body and releases transport buffers and
the active-request permit. Machine-god creates no internal runtime and spawns no
producer or per-request task, retry, or backoff. Reqwest/Hyper does spawn a
connection-dispatch task on the host runtime. Cancellation returns promptly
after dropping machine-god's owned body; socket teardown is asynchronous and
requires the host runtime to remain driven. HTTP/1 is fixed so that dispatcher
closes a stalled request's socket rather than merely cancelling a stream on a
shared HTTP/2 connection. Cancellation and drop do not claim to recall bytes
already transmitted or prove what the remote peer received.

The configured connect and request timeouts are independent terminal bounds
when cancellation is not requested. With default limits they are 30 seconds
and 10 minutes. Timeout failures use fixed redacted provider errors; no Reqwest,
Hyper, Tokio, Rustls, socket, or operating-system message is reflected. Polling
when no Tokio runtime handle is active returns a fixed, redacted, non-retryable
`Transport` error and performs no request. A runtime handle without enabled I/O
or time violates the API precondition; Tokio may panic, and the repository's
abort-on-panic release profile may terminate that host process.

## Separate model-catalog HTTP transport

The delivered `models [--json]` path uses a separate GET transport; it
does not change the POST generation transport above. Under the dedicated non-WASM
`ai-gateway-model-catalog-http` gate, native publicly exports
`AiGatewayModelCatalogHttpTransport`, `AiGatewayModelCatalogHttpEndpoint`,
`AiGatewayModelCatalogHttpLimits`, fixed construction error kinds, and their
endpoint/time/capacity/chunk constants. `new` takes an optional validated
`AiGatewayBearerToken` and selects the fixed production endpoint and defaults;
`with_endpoint_and_limits` accepts the optional token plus validated explicit
values. The only alternate endpoint is the same strict class of canonical
numeric-loopback HTTP test URL; it is not reachable from CLI, config, or
environment endpoint input.

The CLI enables only `ai-gateway-model-catalog-http`. Its resolved native
feature graph contains the shared bearer/TLS and catalog HTTP dependencies plus
Hickory's system-configuration support and its transitive protocol, network,
and Moka cache dependencies. The narrow catalog feature activates direct
optional `hickory-proto`, `hickory-resolver`, and `sha2` edges; the resolver dependency
is retained only for bounded platform-configuration parsing and no longer
enables its Tokio resolver integration. Reqwest's built-in `hickory-dns`
feature is disabled. The graph does not activate generation-only direct `bytes`
or `web-fetch-http`, and it still omits Tokio's signal backend. The CLI
dependency, not native catalog HTTP, requests Tokio signal handling. Existing
consumers of `ai-gateway-http` retain direct `bytes`, the catalog feature, and
`web-fetch-http`; this topology change does not alter generation-transport
behavior and makes no performance claim.

Catalog production sends one bodyless HTTP/1.1 GET to
`https://ai-gateway.vercel.sh/coding-agent/v1/models`. Its application-selected
headers are `Accept: application/json`, `Accept-Encoding: identity`,
`User-Agent: machine-god/<package-version>`, and bearer authorization only for
authenticated access. It sends no team, referer,
title, content-type, cookie, proxy, or endpoint-selection metadata. Redirects,
automatic decompression, proxies, cookies, and retries are disabled. Only 200
bodies are read; non-200 bodies are discarded.

The catalog client explicitly installs a machine-god `reqwest::dns::Resolve`
adapter with private bounded DNS exchange. It never selects Reqwest's built-in
Hickory adapter,
default GAI resolver, or non-abortable blocking `getaddrinfo` worker: client
construction explicitly disables Reqwest's built-in Hickory selection before
installing the custom resolver, including if dependency feature unification
enables that alternative. Production transport construction synchronously
snapshots platform DNS configuration and one fallible 32-byte query-ID key
exactly once, before any catalog request or deadline exists and without a Tokio
runtime. Numeric-loopback test construction performs no DNS discovery. The
adapter retains only the validated
snapshot or a fixed unavailable state; it has no configuration loader to call
while a request is being polled.

On generic Unix other than Apple and Android, the snapshot loader follows the
usual `/etc/resolv.conf` symlink but requires the target before and the opened
descriptor after open to be a regular file no larger than 64 KiB. It opens with
`O_CLOEXEC | O_NONBLOCK`, retains at most 64 KiB plus one overflow byte, permits
at most 16 interrupted reads and a finite byte-proportional read-call count,
and repeats the descriptor type/size check at EOF. Directories, special files,
growth past the cap, unavailable files, and malformed input fail closed. Apple
and Windows use Hickory's synchronous platform configuration API only during
construction, then post-validate the returned snapshot. Those platform APIs may
allocate their returned values before machine-god can reject them. Android does
not call Hickory's platform API because it requires initialized NDK process
context and may panic when that context is absent. Android instead retains a
fixed unavailable configuration state at construction, so production catalog
hostname resolution fails closed through the redacted catalog `Transport`
failure without sending a DNS request. Retained successful snapshots are
nevertheless bounded to 32 nameservers, 32 search domains, 8 KiB of aggregate
DNS-name bytes, 64 server connections, and bounded resolver options.
Case-randomization and nonempty avoided-local-port sets are unsupported and
fail closed rather than silently losing configured behavior. The snapshot
retains configured UDP and TCP endpoints, trust-negative flags, request
timeout, attempts, concurrent batch width, TCP-on-error choice, and recursion
choice. Query-ID entropy failure is retained as a fixed unavailable state.

Every lookup uses only that immutable snapshot and construction-time key on the
currently active runtime. A keyed `AtomicU32` sequence derives each 16-bit ID
with bounded SHA-256 work; request polling calls no entropy source and spawns no
resolver task. A and AAAA proceed concurrently without detachment, while each
family tries configured nameservers in bounded concurrent batches and attempt
order. Each server exchange owns one absolute configured timeout. UDP uses a
Tokio socket with OS ephemeral-port selection; a validated truncated response
replays once over configured TCP, configured TCP-only servers work directly,
and TCP-on-error is honored when configured. Responses are capped at 4 KiB and
strictly validate ID, opcode, class, absolute question, section counts, exact
wire exhaustion, response code, and at most 32 stable first-seen addresses.
CNAMEs are checked for conflicts and cycles and may continue across responses
for at most seven aggregate links. Trusted empty NXDOMAIN responses stop that
family's server search; an address from the other family may still succeed.

The Reqwest hostname is normalized to exactly one terminal dot before exchange,
making the production lookup an absolute FQDN and preventing configured domain
or search suffix queries. No runtime-backed resolver handle or cache is retained
across sequential current-thread runtimes. Unavailable configuration, entropy,
or DNS fails closed as the fixed redacted catalog `Transport` failure; there is
no Google or other public-resolver fallback. Dropping or cancelling a pending
catalog request drops its sockets, lookup future, response/request ownership,
timers, and active-request permit. Numeric-loopback test requests bypass DNS as
before.

Default catalog limits are 30 seconds for connect, 30 seconds per attempt, and
8 active requests; explicit concurrency is restricted to 1–32. The provider
computes one checked 30-second absolute operation deadline and passes it unchanged to
each possible call. Every `get` call constructs a new attempt-local
cancellation waiter and Tokio timer for the earlier of its configured attempt
deadline and the provider deadline, covering its permit wait, request,
response head, and body. An authenticated 401/403 may therefore create a second
set of attempt-local waiters, but never a new provider deadline. The body
buffer retains at most 256 KiB; a frame that would cross the inclusive cap is
rejected before any of that frame is appended. Drop releases the request/
response, buffer, and permit. The full parser, fallback, output, and delivery
status are in [`models-cli.md`](models-cli.md).

The concrete deadline waiter checks for a current Tokio handle before it
constructs a Tokio timer. Without a runtime it remains inert, allowing the
provider to poll the concrete request and return the fixed nonretryable
`RuntimeRequired` error; composing the provider over this concrete transport
therefore does not panic in the release profile's abort-on-panic environment.
With a live time-enabled runtime, the independent provider waiter constructs
the same absolute-deadline timer and wakes at that deadline. Deterministic
tests inject a permanently pending Reqwest resolver and paused Tokio time to
prove cancellation and deadline completion, lookup-future drop, permit
restoration, and sequential current-thread runtime teardown without production
DNS or test sleeps. A local UDP DNS fixture proves the retained snapshot works
on each of two sequential current-thread runtimes. A second local fixture proves
strict UDP truncation, TCP replay, concurrent A/AAAA, and bounded cross-response
CNAME continuation. Separate deterministic-ID, injected entropy-failure/no-
packet, malformed-wire, eager-once, oversized-snapshot, unavailable-snapshot,
and generic-Unix bounded-file tests prove that request polling never loads
platform configuration or entropy and that failures remain fixed, redacted,
and permit-safe.

## Slice-33 shared web-search use

The in-progress native [`web_search` slice](web-search.md) reuses the same
credential-bearing `Arc<dyn AiGatewayTransport>` and fixed production endpoint
instead of adding a second HTTP client, credential lookup, proxy, redirect,
certificate, or endpoint policy. Its one private worker request is still one
ordinary transport POST under this document's status, authentication, TLS,
header, cancellation, and no-replay guarantees. Web search adds separate
smaller codec-owned request/stream/record/node/deadline/concurrency ceilings;
the HTTP transport's own limits continue to apply independently.

The ordinary outer provider drops its validated terminal response stream
before delivering queued usage/stop events to core. Consequently the shared
HTTP semaphore permit is returned before a resulting local `web_search` round
starts, including when a custom transport's total capacity is one.

`AiGatewayWebSearchTransport` is non-WebAssembly and available only with
`ai-gateway-http`. The current production reference host remains Linux/macOS-
only. It reuses the already validated configured model and credential source,
advertises only required Perplexity search, and makes no retry or fallback.
The dedicated provider-executed decoder sits above this byte transport; the
HTTP layer neither parses provider tool records nor changes the ordinary outer
generation codec. Composed behavior precursor
`3d2984000301e58762e0940504159aeb55b2389e` passed the complete exact-1.94.1
local gate. Formal cycle 1 rejected exact `89c5ec95`, tree `8d91a55`, including
shared-transport starvation and decoder-allocation findings. Source remediation
from exact isolated components `096b11c4` and `ca0b990a` releases the outer
stream at validated finish,
incrementally retains one bounded SSE record, and injects explicit fallible
deadline authority. Exact composed precursor `e662fa8`, tree `6c0ace9`, passes
the complete local gate; fresh cycle-2 review remains pending, so the slice is
not yet review-green or delivered.

## Deferred scope

The generation transport slice above adds no ambient credential or endpoint
discovery, CLI or config composition, permission prompt, permission mode,
session persistence, provider-executed tools in the HTTP layer or ordinary
generation codec, retry/backoff, custom proxy, custom redirect policy,
custom certificate roots, native enterprise trust, arbitrary destination,
WASM transport, non-Tokio execution for the concrete transport, internal
runtime, extra tool, package, GitHub release, compatibility promotion or
performance claim. The plaintext endpoint is exclusively a deterministic-test
facility and is not a supported production deployment mode.

Schema v2 may declare `transport: "ai_gateway_http"`, but that is data only: it
does not enable the Cargo feature, construct `AiGatewayHttpTransport`, acquire
or attach a bearer token, create or drive Tokio, or issue a request.
