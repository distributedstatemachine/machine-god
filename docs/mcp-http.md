# Owned MCP HTTP exchanges

On Linux and macOS, the native `mcp-http` feature provides actual HTTP/1.1 TCP
and verified TLS exchanges. `McpHttpConnection` is a single-use, explicitly
selected destination and request-header owner, not a pooled client. Its
constructor and unpolled operation futures do not connect or send requests.

## Destination and TLS authority

`McpHttpDestination` binds one admitted [endpoint](mcp-runtime.md) to 1–32
explicit, unique resolved socket addresses. Each address has the endpoint's
port. Literal IP endpoints must match their addresses; `localhost` and plaintext
HTTP remain confined to loopback. Unspecified and multicast addresses are
rejected. Explicit private HTTPS addresses are valid. A trusted native resolver
must establish these addresses for this exact endpoint; this connector does not
perform ambient DNS lookup or treat an arbitrary server response as DNS authority.

`McpHttpTrust` accepts explicit certificate trust anchors, not a configurable
certificate-verification bypass. It preserves hostname verification and SNI,
offers only HTTP/1.1, and disables early data, client authentication, resumption
and key logging. Trust is bounded to 512 anchors and 4 MiB of anchor bytes.
Plaintext connections reject supplied TLS trust; HTTPS requires it. No ambient
proxy, credential, certificate-file or environment discovery occurs here.

Connection failure may advance once through the selected addresses before any
HTTP write. TLS failure is terminal. There is no retry after request submission,
redirect following, connection pooling, pipelining, HTTP/2, automatic listener
reconnection, decompression or consequential request replay.

## Exact proof-bearing plaintext writes

`submit` requires one claimed [native submission](mcp-submission.md) and the
explicit runtime allocation admitted by its trusted caller. It compares runtime
allocation identity and the complete frozen request against its own selected
endpoint and headers before TCP/TLS acquisition. Changed authentication, protocol
headers, target or framing is rejected before connecting.

The connector copies the bounded, already admitted request bytes and sends those
exact bytes through `McpSubmissionHttpDriver`, immediately above the TLS
plaintext stream. There is no second HTTP serializer: Hyper header ordering or
casing cannot change the admitted request. Every bounded write and final flush
retains the original native proof and its five cancellation observers. Partial
acknowledgements advance only the written prefix; no code retries the request.
The response reader retains the owned cancellation observers after final flush.

`McpHttpControl` uses the existing strict protocol-control admission for
discovery/catalog requests, initialized/cancelled notifications and fixed
unsupported-method replies. Explicit GET listener and DELETE session-teardown
forms are separate bodiless typed operations with an SSE-only Accept header.
The runtime must supply admitted session
headers and authorize teardown; protocol metadata cannot mint that authority.
Arbitrary tool calls, application feature methods and successful continuation
replies cannot enter this control lane.

## Response and resource bounds

Response heads and chunk-size/trailer syntax use `httparse`, with strict CRLF
line framing. Content-Length, chunked bodies with bounded extensions/trailers,
and connection-close bodies stream in chunks of at most 16 KiB. Identical repeated
Content-Length values are accepted; differing lengths, Transfer-Encoding plus
Content-Length, unsupported transfer codings, compression and protocol upgrades
are rejected. Up to eight informational responses are accepted within one
aggregate head budget. No-body status semantics do not wait for connection EOF.

Default limits are 64 KiB response-head bytes, 128 headers, 64 MiB body bytes and
128 MiB total plaintext response-wire bytes. Callers may select positive limits
up to 256 KiB head bytes, 256 headers, 1 GiB body bytes and 2 GiB wire bytes.
Chunk-size/extension lines are limited to 8 KiB. Trailers have a separate head
budget and remain separate observations; framing-changing trailers are rejected.
Raw request bounds remain those of the submission head/body. There is one
16 KiB read-ahead buffer and no accumulated response queue. Rustls application
write buffering is explicitly limited to 32 KiB; its protocol framing and
certificate-handshake limits remain enforced by the established TLS library.
Hosts independently
bound simultaneous exchanges and caller-retained response chunks/observers.

Status, headers, trailers and body bytes remain uncommitted observations.
Duplicate header names are preserved rather than silently overwritten. The
runtime must validate content type, status, JSON/SSE, generation, session ID,
authentication and protocol-specific metadata before publication. SSE data can
feed the existing [bounded decoder](mcp-sse.md); this transport does not interpret
events or publish resume cursors.

## Lifetime and completion

The public standalone constructors require an explicit positive operation
deadline at most 24 hours away. Observed configured peers select a separate
native-only prepared-head policy admitting at most `u32::MAX` milliseconds per
exchange, preserving the complete configured timeout without weakening those
existing constructor limits. Both paths reject oversized or expired deadlines.
The same selected deadline and cancellation cover TCP connection, TLS handshake,
request writes,
response head and every body read, including buffered bytes. Userspace waits are
bounded; synchronous OS/library work and scheduling are not real-time guarantees.

The native peer has one private exception for persistent read-only GET listeners:
after exact successful status, session and SSE validation (and same-origin
endpoint admission for deprecated HTTP+SSE), it may promote that existing body's
read lifetime to the selected peer ownership. Connect, TLS, GET writes, response
headers and initial endpoint discovery retain the original finite deadline.
Promotion requires the actual GET-listener lane, rejects repeat promotion, and
never resets framing, byte budgets or completion ownership. A feature-scoped
guard retained through GET acquisition is revalidated at transfer; only that
operation observer is detached as the listener becomes peer-owned. Peer
cancellation and explicit expiry remain. Ordinary protocol, OAuth, application and resumed response bodies
cannot acquire this ownership policy. Public raw constructors remain unchanged.
Owner-controlled listening observes cancellation until explicit close; an
explicit peer expiry remains enforced. Every caller-driven listener observation
still has a finite deadline outside its retained parser future.

`with_clock` explicitly injects the monotonic clock and timer used at every
acquisition/write/read boundary; the existing `new` constructor retains its
system-clock behavior. `from_prepared_head` accepts immutable shared request
data only when its endpoint equals the independently admitted destination.
The [HTTP peer](mcp-http-peer.md) composes these seams without a second serializer.

No task or worker is spawned: the polled exchange future owns acquisition and
the returned body owns its socket. Dropping either releases the socket directly.
Dropping an in-progress body-read future also closes the body rather than
reinterpreting a partially consumed HTTP frame. Malformed/truncated responses and
budget exhaustion close the body permanently. A completed body releases its
socket without requiring a separate task to be driven.

`McpHttpObservation` survives future abandonment and reports whether plaintext
submission was attempted, acknowledged plaintext bytes and ownership completion.
Completion means the local socket owner was released, not remote revocation,
remote execution success or proof that ambiguous writes had no effect. Its
completion observer does not drive the operation: the host must continue polling
or drop the owner. Debug and error output omit endpoint, header and payload data.

Runtime DNS selection, protocol negotiation, listeners/subscriptions, catalog
publication, OAuth, application feature/continuation authority and CLI activation
remain separate native composition. The [implementation plan](implementation-plan.md)
owns complete-feature status and gates.
