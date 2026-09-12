# Native MCP human presentation

`mcp::interaction` connects admitted form and URL elicitation data to the same
bounded `NativeInteractivePromptBridge` used by permission and question prompts.
It creates no additional queue, input reader, background task, or ambient owner.
The presenter is explicitly injected; `UnavailableMcpElicitationPresenter`
returns an explicit unavailable result, never an invented human answer.

## Identity and schema

`McpElicitationPromptRequest::new` requires the exact `ToolContext`, selected
server identity, exposed `ToolName`, and an `Arc<McpElicitationRequest>`. Server
identity is nonempty and at most 256 bytes; the tool identity retains the
core's 128-byte `ToolName` admission. The executor
supplies these identities from selection, not server text.
`McpElicitationPromptSource::ModelTool` retains that exact context and tool.
`new_human_feature` instead accepts the actual `BackgroundOutputOwner`, selected
server, and either `ResourceRead` or `PromptGet`; all other actions are rejected.
Its `HumanFeature` source retains session/incarnation and feature action without
inventing a model turn, call or tool name. Both constructors are inert provenance
admission, not feature execution or continuation authority.

Views expose the source enum through `source()`; there is no unconditional
model-only context/tool getter. Debug is redacted. Inbox admission compares the
real source session/incarnation with its active owner and checks the scope
captured when the presenter is called, including for futures not yet polled.

The original admitted elicitation object is shared unchanged. Nested MRTR forms
therefore retain their inherited 256-field limit rather than being reparsed
through the standalone 64-field contract. Every admitted field type and exact
number remains available through the typed form schema. These forms do not use
the four-question adapter. [MRTR admission](mcp-mrtr.md) remains responsible for
the protocol revision, schema grammar, URL data, and exact response rules.

## Replies and lifecycle

`NativeInteractivePromptView::elicitation` exposes the typed request.
`NativeInteractivePromptResponse::Elicitation` takes bounded
`McpElicitationAnswerInput`. Input construction only admits at most 128 KiB of
raw JSON; it does not establish schema validity or consent. Inbox reply validates
against the exact displayed request with the existing MRTR validator, then
stores canonical response data. Numeric lexemes are preserved. Decline and
cancel are distinct actions; their ignored content is omitted canonically.
URL acceptance rejects content and does not open a browser.

Modern URL requests retain a separate typed recovery question when a browser
handoff was not confirmed. Its only choices are continue manually, retry the
browser, and cancel. Recovery retains the exact original source and request,
uses the same inbox limits and invalidation rules, and charges 64 additional
request bytes plus 64 bytes for an unconsumed answer. The CLI requires the exact
acknowledged presentation token and does not repeat the authorization URL in
the recovery display. A retry choice is data for the native effect owner, not
permission to reopen a browser or resubmit a tool call on its own.

There is no legacy URL-completion prompt, completion answer, or manual
completion/retry API. Modern consent and recovery do not wait for legacy
completion notifications or manufacture remote completion evidence.

Validation occurs outside the inbox lock. Admission then rechecks the token,
displayed/unanswered state, exact payload Arc, and response budget under lock.
Wrong-kind, stale, foreign-owner, duplicate, invalid, and over-budget replies
cannot consume a prompt. An answered marker remains set while a ready response
is taken, preventing reentrant cleanup from reopening that token. Existing
permission-rule invalidation and wake-outside-lock behavior remain intact.

Unpolled calls enqueue nothing. Dropping a pending call unregisters it. Engine
cancellation wins over a ready answer and returns `Cancelled`; UI cancellation
instead supplies the protocol's canonical cancel action. Inbox deactivation,
reactivation, closure, or drop invalidates queued and ready responses. No model
text is interpreted as human input.

## Bounds and authority

The existing pending-count and aggregate request-byte limits also cover
elicitation. A stored conservative MRTR charge accounts for each admitted
request's raw data, typed fields, and container overhead, including Arc control
storage. Inbox charging adds the actual source's retained identity strings,
the selected server and 256 bytes of prompt
bookkeeping. Shared request bytes are charged for each queued prompt, even when
the Arc storage is shared. Saturating overflow cannot fit the finite inbox cap.
Returned views are ordinary caller-owned references; retaining them after a
token expires does not retain queue capacity or create response authority.

`NativeInteractivePromptLimits::with_response_bytes` lowers a separate aggregate
ready-response budget, positive and at most the 8 MiB default. This default is
above all previously accepted question/permission responses. Canonical MCP
responses charge their raw bytes plus 64 bytes; existing question answers charge
their strings and container bookkeeping. Charges remain until consumption or
removal, including when no unanswered prompt remains. An explicitly tiny budget
may reject a UI cancel reply; dropping/cancelling the producer or closing the
inbox still removes it without a budget bypass.

The validated answer is data only. It is not browser-launch authority,
completion evidence, permission to execute a tool, or proof permitting a
continuation/resubmission. Those decisions remain separate native runtime
responsibilities. Sampling and roots have no presentation implementation here.
