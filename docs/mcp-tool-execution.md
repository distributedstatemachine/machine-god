# Native MCP tool execution and archives

`NativeMcpArchivedToolExecutor` supplies the concrete `NativeMcpToolExecutor`
implementation for already selected dynamic tools. Its constructor requires an
explicit `NativeToolResultArchiveAdapter`; it does not discover directories,
prepare storage, start workers, acquire a peer or retain an engine/runtime.
The default `execution_policy()` advertises no form or URL responder.
`with_form_responder` explicitly retains an actual `McpElicitationPresenter`
(normally the native interactive prompt bridge) and enables form advertisement.
URL support remains false. Progress metadata uses the actual peer-reserved ID.

## Admission and distinct results

Execution retains the original `NativeMcpRuntimeToolCall`, arguments, options and
exact native ownership throughout `first_exchange`. That exchange consumes the
existing peer-minted reservation, retains the original permission proof,
serializes actual peer access and races cancellation/deadlines. The public call
API exposes no raw write, second attempt or automatic retry.

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

## Consented modern form continuation

Only the concrete native executor's private sealed-response path can continue.
Each response carries the exact round allocation. Its writer-completion marker
is set only after every exact request byte is acknowledged, the final flush
succeeds and the final original-proof checkpoint passes. Partial writes,
intermediate flushes, errors, dropped futures, malformed envelopes and ambiguous
responses cannot open another round. Calling the public result parser on arbitrary
bytes cannot manufacture this private custody.

The native call keeps one immutable original grant allocation, canonical
arguments, options, turn, schema/configuration/authentication/runtime binding and
original projected HTTP head. The registry's original call remains claimed;
continuation neither reopens its slot nor asks the model for fresh permission.
The retained original proof is revalidated before and after human input, queue
acquisition, response admission and every synchronous writer boundary.

With an explicitly supplied form presenter, a correlated modern input-required
result presents its exact typed requests through the native inbox. Every request
must be a supported form before presentation begins. Answers must match the
entire original request-key set and schema; accept, decline and cancel remain
distinct wire actions. A form cancel supplies explicit cancel actions for the
remaining forms without further prompts, matching the pinned responder.
An empty request map, including state-only input, remains unresolved: the pinned
`elicitation_interaction.zig` responder rejects zero requests. Unsupported URL,
sampling, roots and legacy requests, or an unavailable presenter, keep the
explicit input-required projection and stop-after-tool behavior above.

Actual answers and their exact round produce non-clone native consent custody.
Consuming it acquires a fresh serialized peer lane and peer-minted RPC ID;
progress IDs also renew when enabled. Only `inputResponses` and `requestState`
are added as `params` siblings to the unchanged original arguments/name, with
the original client capability policy. HTTP reuses the exact projected head
allocation, recomputing only body framing/length. Initial and raw requests retain
their 128 KiB cap; this typed continuation path has an independent 384 KiB cap
for 64 KiB arguments, 128 KiB state, 128 KiB aggregate answers and metadata.
Exact numeric lexemes, null-versus-absent state and private-looking JSON keys
survive without a `Value` conversion in this wire path.

There are at most eight continuations after the initial call. Further
input-required responses produce an explicit bounded protocol-failure result,
without another prompt or reservation. Each admitted input-required response
with an actual responder and an available continuation round starts a fresh
30-minute interaction deadline before its presenter is polled. Unresolved
no-responder and exhausted-round results do not read a new interaction clock.
Multiple rounds may therefore exceed 30 minutes in total. Once exact consent
is admitted within that deadline, the queue and resumed exchange use a fresh
normal operation timeout, not the preceding human deadline.
Turn/caller/preparation/runtime authority
cancellation wakes pending consent and network work. Dropping the waiting
prompt removes its inbox ownership and cancels its token; no response is replayed.
Native permission resets and rule-publication transitions also wake the wait;
the notification-only observer revalidates the original proof without changing
Yolo policy. Permission is checked again after an answer, so a revoked original
grant cannot submit a continuation. External core metadata changes retain their
existing synchronous-checkpoint boundary, not a new native notification claim.

This owner does not launch browsers, perform URL/legacy completion retries,
sampling or roots operations. Those need separate actual native responders and
typed custody; server data and serialized context IDs never supply authority.

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
