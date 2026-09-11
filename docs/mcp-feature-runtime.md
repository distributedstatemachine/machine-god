# Native MCP feature codecs

`machine_god_native::mcp::feature` derives bounded protocol data from the existing
`McpFeatureRequest`, exact server selection, admitted descriptor catalogs and
typed peer capabilities. Exchanges, results and metadata are data only: they do
not open a connection, publish a generation, reserve a peer ID, grant permission,
obtain consent, create a responder or send a request. Runtime ownership must bind
them to live configuration, authentication, catalog and turn generations before
any transport effect and again before publishing results.

## Requests and descriptor identity

`McpFeatureExchange::prepare` selects only these seven fixed methods:

| Feature action | Protocol method |
| --- | --- |
| `resource_list` | `resources/list` |
| `resource_templates` | `resources/templates/list` |
| `resource_read` | `resources/read` |
| `prompt_list` | `prompts/list` |
| `prompt_get` | `prompts/get` |
| `prompt_complete` | `completion/complete`, `ref/prompt` |
| `resource_complete` | `completion/complete`, `ref/resource` |

The peer supplies a separately reserved nonnegative integer request ID and an
admitted protocol/transport pair. No arbitrary-method or raw-parameter constructor
exists. List cursors, including an empty cursor, are legal only for list actions
and have a 4,096-byte bound. The existing typed request retains its 64 KiB
canonical-input bound; native envelopes have a separate 128 KiB ceiling.
Request retention separately charges shared catalog storage, wire/decoded text,
fixed records and sparse argument/context map slots before cloning request data.

Server, URI/template and prompt identities are exact bytes. Resources require
resource capability, prompts require prompt capability, and either completion
also requires completion capability. Catalog families must be unique and match
the selected protocol. Prompt gets reject unknown argument keys and missing
required arguments before encoding. Completion argument names remain
server-authoritative, matching the producer, rather than being rejected merely
because they are absent from prompt argument metadata.

The shared `McpClientMetadata::for_protocol` codec supplies exactly the same
modern protocol version, client information, elicitation advertisement and
optional progress token for tool and feature requests. Legacy metadata is absent
unless a progress token exists. Advertised form/URL support is data, not evidence
of a responder or consent. Continuation responses/state are not accepted through
the ordinary request constructor.

## Pinned URI-template matching

Concrete resource identities win before template matching. Otherwise the first
matching byte-sorted admitted template is selected under one shared
1,048,576-step budget. `matches_resource_template` only accepts an already
admitted descriptor; it does not duplicate template grammar admission.

Matching preserves the pin's single-variable RFC 6570 subset: simple, reserved,
fragment, label, path, path-parameter, query and query-continuation operators.
Percent escapes remain literal spelling; ordinary expansions accept unreserved
bytes and valid `%HH`, while reserved/fragment forms also allow reserved bytes.
Named forms preserve their exact variable names and operator-specific empty-value
rules. Ambiguous literal boundaries use the producer's bounded search order.
Work exhaustion is an error, not a no-match result. Matching performs no URI
normalization or external lookup. The pin has no value-to-URI expansion API;
this component deliberately adds no unpinned expansion feature.

## Responses and atomic catalogs

`admit_response` checks the full duplicate-free JSON-RPC envelope against the
exchange's exact integer ID before selecting its fixed read/get/completion
parser. Wrong IDs or response kinds, malformed success, invalid fields and
budget exhaustion never trigger fallback or become successful results.
Responses preserve their complete original raw JSON, including unknown metadata,
large numeric lexemes, signed zero and private-looking object keys.

Resource contents retain URI, MIME type, text/blob and their raw annotations and
metadata. Prompt messages retain exact role (`user` or `assistant`), raw message
and typed content kind: text, image, audio, resource link or embedded resource.
Image/audio/blob fields require valid standard base64. Resource-link content
names use the producer's 4,096-byte limit, independently of 256-byte catalog
names. Shared crate-private content admission is also available to native tool
result composition after its strict envelope admission; it grants no authority.

Completions preserve value order and duplicates, exact nonnegative integral
`u64` totals (including decimal/exponent spelling) and optional `hasMore`.
Cache hints preserve optional TTL and private/public scope; negative TTL clamps
to zero, with exact bounded decimal validation. Protocol failures retain their
original bounded message/data while public debug displays remain redacted.

Modern read/get `input_required` results become explicitly
`UnvalidatedInputRequired` handoff data. Only envelope correlation, JSON and
handoff byte/depth bounds are established here. A separate MRTR decoder must
validate requests/state and obtain explicit input consent before continuation;
this marker neither approves input nor labels the requests fully admitted.
Legacy and completion `input_required` responses are rejected.

`McpFeatureCatalogLoad` binds a list exchange's server, family and version. Every
page uses the existing `McpCatalogBuilder` cursor, identity, cache and aggregate
checks. `finish` then performs complete `McpDescriptorCatalog` admission without
rewriting pages or duplicating descriptor/schema parsing. Any failure closes the
load; unfinished or malformed candidates cannot replace prior usable catalogs.

## Independent resource and projection bounds

Complete envelopes have lowerable limits of 16 MiB, 262,144 JSON values/keys and
depth 32 with the root at zero. Read/get results allow 256 contents/messages,
1 MiB content fields and 4 MiB aggregate content. Metadata remains 128 KiB and
descriptions 64 KiB. Completion bounds are 100 values, 4,096 bytes per value and
64 KiB total value bytes. Retained response admission charges five times original
JSON bytes plus bounded typed-item slots against 64 MiB before copying retained
data. Transient parsing is separately bounded by one envelope/content item;
no detached task, timer, queue or reference I/O exists.

These full native results are not silently truncated to the older
[`mcp_features`](mcp-features.md) adapter. That adapter independently limits both
authority payloads and complete serialized model output to 64 KiB and 4,096 JSON
nodes. Runtime/model integration must explicitly resolve that mismatch; this
codec does not advertise a full result as fitting the smaller projection.

Behavior follows fx `b1774fbf6c7602b503026f96f6e960e946c692ef`, particularly
`features/resources.zig`, `prompts.zig`, `completion.zig`, `common.zig` and exact
feature snapshot selection in `mcp_runtime.zig`. This component is not a complete
MCP runtime delivery or a performance claim.
