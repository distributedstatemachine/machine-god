# Resolved MCP remote headers

`mcp::headers::McpResolvedHeaders` resolves an admitted `McpRemoteConfig`
against an explicitly supplied captured environment lookup and an optional
active OAuth access-token byte slice. It performs no ambient environment,
network, filesystem, process, credential refresh or authentication effects.
The lookup callback remains caller-owned; hosts should provide captured data,
not a live environment-discovery callback.

Resolution follows `buildResolvedHeaders` in `src/core/mcp/mcp_runtime.zig`
at the pinned fx revision: static profile headers first, then configured header
environment bindings, then generated Authorization. Missing configured header
values are errors. Active OAuth credentials take precedence over bearer-token
environment lookup, which is not called at all when an active token is supplied.
OAuth configuration alone is not an active credential. Without active OAuth,
a configured bearer environment variable is required. Empty captured values
and empty explicit active tokens are present values, matching the producer.
Bearer generation only prefixes `Bearer `; it does not invent a token grammar.

Separately supplied resolved/ACP headers are admitted between environment
headers and generated Authorization. `from_resolved` admits a standalone
resolved set without profile data or environment lookup. Authorization is
valid in these resolved inputs, unlike
[profile configuration](mcp-cli.md#profile-configuration-codec). An explicit
Authorization combined with a generated one is a duplicate error, not an
override. All duplicate names are rejected case-insensitively across the final
combined set. Existing spelling and iteration order are retained; static and
environment groups follow the validated codec's deterministic map order.

## Grammar and bounds

Names must be nonempty ASCII HTTP tokens. The pinned transport-owned fields
are forbidden: Accept, Accept-Encoding, Connection, Content-Length,
Content-Type, Host, Last-Event-ID, MCP-Method, MCP-Name, MCP-Protocol-Version,
MCP-Session-ID, Transfer-Encoding and every `MCP-Param-` prefix, case-insensitively.
Values reject bytes below 0x20 except HTAB, and reject DEL (0x7f). Raw non-UTF-8
bytes above 0x7f, spaces, tabs and empty values retain their exact bytes. This is
the pinned resolved-header byte grammar, distinct from the configuration codec's
stricter active Unicode-string checks.

The final set admits at most 128 headers, 16 KiB per name or value, and 512 KiB
of combined name/value bytes. All limits are inclusive; generated Authorization
and its seven-byte prefix count. Thus 128 configured fields plus a generated
Authorization exceed the final bound. These finite limits intentionally bound
the producer's otherwise unbounded resolved collection. Count preflight and
borrowed field validation precede owned name/value copies. Retained fields and
collections are compact; clones share their immutable allocation.

## Private authentication identity

Debug and error forms omit names, environment references and values. Iteration
intentionally exposes secret-bearing bytes only for trusted transport composition.
There is no public-state serialization implementation.

`authentication_identity_bytes()` deliberately returns an owned secret-bearing
encoding for the native submission binding. It begins with `MGH1`, a big-endian
u32 field count, then entries sorted by lowercase ASCII name. Each entry contains
a u32 name length, lowercase name, u32 value length and exact raw value bytes.
This exact length-framed identity has no hash collisions, delimiter ambiguity,
header-order dependence or header-name case dependence. Its maximum allocation
is 525,320 bytes (512 KiB plus 128 eight-byte entry frames and an eight-byte
prefix). The header object does not retain a second encoded representation.

Only trusted native composition should request, retain or compare these bytes;
never log them, expose them as public state or truncate them to fit another
budget. A caller with a smaller permission-key budget must reject or disable
persistent reuse explicitly. This identity covers the complete resolved header
set, not endpoint, configuration generation, credential provenance, session or
turn. Runtime authority must bind those separately. Byte equality is not a
constant-time credential-verification primitive, and this module does not claim
secure erasure of allocator storage or grant network authority.
