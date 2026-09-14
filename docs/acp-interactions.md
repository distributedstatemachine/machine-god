# ACP human interaction

ACP uses the native permission and human-input owners. Client replies are data,
not permission, execution, browser or persistence authority. Each pending reply
must retain its exact session incarnation, operation and request round; stale
replies cannot be rebound to a replacement session.

## Wire projection and replies

Native ACP projection consumes actual engine observations and immutable native
inbox views. Text and reasoning deltas become `agent_message_chunk` and
`agent_thought_chunk`; observed model tool calls become pending `tool_call`
updates, native starts become in-progress calls and native finishes become
completed or failed `tool_call_update` messages. Tool IDs and exact JSON
arguments/results are preserved. Updates include native incarnation, turn and
sequence labels in `_meta.machineGod`; those labels are data, not authority.
Provider stops and terminal engine events never emit a final prompt response:
only the separately finalized native interactive outcome can do so.

Local commands occupy the same single active-prompt lane, independently of a
pending session selection. Close and replacement can cancel a native command;
its exact control receipt remains with the old session until output acquisition.
The command update and original prompt reply precede the replacement response
and activation of its client registry. Synchronous command observations retain
their original principal even if that native session has since retired. Command
capabilities are refreshed after selection and model changes. EOF drains owned
command effects without requiring an output consumer; cancellation metadata does
not assert rollback of effects that may already have completed.

Duplicate active request IDs are rejected without retaining their parameters;
discarding even deeply constructed native-call parameters uses iterative cleanup.

Permission projection requires both the inbox view and its actual tool call
from the live native permission-review context. A permission request ID is not
a tool-call ID, and event-order guessing is not accepted provenance. The
`session/request_permission` options are `allow_once`, `allow_always` (displayed
as allowing this session), and `reject_once`. `allow_always` maps only to the
existing volatile `AllowSession` decision; it never publishes a persistent rule.
The exact selected/cancelled outcome shape is checked before inbox submission.

MCP form and URL requests use modern `elicitation/create`, with the actual
native `sessionId`, and `toolCallId` only for an actual model-tool source. Human
feature requests do not invent a turn or call. Form schemas preserve exact
number tokens and remove MCP dialect/presentation annotations `$schema` and
`enumNames`; source `_meta` remains bounded inert data. URL requests require an
opaque host-generated ID obtained from the exact registered native request.
Peer-supplied scope fields cannot replace the actual native source, and a
peer-supplied URL `elicitationId` is rejected by native admission.
The display message identifies the independently captured MCP server and, for
URLs, the validated host before showing the server's request text.

Ordinary `ask_user_question` uses the same modern form surface, with ordered
`question_1` through `question_4` string fields and the real tool context.
Native question text and suggested options are displayed, while free-text
answers remain supported. Every question requires one nonblank answer; the
native aggregate 4 KiB UTF-8 answer bound remains enforced independently of
the form's per-field character hint. Additional or missing fields are rejected.
Declining or cancelling the form maps to native question cancellation.

Projection validates borrowed JSON before cloning or serializing, uses the
shared ACP frame limits and bounded serialization, and redacts Debug/errors.
MCP answers must have the exact action/content shape, fit the 128 KiB native
answer ceiling and validate against the original admitted native schema.
Successful decoding is not freshness evidence: the driver must retain the
original token and exact owner through outbound correlation and submit replies
to the native inbox, which independently rejects stale or cross-session tokens.
The projection layer owns no I/O, pending RPC IDs, permission grants or browser.

## Modern client-managed URLs

The native MCP presenter may explicitly select a client-managed URL endpoint.
This selection, not untrusted capability flags, enables URL support without a
local browser launcher. Client URL consent never invokes the native browser or
its recovery prompt, even if a browser launcher is also available. Native
interactive CLI presentation keeps its explicitly selected local-browser path.

Registration occurs only after native request admission and revalidation, before
presentation. It is bounded and nonblocking and does not submit anything by
itself. Each registration owns an exactly-once terminal observation. Only an
accepted URL is retained through the continuation; declining, cancelling or
dropping a pending answer abandons its registration. Answer acceptance and
browser handoff are never operation-completion evidence.

For model tools, successful terminal observation follows the final admitted
response and native archive publication. Human resource-read and prompt-get
actions retain the same custody until their final admitted response. Unresolved
input, exhausted continuation rounds and protocol failure are not successful
completion. Any early error, cancellation, stale authority or dropped operation
abandons its remaining registrations. Completion does not imply that the
remote tool's application-level result was successful.

At most 256 completion registrations can be retained per operation: eight
rounds of 32 requests. The ACP endpoint must independently bound its pending
correlations and queued output, discard never-submitted or stale registrations,
and emit modern `elicitation/complete` only for accepted, completed requests.
Callbacks enqueue without blocking; they do not perform I/O in destructors.
No legacy URL registry, inbound legacy completion or automatic retry is added.

`NativeAcpElicitationPresenter` adapts the existing native prompt bridge. Its
256-entry limit includes pending registrations and queued completion notices;
pre-submission request retention has a separate 8 MiB bound. Admission to the
wire output lane releases retained request bytes, keeping only compact host
correlation and the exact principal. IDs never repeat within the endpoint,
including same-session reactivation. Deactivation discards old registrations
and queued notices without publishing success. No request or answer is saved
to profile configuration or credentials.

See the [implementation plan](implementation-plan.md) for integration and
delivery gates; this contract is not a delivery-status ledger.
