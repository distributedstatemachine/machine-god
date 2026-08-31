# Injected `mcp_select_tool`

`mcp_select_tool` exact-selects one admitted executable dynamic tool from one
bounded point-in-time MCP catalog snapshot. A successful selection installs a
typed, executable, turn-local registration in core. Its schema is advertised
on the next model step in the same turn and the exact registered implementation
can then receive an ordinary tool call.

This is the pinned-fx timing boundary: a selected tool is never callable in the
same provider response as selection, never becomes engine-global state, and
never survives into another turn or session.

## Input and preparation

The input object has one required `name` string:

```json
{"name":"mcp_github_create_issue"}
```

- `name` uses the core `ToolName` grammar: 1-128 ASCII bytes containing only
  letters, digits, `-`, `_`, `.`, or `:`.
- The already-deconflicted advertised name is consumed verbatim. There is no
  prefix matching, sanitization, case folding, alias expansion, or guessing.
- As in pinned fx, additional object fields are ignored during preparation.
  Missing/non-string names, malformed names, and a mismatched registered
  selector name are invalid.
- Preparation canonicalizes the execution arguments to `{"name":NAME}`, caps
  the compact form at 512 bytes, and uses
  `PreparedToolCall::without_authority`. It acquires no catalog and exercises
  no host authority.

The selector requires the typed `execute_for_turn` orchestration path. A raw
`execute` call cannot install a next-round registration and therefore fails
closed without acquiring the catalog or claiming success. Engine calls always
pass through preparation and `execute_for_turn`.

## Executable catalog boundary

Search and selection share the exact injected `Arc<dyn McpToolCatalog>`.
Discovery, visibility, naming, collision resolution, policy admission, schema
acquisition, and executable routing happen before a ready immutable snapshot
is published. Selection acquires that snapshot once and does not refresh or
re-resolve the selected entry.

An entry may remain metadata-only for search. It is selectable only after the
host attaches one `Tool` whose captured `ToolSpec.name` exactly equals the
admitted dynamic name. The input schema must have an object root, at most 64
container levels and 4,096 JSON nodes, and the complete serialized selected
`ToolSpec` must fit 64 KiB. Its retained bytes count against the existing 8 MiB
snapshot budget. Name/schema/executor therefore travel as one immutable
descriptor; private search text can never be interpreted as an executable
schema.

A ready snapshot is scanned for case-sensitive byte equality across at most
1,024 entries. Cancellation is checked between entries. A metadata-only match
and an absent or hidden name use the same fixed not-found failure.

A discovering snapshot returns a bounded retry result and installs nothing:

```json
{
  "name": "mcp_github_create_issue",
  "selected": false,
  "state": "discovering",
  "retryable": true,
  "schema_advertised": false
}
```

## Result and next-round activation

After exact executable resolution, the successful model-visible result matches
the pinned behavior:

```text
Selected dynamic MCP tool `mcp_github_create_issue`. Its executable schema will be available on the next model step; call `mcp_github_create_issue` with arguments matching the selected schema.
```

Names use the shared model-context scalar encoder. No schema, callback,
credential, private search text, catalog diagnostic, or transport state appears
in the result. The complete compact serialized `ToolOutput` is capped at 4 KiB.

The selector returns the visible output and an opaque `TurnToolRegistration`
through `Tool::execute_for_turn`. Core validates the candidate against the
complete static-plus-turn-local catalog:

- it cannot collide with a static tool;
- exact reselection of the same captured registration allocation is
  idempotent, while a same-name registration from any different capture fails
  closed even when its visible specification is equal;
- aggregate schema depth, JSON nodes, and serialized catalog bytes remain
  within `EngineLimits`; and
- an error output cannot install a registration.

Core validates before persistence, replaces the selector's durable placeholder,
and only then inserts the already-validated registration without another await
or cancellation point. The following provider request merges its exact schema
into `ModelRequest.tools`; subsequent call admission and dispatch resolve the
same captured implementation. Distinct registrations remain appended in their
successful selection order; exact idempotent reselection does not move an
existing entry. Dynamic execution uses the ordinary preparation, permission,
cancellation, event, result-bound, and persistence pipeline. The selection
itself grants no execution permission.

The overlay lives only inside `run_turn_inner`. Completion, failure, or
cancellation drops it. Another prompt on the same session and every other
session begin with the static engine catalog.

## Lifecycle and errors

Snapshot acquisition begins only when the execution future is polled.
Pre-cancellation prevents acquisition. Cancellation independently wakes a
pending catalog future and wins over ready/error results observed in the same
poll. Dropping a pending selection releases its arguments, snapshot future,
and immutable descriptor without a detached task, thread, timer, watcher, or
cache.

Tool failures use fixed redacted codes:

| Kind | Code | Retryable |
| --- | --- | --- |
| `InvalidInput` | `mcp_select_tool_invalid_arguments` | no |
| `InvalidInput` | `mcp_select_tool_not_found` | no |
| `InvalidInput` | `mcp_select_tool_resource_limit` | no |
| `Unavailable` | `mcp_select_tool_unavailable` | yes |
| `Unavailable` | `mcp_select_tool_turn_orchestration_required` | no |
| `Cancelled` | `mcp_select_tool_cancelled` | no |

Errors and debug forms never include requested names, catalog metadata, private
search text, schemas, provider data, credentials, or implementation diagnostics.

## Remaining MCP boundary

This slice supplies the compatible schema-advertisement and executable-dispatch
seam for explicitly injected tools. Later M05 work owns production MCP protocol
discovery/transports, authentication, catalog generation and revocation policy,
raw-name routing, resources and prompts, and `mcp_features`.
