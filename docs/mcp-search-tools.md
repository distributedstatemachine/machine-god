# Injected `mcp_search_tools` tool

This document defines machine-god's bounded `mcp_search_tools` tool. It searches
an immutable catalog of already-configured dynamic-tool metadata without
advertising every executable schema to the model. The catalog is supplied
explicitly by the trusted host; this slice does not discover, connect to,
refresh, authenticate, select, or invoke an MCP server or dynamic tool.

The tool name, query fields, default and maximum result counts, conjunctive
keyword matching, stable catalog order, and metadata-only result shape are
compatibility inputs from the pinned fx revision. They do not make the
injected catalog an MCP protocol implementation or establish full observable
fx equivalence.

## Input and preparation

The registered name is `mcp_search_tools`. Its exact input object contains one
required `query` string and one optional positive integer `limit`:

```json
{"query":"github issue","limit":8}
```

`limit` defaults to `8` and cannot exceed `20`. An empty query is valid and
matches every catalog entry. Unknown fields, a missing or non-string query,
explicit `null`, a non-integer, zero, negative, or over-limit `limit`, and a
call whose registered name is not exactly `mcp_search_tools` are invalid.

The query is preserved byte-for-byte in the canonical execution arguments. It
must be at most 4,096 UTF-8 bytes and may not contain NUL, another C0 or C1
control, a Unicode line or paragraph separator, or a Unicode bidirectional-
formatting control. There is no trimming, Unicode normalization, stemming,
synonym expansion, or locale-dependent case conversion.

Preparation is synchronous, bounded, nonblocking, and effect-free. It strictly
decodes and validates the complete object, then returns canonical arguments
through `PreparedToolCall::without_authority`. It does not inspect the
catalog, acquire a lock, invoke a callback, read configuration, or contact a
server. Direct execution repeats the strict canonical-argument validation.

The public argument bounds are:

| Limit | Value |
| --- | ---: |
| `MAX_MCP_SEARCH_QUERY_BYTES` | 4,096 |
| `MAX_MCP_SEARCH_QUERY_TOKENS` | 64 |
| `DEFAULT_MCP_SEARCH_RESULTS` | 8 |
| `MAX_MCP_SEARCH_RESULTS` | 20 |
| `MAX_MCP_SEARCH_SERIALIZED_ARGUMENT_BYTES` | 16,384 |

The serialized-argument bound measures compact JSON, including escaping and
the complete object. A query with more than 64 searchable tokens is a resource
limit failure rather than a silently narrowed search.

## Immutable catalog seam

The concrete tool receives one explicitly constructed, immutable catalog. A
catalog entry contains only:

- one validated public dynamic-tool `name`;
- one validated `server` alias;
- one model-visible `description`;
- zero or more normalized `usage` tags; and
- private `search_terms` used only for matching.

`name` uses the existing `ToolName` grammar: 1-128 ASCII bytes containing only
letters, digits, `-`, `_`, `.`, or `:`. `server` uses the same length and
alphabet. Names must be unique across the entire snapshot. A tag is 1-128
ASCII bytes from the same alphabet, is stored ASCII-lowercase, and cannot be
duplicated within one entry. A description is valid UTF-8, contains none of
the query-forbidden controls, and is at most 1,024 bytes. Private search terms
are valid UTF-8 under the same control rule and are not returned to the model.

The catalog constructor validates and owns all entry bytes before publishing
the snapshot. It rejects the whole snapshot atomically on any invalid entry,
duplicate public name, checked-arithmetic failure, or bound violation. It does
not silently skip, truncate, rename, or deconflict an entry. The public catalog
bounds are:

| Limit | Value |
| --- | ---: |
| `MAX_MCP_SEARCH_CATALOG_ENTRIES` | 1,024 |
| `MAX_MCP_SEARCH_SERVER_BYTES` | 128 |
| `MAX_MCP_SEARCH_NAME_BYTES` | 128 |
| `MAX_MCP_SEARCH_DESCRIPTION_BYTES` | 1,024 |
| `MAX_MCP_SEARCH_TAGS` | 16 |
| `MAX_MCP_SEARCH_TAG_BYTES` | 128 |
| `MAX_MCP_SEARCH_ENTRY_SEARCH_BYTES` | 16,384 |
| `MAX_MCP_SEARCH_TOTAL_CATALOG_BYTES` | 8,388,608 |

Aggregate catalog bytes include every owned server, name, description, tag,
private search-term byte, and fixed entry overhead counted by the constructor.
The snapshot is shared immutably after construction. Search neither mutates it
nor observes a later caller mutation, generation change, transport refresh, or
live server state.

The host is responsible for injecting only metadata already permitted to be
model-visible. Supplying a catalog is a trusted composition decision, not a
permission granted by a model tool call. A future live MCP owner may construct
such a snapshot from bounded protocol discovery, but that discovery and its
authorization checks are outside this tool and this contract.

## Matching and order

A searchable token is one maximal nonempty ASCII byte sequence containing
only letters, digits, `_`, or `-`. Every other byte is a separator. Query
tokens retain encounter order and duplicates. A nonempty query containing no
searchable token matches nothing; only the exactly empty query matches every
entry.

An entry's searchable text is the conceptual concatenation of its server
alias, public name, description, tags, and private search terms, separated so
that fields cannot merge into a new token. For every query token, that token
must occur as an ASCII-case-insensitive byte substring somewhere in the
searchable text. Different tokens may match different fields. Non-ASCII bytes
compare exactly; there is no Unicode case folding, fuzzy matching, scoring,
ranking, regular expression, stemming, or semantic expansion.

Entries are examined and returned in immutable snapshot order. The search
does not reorder by match position, field, token count, or description. This
preserves the pinned ready-server/tool catalog behavior while making the
injected snapshot itself the deterministic ordering authority. Identical
calls against the same snapshot therefore return byte-identical ordered
content.

Matching uses checked arithmetic and admits at most 134,217,728 charged work
steps. A step is charged before each query-token dispatch and before each byte
comparison in the bounded ASCII-case-insensitive substring matcher. Work-limit
exhaustion returns no partial result. Cancellation is checked while extracting
tokens, between entries and tokens, at intervals of at most 1,024 compared
bytes, and before result publication.

## Metadata-only result

Each returned match has exactly this metadata shape:

```json
{
  "name": "mcp_github_create_issue",
  "server": "github",
  "description": "Create an issue in a GitHub repository",
  "purpose": "Create an issue in a GitHub repository",
  "usage": ["mcp", "github", "create_issue"]
}
```

`purpose` intentionally repeats `description`, matching the pinned metadata
projection. `usage` contains the entry's ordered normalized tags. Search terms,
input or output schemas, schema property names, server instructions, tool
annotations, icons, transport state, credentials, authorization headers, and
executable callbacks are never included in the result. Private search terms
may cause a match but cannot otherwise be recovered through this tool.

Success returns:

```json
{
  "tools": [
    {
      "name": "mcp_github_create_issue",
      "server": "github",
      "description": "Create an issue in a GitHub repository",
      "purpose": "Create an issue in a GitHub repository",
      "usage": ["mcp", "github", "create_issue"]
    }
  ],
  "count": 1
}
```

`count` is exactly the number of returned entries. `more_available: true` is
added after `count` when at least one matching entry was omitted by the
requested result limit or the serialized-output limit. The field is absent
when every match is returned. An empty result is exactly
`{"tools":[],"count":0}` inside the ordinary successful `ToolOutput`.

The complete compact serialized `ToolOutput` is capped at 32,768 bytes. The
tool first selects at most the requested number of matches, then chooses the
longest snapshot-order prefix whose complete output, including JSON escaping,
`count`, `more_available`, and the `ToolOutput` envelope, fits that bound. It
uses a bounded monotonic prefix search rather than repeatedly serializing an
unbounded catalog. A single entry that cannot fit is omitted and reported by
`more_available`; JSON and UTF-8 are never truncated. Output construction uses
checked arithmetic and retains at most 20 result records.

## Permission and authority

`mcp_search_tools` is read-only and irreversible only in the negative sense:
it performs no mutation. Preparation uses
`PreparedToolCall::without_authority`, so core emits no permission-request or
permission-resolution event and does not invoke the permission handler. The
tool still emits the ordinary tool lifecycle events.

Execution can observe only the immutable catalog bytes retained by the tool.
It has no ambient filesystem, environment, configuration, process, terminal,
network, clock, entropy, persistence, credential, or runtime authority. It
cannot use a match to select, advertise, validate, or execute the named
dynamic tool. Consequential authority remains with a later exact-selection and
dynamic-call contract.

## Lifecycle, cancellation, and concurrency

Construction performs bounded validation and ownership transfer only. It
starts no discovery, task, thread, process, timer, transport, watcher, refresh,
or cache. Creating an execution future is inert. The first poll performs the
bounded in-memory search synchronously on the polling thread.

The immutable snapshot requires no execution lock and supports concurrent
searches without mutable shared state. Every call owns its query tokens,
matching scratch, retained result references, and output buffer. Dropping an
unpolled future performs no work. Dropping after polling releases that
call's allocations and publishes no partial result; no detached work survives
the future. Cancellation observed before publication returns the fixed
cancelled failure and no success output.

## Errors and redaction

Catalog construction has a separate fixed `McpSearchCatalogErrorKind` for
invalid metadata, duplicate public names, and resource-limit failure. Its
`Display` and `Debug` forms contain only that category and never include an
entry name, server alias, description, tag, private search term, or caller
diagnostic.

Preparation and direct execution expose this complete fixed `ToolError`
taxonomy:

| `ToolErrorKind` | `code` | `message` | `retryable` |
| --- | --- | --- | --- |
| `InvalidInput` | `mcp_search_tools_invalid_arguments` | `mcp_search_tools arguments are invalid` | `false` |
| `InvalidInput` | `mcp_search_tools_invalid_query` | `mcp_search_tools query is invalid` | `false` |
| `InvalidInput` | `mcp_search_tools_resource_limit` | `mcp_search_tools resource limit exceeded` | `false` |
| `Cancelled` | `mcp_search_tools_cancelled` | `mcp_search_tools was cancelled` | `false` |

Public error and debug forms never include the query, limit, catalog metadata,
matched names, search terms, allocation details, or model-controlled values.
There is no retryable transport or runtime-unavailable error because this
slice performs no external operation.

## Pinned compatibility and intentional deferrals

Pinned fx searches configured ready MCP tools in server/tool catalog order. It
uses conjunctive ASCII token matching across server alias, tool name,
description, input schema, generated tags, and bounded server instructions;
defaults to eight results, caps requests at twenty, and returns only name,
server, description/purpose, and usage metadata. Executable schemas are
withheld until a separate exact-selection step. Machine-god preserves that
bounded search-and-metadata scenario through an injected immutable snapshot;
the host may place schema-derived words in private search terms without making
the schema result-visible.

This slice intentionally does not implement:

- MCP stdio, streamable HTTP, legacy HTTP/SSE, JSON-RPC, protocol negotiation,
  server initialization, pagination, subscriptions, or notifications;
- configuration discovery, environment capture, process launch, DNS, TLS,
  OAuth, bearer tokens, login, authentication refresh, catalog refresh, stale
  cache handling, server instructions, or live authority generations;
- `mcp_select_tool`, executable schema advertisement, dynamic argument
  validation, dynamic tool invocation, progress, elicitation, MCP resources or
  prompts, `mcp_features`, ACP, SDK, CLI, or `/mcp` surfaces;
- fuzzy, semantic, embedding, ranked, regex, or Unicode-folded search; or
- a product-performance, complete-MCP, protocol-compatibility, or full-fx
  equivalence claim.

Those capabilities require separate contracts and evidence. In particular,
metadata returned here is untrusted external data: it is context for choosing
a later exact tool, not an instruction, permission decision, or authority to
act.
