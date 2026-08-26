# Injected-transport AI Gateway provider

This page is the normative contract for the sixth bounded Milestone 03 slice.
It adds an executor-neutral `AiGatewayProvider` codec to
`machine-god-native`, behind an explicitly injected `AiGatewayTransport`. The
current CLI does not construct this generation provider or make generation
requests. Its delivered `models [--json]` command uses the separate
bounded catalog provider documented in [`models-cli.md`](models-cli.md).
The separate optional [`ai-gateway-http` transport](ai-gateway-http.md) is one
possible native injection; custom transports remain supported. The separate
[`native credential discovery`](ai-gateway-credentials.md) can
produce that transport's explicit bearer input without giving the codec ambient
authority.
The separate integrated [`native configuration schema v2`](configuration.md)
can declare this provider and a validated default model, but it does not
construct the codec, inject a transport, discover a credential, create a
runtime, or change the CLI.

The wire shape is deliberately scoped to the behavior needed from pinned
[`vercel-labs/fx` revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`](https://github.com/vercel-labs/fx/commit/b1774fbf6c7602b503026f96f6e960e946c692ef).
It is not a claim of compatibility with a current Vercel AI Gateway protocol,
full fx equivalence, or a measured performance improvement.

The in-progress slice-33 [`web_search` tool](web-search.md) does not broaden
this outer generation codec. Its private worker reuses an injected
`Arc<dyn AiGatewayTransport>` but owns a separate bounded request projection and
provider-executed response decoder in native. `AiGatewayProvider` continues to
reject `providerExecuted: true` and response-side `tool-result` events exactly
as documented below.

## Public boundary

`AiGatewayProvider` implements core's `ModelProvider`. `new` takes a default
model of 1–128 visible ASCII bytes (`0x21` through `0x7e`) and an
`Arc<dyn AiGatewayTransport>` with
`AiGatewayLimits::default()`; `with_limits` also takes explicit limits. Its
stable provider name is `AI_GATEWAY_PROVIDER_NAME` (`vercel_ai_gateway`). The
public wire constants are `AI_GATEWAY_PROTOCOL_VERSION` (`0.0.1`) and
`AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION` (`4`). Provider and request
debug representations reveal structure only; they do not reveal model input,
headers, bodies, response bytes, or transport-controlled errors.

The configuration slice publishes `AI_GATEWAY_DEFAULT_MODEL`
(`zai/glm-5.2`) and `AI_GATEWAY_MAX_MODEL_BYTES` (`128`). Configuration-file
models, provider defaults, and request overrides use the same validator:
1–128 bytes, each in visible ASCII `0x21` through `0x7e`. This sharing changes
no codec selection rule; a request override still wins over a constructed
provider's default.

The transport receives one owned `AiGatewayTransportRequest` containing the
encoded body and fixed request metadata, plus the turn's `CancellationToken`.
It returns an `AiGatewayByteStream`, whose chunks may split any UTF-8 code point,
JSON token, field, or record delimiter. For a valid request whose returned
future is polled through startup, the provider invokes the transport exactly
once. An unpolled, pre-cancelled, or invalid request invokes it zero times. The
provider never detaches a task or thread.

The injected host transport owns all effects and policy needed to deliver that
request: endpoint and URL selection, DNS, proxy behavior, HTTP method, TLS,
authentication and credentials, response-status validation, redirect policy,
timeouts, and any retry policy. A transport must return only the byte stream of
an accepted streaming response or a redacted `ProviderError`. This provider
does not inspect HTTP status or error bodies and does not retry.

The integrated native transport narrows those host choices to a pinned
production HTTPS endpoint, an explicitly injected bearer token, and a fixed
hardened HTTP policy. Its limits, status mapping, cancellation behavior and
loopback-only plaintext test endpoint are normative in
[`ai-gateway-http.md`](ai-gateway-http.md). These are transport guarantees, not
new codec responsibilities.

The request metadata contains these exact headers:

| Header | Value |
| --- | --- |
| `content-type` | `application/json` |
| `ai-gateway-protocol-version` | `0.0.1` |
| `ai-language-model-specification-version` | `4` |
| `ai-language-model-id` | selected model |
| `ai-language-model-streaming` | `true` |
| `x-session-id` | core session ID |
| `x-session-affinity` | the same core session ID |

Machine-god does not add authorization, team, endpoint, accept, referer, title,
or user-agent values. In particular, it does not impersonate fx's referer,
title, or user agent. A host may add transport-level metadata under its own
identity and security policy.

## Request projection

The selected model is `ModelRequest.options.model` when present and otherwise
the provider's default. The override has the same 1–128 visible-ASCII-byte
rule; spaces, controls, non-ASCII text, and longer values are invalid.
Temperature and inference metadata have no pinned wire projection and are
ignored rather than making an otherwise valid request fail. Metadata JSON is
still traversed under the same structural depth and node limits as other owned
request JSON before it is discarded iteratively; temperature has no JSON
structure to traverse.
Neither field is serialized. A present `max_output_tokens` becomes the sole
optional body field `maxOutputTokens`; zero is invalid.

The body has only `prompt`, `tools`, `toolChoice`, and the optional
`maxOutputTokens`. `toolChoice` is `{"type":"auto"}` when tools are present
and `{"type":"none"}` when the tool list is empty. Each `ToolSpec` is a
Gateway function tool with its validated name, description, and JSON Schema
under `inputSchema`:

```json
{
  "prompt": [],
  "tools": [
    {
      "type": "function",
      "name": "read_file",
      "description": "Read a file",
      "inputSchema": {"type": "object"}
    }
  ],
  "toolChoice": {"type": "auto"},
  "maxOutputTokens": 4096
}
```

The accepted provider-neutral transcript projection is intentionally narrow:

- each system or user message contains exactly one text block;
- an assistant message contains an optional single leading text block followed
  by one or more complete tool-call blocks, or just its single text block;
- each tool message contains exactly one tool-result block whose call ID
  resolves uniquely to calls in the immediately preceding assistant group; and
- a tool result uses that resolved call's tool name and serializes the complete
  `ToolOutput` as the Gateway text output value.

Assistant calls are emitted as `tool-call` content with `toolCallId`,
`toolName`, and JSON `input`. Tool results are emitted as `tool-result` content
with that same ID and name and an output of `{"type":"text","value":"..."}`.
An assistant call/result block must be complete: calls have unique IDs in their
message, and the following tool messages supply exactly one result for each
call without intervening roles. Those results may appear in any order, but an
orphan, duplicate, missing, or name-conflicting result is invalid history.
`ContentBlock::Json`, a role with an unsupported block kind, and otherwise
invalid role/content combinations are rejected before the transport is called.
Cheap message, tool, selected-model, and per-role content-count checks run while
the iterative request guard is still armed and before any JSON traversal.
Traversal then checks cancellation at every metadata value, tool, message,
content block, and JSON node.

## Streaming response

The decoder consumes newline-delimited records and recognizes the pinned
single-line data-stream shape. Each recognized record is exactly one line
beginning `data: ` followed by either one JSON object or `[DONE]`. An SSE-style
blank line may follow but is not required. Both LF and CRLF delimiters are
accepted. Blank lines, comment lines and non-data SSE fields are bounded and
ignored. A data line missing the exact `data: ` prefix is therefore not
interpreted as model data. Arbitrary transport chunk fragmentation is accepted,
but invalid UTF-8, malformed JSON, duplicate JSON fields, and malformed schemas
for supported event types fail closed. An unknown JSON event type is a bounded
no-op.

Supported JSON events are:

| Gateway event | Core event |
| --- | --- |
| `text-delta` with string `delta` | `ModelEvent::TextDelta` |
| `reasoning-delta` with string `delta` | `ModelEvent::ReasoningDelta` |
| complete `tool-call` | `ModelEvent::ToolCall` |
| valid `finish.usage` | `ModelEvent::Usage` before stop |
| valid `finish.finishReason.unified` | exactly one `ModelEvent::Stop` |

A tool call may carry its complete JSON `input` in the final `tool-call`, or it
may be reconstructed from one `tool-input-start`, zero or more matching
`tool-input-delta` records, one `tool-input-end`, and a matching final
`tool-call`. A complete input carried by the final event is authoritative; an
otherwise missing input uses the completed streamed value with the same ID. An
explicit input with the same final and provisional ID replaces the provisional
value, even if the provisional bytes end early or do not form valid JSON, while
the name must remain consistent. A finalized-ID tombstone ignores later bounded
delta/end records for that same provisional ID. If the final ID differs from a
provisional streamed ID, an explicit final name and
input reconcile it only when exactly one ended provisional input has that name
and a structurally equal JSON value. An ambiguous match, an identity/name
conflict, or unequal changed-ID input rejects the response. Ended streamed
inputs are parsed once and retain either their bounded parsed value or an
invalid marker. Valid values use a bounded canonical index that normalizes
signed floating zero, matching structural JSON equality without repeatedly
parsing or scanning every candidate. Invalid streamed JSON fails only if it is
needed as fallback or remains unresolved. Call IDs and names
must be nonempty and valid for core, final IDs must be unique, argument JSON
must be complete, and no more than the configured number of calls may be
accumulated. Only the validated final ID, name, and arguments are emitted to
core; start, delta, and end records never expose a partial call.

The decoder rejects unmatched, conflicting, duplicate, or unfinished tool input
state, and rejects late input for an unresolved state. Bounded delta/end records
that arrive after an authoritative exact-ID final are ignored through the
finalized tombstone. It also rejects provider-executed calls, `tool-result` response
events, provider error events, a unified `error` finish, and any additional JSON
record already received in the same chunk after finish. `[DONE]` emits nothing
and can terminate the stream only after one valid finish; EOF has the same
requirement. A stream that ends with `[DONE]` or EOF but no finish is a protocol
failure. Once the stop is yielded, later transport chunks are not polled.

Every transport item must contain a nonempty byte chunk. An empty successful
chunk is a protocol failure rather than an opportunity for a source to keep the
executor in a ready loop. One poll processes at most one nonempty chunk that
produces no event; it schedules another poll and yields so a ready source of
comments, ignored fields, unknown events, or partial records cannot monopolize
the executor. Event-producing chunks still expose events one at a time through
ordinary stream polls.

The unified finish mapping is exact:

| `finishReason.unified` | `StopReason` |
| --- | --- |
| `stop` | `Completed` |
| `tool-calls` | `ToolCalls` |
| `length` | `MaxOutputTokens` |
| `content-filter` | `ContentFilter` |
| `other` | `Other("other")` |
| `error` | provider protocol failure; no stop |

When present, usage requires nonnegative integer totals. `inputTokens.total`,
`outputTokens.total`, and `inputTokens.cacheRead` map to `input_tokens`,
`output_tokens`, and `cached_input_tokens`. Usage is emitted at most once,
immediately before the stop it accompanies. Missing usage emits no usage event;
malformed or overflowing usage fails the response rather than silently
changing a total.

## Independent resource limits

`AiGatewayLimits::default()` applies every limit independently:

| Resource | Default maximum |
| --- | ---: |
| encoded request body | 12 MiB (12,582,912 bytes) |
| one transport chunk | 1 MiB (1,048,576 bytes) |
| one decoded data record | 1 MiB (1,048,576 bytes) |
| undecoded receive buffer | 1 MiB (1,048,576 bytes) |
| total response bytes | 16 MiB (16,777,216 bytes) |
| response records | 8,192 |
| request messages | 4,096 |
| request tool specifications | 1,024 |
| simultaneously reconstructed streamed tool inputs | 64 |
| final response tool calls | 64 |
| historical or response arguments per tool call | 64 KiB (65,536 bytes) |
| aggregate request JSON nodes; each decoded response/argument JSON tree | 262,144 |

Crossing any bound fails before retaining the excess object. Custom limits do
not make one budget substitute for another. In particular, a small record does
not grant additional total-response capacity, and unused capacity in one tool
call does not enlarge another call's argument budget. The request-side node
budget aggregates inference metadata, tool schemas, message JSON blocks,
historical call arguments, and tool-result content. Each response record and
each reconstructed streamed argument parse receives a fresh node budget.
Serialized historical tool results also share one cumulative request-projection
budget while the body is built, so intermediate retained strings cannot each
claim the full request allowance.

## Errors and cancellation

Construction rejects an invalid default model or invalid limits through fixed
`InvalidModel` and `InvalidLimits` configuration categories. Request
validation, protocol parsing, resource exhaustion, and cancellation use fixed
redacted provider-error categories. Codec-generated errors and debug output
never reflect prompt text, tool arguments or results, model response bytes,
credentials, endpoint values, or malformed record contents. Request/history
errors are non-retryable invalid-request failures; decoder and bound failures
are non-retryable protocol failures; cancellation is non-retryable and
distinct. A trusted injected transport error is passed through unchanged so its
authentication, rate-limit, availability, transport, code, and retryability
classification survives; the transport must redact that error before returning
it.

Cancellation is checked before and after request encoding, before the transport
call, while its startup future is pending, between received chunks, between
decoded records, before each yielded event, and at terminal processing. The
provider registers for cancellation wakeups, so cancellation does not require
another response byte to become observable. The transport receives the same
token and must arrange an equivalent wakeup while its own future or byte stream
is pending. If cancellation becomes ready during the same poll that would
otherwise return a terminal response event or terminal response failure,
cancellation wins. The decoder retains a cancellation waiter only while its
stream poll returns `Pending`; every ready event, error, stop, and end outcome
deregisters it before returning, so later cancellation cannot spuriously wake
an inactive poller. Dropping the provider future or event stream drops all owned
transport futures, streams, buffers, and partial tool state. Before an owned
`ModelRequest` leaves its guard, all of its JSON trees, including ignored
metadata, pass aggregate depth/node validation. Early rejection, cancellation,
or drop drains guarded JSON iteratively before dropping the outer request, so a
deep hostile tree does not fall back to recursive `serde_json::Value`
destruction. Accepted trees are within the fixed safe depth ceiling. No detached
task, thread, timer, or retry survives cancellation or drop.

## Adjacent slice-33 web-search worker

The native web-search adapter is deliberately adjacent rather than a mode of
`AiGatewayProvider`. After core has approved a local `web_search` call, the
adapter makes one private required-tool request for exactly
`gateway.perplexity_search` / `perplexity_search`, with `maxResults: 10` and
`maxTokens: 4096`, over the same injected transport boundary. A separate strict
codec requires exactly one provider-executed call, one matching final result,
and a strict successful raw-v4 finish whose usage/raw-reason metadata is
validated and discarded. Its 16 KiB request, 256 KiB response, 64 KiB record,
256-record, 16,384-node, 30-second, and concurrency bounds do not borrow
unused capacity from `AiGatewayLimits`. The complete contract is
[`web-search.md`](web-search.md).

After the outer provider has validated its finish event, it immediately drops
the owned byte-stream source before delivering any queued usage/stop event.
That terminal ordering preserves same-chunk late-frame rejection while
releasing a shared transport permit before core begins the local tool round;
a capacity-one shared transport can therefore run the nested worker request.

No provider-executed value crosses the provider-neutral `ModelProvider`
boundary. The inner result becomes a bounded local `ToolOutput` and returns to
the ordinary core tool round. Cycle-1 source remediation is composed, and exact
precursor `e662fa8`, tree `6c0ace9`, passes the complete local gate. Exact
cycle-2 remediation `366cef9`, tree `40c05cb`, also passed its complete gate.
Formal cycle 3 rejected exact candidate `aef6abe`, tree `5abcef3`, with a
deduplicated `1/0/2/2`. Exact isolated components `5d45dca` and `454f8fd`
compose its remediation. Exact precursor `b834205`, tree `f3557a5`, passes the
complete replacement gate. Formal cycle 4 rejected exact `cc1d3d1`, tree
`ad0c3d3`, with a deduplicated `0/0/1/1`. Exact remediation precursor `2e9c44d`,
tree `3e25daa`, passes the complete replacement gate; formal cycle 5 review is
pending, and the slice makes no compatibility or performance claim.

## Deferred scope

This generation-codec slice adds no URL or HTTP client, socket, DNS, proxy,
TLS, native credential lookup, authorization header, status-code mapping, retry/backoff,
clock, async runtime, endpoint selection, team routing, or model-catalog logic,
provider-executed tool support in this outer codec, image, structured-output,
temperature, or metadata support. It adds no CLI wiring or commands,
production permission prompt, or
permission mode beyond `ask`. The native session store remains a separate
library boundary. Native configuration may declare the codec's provider kind
and model but does not compose or invoke this codec.

The optional native transport supplies only the separately documented bounded
HTTP/TLS/authentication/status subset. It does not discover credentials or
endpoints and does not compose the provider into the CLI. The separate native
credential adapter discovers only the explicit bearer input; it does not
change this codec boundary or compose either component into the CLI. The
configuration slice stores no credentials and likewise performs no
composition.

The catalog path is an adjacent implementation, not a mode of
`AiGatewayProvider`: native owns its fixed GET transport, authenticated/public
fallback, response parser, bounds, and ordering; core owns only catalog types
and a trait; the CLI owns runtime composition and output. Catalog credentials
may be absent for anonymous listing without changing this generation codec's
required authenticated host composition. The CLI selects the dedicated
non-WASM `ai-gateway-model-catalog-http` feature. That feature does not enable
`web-fetch-http` or the optional direct-generation `bytes` edge, and it does
not compile the web-fetch or direct-generation production modules. The broader
`ai-gateway-http` feature enables the catalog, web fetch, and the direct
generation transport. The catalog-only dependency tree still contains
`hickory-resolver`, `hickory-proto`, and Hickory Resolver's
transitive Moka graph. Machine God uses those Hickory packages for bounded
platform-resolver configuration parsing and bounded DNS protocol construction
and decoding, not for request-time resolution. Request-time DNS uses Machine
God's private bounded resolver over Tokio UDP/TCP. The Reqwest client explicitly
disables its Hickory resolver and receives the private resolver instead of its
default getaddrinfo path. The CLI manifest separately enables Tokio's signal
backend for command cancellation.
The injected catalog transport supplies both its request future and a separate
absolute-deadline waiter. The native provider polls cancellation, that waiter,
and the request in precedence order, so a conforming pending request cannot
silently outlive the shared 30-second operation deadline.

It also adds no compatibility or performance evidence. The pinned fx checkout
and Zig toolchain remain benchmark-only inputs and are not Rust product runtime
dependencies.
