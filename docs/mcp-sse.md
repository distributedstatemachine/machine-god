# MCP server-sent event framing

The native MCP SSE module owns effect-free incremental modern data framing.
It does not connect, retry, interpret JSON-RPC, admit endpoints or grant
execution authority. The [implementation plan](implementation-plan.md) owns
live feature and delivery state.

## Modern data semantics

The data consumer follows the modern `streamable_http.zig` behavior at fx
`b1774fbf6c7602b503026f96f6e960e946c692ef`; deprecated consumer modes are removed.
LF, CR and CRLF work across arbitrary chunks. CR terminates its line immediately;
one following LF is consumed without creating an extra line.

Fields are case-sensitive, split at the first colon, and strip at most one
leading ASCII space. Only data fields contribute to an event. Comments, event
names, IDs, retry fields and unknown fields are ignored, not retained metadata.
Subsequent data fields gain a newline only when accumulated data is nonempty;
leading empty data fields disappear. Empty accumulated data emits nothing.
No UTF-8 BOM is stripped: a BOM before a field makes that field unknown, while
a BOM inside data remains data. These are MCP data-consumer semantics, not a
browser EventSource implementation.

Every completed line, including ignored lines, must be valid UTF-8. Malformed
encoding is rejected, not replaced. Strict UTF-8 admission and the finite
budgets below are deliberate additional native bounds.

## Progress and EOF

`SseDecoder::new` accepts only `SseLimits`; there is no legacy mode.
`push` returns consumed bytes and at most one nonempty data event. The caller
resubmits only the unconsumed tail, which has not yet been charged. Empty input
is a no-op, never EOF. Callers yield between bounded pushes.

A blank line dispatches an event. Split CRLF never dispatches twice. Events
have only data: no ID, event name, cursor, retry or control-frame API exists.
JSON-RPC admission remains the [protocol decoder's](mcp-runtime.md) responsibility.

`finish` never manufactures a final event. It rejects unterminated lines,
pending nonempty event data and invalid final UTF-8. Complete ignored/comment
lines and empty data may end cleanly. Errors and finish permanently close the
decoder and release retained buffers; later operations return `Closed`.

## Resource boundary

All limits are positive and validated before allocation. Zero is not unlimited.

| Budget | Default | Hard ceiling |
| --- | ---: | ---: |
| Raw line bytes, excluding terminator | 8 MiB | 16 MiB |
| Joined data bytes, including inserted newlines | 8 MiB | 16 MiB |
| Nonempty lines per block, including ignored lines | 1,024 | 16,384 |
| Emitted events per decoder | 4,096 | 65,536 |
| Consumed raw bytes, including comments and CR/LF | 64 MiB | 1 GiB |
| Consumed raw bytes per push | 16 KiB | 64 KiB |

Separators reset only the block-line budget. Event/raw-byte budgets never reset.
Line and data buffers grow geometrically within their individual ceilings.
Completed lines are interpreted once; joined data moves into the event.
There are no accumulated event queues or metadata allocations. A push ending
a long line may additionally validate/copy up to the bounded line size; the
push-byte bound does not claim constant CPU cost. Caller-retained events need
their own queue budget. Debug/errors omit all payload contents.

Tests retain delimiter splits, exhaustive short-input partitions, ignored
metadata, empty data, BOM/UTF-8, exact/over-limit bounds, EOF poisoning and
allocation-count regressions.
