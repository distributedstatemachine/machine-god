# MCP catalog page assembly

`machine_god_native::mcp::pagination` assembles one complete, immutable raw tool,
resource, resource-template or prompt catalog. It performs no I/O, reads no clock
and grants no execution authority. The caller supplies the protocol, exact
outstanding JSON-RPC ID, requested cursor and monotonic receive timestamp.

The builder checks the selected result array, nonempty remote identity, exact
response correlation, duplicate JSON keys, duplicate item identities within and
across pages, and consistent cache scope. Items are sorted by their exact remote
identity. Serde locates and retains each item's original JSON bytes, preserving
schema numbers, unknown fields and vendor metadata without floating-point
canonicalization. The temporary decoded tree is used only for admission and
identity indexing, not as the retained schema representation.

There is no partial snapshot accessor. `append_response` reports whether another
page is needed; an empty `nextCursor` is a real opaque cursor. The next request
must carry exactly the observed cursor. Repeated cursors, clock regression,
extra pages after completion and all other failures close the builder and release
its accumulated data. `finish` consumes it and requires at least one page and no
outstanding cursor. A failed or unfinished candidate cannot become a snapshot.

## Cache hints and bounds

Behavior follows pinned fx `b1774fbf6c7602b503026f96f6e960e946c692ef`, especially
`src/core/mcp/features/{tools,resources,prompts,common}.zig` and
`src/core/mcp/feature_cache.zig`. An absent `resultType` is allowed in both
protocol families; if present it must be `complete`. Cache scope defaults to
`private`; supplied values must be `private` or `public` and agree across pages.

TTL conversion uses the exact decimal lexeme, not a rounded float. Negative
numeric TTLs, including negative fractions, mean zero. Nonnegative values must
be mathematically integral and fit `u64`; decimal/exponent spellings are allowed.
Lexemes are bounded to 4 KiB and absolute normalized/explicit exponents to one
million, without expanding the number. The enclosing protocol wire admission
still applies. Missing TTL is zero for modern MCP and indefinite for legacy MCP.
Absolute expiry saturates at `u64::MAX`; the catalog retains the earliest page
expiry and the first page's receive timestamp. A public cache hint does not
authorize cross-owner or cross-generation publication.

Default limits are also hard maxima; callers may lower them but cannot select
zero: 64 pages, 4,096 items, 16 MiB cumulative response bytes, 8 MiB cumulative
retained item JSON plus identity bytes, 4 KiB per cursor, and 262,144 cumulative
JSON nodes/keys. Tools retain wire depth 64 (root counted as one).
Resource/template/prompt envelopes use the pinned common feature depth 32 with
root counted as zero, including ignored envelope metadata. This corresponds to
33 in the wire visitor. Cursors additionally occupy at most 64 bounded
entries plus one next-cursor copy. The builder additionally caps the selected
family at the pinned 2,048 tools or 4,096 resources/templates/prompts; callers'
lower item limits still apply. Tool/prompt identity lengths are 256 bytes;
resource/template identities are 64 KiB. Original response bytes, including
ignored metadata, are charged before parsing. Tree limits apply during parsing,
not only after allocation; raw item copies are charged before retention.
Ordered maps avoid quadratic duplicate detection. Debug and errors omit all
remote identities, cursors, metadata and response content.

## Publication boundary

`McpRawCatalog` deliberately does not assert that item schemas or descriptors
are executable or valid for a feature. Trusted native composition must validate
every complete item, bind server/configuration/protocol/authentication and
connection generation, enforce aggregate host budgets, and publish atomically.
On reload failure it must retain the previous usable runtime. Refresh
coalescing, TTL scheduling, notification invalidation, executable tool naming,
schema admission and per-conversation routing belong to that runtime boundary,
not this page assembler. Raw metadata never restores permissions or grants
filesystem, process or network authority.
