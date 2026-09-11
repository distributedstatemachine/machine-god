# Native MCP tool execution and archives

`NativeMcpArchivedToolExecutor` supplies the concrete `NativeMcpToolExecutor`
implementation for already selected dynamic tools. Its constructor requires an
explicit `NativeToolResultArchiveAdapter`; it does not discover directories,
prepare storage, start workers, acquire a peer or retain an engine/runtime.
The matching `execution_policy()` advertises no form or URL responder. Progress
metadata remains constructor-selected and preserves the actual reserved ID.

## Admission and distinct results

Execution retains the original `NativeMcpRuntimeToolCall`, arguments, options and
exact native ownership throughout the sole `first_exchange`. That exchange
consumes the existing permission proof and peer-minted reservation, serializes
actual peer access and races cancellation/deadlines. No raw write, second attempt,
automatic retry or response-derived authority is exposed.

After revalidation, original correlated response bytes enter
`NativeMcpToolResultAdmission` with the actual descriptor, protocol, request ID,
server, exposed tool and runtime allocation. Invalid content, output-schema
instances, malformed input requests and mismatched envelopes produce a bounded
redacted tool error, never a fabricated successful result.

- Complete results retain the entire admitted JSON value and actual `isError`
  bit, including structured content, metadata, exact numeric lexemes, signed zero
  and literal private-looking JSON keys.
- Protocol failures have `is_error: true`, `resultType: "protocol_failure"` and
  the complete admitted error object under `error`. They remain distinct from a
  completed tool's `isError` and do not request turn termination.
- Valid unresolved input has `is_error: true`, `resultType: "input_required"`,
  the typed rendered `inputRequests` and any exact `requestState`. Modern input
  and admitted legacy URL-required data use that same explicit projection.
  Its `ToolExecution::finish_turn()` stops after the stored result and delivered
  event. Later sibling calls retain unknown placeholders; no further provider
  round or sibling preparation, permission or execution occurs.

No browser, form interaction, sampling, roots request or continuation runs here.
Typed input data is not consent. A future continuation owner must retain the
actual original call, validate typed responses and acquire a fresh exact grant;
serialized result/state or context IDs cannot reconstruct authority.

## Archive bounds and completion ownership

The runtime policy and adapter agree on 64 KiB/4,096-node original inputs,
68 KiB/4,224-node prepared envelopes, and complete outputs bounded to 4 MiB plus
16 KiB plus the 29-byte `ToolOutput` wrapper, with 262,144 content nodes.
Response decoding keeps its independent raw-envelope, depth and retained-data
limits. Direct input publication checks bounds before cloning arguments.
Constructing or dropping an unpolled future starts no archive worker.

The existing archive adapter leaves serialized outputs at or below 64 KiB inline.
Larger complete results are published losslessly to the explicitly retained
archive and replaced only in durable history with its bounded reference. The
complete `ToolFinished` event remains exact. References retain the original error
bit, owner context and digest-bound bytes and are readable through the shared
archive reader. Input publication uses the same adapter and never changes the
original arguments sent for permission or execution.

The explicit `NativeMcpToolCompletionPolicy` defaults to `Cancellable`. This concrete
archive executor selects `CompletionWinsAfterFirstPoll` through core's existing
completion-wins preparation.
Queued/network work remains cancellation-aware. It revalidates after exchange,
after result admission and immediately before beginning result publication.
Once publication is polled, its owned worker finishes and its receipt is not
discarded by a later route or caller cancellation. The runtime preserves that
successful execution; core persists its exact result before observing deferred
cancellation. Validation, archive and transcript-save failures do not become
successful turn completion. Other executors keep ordinary post-return runtime
validation unless they explicitly accept this completion ownership contract.

Input publication has a different existing boundary: it is cancellable before
the requested tool action, and its adapter may return cancellation after a
publication raced that cancellation. An unadvertised archive may consequently
remain charged; cancellation is not a statement that no archive bytes exist.
This executor does not turn such input-publication cancellation into permission,
execution, a reused result or an automatic retry.

## Pinned semantics and presentation boundary

The pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` distinguishes
complete, protocol-failure and input-required status in
`src/core/mcp/tool_result.zig`; `src/core/tooling/tool_runtime.zig` requests a turn
finish for unresolved input. Its bounded model rendering is not an archival
format. Native complete events and archives here intentionally retain exact
admitted data instead of substituting the pin's lossy truncation. Rendering and
model-facing disclosure remain separate from archival fidelity; these JSON data
values are not safe terminal instructions or a claim of secret masking.

See [result admission](mcp-tool-results.md), [runtime ownership](mcp-runtime-publication.md),
[archive paging](read-tool-result.md) and [core orchestration](core-api.md).
Feature delivery and remaining runtime/CLI integration are tracked only in the
[implementation plan](implementation-plan.md).
