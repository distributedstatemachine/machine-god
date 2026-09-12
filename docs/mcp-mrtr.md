# MCP input-required and elicitation codecs

The native `mcp::mrtr` module admits inert multi-round-trip request (MRTR)
data against fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`,
`src/core/mcp/mrtr.zig` and `src/core/mcp/elicitation.zig`. Structural URI
behavior follows the official
[Zig 0.16.0 URI source](https://codeberg.org/ziglang/zig/src/tag/0.16.0/lib/std/Uri.zig),
not URL normalization or a newer standard-library revision.
It performs no transport, browser, filesystem, sampling or roots operation.
An accepted action string is data, not evidence of user consent, completion,
permission or a valid continuation generation.

## Requests and responses

`McpInputRequired::parse` accepts a modern result containing `inputRequests`,
`requestState`, or both. State-only results and explicitly null opaque state
remain distinguishable from absent state. Requests use the closed methods
`sampling/createMessage`, `roots/list`, and `elicitation/create`. Request keys
must be nonempty and unique. Unknown methods fail the entire admission.
The enclosing operation owner must independently verify the JSON-RPC response
ID, negotiated protocol and `resultType`; this codec does not invent that proof.

`requests()` exposes typed owned payloads in producer insertion order, as do
form fields and choices. Sampling and roots retain exact raw
parameters after their pinned shape checks. Sampling accepts the pinned text,
image, audio, tool-use and recursive tool-result content shapes; its media check
is string-shape validation, not the distinct tool-result base64 policy.
The codec intentionally does not implement either sampling or roots.
`render_requests_json()` provides a bounded method/parameter map for a UI
adapter, not an executable request or arbitrary-method transport escape.

`validate_responses` requires exactly the original request keys and validates
each method-specific result. The returned `McpValidatedResponses` retains the
original response map and typed dispositions. Its separate `wire_json()` keeps
sampling and roots results intact, but reduces elicitation responses to the
originating-wire fields: action, and content only for an accepted form.
Ignored content attached to decline/cancel and unrelated elicitation response
extensions are never forwarded through that projection. Exact form numbers and
literal private-looking object keys are not rounded or coerced.

## Elicitation forms and URLs

`McpElicitationRequest::parse` also admits direct elicitation parameters for
modern MCP, `2025-11-25`, and `2025-06-18`. The oldest supported elicitation
revision allows only omitted-mode forms, enum/enumNames choices and primitive
fields; explicit mode, patterns, titled oneOf and multi-select arrays fail.
Other protocol revisions are not silently treated as one of these wires.
Standalone parsing uses the pinned elicitation ceilings of 8 KiB messages,
64 fields/options and 1 KiB labels. Nested modern MRTR explicitly inherits
its 64 KiB string/message/label and 256 field/option bounds. Legacy URL-required
errors retain the standalone 8 KiB message preflight before request-map admission.

Forms expose typed fields and choices, original schemas/defaults, names,
titles, descriptions and field-level validation. The restricted form language
is intentionally not the general [JSON Schema](mcp-schema.md) contract:

- Root type must be exactly `object`, with an object `properties`; root title
  and description are unsupported on MCP. `additionalProperties`, if present,
  must be false. Unknown root/field keywords remain ignored annotations.
- Required field names must exist and be unique. Accepted content cannot add
  unknown fields. Defaults must themselves satisfy the selected field kind.
- String, exact number/integer, boolean, single-select and multi-select fields
  follow the pinned constraints. Multi-select choices must be unique; titled
  choices may share labels but not values. MCP titled-choice descriptions are
  unsupported. Selection values intentionally ignore ordinary string length,
  pattern and format constraints after those constraints pass schema admission.
- Numeric minimum, maximum and multipleOf use the shared exact decimal engine.
  Length/item limits require integer lexemes; decimal/exponent spellings are
  not accepted for those specific schema members. String lengths count Unicode
  codepoints. Email, URI, date and date-time use pinned structural checks;
  unknown formats fail rather than silently becoming annotations.
- Patterns reuse the bounded local pattern engine, but unsupported grammar is
  an elicitation schema error, not server-authoritative delegation. Compiled
  patterns are temporary: no per-field compiled-state cache is retained.
- Names and titles containing pinned normalized secret terms fail before a
  form can be published. Descriptions are not treated as input-field names.

URL mode rejects requestedSchema and embedded credentials. URLs must use HTTPS
or HTTP with the exact loopback host `localhost`, `127.0.0.1`, or `[::1]`.
The original URL and decoded host are retained without browser/network effects;
alternate numeric loopback spellings are not normalized into allowed hosts.
Punycode and non-ASCII hosts are classified for a warning, not denied merely
because of their spelling. Modern URL parameters reject elicitationId;
`2025-11-25` requires it. Accepted URL responses cannot contain content.
`url_host_bytes()` preserves decoded non-UTF-8 bytes; `url_host()` is available
only when those bytes are UTF-8. The pinned structural URI parser is deliberately
not full RFC/hostname admission: invalid percent escapes remain literal, and
its 255-byte decoding buffer applies only to hosts containing a percent sign.
Unescaped hosts remain bounded by the original URL limit. These quirks are inert
compatibility data, not permission to launch a browser against an unchecked URL.

`parse_legacy_url_required` admits the data of `2025-11-25` error `-32042`
only. Its nonempty URL request list rejects duplicate IDs and forms.
`legacy_retry_without_responses()` describes the originating wire, not a retry
decision. Browser acceptance, matching completion notifications or explicit
manual retry, and exact context/generation custody belong to the native owner.
The codec does not turn an accepted URL action into completion proof.

## Independent bounds and ownership

All public raw JSON entrypoints use shared protocol admission before borrowed
maps or retained copies: duplicate keys, depth and node exhaustion fail even
inside unknown fields, schema annotations, opaque state and response content.
All limits are positive and lowerable only. Defaults are 32 requests, 128 KiB
input/response JSON, 256-byte names/object keys, 64 KiB strings, 256 entries per
collection, and depth 32 with the root at depth zero. Rust additionally bounds
JSON values and object keys to 65,536 nodes and retained data to 16 MiB.

The conservative retained charge is six times source JSON bytes, plus 256 bytes
per JSON value/object key and 4 KiB fixed overhead, checked before owned
construction. It covers overlapping raw payload/schema/default copies,
decoded strings, choices and sparse container overhead; it is not allocator
telemetry. Requests and validated response sets expose their charge so custody
owners can apply aggregate budgets. One strict JSON tree and bounded borrowed
maps are temporary during admission. Field validation retains no instance;
one pattern/number evaluation is temporary at a time. Callers must bound how
many separately admitted objects they retain and how many rounds they permit.

Pattern defaults are 512 source bytes, nesting 64, 2,048 states, repetition
1,024 and 100,000 evaluation steps per match. Number defaults are 4,096 lexeme
bytes, absolute exponent 1,000,000 and 8,192 expanded digits. Exhaustion rejects
the operation, never weakens a form constraint. Error/debug output does not
include user responses, opaque state, URLs, field names or messages.

Actual operation correlation, one-shot answer consumption, cancellation,
deadline, changed-auth handling, browser consent and continuation submission
remain separate native responsibilities. These data codecs neither consume
their request nor grant execution authority when called repeatedly.

The [native archived executor](mcp-tool-execution.md#consented-modern-form-continuation)
separately composes correlated modern form requests with the real typed prompt
inbox, original native proof custody and fresh bounded submissions. Its private
round allocation is not obtainable from these public codecs. URL/legacy retry,
sampling and roots remain outside that form continuation path.

## Native legacy completion observations

`mcp::completion` classifies exact `notifications/elicitation/complete` envelopes
and retains bounded native observation windows. A source is a unique local
allocation selected for one runtime/connection/client/authentication lifetime;
remote JSON and equal generation numbers cannot recreate it. Native routing must
open the window before submitting the originating operation and register every
exact elicitation ID atomically before presenting browser consent.

Unknown early notifications are retained in every matching open window, never
only the first. Registration promotes matching records from its own window.
Duplicates do not extend the ten-minute early-record lifetime; records at the
exact expiry timestamp remain eligible, matching the pin. Verified candidates
remain source-wide duplicate tombstones until invalidation; they are not an
unbounded history. Dropping a window discards its unverified early records and
cancels its waiters. Source invalidation permanently closes that allocation;
replacement requires a fresh source.

Independent lowerable ceilings are 32 windows, 32 waiters, 64 early records per
window, 1,024 candidates and 4 MiB conservatively charged retained ownership.
Each waiter admits at most 32 nonempty IDs of at most 256 UTF-8 bytes. Charge
permits stay with outstanding handles and observations even after registry
removal; fixed reservations also cover container capacity retained by the
registry and each window after entries expire or are removed.
Constructors and unpolled waits start no clock, transport or task;
observations receive explicit timestamps, and pending waits use an injected
monotonic clock. Each waiter admits only one active asynchronous subscription
owner; dropping that wait releases its slot. Cancellation and completion wake
callbacks outside state locks.

An opaque observation binds the exact source, window and complete ID set. It is
not browser consent, an execution grant or a reusable retry permit. The native
operation still must obtain actual consent, preserve its original proof, verify
current ownership and consume its separately owned continuation before any
legacy retry. Transport driving, browser launching and that consuming operation
remain distinct composition responsibilities, not effects of this registry.
