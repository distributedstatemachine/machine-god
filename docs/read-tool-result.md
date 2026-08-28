# Native `read_tool_result` tool

This page is the normative contract for the native, session-backed
`read_tool_result` tool. It lets a model page through a large prior
`ToolOutput` whose AI Gateway request projection supplied an opaque handle.
It is a range-only reader, not a general session-inspection or search API.

## Model-visible schema

The registered name is `read_tool_result`. Its advertised schema and direct
preparation accept only this strict object:

```json
{
  "handle": "tool-result-sha256-<64 lowercase hexadecimal digits>",
  "start_byte": 1,
  "byte_count": 8192
}
```

`handle` is required. `start_byte` and `byte_count` are optional positive JSON
integers; they default to `1` and 8,192. Unknown fields, non-integers, negative
values, zero, floating-point values, and handles outside the exact syntax are
rejected. `byte_count` is at least 4 and at most 16,384 bytes. `start_byte` is
1-based and may be at most 65,537, matching one past the current engine's
64 KiB serialized-result source ceiling.

The tool does not accept `query`, regular expressions, result IDs, call IDs,
session IDs, incarnation IDs, paths, or archive locations.

## Handle and session scope

A handle is an opaque content-bound token. Callers must preserve it exactly and
must not infer identity or storage location from it. The strict external syntax
is the fixed `tool-result-sha256-` prefix followed by exactly 64 lowercase
hexadecimal digits. A handle binds all of these values:

- the session ID;
- that session's incarnation ID;
- the original tool-call ID; and
- the exact compact JSON serialization of the complete `ToolOutput`.

Execution loads only `ToolContext.session_id` through the injected
`SessionStore`, requires the loaded record's incarnation to equal
`ToolContext.session_incarnation_id`, and searches bounded prior tool-result
blocks in that record. It neither trusts nor accepts scope fields supplied by
the model. A handle from another session or incarnation has the same fixed
not-found result as an unknown handle. Resetting a session creates a new
incarnation, so every handle from the prior incarnation becomes unavailable.
Changing the retained result bytes also prevents a stale handle from matching.

The durable `SessionRecord` is the only result source. The tool does not create
an index, cache, sidecar, external archive, file, or database record. A result
that is no longer present in the current durable record cannot be read through
its old handle.

## Range result

Before slicing, the matched complete `ToolOutput` is serialized as compact JSON
UTF-8. `start_byte` must be an exact UTF-8 boundary or exactly
`total_bytes + 1`; an offset inside a code point or beyond one-past-EOF is
invalid. The candidate end is reduced to the nearest preceding UTF-8 boundary,
so `serialized_tool_output` is always valid UTF-8 and never exceeds
`byte_count`. A non-EOF page contains at least one Unicode scalar because the
minimum page request is four bytes.

Success has exactly these fields:

```json
{
  "handle": "tool-result-sha256-<64 lowercase hexadecimal digits>",
  "start_byte": 1,
  "end_byte": 8192,
  "total_bytes": 32768,
  "serialized_tool_output": "<UTF-8 range of compact ToolOutput JSON>",
  "has_more": true
}
```

`end_byte` is inclusive. When `start_byte == total_bytes + 1`, the returned
string is empty, `end_byte == total_bytes`, and `has_more` is `false`.
Concatenating successive contiguous pages reconstructs the exact compact JSON
source bytes; an individual page is not promised to be complete JSON.

## Projection boundary

AI Gateway projection is conditional on the same request advertising
`read_tool_result`. A complete compact `ToolOutput` of exactly 16 KiB remains
inline. A larger result through the reader's inclusive 64 KiB source ceiling is
replaced on that provider request by a projection envelope containing its
opaque handle, complete source length, error disposition, reader name, and a
preview of at most 4 KiB of source UTF-8. A source above that ceiling remains
complete instead of receiving an unserviceable handle; it must fit every
ordinary Gateway request budget. The separate wire budget bounds the selected
full or envelope text, while the final request-body limit bounds the fully
escaped outer request. The durable transcript remains complete and unchanged.

A result produced by `read_tool_result` is never projected again, even when
JSON escaping makes its complete wire form exceed the 16 KiB projection
threshold. It remains subject to the same source, wire, and final request-body
limits, preventing nested handles while preserving bounded requests.

Source accounting and projected-wire accounting are independent. A result must
first fit the source budget; projection cannot make an otherwise inadmissible
source valid. The selected full or projected representation then consumes the
wire budget, and the complete outer request remains subject to its serialized
body limit. The exact Gateway envelope is specified in
[`ai-gateway.md`](ai-gateway.md#conditional-large-result-projection).

## Limits and lifecycle

Default `ReadToolResultLimits` and fixed reader ceilings enforce these
independent inclusive bounds:

| Resource | Default maximum |
| --- | ---: |
| Incoming compact arguments | 512 bytes |
| Simultaneous active reads | 2 |
| Hard configurable active-read limit | 8 |
| Transcript messages traversed | 4,096 |
| Content blocks traversed | 65,536 |
| Aggregate stored JSON nodes | 65,536 |
| Stored JSON container depth | 64 |
| Prior tool-result blocks scanned | 4,096 |
| One compact prior tool result | 64 KiB |
| Aggregate compact result bytes scanned | 8 MiB |
| Default returned page | 8 KiB |
| One returned page | 16 KiB |
| Handle start offset | 65,537 |

Construction with explicit limits rejects zero, internally inconsistent, or
hard-limit-exceeding values. Preparation measures incoming JSON with the same
iterative compact encoder used for stored results, checks depth and node bounds,
and owns even rejected arguments through iterative destruction. It is
synchronous, nonblocking, effect-free, and does not load a session.

`execute` returns an inert future. First poll checks cancellation and acquires
active-read capacity without waiting. The tool performs at most one store load,
immediately guards the returned record for iterative destruction, then performs
one cancellation-aware prepass across the bounded record to validate message,
content-block, JSON-depth, and aggregate-node limits and find the newest current
assistant-call boundary. It scans only messages preceding that boundary,
newest first, so current sibling results and placeholders do not consume
prior-result limits. Every candidate has its own inclusive 64 KiB compact
source ceiling in addition to the aggregate scan-byte budget. Compact
serialization uses one reused, geometrically grown fallible buffer and checks
cancellation while scanning string and key bytes in chunks of at most 1 KiB
before fixed-digest comparison and UTF-8 range selection. The tool spawns no
task or thread and performs no retry. Drop cancels ownership of the store future
and releases capacity; a conforming injected store keeps its effects owned by
that future or completes its own cleanup on drop.

## Authority and observations

Preparation uses `PreparedToolCall::without_authority`. Core creates no
permission request, emits no `PermissionRequested` or `PermissionResolved`
event, and does not invoke the permission handler. The tool emits ordinary
tool lifecycle events through core; it adds no policy or authority event.

The injected `SessionStore` is explicit host authority outside permission
policy. The native reference host shares the exact `Arc<FileSessionStore>`
allocation, erased as `Arc<dyn SessionStore>`, with the engine and this reader.
The tool has no ambient filesystem, environment, process, terminal, network,
clock, entropy, or runtime authority.

Successful page text is intentionally model-visible and becomes an ordinary
durable tool result. Handles and output are structural but not secret-bearing
capabilities: scope validation prevents cross-session use, while repository and
host access controls remain responsible for the underlying session data.

## Errors and cancellation

Preparation and direct execution return this complete fixed `ToolError`
taxonomy. `Display` is always `<code>: <message>`.

| `ToolErrorKind` | `code` | `message` | `retryable` |
| --- | --- | --- | --- |
| `InvalidInput` | `read_tool_result_invalid_arguments` | `read_tool_result arguments are invalid` | `false` |
| `InvalidInput` | `read_tool_result_resource_limit` | `read_tool_result resource limit exceeded` | `false` |
| `Unavailable` | `read_tool_result_not_found` | `tool result is unavailable` | `false` |
| `Unavailable` | `read_tool_result_unavailable` | `tool result store is unavailable` | injected store value |
| `Unavailable` | `read_tool_result_busy` | `read_tool_result is busy` | `true` |
| `Cancelled` | `read_tool_result_cancelled` | `read_tool_result was cancelled` | `false` |

Missing records, wrong incarnations, unknown handles, and handles whose prior
result is outside the bounded scan all collapse to `read_tool_result_not_found`.
An injected record exceeding the fixed JSON depth or aggregate-node ceiling is
`read_tool_result_resource_limit` and is destroyed iteratively.
An argument exceeding its compact-byte, JSON-depth, or node ceiling is also
`read_tool_result_resource_limit`; malformed bounded arguments are
`read_tool_result_invalid_arguments`. Cancellation observed during direct
argument measurement wins over either classification, and owned arguments are
destroyed iteratively even when preparation rejects them or an execution future
is never polled.
A store failure preserves only `SessionStoreError.retryable`; its kind, code,
message, debug detail, session identity, and persistence diagnostics are
discarded. Public error and debug forms do not expose session IDs,
incarnations, handles, result content, store paths, or injected diagnostics.

Cancellation is checked before store load, while the load future is pending,
after it resolves, during bounded traversal and serialization, and before
success. Cancellation observable at a return boundary wins over success or
another error. Direct execution reports the fixed cancelled error; engine-owned
turn cancellation retains core's normal cancelled-turn precedence.

## Intentional exclusions

This slice does not implement `query` search, server-side filtering, external
result archives, non-session result lookup, or a new CLI surface. It does not
increase the engine's current 64 KiB per-result source ceiling. It makes no
full-fx equivalence, protocol compatibility, or performance claim.
