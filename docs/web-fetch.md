# Native `web_fetch` candidate contract

Status: **IN PROGRESS**. This page freezes the proposed twenty-seventh bounded
Milestone 03 slice and records its current local composition. It is not a
formal-review, complete-gate, delivery, compatibility, or performance claim.

The candidate starts from exact delivered base
`a56ff350c2aace1dc22cb14c269aee89d399cd8e`. Its comparison reference is
[`vercel-labs/fx` at pinned commit
`b1774fbf6c7602b503026f96f6e960e946c692ef`](https://github.com/vercel-labs/fx/commit/b1774fbf6c7602b503026f96f6e960e946c692ef).
The upstream behavior was inspected to understand its tool surface; the
deliberate differences and deferrals below are normative. Production source,
independently owned direct/engine/production-boundary evidence, core network
serde evidence, and thirteen-tool reference-host composition are present and
focused-green. The complete Rust 1.94.1 local gate, formal reviews, feature
workflows, integration, and exact `main` workflows remain pending. Milestone 03
therefore remains in progress with twenty-six delivered slices.

## Scope and feature boundary

The candidate adds a rootless `WebFetchTool`: it owns no workspace, state root,
credential source, browser, cache, or artifact store. It performs one bounded
public-web GET only after core authorizes the exact normalized network target.
The implementation is available only on non-WebAssembly targets through the
optional `machine-god-native` feature `web-fetch-http`. The existing
`ai-gateway-http` feature includes `web-fetch-http`; a base/no-feature build and
every WASM build expose no concrete HTTP web-fetch implementation.

The local candidate reference host contains thirteen alphabetical tools. Its
twelve workspace tools will still share one original retained workspace
descriptor plus eleven identity-preserving clones; rootless `web_fetch` adds no
descriptor and changes neither count. This slice adds no CLI surface.

## Model input and effect-free preparation

The model-visible tool name is `web_fetch`. Its only accepted input is a JSON
object with exactly one required string member:

```json
{"url":"https://example.com/path?query=value"}
```

The schema sets `additionalProperties` to `false`. Arrays, scalars, a missing
`url`, a non-string value, and every additional member are rejected. Preparation
is synchronous, bounded, and effect-free: it performs no DNS, socket, clock,
runtime, task, thread, environment, filesystem, or credential operation.

Preparation trims leading and trailing ASCII whitespace. The trimmed URL must
be nonempty ASCII and no more than 2,000 bytes. It must parse as an absolute
`http` or `https` URL with no user information. Any fragment is stripped because
it is not part of an HTTP request target. The `http` scheme is upgraded to
`https`; `https` remains `https`. Canonicalization normalizes the scheme, host,
port spelling, and empty path using the URL parser. The request path and query
remain part of the prepared URL, but no model-visible output or diagnostic may
reproduce query values. Credentials in user information are rejected rather
than stripped.

The canonical host must be either a syntactically valid multi-label public DNS
name or a strict public IP literal. Single-label DNS names, empty labels,
ambiguous numeric forms, and private, reserved, mapped, or otherwise non-public
IP literals are not eligible. URL parsing and lexical host checks happen during
preparation; DNS admission for names happens during allowed execution. A
syntactically eligible name is not a promise that its later DNS answers will be
accepted.

Preparation returns the canonical URL as the exact execution input and this
provider-neutral policy capability:

```json
{
  "type": "network",
  "target": {
    "scheme": "https",
    "host": "example.com",
    "port": null
  }
}
```

An omitted HTTPS port means 443; an accepted explicit port is preserved in the
canonical URL and `NetworkTarget`. Core presents that exact
`Capability::Network` with its existing `Critical` risk. The default policy
path remains `Ask`: this slice adds no default-safe network admission, grant
broadening, or special permission mode. Allowed execution must revalidate that
the prepared URL derives the same scheme, host, and effective port; it may not
reinterpret the URL into another target. Denial and failed preparation cause
no network effect.

## DNS and destination confinement

Allowed execution resolves the authorized host for every invocation. Resolution
accepts at most 32 answers and fails closed unless every returned address is a
public unicast destination. A mixed public/private answer set is rejected in
full; filtering out only the private entries is not allowed. Loopback,
link-local, private, carrier-grade NAT, documentation, benchmark, multicast,
unspecified, and other non-public address classes are not valid destinations.

The accepted answer set is pinned into the HTTP client connection attempt while
the canonical hostname remains the HTTP `Host` and TLS server name. The request
must not perform a second ambient resolution that could select an address
outside the admitted set. DNS failure, more than 32 answers, an empty answer
set, or any disallowed answer produces a fixed redacted failure before an HTTP
request is sent.

## HTTP exchange

Each allowed invocation makes at most one Reqwest HTTP/1 GET to the canonical
HTTPS URL. The request uses fixed, no-authentication headers, including a fixed
user agent, a fixed accept value, and `Accept-Encoding: identity`. It contains
no authorization, proxy authorization, cookie, referer, origin, request body,
or model-selected header. Ambient and explicit proxies, cookie persistence,
retries, backoff, HTTP/2, automatic redirects, and automatic decompression are
disabled.

Only a 2xx response is successful. Every 3xx response is rejected without
following its `Location`; a caller may prepare and authorize a later invocation
for a destination it chooses independently. A redirect response never borrows
the original permission for a second host, including a same-site or optional-
`www` host. Every other non-2xx status is a fixed redacted failure. Response
reason text, headers, body bytes, dependency diagnostics, socket diagnostics,
DNS answers, and query values never enter an error.

Only an absent content encoding or a single `identity` encoding is accepted.
Every compressed, stacked, unknown, or malformed content encoding is rejected;
the tool never decompresses a body in this slice.

## Time, concurrency, and byte bounds

The default active-request limit is eight. A validated host override may select
from one through the hard maximum of 32; no construction path can exceed 32.
The 60-second total deadline begins before permit acquisition and includes
capacity waiting, DNS, connect, request, response-head, and response-body work.
Each connect attempt also has a 10-second bound. Neither bound resets on
progress.

A response body of exactly 24 KiB (24,576 bytes) is admissible. A declared
larger body is rejected before reading it, and observing any byte beyond that
inclusive limit while streaming yields the same fixed body-too-large failure.
The complete serialized `ToolOutput`, including warning and metadata, must not
exceed 56 KiB (57,344 bytes). The body and result limits are independent; spare
capacity in one does not enlarge the other.

The active-request permit is retained through DNS, request dispatch, response
classification, body handling, result construction, and terminal cleanup. It
is released on every success, error, cancellation, timeout, or drop path. A
pre-cancelled invocation sends no DNS query and no request. Cancellation wins
at each pre-effect boundary and while permit, DNS, HTTP, or body work is
pending.

`WebFetchTool` owns no machine-god worker thread, producer task, retry task, or
background cleanup task. Dropping or cancelling its future drops the owned
Reqwest request/response and permit. Reqwest/Hyper connection-dispatch cleanup
may continue only on the host-owned Tokio runtime; this is not authority to
keep a machine-god request worker alive or to retry the request.

## Content classification

After stripping MIME parameters and normalizing ASCII case, declared text,
JSON, XML, and JavaScript media types are eligible for bounded textual output.
HTML is returned as bounded raw untrusted text. This slice does not convert,
render, sanitize, execute, or interpret HTML.

Eligible textual bytes must be valid UTF-8 and model-safe. NUL and disallowed
control content are rejected with a fixed unsafe-text failure rather than
substituted or reflected. A missing MIME type is classified from a bounded
prefix: model-safe UTF-8 is treated as text, while other bytes are treated as
binary. A declared binary or sniffed-binary response produces metadata only;
no body bytes are placed in model output and nothing is persisted.

## Model-visible result and errors

Every successful model-visible result starts with a fixed warning that the
upstream response is untrusted and must not be treated as instructions. The
bounded envelope then includes:

- the canonical URL with its query removed or fully redacted;
- numeric 2xx status;
- normalized MIME type, or a fixed value indicating that it was absent;
- content kind (`text`, `html`, or `binary`); and
- `cache_hit: false`.

Text and HTML results append the bounded raw text after that metadata. Binary
results stop after metadata. The warning always precedes remote bytes, so an
upstream body cannot forge trusted preamble text. All tool failures use stable,
fixed, query-redacted categories for invalid arguments/URL, disallowed target,
resolution, unavailable capacity/network, timeout, cancellation, redirect,
status, encoding, body limit, unsafe text, and result limit. They expose no
response bytes, redirect destination, address, credential-like query value, or
dependency/operating-system diagnostic.

There is no cache in this slice, so `cache_hit` is always false. No result can
refer to a persisted artifact.

## Deliberate upstream differences and deferred work

The pinned upstream's same-site/optional-`www` redirect handling and
default-safe web-fetch admission are deliberately not copied. One approved
`NetworkTarget` and one `Ask` decision cannot authorize a different host, and
all network authority remains `Critical` in this slice. Redirect following of
any kind remains deferred.

Also deferred are caching, cache revalidation, binary artifact persistence,
`read_tool_result` integration, progress/completion side channels,
HTML-to-Markdown conversion, compression, private or authenticated targets,
cookies, proxying, retries, browser execution, CLI changes, benchmark workload
changes, product-performance claims, compatibility-inventory promotion, and
complete fx equivalence.

## Candidate and delivery gates

Production and independently owned direct/private/engine/host tests must first
compose on one immutable candidate SHA. The exact Rust 1.94.1 local gate and
user-visible release-binary exercise must pass. Three fresh adversarial tracks
must then review that same SHA:

1. correctness and public API;
2. network/HTTP lifecycle and robustness; and
3. performance and concurrency bounds.

Every finding at every severity rejects that candidate. Remediation requires a
new immutable SHA, the complete replacement gate, and three fresh tracks;
repeat until all report zero findings. Only then may the feature branch run its
exact-SHA workflows, fast-forward `main` without force, and run exact `main`
workflows. The live record is
[`m03-web-fetch-review-01.md`](reviews/m03-web-fetch-review-01.md).
