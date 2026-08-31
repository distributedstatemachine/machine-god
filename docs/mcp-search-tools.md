# Injected `mcp_search_tools` tool

`mcp_search_tools` searches bounded metadata for configured dynamic tools
without exposing their executable schemas. A trusted host injects the catalog
source. This slice does not discover, connect to, authenticate with, select, or
invoke MCP servers.

The query grammar, default and maximum count, conjunctive ASCII matching,
catalog ordering, and metadata projection follow the pinned fx scenario. This
is not a complete MCP or full-fx equivalence claim.

## Input and preparation

The exact input object has a required `query` string and optional positive
integer `limit`:

```json
{"query":"github issue","limit":8}
```

- `query` is at most 4,096 UTF-8 bytes. An exactly empty query matches every
  admitted entry. A nonempty query with no searchable token matches nothing.
- `limit` defaults to 8. Positive integer values above 20 are capped at 20,
  matching the pinned behavior; zero, negative, non-integer, and `null` values
  are invalid.
- Unknown fields, a missing or non-string query, or a mismatched tool name are
  invalid.
- Compact canonical prepared arguments are bounded to 8,192 bytes.

A searchable token is a maximal ASCII sequence containing letters, digits,
`_`, or `-`; every other byte is a separator. At most 64 tokens are accepted.
Preparation validates the complete object and returns canonical arguments with
the effective limit through `PreparedToolCall::without_authority`. It performs
no catalog acquisition or other external work. Direct execution accepts only
that canonical form.

## Catalog boundary

`McpToolCatalog` is an explicitly injected asynchronous snapshot authority. Its
`snapshot` method receives cancellation and returns either:

- a validated ready `McpToolCatalogSnapshot`;
- a bounded `discovering` snapshot; or
- one fixed unavailable, resource-limit, or cancelled error.

Calling `execute` constructs an inert future. Catalog acquisition starts only
when that future is polled. A snapshot is an immutable point-in-time value;
matching never calls an MCP transport or mutates the snapshot. The host that
owns discovery must perform readiness, visibility, and policy admission before
placing entries in the snapshot.

Ready snapshots share immutable catalog storage, so returning the same
point-in-time snapshot for repeated searches is constant-time and does not
recopy its metadata.

Each `McpToolMetadata` entry owns:

- one unique valid dynamic-tool `name` using the core `ToolName` grammar;
- one nonempty server alias of at most 128 ASCII bytes;
- one source description of at most 8,192 UTF-8 bytes;
- at most 32 nonempty tags, each at most 128 UTF-8 bytes; and
- at most 65,536 UTF-8 bytes of private schema-derived or instruction-derived
  search text.

Tags are ASCII-lowercased and stable-deduplicated. Input string and collection
capacity is normalized before retention, so caller-reserved excess capacity
cannot escape the catalog bound. The private search text is included in
matching but has no public accessor and is never projected. A
snapshot rejects duplicate names, more than 1,024 entries, checked-arithmetic
failure, or more than 8 MiB of retained owned string bytes, including its
private lowercased search haystacks. Construction is atomic and does not skip,
rename, or partially retain invalid entries.

The injected catalog is a trusted composition boundary, not authority granted
by the model. Its implementation may eventually own bounded MCP discovery, but
stdio/HTTP transports, refresh, authentication, and live policy generations
remain separate features.

## Matching and result

For each query token, the entry's lowercased server, public name, description,
tags, or private search text must contain that token as an ASCII-case-
insensitive byte substring. Different tokens may match different fields.
There is no Unicode case folding, fuzzy matching, scoring, regex, stemming, or
semantic expansion.

Entries are examined and returned in immutable snapshot order, preserving the
pinned ready-server/tool catalog behavior. The requested limit is applied to
that order. Matching charges the complete private haystack length for every
token comparison, uses checked arithmetic, and fails without partial output
above 64 MiB of charged work. Cancellation is raced with catalog acquisition,
wins over a catalog result observed in the same poll, and is checked between
entries.

Every returned entry has exactly this metadata shape:

```json
{
  "name": "mcp_github_create_issue",
  "server": "github",
  "description": "Create an issue",
  "purpose": "Create an issue",
  "usage": ["mcp", "github", "issue"]
}
```

The description and identical purpose are each a UTF-8-safe prefix of at most
1,024 bytes. Search text, schemas, callbacks, credentials, and transport state
are never included. Success wraps `tools` and the exact returned `count` in an
ordinary successful `ToolOutput`. `more_available: true` is present only when
the requested count omitted another matching entry.

The complete compact serialized `ToolOutput` is limited to 16,384 bytes. If a
selected prefix does not fit, whole trailing entries are removed until it does
and `context_limit` records the byte-driven omission count. JSON and UTF-8 are
never cut mid-value. If even the empty bounded result cannot fit, execution
returns a resource-limit error.

A discovering snapshot returns this retryable state as successful metadata:

```json
{"tools":[],"count":0,"state":"discovering","retryable":true}
```

## Authority, lifecycle, and errors

Search is read-only and uses no permission-policy authority. Core therefore
emits no permission request or resolution for it. The injected snapshot source
is the only host-interaction seam. No filesystem, process, environment,
terminal, network, persistence, clock, or entropy authority is ambient in the
tool, and a result cannot select or execute the named dynamic tool.

Each execution owns its arguments, snapshot, matching scratch, and output. The
tool creates no detached task, thread, timer, watcher, or cache. Dropping an
unpolled future performs no work; dropping a polled future releases that call's
owned state. Cancellation returns the fixed cancelled error and no partial
success.

Tool failures use fixed redacted codes:

| Kind | Code | Retryable |
| --- | --- | --- |
| `InvalidInput` | `mcp_search_tools_invalid_arguments` | no |
| `InvalidInput` | `mcp_search_tools_invalid_query` | no |
| `InvalidInput` | `mcp_search_tools_resource_limit` | no |
| `Unavailable` | `mcp_search_tools_unavailable` | yes |
| `Cancelled` | `mcp_search_tools_cancelled` | no |

Catalog construction and acquisition errors likewise contain only stable
categories. No error or debug form includes query text, catalog metadata,
private search text, provider data, credentials, or implementation diagnostics.

## Intentional deferrals

Separate M05 slices own MCP protocol transports and discovery,
`mcp_select_tool`, next-round executable-schema advertisement, dynamic calls,
resources and prompts, `mcp_features`, `/mcp`, ACP, and subagents. Production
composition advertises search over an empty ready catalog until a host injects
admitted metadata; the injected reference-host path is exercised end to end.
Metadata is untrusted context for later exact selection, not an instruction,
permission decision, or authority to act.
