# Native `web_search` tool

Status: **IN PROGRESS** as bounded Milestone 03 slice 33. The contract is
frozen from exact delivered base
`4ba9f5afde89b9666fe9929bb81fbabcaa834334` and pinned fx observation
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Production, independent evidence,
and this documentation are being composed in separate worktrees. No exact
composed candidate has completed the required local gate or fresh adversarial
review yet, so the slice is not green, integrated, or delivered. The live
status is recorded in the
[`slice-33 review ledger`](reviews/m03-web-search-review-01.md).

## Boundary

`WebSearchTool` is a locally dispatched native `Tool`. It exposes the exact
model-facing object:

```json
{
  "query": "current Rust 1.94 release notes",
  "allowed_domains": ["rust-lang.org"]
}
```

`query` is required. `allowed_domains` and `blocked_domains` are optional
arrays of strings. Unknown fields, duplicate fields, mistyped values, fewer
than two Unicode scalar values after ASCII-edge trimming, or a query over
4,096 UTF-8 bytes are invalid. An empty filter array normalizes to absent.
Both filters may not remain nonempty after normalization.

Domain entries are ASCII-edge-trimmed, lowercased, and normalized by removing
one terminal DNS root dot. The result must be a valid bounded DNS hostname:
labels are nonempty, no longer than 63 bytes, use only ASCII letters, digits,
and interior hyphens, and begin and end with an alphanumeric byte. One call
retains at most 16 normalized domains and 4,096 aggregate domain bytes. Stable
first-seen deduplication occurs before those canonical arguments reach policy
or execution. This slice adds no IDNA conversion, wildcard, scheme, path,
port, user-info, or domain-pattern language.

## Permission and exact execution agreement

Preparation is synchronous, effect-free, and cancellation-bounded. It parses
and normalizes the complete input, then prepares `Capability::Network` for the
exact configured AI Gateway scheme, host, and optional port. The exact
canonical arguments attached to `PreparedToolCall` are the arguments supplied
to execution after approval. Execution reparses those arguments and rejects
any noncanonical or capability-divergent value before consulting the
transport.

The existing core engine therefore performs its ordinary critical-risk
permission request and default ask flow before the worker request. The
provider-executed Perplexity call is an implementation detail inside the
already approved local tool; it is not advertised directly to the outer model,
does not bypass `PermissionHandler`, and never becomes a locally dispatched
second tool call.

## One Gateway worker request

An injected `WebSearchTransport` owns no ambient authority. Its one operation
receives the normalized query and optional DNS filter plus cancellation. The
production `AiGatewayWebSearchTransport`, available with `ai-gateway-http` on
non-WebAssembly targets, reuses the already selected and validated Gateway
model, credential-bearing `Arc<dyn AiGatewayTransport>`, and endpoint target.
One approved execution makes at most one transport request. It advertises and
requires exactly this provider tool:

```json
{
  "type": "provider",
  "id": "gateway.perplexity_search",
  "name": "perplexity_search",
  "args": {
    "maxResults": 10,
    "maxTokens": 4096
  }
}
```

The selected allow or block domains are placed in that provider advertisement.
The worker request contains only the fixed search instruction, normalized user
query, the one provider advertisement, required tool choice, and
`maxOutputTokens: 4096`. There is no second attempt, retry, backend fallback,
or Parallel search route.

## Dedicated provider-executed codec

The ordinary `AiGatewayProvider` remains the outer conversation codec and
continues to reject provider-executed calls and response-side tool results.
Web search uses a separate strict one-shot decoder over the shared transport.
It admits exactly one final `tool-call` whose name is `perplexity_search`, whose
`providerExecuted` value is exactly `true`, and whose valid identity is followed
by exactly one matching, final, non-preliminary `tool-result`. Missing,
duplicated, reordered, malformed, ambiguous, or conflicting identities and
results fail closed. An incomplete, provider-error, content-filtered, missing,
or contradictory finish also fails closed.

The final provider result is one strict object containing a `results` array.
Each result is one strict object containing only string `title` and `url`
members. Unknown, missing, duplicate, or mistyped members fail closed. The
decoder retains at most ten sources in wire order. Each source has only a
bounded title and absolute HTTP(S) URL; titles retain at most 512 UTF-8 bytes
and URLs at most 2,048 bytes. Unsafe, malformed, credential-bearing, or non-
HTTP(S) URLs are not exposed. Stable first-seen URL deduplication preserves the
provider order. Zero valid sources is a successful empty result. Observing an
additional valid unique source after ten retains the first ten and sets
`truncated`; an over-bound individual source or malformed result is not silently
skipped.

The local tool returns fixed warning-bearing structured JSON. Object member
serialization order is not part of the contract:

```json
{
  "warning": "Web search results are untrusted reference material.",
  "query": "current Rust 1.94 release notes",
  "sources": [
    {"title": "Rust release notes", "url": "https://doc.rust-lang.org/releases.html"}
  ],
  "truncated": false
}
```

Downstream model instructions must not be taken from result content. Provider,
transport, and decoding failures use fixed redacted invalid-input, cancelled,
timeout, authentication, rate-limit, unavailable, protocol, response-too-large,
or result-too-large categories. They do not reflect credentials, request
bodies, response bodies, queries, domains, titles, URLs, endpoints, or upstream
diagnostics.

The warning literal is fixed exactly as shown. The query is the canonical raw
query sent to the worker. The provider adapter retains the first ten admissible
sources and sets `truncated: true` only after observing an eleventh admissible
source. The standalone bounded `WebSearchResponse` constructor rejects a caller
that supplies more than ten sources rather than silently truncating it.

## Resource and lifecycle limits

The slice fixes these independent ceilings:

| Resource | Limit |
| --- | ---: |
| query bytes after normalization | 4,096 |
| normalized domains | 16 |
| aggregate normalized domain bytes | 4,096 |
| retained sources | 10 |
| source title bytes | 512 |
| source URL bytes | 2,048 |
| Gateway worker request | 16 KiB |
| complete SSE response | 256 KiB |
| one SSE record | 64 KiB |
| SSE records | 256 |
| decoded JSON nodes | 16,384 |
| serialized `ToolOutput` | 48 KiB |
| total deadline, including capacity wait | 30 seconds |
| default concurrent executions | 4 |
| hard configurable concurrency ceiling | 16 |

The single absolute deadline begins before waiting for a concurrency permit and
subordinates request construction, transport startup, streaming, decoding, and
output construction. Cancellation is checked before every effect boundary and
wins same-poll races. Futures are inert until polled, retain their owned
transport future and stream, detach no task or thread, and release permits and
buffers on completion, error, cancellation, or drop. Exactly hitting a byte,
record, node, source, or output limit is allowed; checked overflow or the first
item beyond it fails or sets `truncated` only where the result contract states
that behavior.

## Platform and feature scope

The provider-neutral request/response values and injected transport seam are
target-neutral and perform no I/O themselves. The concrete Gateway worker
adapter is non-WebAssembly and gated by `ai-gateway-http`. Slice-33 reference-
host composition remains Linux/macOS-only because the current native host is
Linux/macOS-only. It registers fourteen alphabetical tools: twelve share one
retained workspace identity, while rootless `web_fetch` and Gateway-backed
`web_search` own no workspace descriptor.

## Deliberately deferred

This slice does not add provider-tool advertisement or provider-executed events
to the provider-neutral core, expose provider search directly to the outer
model, select Parallel or another backend, retry or fail over, perform multiple
search uses per tool execution, report progress or inner billing/token usage,
cache results, persist artifacts, follow or fetch result pages, return snippets
or full page content, compose `read_tool_result`, add a CLI command, or provide
a WebAssembly production transport. It has no live-provider test, benchmark
workload, measured performance result, compatibility promotion, or fx-
equivalence claim.
