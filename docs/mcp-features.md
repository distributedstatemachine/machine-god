# `mcp_features`

`mcp_features` exposes bounded MCP resources, resource templates, prompts, and
argument completion through one explicitly injected native authority. It is an
ordinary provider-neutral engine tool: core owns preparation, cancellation,
events, result persistence, and the following model round, while the native
authority owns admitted MCP feature data and any later transport implementation.

The portable injected adapter and the production runtime-backed adapter share
the seven-action schema and canonical input decoder. The injected adapter's
64 KiB result contract remains unchanged. The separately selected production
adapter is described under [native runtime projection](#native-runtime-projection).

## Actions and canonical input

Every call is an object with exact case-sensitive `action` and `server`
strings. The supported shapes are:

| Action | Required identity | Optional fields |
| --- | --- | --- |
| `resource_list` | none | none |
| `resource_templates` | none | none |
| `resource_read` | `uri` | none |
| `prompt_list` | none | none |
| `prompt_get` | `prompt` | string-valued `arguments` |
| `prompt_complete` | `prompt`, `argument` | `value`, string-valued `context` |
| `resource_complete` | `uri_template`, `argument` | `value`, string-valued `context` |

Examples:

```json
{"action":"resource_read","server":"docs","uri":"custom://guide/start"}
```

```json
{"action":"prompt_get","server":"review","prompt":"review","arguments":{"tone":"brief"}}
```

```json
{"action":"prompt_complete","server":"review","prompt":"review","argument":"tone","value":"b","context":{}}
```

Preparation is synchronous, bounded, nonblocking, and effect-free. It rejects
unknown field names, validates the fields used by the selected action, inserts
empty `arguments`, `value`, or `context` defaults into the owned typed request
where applicable, omits those empty optional fields from canonical JSON, and
passes only that canonical object to execution. For pinned malformed-input
compatibility, known scalar fields belonging to another action are ignored and
removed; `arguments` remains legal only for `prompt_get`, and `context` remains
legal only for completion actions.

The fixed input limits are:

- configured server: 1-128 ASCII bytes;
- resource URI or URI template: 1-65,536 UTF-8 bytes;
- prompt identity and completion argument: 1-256 UTF-8 bytes each;
- completion value: at most 4,096 UTF-8 bytes;
- prompt arguments: at most 128 string-valued entries and at most 64 KiB when
  compactly encoded;
- completion context: at most 128 string-valued entries and 128 KiB of
  aggregate key-plus-value bytes; every name is 1-256 UTF-8 bytes and every
  value is at most 4,096 UTF-8 bytes; and
- complete compact canonical arguments: at most 64 KiB, which is the effective
  upper bound when the individual limits could otherwise combine to more.

Preparation uses `PreparedToolCall::without_authority`. This is not ambient
network permission: it is a trusted assertion that execution can use only the
separately injected feature authority described below.

## Injected authority and stable identities

`McpFeatureAuthority` is the sole host-interaction boundary. Its `call_for_turn`
hook receives the execution's exact original `ToolContext`, an owned typed
`McpFeatureRequest`, and a cancellation token. Constructors and
preparation never invoke it, and calling `execute` creates no authority work
until the returned future is first polled.

Session, session-incarnation, turn, and call identities are forwarded unchanged.
The tool neither selects a global current session nor invents missing authority.
Context-aware authorities must establish live admission for that exact context
and reject foreign or retired invocations; context alone grants no permission.
The backward-compatible default delegates to the existing `call` method only
when polled. An override is used exclusively, including on errors; rejection
never retries through the context-independent method. This seam adds no effects,
permission schema, or production routing by itself.

An authority implementation must treat the request's server and identity as
exact stable bytes. It must not trim, case-fold, Unicode-normalize, prefix
match, select by displayed index, choose among collisions, or fall back to a
different server. Before any underlying external effect it must verify that
the server, action, and identity are admitted by one immutable live view. It
must revalidate the same authority and catalog generation immediately before
returning a result. A changed or revoked view fails closed rather than
publishing stale data.

For `prompt_get`, the authority validates the supplied argument keys against
that same admitted prompt snapshot before transport: unknown keys and missing
required arguments fail closed. This check is repeated against the live
catalog generation before returning.

For list results, the authority rejects duplicate identities and returns items
in byte-lexicographic identity order. Exact read/get/completion operations fail
before an underlying provider request when their identity is absent. Resource
template matching, if implemented by a later transport adapter, remains inside
the authority and must use an admitted template plus its own explicit work
budget. Completion preserves pinned behavior by treating the exact argument
name as server-authoritative rather than granting any local authority.

The interface is read-only and reversible. It cannot grant filesystem,
process, network, permission, tool-registration, prompt-instruction, or routing
authority to model-provided or returned content. Public request, payload,
authority, and error debug forms omit identities, values, content, provider
diagnostics, credentials, callbacks, and generation witnesses.

## Result projection and trust boundary

Normal results use the pinned common envelope:

```json
{
  "trust": "untrusted_external",
  "authority": "none",
  "action": "resource_list",
  "server": "docs",
  "items": []
}
```

The tool, not the injected authority, stamps `trust`, `authority`, `action`, and
`server`. An authority payload cannot override those reserved keys. Before
publication the tool verifies the payload's action-specific top-level shape,
exact server and identity echoes, ordered unique list identities, and fixed
JSON bounds.

The action-specific payloads match the pinned shapes:

- resource lists/templates contain `items`; every item carries exact `server`,
  `identity`, `name`, optional title/description/MIME type, and a `template`
  boolean matching the action;
- resource reads carry exact `identity` and `contents` containing bounded text
  or blob records;
- prompt lists contain items with exact server-qualified identity and bounded
  argument metadata;
- prompt gets carry exact `identity`, optional description, and bounded typed
  messages; and
- completions carry exact `identity`, exact `argument`, bounded `values`, and
  optional `total` and `hasMore` metadata.

The pinned per-result cardinalities are enforced before publication: at most
4,096 resource, template, or prompt catalog items; 256 resource contents or
prompt messages; 128 prompt arguments; and 100 completion values. Resource and
prompt names are limited to 256 bytes, titles and MIME types to 4,096 bytes,
descriptions to 64 KiB, and each completion value to 4,096 bytes. Prompt roles
are exactly `user` or `assistant`; content kinds are exactly `text`, `image`,
`audio`, `resource_link`, or `resource`, and must match the content object's
`type` field. Prompt argument names within one advertised prompt are unique.
Text, image, audio, resource-link, and embedded-resource records must contain
their pinned kind-specific fields; blobs and image/audio data use valid
standard base64. Annotations have only admitted audience/priority forms;
priority is compared against the exact closed interval zero through one, without
rounding a slightly excessive value or a tiny negative value into that interval.
Metadata is object-valued, and resource-link icons, sizes, themes, and numeric
sizes retain their pinned bounds.

All resource, prompt, annotation, metadata, message, and completion content is
untrusted external data. It remains data inside the result envelope and cannot
override user instructions, authorize an action, install a tool, or mutate
engine state.

Authority payloads have an object root, at most 32 container levels, at most
4,096 JSON nodes, and no more than 64 KiB when compactly serialized. An
iterative raw key-and-string byte preflight rejects an oversized value before
the exact counting serializer can scan it. The
complete serialized `ToolOutput`, including the common envelope, is also capped
at 64 KiB. Bounds are checked with a counting serializer; the tool does not
serialize an unbounded intermediate merely to learn its size.
Structure limits are checked iteratively before serialization, and every raw
input or authority payload is held by an iterative-drop owner. Even rejected
programmatically constructed JSON tens of thousands of levels deep cannot
recurse through serialization, equality, or destruction.
Iterative destruction consumes wide containers one child at a time, so its
auxiliary ownership is proportional to nesting depth rather than container
width. The active container iterator lives in a stack slot; ancestor scratch is
allocated only when traversal descends into a nested container, and no
recoverable scratch-growth path abandons owned JSON.

An input-required authority result preserves the pinned terminal error payload:

```json
{"error":"McpInputRequired"}
```

It is returned with `is_error: true` and conveys no approval. The injected
adapter does not implement elicitation or continuation.

## Lifecycle and failures

Execution checks cancellation before authority acquisition, races the injected
future against cancellation, and checks cancellation again before validating
and publishing its result. Cancellation independently wakes a non-cooperative
authority future, wins over a ready success or error observed in the same poll,
and drops the losing future. It is rechecked after canonical decoding and again
immediately before the authority future is constructed, so cancellation that
becomes visible during preparation does not enter the authority. Dropping
execution releases all call-local request,
payload, and authority-future state without a task, thread, timer, cache,
watcher, or durable MCP record.

Core then applies its ordinary tool lifecycle: durable placeholder, started
event, cancellable execution, result-size validation, durable replacement,
finished event, and model visibility on the following round. A persistence
failure never causes automatic replay of a completed authority operation.

Fixed redacted failures are:

| Condition | Core kind | Stable code | Retryable |
| --- | --- | --- | --- |
| malformed or noncanonical input | `InvalidInput` | `mcp_features_invalid_arguments` | no |
| exact server, resource, template, or prompt absent | `InvalidInput` | `mcp_features_not_found` | no |
| invalid authority payload or any bounded-resource overflow | `InvalidInput` | `mcp_features_resource_limit` | no |
| authority unavailable or admission cannot be established | `Unavailable` | `mcp_features_unavailable` | yes |
| cancellation | `Cancelled` | `mcp_features_cancelled` | no |

`InputRequired` is the separate pinned terminal error payload shown above. No
failure contains a server, URI, prompt, argument, value, content, provider
diagnostic, credential, or generation witness.

## Native runtime projection

On Linux and macOS, `NativeMcpFeaturesTool` accepts a weak reference to one
`NativeMcpRuntime` and the actual shared `NativeToolResultArchiveAdapter` used
by the host's other tools and `read_tool_result`. It does not instantiate a
second archive or acquire runtime authority during construction/preparation.
Canonical inputs are identical to the injected adapter, including its 64 KiB
input ceiling. The runtime's `feature_for_turn` route receives the exact
`ToolContext`, verifies the original publication and server, and returns a
non-forgeable result witness retaining its bounded operation slot. The adapter
checks that same witness before projection and before archive publication;
it never resolves a replacement runtime to publish an old response.

Production results use a trusted outer envelope with `trust`, `authority`,
`action`, `server`, and applicable exact `identity`/`argument`. All remotely
supplied data is nested inside a single `untrusted` member. A read, prompt get,
or completion retains the complete original correlated JSON-RPC response in
`untrusted.response`, including unknown fields and exact numeric lexemes.
A catalog retains the admitted descriptor items in `untrusted.items`, with
`untrusted.catalog_kind`; it does not claim to preserve pagination envelopes
or discarded page metadata. Remote fields named `trust`, `authority`, `action`,
`server`, or `identity` remain nested data and cannot override the outer values.
Literal JSON keys resembling a decoder's private representation remain keys.

The complete output bound is 16 MiB plus 512 KiB of envelope allowance plus
the 29-byte `ToolOutput` wrapper, with 262,144 source value nodes plus 64
envelope nodes and at most 64 container levels. Before copying any raw JSON
into a `Value` tree, projection checks aggregate source bytes, the compact
escaped trusted envelope, added nesting, and aggregate value-node counts.
Admitted valid raw JSON conservatively bounds its compact representation;
numeric lexemes are preserved, not converted through floating point. The
existing counting serializer verifies the completed projection again.
Projection overflow fails explicitly; content is not silently truncated or
forced through the legacy injected adapter's 64 KiB result policy.

The actual shared archive retains oversized complete outputs losslessly while
providing bounded persisted references and previews. Execution checks caller
cancellation and the original runtime witness before handing off publication.
Once completed-result publication is polled, it owns completion; a later
cancellation does not erase the durable receipt or make replay permissible.
The prepared tool opts into core's completion-wins lifecycle for that reason.
A correlated protocol failure sets `is_error`; an unresolved input-required
response additionally stamps `stop: "McpInputRequired"` and requests explicit
`finish_turn`, without inventing consent or another exchange.

Malformed canonical input retains the shared input errors. An unavailable
runtime reports the redacted `mcp_features_native_unavailable` error; bounded
native operation or projection overflow reports
`mcp_features_native_projection_limit`. Neither is retryable. Runtime/caller
cancellation, including a retired result witness, retains `mcp_features_cancelled`.

## Injected reference-host seam

The native reference host always advertises `mcp_features`. Its ordinary
production composition supplies an inert empty authority that performs no MCP
I/O and fails unavailable. The explicit MCP composition seam accepts one
shared feature authority alongside the existing shared tool catalog. Host
composition stores the allocations but never polls or snapshots either one.

Transport, catalog admission and production runtime routing are separate native
owners; see [feature exchanges](mcp-feature-runtime.md) and
[runtime publication](mcp-runtime-publication.md). The injected seam alone
does not authorize those effects. CLI activation, authentication, continuation
and lifecycle composition must explicitly select their corresponding native
owners rather than interpreting returned content as authority.
