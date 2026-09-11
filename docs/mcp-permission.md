# Native MCP permission preparation

The native MCP adapter composes the existing permission controller; it does not
implement a second policy engine. Core remains provider-neutral and the CLI
receives no permission or transport authority from tool names or serialized IDs.

## Exact routing and evidence

`NativeMcpPermissionPreparer` first consults the same actual retained builtin
registrations used by native target preparation. A duplicate registration is an
error. A matching allocation dispatches to the existing builtin preparer, even
when its name starts with `mcp_`. Every other name must resolve through the
explicitly injected `NativeMcpPermissionAuthority` to an immutable
`McpToolRequest`. No prefix or unknown-tool fallback grants access. `None` means
no exact MCP registration; a malformed, ambiguous or retired registration must
return an error, not a fallback route.

Hosts with live workspace contexts inject that router as well. Review and key
presentation use the exact turn's admitted primary workspace, not a mutable
manager or the startup root. A missing/retired injected scope rejects preparation
without fallback; the scope remains checked through core admission.

Runtime resolution receives the exact permission request, invocation and live
`NativeMcpTurnContext`. The adapter reserves the typed request in that context's
submission registry, which checks the capability, tool, call, arguments and
runtime binding again. HTTP argument headers cannot change the canonical
arguments reviewed by permission policy. The admitted schema and exact numeric
argument tokens are retained without a `serde_json::to_value` normalization.

Automatic review uses the real `NativePermissionContexts` snapshot, selected
source model, pending call and proven root context. Its tool action always
contains the complete admitted raw schema and sets `schema_required = true`.
Whole-schema server-authoritative validation does not waive either requirement.
The existing 16-KiB complete review-packet limit remains in force: oversized
evidence requests fail boundedly and require replanning, never schema omission,
truncation or automatic approval. Ask/Yolo can execute an otherwise admitted
request whose evidence cannot fit an automatic review packet.

## Policy and reusable decisions

The existing controller owns Ask, Auto and Yolo, ordered configured rules, saved
rules, turn/session grants, prompting and revocation. MCP annotations and risk
hints provide no unconditional bypass. Configured rules match the exact exposed
tool name and canonical argument JSON as a non-path target; remote fields do
not become native filesystem or URL authority. Configured and saved denies,
explicit asks, automatic uncertainty and Yolo preserve the controller's existing
semantics. Auto uncertainty is a replanning denial, not permission to prompt.

Saved-rule and turn/session-grant keys use the `StructuredTool` namespace and
versioned UTF-8 byte-length frames. They bind workspace, server, exposed and
remote tool names, a domain-separated digest of exact canonical arguments, and
a domain-separated SHA-256 fingerprint of separately length-framed configuration,
schema and resolved authentication binding bytes. Raw configuration and credential bytes are not
persisted in the key; arbitrary argument values are not persisted there either.
The fingerprint is an identity mechanism, not encryption, secure erasure,
remote revocation evidence or an execution token. As with exact
action keys generally, the key is not a credential store and digest secrecy is
not claimed for low-entropy inputs.

Eligibility measures the full framed identity including the original exact
argument text against the existing 4096-byte identity limit, before replacing
that text with its digest for persistence. Overflow disables saved-rule proposals
and reusable grants for that action only. It does
not truncate the identity, substitute a broader key, or prevent fresh one-shot
permission and execution. Raw schemas need not fit the identity limit because
their exact bytes participate in the runtime fingerprint.

## Lifecycle ownership

Construction and unpolled futures perform no lookup, reservation, review or
effect. At most four MCP preparations/reviews await core admission concurrently;
dropping or failing preparation returns its permit and owned reservation. After
core publication the registry's independent 64-slot exact-turn bound applies.
There is no engine/session reverse ownership cycle.

An optional peer-minted, non-clone request marker travels unchanged from the
typed request through preparation, core admission and final submission. It binds
the peer's exact allocation, not just an equal RPC ID. Peers retain only a weak
observer and can reclaim an abandoned unsent slot on their next owned operation;
marker destruction performs no callback, locking, I/O or remote cancellation.
Request IDs are never recycled. The explicit raw-ID API remains separate and
does not accept a foreign marker in place of its manually owned reservation.

The adapter checks review-source liveness before and after automatic review and
at proof binding. That authorization-only source snapshot is not misused as an
execution token after core leaves authorization. Core admission instead retains
the exact MCP turn and the concrete `NativePermissionExecutionProof`; the
existing submission machinery revalidates runtime, turn, reservation, policy and
cancellation at publication, claim and actual writer checkpoints. Queue waiting
cannot bypass later revocation. No consequential partially written request is
automatically replayed.

`close_turn` retires only the selected session/incarnation/turn, including an
already-cancelled turn whose public snapshot is unavailable. Retirement drops
proofs and wakes waiters outside router/session locks. Other turns remain live,
and the old registration remains exclusive until its owner drops it. Future
cancellation, conversation finalization and runtime replacement do not restore
old routes or grants.

This adapter is a permission/submission component. Actual profile activation,
runtime catalog publication, peer ownership, authentication and CLI control
composition remain separate native responsibilities described by the
[implementation plan](implementation-plan.md).
