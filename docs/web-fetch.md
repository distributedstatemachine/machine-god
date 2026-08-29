# Native `web_fetch` contract

`WebFetchTool` performs one bounded public-web GET after core authorizes the
exact normalized network target. The deliberate compatibility differences use
the pinned fx revision recorded in the implementation plan as input; this
document defines machine-god's normative behavior.

## Scope and feature boundary

`WebFetchTool` is rootless: it owns no workspace, state root,
credential source, browser, cache, or artifact store. It performs one bounded
public-web GET only after core authorizes the exact normalized network target.
The implementation is available only on non-WebAssembly targets through the
optional `machine-god-native` feature `web-fetch-http`. The existing
`ai-gateway-http` feature includes `web-fetch-http`; a base/no-feature build and
every WASM build expose no concrete HTTP web-fetch implementation. Linux/macOS
default builds retain direct `sha2` for independent copy/session hashing.
FreeBSD/Windows default trees omit it, while `web-fetch-http` supplies the
optional edge on those targets and includes the dependency on every supported
non-WASM target. WASM trees omit it with or without the feature.

Every production and explicitly injected/custom reference-host composition
uses the canonical [tool catalog](native-reference-host.md#tool-catalog).
`web_fetch` remains rootless and adds no workspace descriptor. The tool adds
no CLI surface.

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
ambiguous numeric forms including a trailing-dot IPv4 literal, and private,
reserved, mapped, or otherwise non-public IP literals are not eligible. The
exact ASCII-case-insensitive full-label suffix denylist, after removal of one
trailing root dot, is `.alt`, `.arpa`, `.example`, `.home`, `.internal`,
`.invalid`, `.lan`, `.local`, `.localhost`, `.onion`, and `.test`. Matching is
allocation-free. The `.arpa` entry is machine-god public-web product policy for
the limited-use infrastructure TLD that [IANA designates exclusively for
Internet-infrastructure purposes](https://www.iana.org/domains/arpa) and
[RFC 3172 describes as infrastructure rather than general host
naming](https://www.rfc-editor.org/rfc/rfc3172#section-2). It is broader than
the individual [IANA special-use registry](https://www.iana.org/assignments/special-use-domain-names/)
entries and covers all current ARPA entries and their descendants, including
`ipv4only.arpa`, `resolver.arpa`, reverse names such as `10.in-addr.arpa`, and
every subdomain below them. The `.alt` entry likewise keeps the non-DNS
namespace described by [RFC 9476](https://www.rfc-editor.org/rfc/rfc9476#section-2)
out of public DNS execution.

The exact terminal-label boundary does not reject names such as
`public.notarpa` or `resolver.arpa.example.com`. Documentation domains
`example.com`, `example.net`, and `example.org` are intentionally not denied
lexically, consistent with [RFC 6761's application guidance](https://www.rfc-editor.org/rfc/rfc6761#section-6.5),
and the maintained `example.com` fixture remains eligible. A trailing root dot
never bypasses a rejected suffix or makes a numeric spelling eligible. URL
parsing and these bounded lexical host checks happen during preparation.
During allowed execution, DNS admission separately requires the complete
resolved address set to satisfy the public-IP policy; lexical eligibility is
not a promise that later DNS answers will be accepted.

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
path remains `Ask`: the tool adds no default-safe network admission, grant
broadening, or special permission mode. Allowed execution must revalidate that
the prepared URL derives the same scheme, host, and effective port; it may not
reinterpret the URL into another target. Denial and failed preparation cause
no network effect.

## DNS and destination confinement

Native production-transport construction synchronously snapshots the host's
system resolver configuration and one random query-ID seed outside invocation
timing. It stores the first UDP-configured nameserver and a seed-backed atomic
per-query sequence. Hostname execution does not reread resolver configuration
or entropy per request. A configuration/seed read failure or missing usable
nameserver is retained so later hostname execution returns the same fixed,
retryable unavailable result without retrying either prerequisite until a new
transport is constructed. A literal IP destination requires no nameserver,
query ID, or DNS query and remains eligible when either snapshot failed.

For a hostname, the invocation sends one rooted Internet-class A query and then
one rooted Internet-class AAAA query directly to that nameserver on owned Tokio
UDP sockets connected to the snapshotted nameserver address. Each query ID
derives from the construction-time random seed and one atomic sequence step;
invocation execution makes no blocking entropy call, and query-ID generation
spawns or detaches no work. A 4,097-byte UDP receive buffer supplies an explicit
overflow witness and rejects any datagram over the 4 KiB inclusive cap; the raw
datagram retained after that check is therefore at most 4,096 bytes.

Replay admission is ordered and fail-closed. Raw UDP header counts first require
at least 12 bytes, `QDCOUNT == 1`, `ANCOUNT <= 39`, `NSCOUNT <= 128`, `ARCOUNT
<= 128`, and aggregate `ANCOUNT + NSCOUNT + ARCOUNT <= 128`. Hickory then
decodes only the header and single complete question before the truncation flag
can authorize another effect. The ID must equal the outstanding query, `QR`
must identify a response, opcode must be query, RCODE must be no-error, and the
question must contain the exact rooted query name, requested record type, and
Internet class. A mismatch or malformed question fails without constructing or
polling TCP work. For an otherwise valid `TC=1` UDP response, the resource-
record tail may be cut or shorter than its declared counts imply; that discarded
tail is deliberately not fully decoded. After one fresh cancellation/deadline
boundary, the invocation performs exactly one TCP exchange of the same query to
the same snapshotted nameserver socket address. UDP response handling cannot
rebind that destination, and there is no other DNS retry, search-suffix
expansion, cache, libc `getaddrinfo`, resolver thread, or spawned resolver task.

A non-truncated UDP response and every TCP response retain the strict complete
path: checked arithmetic requires the count-implied minimum `12 + 5 * QDCOUNT +
11 * (ANCOUNT + NSCOUNT + ARCOUNT)`, Hickory decodes the full declared message,
the decoder must be exhausted with no undeclared trailing bytes, and the
complete response is validated. A TCP length prefix outside 12 through 4,096
bytes is rejected before body allocation, the same raw count caps apply, and a
TCP response that still carries the truncated flag is invalid. The
39-answer cap covers the 32 admitted terminal addresses plus at most seven CNAME
links in the eight-name chain. A complete response may contain at most one
consistent rooted CNAME chain of eight names including the original; only
requested-type, Internet-class addresses owned by its terminal name are
admitted.

The combined A/AAAA result accepts at most 32 addresses and fails closed unless
every returned address is a public unicast destination. Every returned address
counts toward that cap. After full validation, the transport stably
deduplicates the addresses in first-seen A-then-AAAA order before HTTP-client
construction, so an overlapping or repeated answer cannot create repeated
connection attempts. A mixed public/private answer set is rejected in full;
filtering out only the private entries is not allowed. Loopback, link-local,
private, carrier-grade NAT, documentation, benchmark, multicast, unspecified,
and other non-public address classes are not valid destinations.

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
the tool never decompresses a body.

## Time, concurrency, and byte bounds

The default active-request limit is eight. A validated host override may select
from one through the hard maximum of 32; no construction path can exceed 32.
The 60-second total deadline begins before permit acquisition and includes
capacity waiting, DNS, connect, request, response-head, and response-body work.
Each HTTP connect attempt and each truncated-DNS TCP replay connect also has a
10-second bound. DNS TCP connect is governed by cancellation and the earlier
of its configured connect-timeout deadline or the invocation's overall
deadline. Neither bound resets on progress.

Construction is runtime-independent, but polling the production transport has
the same explicit host precondition as the native AI Gateway HTTP
transport: a current host-owned Tokio runtime with both I/O and time drivers
enabled must remain driven through protocol and socket teardown. No current
runtime handle returns fixed `RuntimeRequired`. Tokio exposes no stable safe
query that proves both drivers are enabled, so polling on a current runtime
that lacks either driver may panic; the repository's release panic policy can
turn that host-precondition violation into process termination. This is a
documented `# Panics` API boundary, not a typed runtime-detection guarantee.
Exactly one cancellation waiter and one outer machine-god Tokio invocation-
deadline sleep are allocated per bounded invocation and reused across permit,
DNS, HTTP, and body waits. Each truncated A or AAAA DNS TCP replay additionally
owns one short-lived configured connect-timeout sleep, so one invocation owns
at most two such sequential DNS replay sleeps. Reqwest/Hyper may own bounded
HTTP connection-attempt timers. The outer timer is never reallocated. Each DNS
replay timer is allocated once when that replay begins; none resets or extends
the outer absolute deadline. After bounded transport work, the final
synchronous boundary checks cancellation/deadline state directly without
allocating a second waiter.
The native transport receives that invocation's absolute deadline and
cancellation token without creating a second waiter. It checks both at every
pre-effect transition before A, AAAA, a truncated-answer TCP replay, HTTP
dispatch, and each response-body read. These checks are authoritative even
when the preceding phase completed immediately without yielding. A fired
boundary returns the fixed cancellation or timeout result before starting the
next effect.

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
keep a machine-god request worker alive or to retry the request. The one outer
machine-god invocation-deadline sleep and each short-lived DNS replay connect-
timeout sleep are owned by the future and are never detached.

## Content classification

After stripping MIME parameters and normalizing ASCII case, declared text,
JSON, XML, and JavaScript media types are eligible for bounded textual output.
HTML is returned as bounded raw untrusted text. The tool does not convert,
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

There is no cache, so `cache_hit` is always false. No result can
refer to a persisted artifact.

## Deliberate upstream differences and deferred work

The pinned upstream's same-site/optional-`www` redirect handling and
default-safe web-fetch admission are deliberately not copied. One approved
`NetworkTarget` and one `Ask` decision cannot authorize a different host, and
all network authority remains `Critical`. Redirect following of
any kind remains deferred.

Also deferred are caching, cache revalidation, binary artifact persistence,
`read_tool_result` integration, progress/completion side channels,
HTML-to-Markdown conversion, compression, private or authenticated targets,
cookies, proxying, retries, browser execution, CLI changes, benchmark workload
changes, product-performance claims, compatibility-inventory promotion, and
complete fx equivalence.
