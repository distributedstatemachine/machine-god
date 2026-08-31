# Injected `mcp_features`

`mcp_features` exposes bounded MCP resources, resource templates, prompts, and
argument completion through one explicitly injected native authority. It is an
ordinary provider-neutral engine tool: core owns preparation, cancellation,
events, result persistence, and the following model round, while the native
authority owns admitted MCP feature data and any later transport implementation.

This slice implements the complete pinned seven-action tool surface without
adding production MCP discovery, transport, authentication, or connection
management.

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
the empty `arguments`, `value`, or `context` defaults where applicable, and
passes only the canonical object to execution. For pinned malformed-input
compatibility, known scalar fields belonging to another action are ignored and
removed; `arguments` remains legal only for `prompt_get`, and `context` remains
legal only for completion actions.

The fixed input limits are:

- configured server: 1-128 ASCII bytes;
- resource URI or URI template: 1-65,536 UTF-8 bytes;
- prompt identity and completion argument: 1-256 UTF-8 bytes each;
- completion value: at most 4,096 UTF-8 bytes;
- prompt arguments: string values and at most 64 KiB when compactly encoded;
- completion context: at most 128 string-valued entries and 128 KiB of
  aggregate key-plus-value bytes; and
- complete compact canonical arguments: at most 64 KiB, which is the effective
  upper bound when the individual limits could otherwise combine to more.

Preparation uses `PreparedToolCall::without_authority`. This is not ambient
network permission: it is a trusted assertion that execution can use only the
separately injected feature authority described below.

## Injected authority and stable identities

`McpFeatureAuthority` is the sole host-interaction boundary. It receives an
owned typed `McpFeatureRequest` and a cancellation token. Constructors and
preparation never invoke it, and calling `execute` creates no authority work
until the returned future is first polled.

An authority implementation must treat the request's server and identity as
exact stable bytes. It must not trim, case-fold, Unicode-normalize, prefix
match, select by displayed index, choose among collisions, or fall back to a
different server. Before any underlying external effect it must verify that
the server, action, and identity are admitted by one immutable live view. It
must revalidate the same authority and catalog generation immediately before
returning a result. A changed or revoked view fails closed rather than
publishing stale data.

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

All resource, prompt, annotation, metadata, message, and completion content is
untrusted external data. It remains data inside the result envelope and cannot
override user instructions, authorize an action, install a tool, or mutate
engine state.

Authority payloads have an object root, at most 64 container levels, at most
4,096 JSON nodes, and no more than 64 KiB when compactly serialized. The
complete serialized `ToolOutput`, including the common envelope, is also capped
at 64 KiB. Bounds are checked with a counting serializer; the tool does not
serialize an unbounded intermediate merely to learn its size.

An input-required authority result preserves the pinned terminal error payload:

```json
{"error":"McpInputRequired"}
```

It is returned with `is_error: true` and conveys no approval. This slice does
not implement elicitation or continuation.

## Lifecycle and failures

Execution checks cancellation before authority acquisition, races the injected
future against cancellation, and checks cancellation again before validating
and publishing its result. Cancellation independently wakes a non-cooperative
authority future, wins over a ready success or error observed in the same poll,
and drops the losing future. Dropping execution releases all call-local request,
payload, and authority-future state without a task, thread, timer, cache,
watcher, or durable MCP record.

Core then applies its ordinary tool lifecycle: durable placeholder, started
event, cancellable execution, result-size validation, durable replacement,
finished event, and model visibility on the following round. A persistence
failure never causes automatic replay of a completed authority operation.

Fixed redacted failures distinguish invalid arguments, invalid authority
payload, not-found/admission failure, unavailable authority, resource limits,
and cancellation. No failure contains a server, URI, prompt, argument, value,
content, provider diagnostic, credential, or generation witness.

## Reference host and remaining boundary

The native reference host always advertises `mcp_features`. Its ordinary
production composition supplies an inert empty authority that performs no MCP
I/O and fails unavailable. The explicit MCP composition seam accepts one
shared feature authority alongside the existing shared tool catalog. Host
composition stores the allocations but never polls or snapshots either one.

Later M05 slices own stdio/HTTP transport discovery, JSON-RPC framing,
pagination, OAuth and credential rotation, cache TTL/stale behavior,
subscriptions, reconnection, production connection management, resource
template expansion, elicitation/continuation, `/mcp`, ACP/TUI integration,
subagent propagation, and MCP-specific persistence. None of those concerns is
implemented or implicitly authorized here.
