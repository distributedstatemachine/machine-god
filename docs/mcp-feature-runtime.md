# Native MCP feature codecs and owned exchanges

`machine_god_native::mcp::feature` derives bounded protocol data from the existing
`McpFeatureRequest`, exact server selection, admitted descriptor catalogs and
typed peer capabilities. Exchanges, results and metadata are data only: they do
not open a connection, publish a generation, reserve a peer ID, grant permission,
obtain consent, create a responder or send a request. Runtime ownership must bind
them to live configuration, authentication, catalog and turn generations before
any transport effect and again before publishing results.

## Native-selected transport execution

`McpStdioPeer::feature` and `McpHttpPeer::feature` compose those codecs with an
owned, serialized connection. They accept the exact typed request, selected
server/catalogs, opaque `McpFeatureControlAuthority`, explicit lowerable operation
bounds, timestamp origin and deadline. The peer allocates every nonnegative
integer ID, including every list page, from its existing never-reused sequence.
The closed result is either a complete admitted descriptor catalog or a complete
typed response. No raw arbitrary-method constructor or `tools/call` bypass is
added. List actions use this guarded feature path, not the startup catalog lane.

Only crate-local native composition can mint control authority. A model operation
retains its actual routed `NativeMcpTurnContext`; an idle human command instead
retains an explicit command/host lifetime token, never a fabricated core turn.
Both retain operation and route-retirement signals and at most eight selected
authentication/authority guards. Descriptors, metadata advertisements, model text
and returned data cannot mint this authority. Selecting and retaining the exact
published server/generation remains the native facade's responsibility.
The exact publication retirement flag is checked as well as its cancellation
token: marking retirement under a publication lock cuts off writes before the
token wakes waiters outside that lock. No callbacks are invoked under that lock.

All signals are observed during waits and again at each actual queued stdio
write/suffix/completion or HTTP plaintext write/flush above TLS. Model context is
revalidated at these checkpoints independently of caller-supplied tokens. HTTP
connect/read lifetimes retain the same
guard. Cancelling or dropping a polled exchange closes owned transport state;
no request or acknowledged prefix is replayed. Malformed result admission also
retires the exchange. Successful data is rechecked against authority/deadline
before return; it does not itself publish a generation or authorize continuation.

Modern HTTP derives its exact `Mcp-Method` from the prepared exchange and retains
the selected endpoint/headers. The pin supplies `Mcp-Name` only for `tools/call`,
so feature exchanges add neither name nor parameter projection headers. Form and URL advertisements
default to false; enabling data advertisements does not install a responder.

HTTP feature response parsing uses the explicitly selected codec bounds, up to
the existing 16 MiB/262,144-node hard limits, then restores ordinary peer bounds.
Modern SSE response readers retain bounded line/data capacity; actual response
limits and independent notification budgets still apply. Stdio framing limits are immutable launch authority:
`wire_limits` exposes them, and insufficient capacity rejects a feature before
sending. A full-feature launch selects 16 MiB, depth 64 and 262,144 nodes; callers
may explicitly lower matching codec bounds, but neither side silently widens a
selected lower limit. Catalog pages still pass through the existing atomic
assembler and descriptor admission without numeric normalization or truncation.

The public methods project response data and discard private continuation
custody. Crate-local runtime composition instead uses an owned feature round:
only correlated modern resource reads and prompt gets can retain a pending
round. Consuming it preserves the original typed request, arguments, selected
descriptor snapshot, metadata, limits and control authority. It neither clones
whole catalogs nor selects descriptors again. The original peer allocation
identity is checked before allocating a fresh, never-reused request ID; a
different peer cannot resume the round even at the same endpoint.

The private continuation encoder revalidates answers against the retained input,
adds exact `inputResponses`, and includes `requestState` if and only if the
correlated response supplied it. Absent state and explicit null stay distinct;
raw numeric spelling is preserved. Continuation does not provide a public replay
or arbitrary-method API. Original control guards remain attached through actual
transport writes and response admission. Unpolled futures are inert; dropping a
polled exchange keeps the same owned-close and no-prefix-replay behavior.

These transport methods do not themselves obtain consent, launch browsers, add
subscriptions or TTL refresh, implement sampling/roots, or project model output.
Human interaction remains separately owned native runtime composition.

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
optional progress token for tool and feature requests. Advertised form/URL support is data, not evidence
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
Completion `input_required` responses are rejected.

The owned peer-round path performs that bounded MRTR decode before retaining
custody, while leaving the public response marker unchanged. Its retained-byte
check jointly charges the original exchange, correlated raw response and typed
input against the original feature codec budget. MRTR parsing also preserves its
independent lowerable byte/node/retention limits. Each resumed envelope remains
subject to the original request and response budgets.

`McpFeatureCatalogLoad` binds a list exchange's server, family and version. Every
page uses the existing `McpCatalogBuilder` cursor, identity, cache and aggregate
checks. `finish` then performs complete `McpDescriptorCatalog` admission without
rewriting pages or duplicating descriptor/schema parsing. Any failure closes the
load; unfinished or malformed candidates cannot replace prior usable catalogs.

## Native model and human routing

`NativeMcpRuntime::feature_for_turn` selects the actual registered turn and its
existing publication pin. A foreign or retired context cannot select a current
replacement. `human_command()` instead creates an explicit non-clone command
lifetime with a weak runtime reference; it never manufactures a model turn.
Construction and unpolled operations perform no I/O or clock observation.
Each polled operation selects the exact case-sensitive configured server.

Both routes use the same bounded peer queue as tools and retain the original
turn/command, operation, configuration/authentication and runtime signals through
the final transport write, response admission and returned result. Publication's
shared retirement flag is checked at its atomic cutoff, before deferred token
wakeups. Closing a command cancels its pending operations; it does not close an
unrelated server or another command.

Non-list actions lazily fetch their required catalog families through guarded
typed exchanges under one operation deadline. Exact resources avoid a needless
template fetch; template fallback uses the existing admitted matcher. No caller
catalog or expired startup snapshot supplies identity evidence. This path does
not cache results: TTL/notification-driven caching remains separate work.

`NativeMcpFeatureResult` retains the original selection and exposes data through
`reply()`, with separate `revalidate()` and `cancelled()` checks for projection
and archive composition. Reading data alone asserts no continuing authority.
At most two operations/results coexist per runtime (or the lower configured
pending limit); keeping a result retains its slot until drop. Catalog accumulation
is bounded to 64 MiB, with at most one separately admitted 64 MiB incoming family
before that aggregate check, plus bounded wire/parser storage. No hidden result
queue or detached observer retains additional generations. Catalog replies retain
original descriptors, not discarded page-envelope metadata.

Read/get input rounds compose with the same native presenter and URL launcher as
tools. The human owner is the retained conversation principal, not model text or
an invented turn. The runtime releases the serialized peer lane while collecting
input but retains the operation slot and exact round. It permits at most eight
consuming continuations, bounds each human round to 30 minutes, and starts a fresh
configured transport deadline only after input collection. Unsupported,
state-only or exhausted input produces a failed receipt rather than a successful
partial feature result. Returning to transport rechecks the original authority
and peer without rediscovery or catalog replacement. URL work retains the exact
active round and authority until its owned cleanup completes or is dropped.

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
