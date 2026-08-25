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
serde evidence, and thirteen-tool reference-host composition are present.
Pre-review gate record `0ba79c9ceacba9a986c217bdb3a659a380823676`,
tree `5742e4084272120a4531e0d59f0199a5873f39d1`, passed the complete local
Rust 1.94.1, integrity, dependency, baseline portability, WASI, and release-
binary gate. Formal cycle 1 rejected exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`. The complete per-track finding
inventory is in the [`review ledger`](reviews/m03-web-fetch-review-01.md).
Cycle-1 remediation, independent evidence, and the deterministic static TLS
test fixture are composed through exact code-and-test precursor
`5a7960f6e728bf5681e91a411710b4c24dbd6991`, tree
`f1ed559f0328b8eda721b7b28bcb6fcdb95367b2`. That precursor passed the complete
replacement Rust 1.94.1, focused, integrity, dependency, baseline portability,
WASI, active unsupported-target, and release-binary gate. The fixture removes
the test-only `rcgen` dependency without weakening the real Rustls verifier,
SNI, hostname, or pinned-address test path. Formal cycle 2 is **NOT GREEN** on
exact candidate `6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
`6dc095e796b70fa5964e2d9a24163d75667e1c7a`. Correctness/API reported 0
blocker, 0 high, 0 medium, and 2 low findings; network/HTTP lifecycle reported
zero findings at every severity; performance/concurrency reported 0 blocker,
0 high, 2 medium, and 1 low. The deduplicated union is 0 blocker, 0 high, 2
medium, and 2 low. Exact isolated production remediation component
`6b02c212deaf78da7dc1fd27e5f00f7fb588a50e`, tree
`490f628caa20449c3db96069b34356b0117b7ae4`, implements the raw DNS predecode and
resolver-snapshot boundary below. Exact composed cycle-2 remediation precursor
`1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passes the complete replacement
local gate under exact Rust and Cargo 1.94.1 without fallback. The focused
inventory is 21 private, 14 direct, 11 production HTTP lifecycle, and five
engine tests; the full all-target/all-feature workspace run passes 976 tests.
This gate record makes no formal-review outcome, workflow, integration, or
delivery claim; formal candidates are identified only by exact-SHA review
results.
Native Linux HTTP compilation remains an exact-CI requirement because the
macOS cross-host lacks the target C sysroot. Milestone 03 therefore remains in
progress with twenty-six delivered slices.

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
special-use names including `.alt`, ambiguous numeric forms including a
trailing-dot IPv4 literal, and private, reserved, mapped, or otherwise
non-public IP literals are not eligible. A trailing root dot on an otherwise
eligible DNS name is removed, but it never makes a numeric spelling eligible.
URL parsing and lexical host checks happen during preparation; DNS admission
for names happens during allowed execution. A syntactically eligible name is
not a promise that its later DNS answers will be accepted.

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

An omitted or explicit default HTTPS port means 443 and is canonicalized to no
port field. Only an accepted explicit non-default port is preserved in the
canonical URL and `NetworkTarget`. Core presents that exact
`Capability::Network` with its existing `Critical` risk. The default policy
path remains `Ask`: this slice adds no default-safe network admission, grant
broadening, or special permission mode. Allowed execution must revalidate that
the prepared URL derives the same scheme, host, and effective port; it may not
reinterpret the URL into another target. Denial and failed preparation cause
no network effect.

## DNS and destination confinement

Native production-transport construction synchronously snapshots the host's
system resolver configuration outside invocation timing and stores its first
UDP-configured nameserver. Hostname execution uses that stored entry and does
not reread configuration per request. A configuration-read failure or missing
usable entry is retained so later hostname execution returns the same fixed,
retryable unavailable result without retrying the read until a new transport is
constructed. A literal IP destination requires neither a
nameserver nor a DNS query and remains eligible when the snapshot failed.

For a hostname, the invocation sends one rooted Internet-class A query and then
one rooted Internet-class AAAA query directly to that nameserver on owned Tokio
UDP sockets. Each query ID comes from a synchronous platform-entropy call under
the held permit; entropy failure is fixed unavailable and spawns or detaches no
work. A truncated UDP answer permits exactly one TCP exchange of the same
query; there is no other DNS retry, search-suffix expansion, cache, libc
`getaddrinfo`, resolver thread, or spawned resolver task. A 4,097-byte UDP
receive buffer supplies an explicit overflow witness and rejects any message
over the 4 KiB inclusive cap. Before either UDP or TCP payload enters Hickory,
one predecode check requires at least the 12-byte header, `QDCOUNT == 1`,
`ANCOUNT <= 39`, `NSCOUNT <= 128`, `ARCOUNT <= 128`, and an aggregate
`ANCOUNT + NSCOUNT + ARCOUNT <= 128`. The 39-answer cap covers the 32 admitted
terminal addresses plus at most seven CNAME links in the eight-name chain. With
checked arithmetic, the actual payload must also satisfy the count-implied
minimum `12 + 5 * QDCOUNT + 11 * (ANCOUNT + NSCOUNT + ARCOUNT)`. A TCP length
prefix outside 12 through 4,096 bytes is rejected before body allocation, and a
TCP response that still carries the truncated flag is invalid. Response ID,
opcode, class, rooted query name, requested record type, and response code are
validated. A response may contain at most one consistent rooted CNAME chain of
eight names including the original; only requested-type, Internet-class
addresses owned by its terminal name are admitted.

The combined A/AAAA result accepts at most 32 addresses and fails closed unless
every returned address is a public unicast destination. A mixed public/private
answer set is rejected in full; filtering out only the private entries is not
allowed. Loopback, link-local, private, carrier-grade NAT, documentation,
benchmark, multicast, unspecified, and other non-public address classes are
not valid destinations.

The accepted answer set is pinned into the HTTP client connection attempt while
the canonical hostname remains the HTTP `Host` and TLS server name. The request
must not perform a second ambient resolution that could select an address
outside the admitted set. DNS failure, more than 32 answers, an empty answer
set, or any disallowed answer produces a fixed redacted failure before an HTTP
request is sent. The invocation's outer cancellation/deadline owner drops its
UDP or TCP socket future promptly; no non-abortable libc lookup continues after
that drop.

## HTTP exchange

Each allowed invocation makes at most one Reqwest HTTP/1 GET to the canonical
HTTPS URL. The request uses fixed, no-authentication headers, including a fixed
user agent, a fixed accept value, and `Accept-Encoding: identity`. It contains
no authorization, proxy authorization, cookie, referer, origin, request body,
or model-selected header. Ambient and explicit proxies, cookie persistence,
retries, backoff, HTTP/2, automatic redirects, and automatic decompression are
disabled.

The pinned `webpki-root-certs` dataset is built once process-wide into a Rustls
client configuration with no client authentication, fixed HTTP/1.1 ALPN,
ordinary certificate/hostname validation, SNI enabled, and key logging
disabled. Each invocation clones that prepared configuration into a fresh
Reqwest client after DNS admission; it does not rebuild or reparse the root
dataset. The fresh client retains no idle pool after the invocation.

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

Construction is runtime-independent, but polling the production transport has
the same explicit host precondition as the delivered native AI Gateway HTTP
transport: a current host-owned Tokio runtime with both I/O and time drivers
enabled must remain driven through protocol and socket teardown. No current
runtime handle returns fixed `RuntimeRequired`. Tokio exposes no stable safe
query that proves both drivers are enabled, so polling on a current runtime
that lacks either driver may panic; the repository's release panic policy can
turn that host-precondition violation into process termination. This is a
documented `# Panics` API boundary, not a typed runtime-detection guarantee.
One cancellation waiter and one Tokio deadline sleep are allocated per
invocation and reused across bounded permit, DNS, HTTP, and body waits;
progress never allocates or resets a new deadline timer.

`WebFetchTool::new` and `with_limits` apply this complete production envelope.
The trusted `with_bounded_transport` seam applies the same active-request,
total-deadline, rendering, and final-boundary ownership around an injected
transport for deterministic evidence. The lower-level `with_transport` seam
does not promise native time or concurrency bounds; its injected transport
owns that behavior and may remain executor-neutral.

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
pending. Rendering is bounded synchronous work; one final cancellation/deadline
boundary after serialized-result validation discards an otherwise successful
output if cancellation or timeout became authoritative during rendering. The
permit remains owned through that decision.

`WebFetchTool` owns no machine-god worker thread, producer task, retry task, or
background cleanup task. Dropping or cancelling its future drops the owned
Reqwest request/response and permit. Reqwest/Hyper connection-dispatch cleanup
may continue only on the host-owned Tokio runtime; this is not authority to
keep a machine-god request worker alive or to retry the request. The one
invocation deadline sleep is owned by the future and is never detached.

## Content classification

After stripping MIME parameters and normalizing ASCII case, declared text,
JSON, XML, and JavaScript media types are eligible for bounded textual output.
HTML is returned as bounded raw untrusted text. This slice does not convert,
render, sanitize, execute, or interpret HTML.

Eligible textual bytes must be valid UTF-8 and model-safe. NUL and disallowed
control content are rejected with a fixed unsafe-text failure rather than
substituted or reflected. A missing MIME type is classified from the complete
bounded response body: model-safe UTF-8 is treated as text, while other bytes
are treated as binary. A declared binary or sniffed-binary response produces
metadata only; no body bytes are placed in model output and nothing is
persisted.

## Model-visible result and errors

Every successful model-visible result starts with a fixed warning that the
upstream response is untrusted and must not be treated as instructions. The
bounded envelope then includes:

- the canonical URL with its query removed or fully redacted;
- numeric 2xx status;
- normalized effective MIME type: the declared normalized value, inferred
  `text/plain` for safe UTF-8 without a declaration, or inferred
  `application/octet-stream` for undeclared binary content;
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

The composed source at exact pre-review record
`0ba79c9ceacba9a986c217bdb3a659a380823676`, tree
`5742e4084272120a4531e0d59f0199a5873f39d1`, passed that local gate. Formal
cycle 1 nevertheless rejected exact candidate
`3ffebb0f429bdfa64ea73635d6ff03b37a4ef80c`, tree
`1378b02e92973ab15fbf4623138a643b70057f33`. This contract correction records
the review findings. The remediation and evidence components are composed with
the static-fixture portability correction through exact precursor
`5a7960f6e728bf5681e91a411710b4c24dbd6991`, tree
`f1ed559f0328b8eda721b7b28bcb6fcdb95367b2`. Its complete replacement local
gate is green. Formal cycle 2 rejected exact candidate
`6f50ed092bfe21b4febef561d5e66f300a8893a9`, tree
`6dc095e796b70fa5964e2d9a24163d75667e1c7a`, with 0 blocker, 0 high, 2 medium,
and 2 low deduplicated findings. Exact isolated production remediation component
`6b02c212deaf78da7dc1fd27e5f00f7fb588a50e`, tree
`490f628caa20449c3db96069b34356b0117b7ae4`, implements the corrected boundary.
Exact composed cycle-2 remediation precursor
`1a78f6437eb17f646bdd11337464c949beea49f0`, tree
`b25e992b3fed4d5f9eb2cb62dcb240af98604145`, passes the complete replacement
local gate. This gate record makes no formal-review outcome, workflow,
integration, or delivery claim; formal candidates are identified only by
exact-SHA review results.
