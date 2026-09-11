# Explicit MCP network admission

On Linux and macOS, `NativeMcpNetwork` implements both HTTP peer destination
selection and `McpAuthNetwork`. Each admission binds the exact parsed URL to
resolved socket addresses and explicitly supplied TLS trust. It stores no
resource headers, OAuth tokens, proxy configuration or browser authority.
OAuth metadata, registration, token and revocation URLs are admitted separately;
resolving an issuer does not authorize forwarding another endpoint's credentials.

## Configuration and effects

`McpResolverConfig::new` accepts explicit nameserver socket addresses, UDP/TCP
selection, negative-answer policy, timeout, attempt count and concurrent-server
count. `literal_only` explicitly disables DNS while retaining literal IP and
localhost endpoints; unavailable DNS never silently selects another resolver.
`capture_system` is a separate synchronous system-configuration operation
for an explicitly owned startup worker. It starts no worker itself. The shared
native selector bounds the system configuration and rejects unsupported options;
Linux reads at most 64 KiB of resolver configuration, while macOS uses the existing
Hickory system selector followed by configuration validation. Callers own startup
worker completion; synchronous OS/library calls are not hard real-time deadlines.

The network constructor is inert. It retains the captured resolver, a host-selected
32-byte secure query-ID key, TLS trust, monotonic clock/timer, owner cancellation
and admission concurrency limit. No request reads resolver files, environment,
hosts files, credentials, certificates or ambient time. There is no system
`getaddrinfo`, `lookup_host`, implicit fallback resolver or detached lookup task.
The deterministic query-ID sequence reuses the native keyed SHA-256 construction;
fixture keys are not production entropy.

Literal IPv4/IPv6 endpoints and exact `localhost` need no DNS. Localhost maps to
IPv4 and IPv6 loopback. Other hosts produce one absolute FQDN without search
suffixes. Private HTTPS destinations are valid; plaintext remains explicit-port
loopback only. The [HTTP connector](mcp-http.md) rejects unspecified, multicast,
foreign literal or non-loopback plaintext addresses. HTTPS requires supplied
trust; plaintext admissions return no TLS trust.

## Shared DNS and bounds

The private `bounded_dns` module contains the existing model-catalog resolver's
configuration selection, bounded Hickory wire decoding, correlated question/
answer validation, query-ID sequence, CNAME checks and owned UDP/TCP exchanges.
The catalog retains its existing timing, retries and reqwest adapter. MCP adds
its own cancellation/deadline orchestration, not another DNS parser or the
public-only address policy of `web_fetch`.

There are at most 32 simultaneous admissions, each covering both A and AAAA
families, at most 32 concurrent nameservers per family, five configured attempts
and seven CNAME hops. A response admits at most 32 address records, 39 answer
records and 128 total resource records in at most 4 KiB of DNS wire data.
The final deduplicated result has at most 32 addresses across both families.
Over-budget or malformed families are not silently omitted to return a partial
result. Empty/negative families are supported, but the complete result must be
nonempty. A nameserver failure may advance to another explicitly selected server
within the finite attempt/deadline budget.

UDP sockets connect to the exact selected nameserver. Response ID, question,
class, opcode and response kind must correlate before a truncated response can
authorize DNS-question replay over the explicitly selected TCP transport.
TCP length prefixes, record counts, trailing bytes and CNAME cycles are checked.
DNS retry never replays an HTTP application request. There is no DNS cache or
retained history that grows with the number of completed admissions.

## Cancellation and ownership

Each admission uses the earlier caller deadline or 30 seconds, including its
semaphore wait. Individual query deadlines use the earlier admission deadline or
configured timeout. The injected clock/timer and both owner and operation
cancellation are observed before acquisition and before publishing addresses.
Concurrent DNS futures own their sockets directly; completion, cancellation or
dropping the caller's future releases pending sockets and permits. No background
worker must finish after the owner disappears.

Address/trust results are immutable data, not self-revoking network capabilities.
The runtime must retain `owner_cancellation()` in subsequent peer/exchange
ownership, and in OAuth operation cancellation. Cancelling DNS admission alone
cannot retroactively close a socket created by an independently authorized
consumer. The [HTTP peer](mcp-http-peer.md) and [OAuth service](mcp-auth.md) own
those later effects and completion evidence. Local socket release does not
claim remote cancellation or credential revocation.

The [implementation plan](implementation-plan.md) owns complete-feature gates
and production host/CLI composition status.
