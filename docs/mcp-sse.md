# MCP server-sent event framing

The native MCP SSE module owns effect-free incremental framing. It does not
connect, retry, interpret JSON-RPC, admit endpoint URLs, update a resume cursor
or grant execution authority. The [implementation plan](implementation-plan.md)
owns current feature and delivery state.

## Explicit pinned consumer modes

Compatibility follows fx b1774fbf6c7602b503026f96f6e960e946c692ef, specifically
src/core/mcp/streamable_http.zig and src/core/mcp/legacy_sse.zig.
Both accept LF, CR and CRLF across arbitrary byte chunks. A CR terminates its
line immediately; one following LF is consumed without creating another line.
Fields are case-sensitive, split at the first colon and remove at most one
leading ASCII space from the value. Comments and unknown fields are ignored.

These are pinned MCP consumer semantics, not browser EventSource semantics:

- Modern mode reads only data fields. It emits only nonempty accumulated data
  and joins subsequent data lines with a newline only when accumulated data is
  nonempty. Thus leading empty data lines disappear. Event names, IDs and retry
  fields have no semantic effect.
- Legacy mode joins every data field, including empty ones, with one newline.
  It retains the last explicit event name and non-NUL ID in each block. Absent
  and explicitly empty names/IDs remain distinct. An ID containing NUL is
  ignored without replacing an earlier valid observation in the block.
- Legacy emits blocks containing any recognized valid field, including
  metadata-only and empty-data priming blocks. Such empty-data observations
  have SseEventClass::Control; they are not JSON-RPC messages. Nonempty data,
  including endpoint-event data, has class Data and remains untrusted.
- Legacy retry parsing follows the producer's decimal u32 parsing: optional
  plus, negative zero and internal underscores are accepted; negative nonzero,
  overflow and malformed values are ignored. Valid values are capped at
  60,000 ms. An invalid repeated retry does not erase an earlier valid one.
- Neither pinned parser strips a UTF-8 BOM. A BOM before a field name makes it
  an unknown field; a BOM inside data remains data. This is intentionally not
  standard browser BOM normalization.

The decoder validates UTF-8 on every completed line, including ignored lines.
Malformed encoding is rejected rather than replaced or forwarded. This strict
admission and the additional finite budgets below are intentional differences
from the producer's permissive byte parsing.

## Events, cursor ownership and EOF

SseDecoder::push returns consumed bytes and at most one event. Callers must
retain and re-submit only the unconsumed tail; those bytes are not yet charged.
Empty input is a no-op, not EOF. A push may return no event after consuming
comments, ignored fields or a bounded partial line. Callers must yield between
bounded pushes instead of draining an unbounded input queue in one task poll.

Events are emitted only at a blank line. A CRLF split immediately after a
dispatching CR does not dispatch again when its LF arrives. No event fields
inherit from earlier events, and blank/comment-only blocks create no phantom
messages. The had_data_field accessor distinguishes explicit empty data from
metadata-only observations. The id accessor returns an uncommitted observation:
None must not erase a cursor, while Some("") is an explicit empty-ID observation,
not absence. The owning transport validates the event and its
connection/generation before deciding whether to update a cursor, interpret a
retry or use an endpoint. JSON-RPC data admission reuses the existing
[protocol decoder](mcp-runtime.md).

SseDecoder::finish never manufactures a final event. It rejects an unterminated
line or mode-specific pending event, and rejects invalid final UTF-8. A trailing
CR is already a line terminator. Complete ignored/comment lines may end cleanly;
modern empty data alone is ignored, while legacy recognized fields still need
a blank separator. Every error and every finish permanently closes the decoder
and releases all retained payload buffers. Later pushes/finishes return Closed.

## Resource boundary

All limits are positive, validated before allocation, and have fixed ceilings.
Zero never means unlimited.

| Budget | Default | Hard ceiling |
| --- | ---: | ---: |
| Raw line bytes, excluding terminator | 8 MiB | 16 MiB |
| Joined event data bytes, including inserted newlines | 8 MiB | 16 MiB |
| Each retained event-name or ID field | 4 KiB | 64 KiB |
| Nonempty lines per block, including comments/unknown fields | 1,024 | 16,384 |
| Emitted events per decoder | 4,096 | 65,536 |
| Total consumed raw bytes, including all CR/LF/comments | 64 MiB | 1 GiB |
| Consumed raw bytes per push | 16 KiB | 64 KiB |

A blank separator resets only the block-line budget. Neither dispatch nor
ignored blocks reset lifetime event/raw-byte budgets. Long-lived transports
must handle budget exhaustion explicitly, not silently reset parser accounting.

Line and data capacities grow geometrically without exceeding their individual
ceilings; replaced metadata reuses its bounded capacity. Each completed line
is validated/interpreted once, and joined data is moved into its emitted event.
There are no accumulated event queues, repeated prefix rescans or per-byte
reallocations. A push ending a long line can additionally validate/copy up to
the bounded line size; the raw-input-per-push limit does not claim constant CPU
cost independent of that line budget. The decoder retains at most a line,
joined data and two metadata strings; caller-retained events require separate
aggregate queue limits. Debug formatting and errors never include field values,
event data, endpoint text or IDs.

Tests include producer-derived endpoint/priming events, all delimiter split
points, exhaustive small-input partitions, UTF-8/BOM behavior, cursor absence,
empty fields, retry forms, exact/over-limit cases, poisoned/EOF states and
allocation-count regressions. These are component checks, not complete
transport or performance acceptance.
