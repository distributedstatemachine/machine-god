# ACP wire boundary

`machine_god_native::acp::protocol` provides effect-free modern ACP v1
JSON-RPC framing, exact JSON envelopes and outbound correlation. The native ACP
driver owns session state, permission and continuation custody, output admission
and shutdown. The thin CLI owns framed transport I/O and writer backpressure.
Correlation labels grant no authority.

The only supported initialization version is integer `1`. There is no older
version negotiation or deprecated SSE transport. Modern MCP Streamable HTTP
response SSE framing is a separate transport concern and remains supported.

## Envelope and resource contract

- Each message is one UTF-8 JSON object terminated by a newline. CRLF is
  accepted as JSON trailing whitespace. Batches, blank lines, duplicate object
  keys (including escaped-equivalent keys), missing JSON-RPC `2.0` and ambiguous
  request/response envelopes are rejected.
- IDs are signed 64-bit integers or strings up to 1,024 UTF-8 bytes. Numeric IDs
  cannot use fractional or exponent notation. Null IDs occur only on error
  responses. Methods are nonempty, control-free strings up to 256 bytes. Params,
  when present, are objects or arrays. Unknown members are inert extensions and
  pay the same admission budgets as known members.
- The frame ceiling is 8 MiB excluding the newline. JSON admission applies a
  64-container depth limit, 65,536 value/object-key token limit and conservative
  32 MiB retention charge (twice source bytes plus 256 bytes per token) before
  allocating the decoded tree. Syntax and UTF-8 validation then use core's exact,
  duplicate-rejecting codec. Arbitrary-precision payload numbers retain their
  original tokens, including negative zero and exponent spelling.
- Incremental decoding retains at most one bounded incomplete frame. It emits
  one oversize error, drains that frame through its newline and resumes framing.
  There is no internal decoded-message queue. EOF rejects unfinished frames,
  even syntactically complete JSON lacking its delimiter, and reclaims partial
  bytes. The driver must still finalize its native owners at EOF.
- Encoding checks constructed envelope shapes, raw payload bytes and JSON
  depth/node limits before recursive serialization, writes through a bounded
  output buffer, then applies the same exact lexical budget. It does not clone
  payload trees. Encoded output
  includes the final newline; callers must separately bound queued frames.
- Debug and error diagnostics omit IDs, methods, request/response payloads and
  remote error text. Explicit wire serialization naturally contains those data.

## Request decoding

The native driver projects admitted parameters into typed modern requests before
session activation or any filesystem work. Initialization requires exactly the
integer token `1`; client extensions confer no filesystem or terminal authority.
Supported methods are `initialize`, `session/new`, `session/load`,
`session/resume`, `session/close`, `session/list`, `session/prompt`,
`session/cancel` and `session/set_config_option`. Unknown
methods return `-32601`; invalid parameters return `-32602`, without echoing
input in diagnostics.

Programmatically constructed parameters pay the same depth, token and raw-byte
budgets as wire input, plus an 8 MiB serialized-parameter ceiling. Rejected trees
are reclaimed iteratively. Native session IDs and pagination cursors retain
their existing bounded formats. Selection requires a control-free absolute
`cwd` of at most 4,096 UTF-8 bytes, without parent traversal; lexical normalization
does not resolve or authorize the directory. Listing accepts omitted parameters
and optional `cwd` and native `cursor` fields.

Selection's `mcpServers` uses a separate bounded 1 MiB JSON writer and the native
ephemeral configuration decoder. Omission and an empty array authoritatively
select no servers; null, profile syntax and deprecated transports are invalid.
Decoding does not start peers or consult profiles. Prompts retain canonical text
and separately bounded advisory resource targets. Configuration changes accept
only `mode` (`ask`, `auto` or `yolo`) and a validated native `model` identifier.
The superseded `session/set_mode` method and duplicate `modes` response are not
implemented; modern `configOptions` is the only wire configuration interface.
Unknown bounded extension fields are inert.

## Correlation ownership

One connection-scoped `AcpPendingRequests` table admits at most 32 outbound
requests. Monotonic host string IDs are not reused after completion,
cancellation or table clearing; counter exhaustion rejects admission. The
driver must retain this table for the connection lifetime.

Each entry carries non-authoritative session-incarnation, turn, operation and
round labels. Completion requires the full exact scope and leaves an entry
untouched on mismatch. Duplicate or invalidated replies cannot settle another
request. Session/turn invalidation and connection clearing return removed IDs
and labels so the driver can settle its separately held native waiters. Merely
dropping this metadata table is not native cancellation or finalization.

These primitives do not advertise client capabilities, grant filesystem or
terminal authority, persist injected MCP configuration, or implement legacy
session import. The complete native ACP feature composes those ownership
boundaries under the [implementation plan](implementation-plan.md).

`acp::client_requests::NativeAcpClientRequests` keeps the connection-lifetime
correlation table together with the actual native inbox and client URL endpoint.
Each activation selects the prepared host's exact permission-context registry.
Independent candidate hosts can validate even a same-ID load without colliding
with the old host's routes. The connection's selected registry supplies the exact
live authorizing call; a tool name, request-ID conversion or observed event order is
not a substitute. The native inbox displays one request at a time, and this
owner retains only that view. It returns at most one bounded encoded frame per
poll, which the I/O driver must retain in its empty bounded output slot until
written. Repeated polls do not retransmit unanswered requests.

Unknown and duplicate reply IDs cannot settle a native waiter. Invalid or remote
error responses cancel the exact pending prompt. Native waiter abandonment wakes
the connection and releases its stale correlation without blocking the next
prompt. Session reactivation invalidates even accepted-but-unconsumed replies
and never reuses outbound RPC identifiers. EOF and output failure close prompt
admission; they do not replace the session driver's owned native settlement.
Modern `elicitation/complete` is encoded only from the registered operation's
actual completion notice, not from answer acceptance.

## Connection orchestration

`NativeAcpConnection` owns initialization, the selected native actor, one prompt
request, one control operation and one ready reply. The CLI owns only framing,
the input chunk and one acquired output frame. A request rejected by bounded
backpressure is returned unchanged to its caller, not copied into an internal
queue. Client responses and cancel notifications can still be admitted while a
normal control operation is pending. Cancel requests additionally need the one
reply slot. Unknown notifications are inert; invalid client replies cannot
settle another waiter.

Native callers constructing envelopes directly pay the same ID/method/shape
checks before reply retention. Ignored notifications and rejected/remote-error
reply trees are reclaimed iteratively, including caller-constructed excessive
nesting; error payloads and text are never retained as native diagnostics.

The connection accepts only modern initialization and native session methods.
Selection effects start during native polling, not request decoding. Listing
uses the factory's explicitly captured read-only catalog authority even before
a session is selected. Configured model choices share the supplied native
catalog without fetching capabilities. Model changes respond only after the
exact session-save receipt; mode changes affect future jobs without writing
profile rules. Load history is emitted incrementally before its selection
response. Engine observations and completed URL notices drain before the old
prompt response and activation of the next permission registry.

Session/configuration replies project one native permission-mode observation,
the current native model and the already supplied catalog, without fetching or
inferring model capabilities, through complete `configOptions`. Native command
changes to the live model emit a complete `config_option_update` for the original
session before the command result and prompt completion. This reports the live
selection even if a subsequent save fails or cancellation arrives; it is not a
persistence receipt. Unchanged configuration does not manufacture an update.
Catalog model order is preserved; the current
model is appended only when absent. Projection preflights all escaped string
bytes and framing before cloning model or session rows, leaving response-envelope
headroom under the 8 MiB wire limit. At most 512 catalog models plus one absent
current model are projected. A session-list page exceeding the factory's
100-entry contract fails instead of truncating rows under an incorrect cursor.

Each listed row requires a valid absolute, control-free UTF-8 native `cwd`.
The production factory resolves an explicitly supplied list `cwd` on its owned
worker before filtering, so existing ancestor aliases match canonical stored
metadata. Missing paths retain their exact filter for deleted-workspace history;
other resolution failures return an error. This observation creates no workspace
or host and grants no tool authority; it is not a retained identity or snapshot.
Records with absent or unrepresentable workspace metadata are omitted, with a
bounded `omittedWorkspace` count in `_meta.machineGod`; no current directory is
invented for them. Optional `title` is only the native title, never a preview.
Known representable activity timestamps become second-precision UTC
`YYYY-MM-DDTHH:MM:SSZ`, matching the pinned
[`sessions.zig` formatter](https://github.com/vercel-labs/fx/blob/b1774fbf6c7602b503026f96f6e960e946c692ef/src/acp/sessions.zig#L1014).
The pure Gregorian formatter supports years `0000` through `9999`, including pre-epoch
times using floor division; unknown times are absent and out-of-range known
times increment `omittedUpdatedAt`. There are no clock or timezone observations.
`scanComplete`, `resultsTruncated` and `skippedInvalid` retain the native scan's
meaning. The native continuation cursor is preserved over the underlying page,
including omitted unrenderable rows; projection never invents a snapshot,
silently skips renderable rows or substitutes its filtered tail as a cursor.

Transport output acquisition is separate from `poll_progress`: a blocked writer
cannot abandon admitted preparation, cancellation, catalog reads or shutdown.
Every newly queued frame schedules acknowledgement polling, including when no
input or native event is forthcoming. Write and flush acknowledgements each
retain their own wake registration; progress never relies on another request.
EOF closes admission and native human waiters, then polls actual retirement.
Unsent presentation may be discarded at this terminal cutoff; it is not a
successful delivery claim. One already-ready protocol reply may survive settled
native shutdown for the transport's final output grace. Output failure follows
the same native cleanup path, with a payload-free failure diagnostic. No
JSON-RPC response is itself a worker-join or checkpoint receipt.

Cancellation during prompt resource preparation reports `stopReason: cancelled`
only after the exact native operation has settled, including cancellation before
its first poll. It does not start a provider or fabricate a checkpoint. EOF waits
for the same worker settlement without treating this typed cancellation as a
native failure. Unrelated resource, persistence and native failures remain errors;
cancellation intent or error text cannot reclassify them.

When a complete input frame is retained under backpressure, the CLI also requests
an independent observation of the original input pipe's writer disconnect. The
existing owned input worker retains an exact FIFO alias (including helper-backed
blocking pipes) and checks hangup without read credit, read-ahead or changes to
shared descriptor flags. Read readiness is subscribed on both platforms so
macOS installs its pipe event filter, but readiness alone never grants read
credit or completes the disconnect observation. A disconnect takes the same
terminal cutoff and native settlement path, even with unread pipe bytes; it is not a claim that buffered
requests or output were delivered. This observation is consulted only for a
backpressured complete frame. Ordinary buffered input keeps its demand-gated
processing. Regular files, terminals and null streams do not manufacture pipe
disconnect from file metadata and retain their normal EOF behavior.
