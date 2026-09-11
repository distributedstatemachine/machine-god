# MCP conversation routing

`mcp::context::NativeMcpContexts` routes tool and permission lookup data to an
already admitted native conversation turn. It performs no configuration reads,
server startup, network or process work, or permission decisions.

## Ownership and admission

An explicitly injected router contains at most 64 weak session routes. Only
`NativeConversation::with_mcp_contexts` enrolls its actual core `Session`; raw
`ToolContext` and `PermissionRequest` values cannot register sessions or turns.
Duplicate live session/incarnation pairs and capacity overflow are rejected.
Dead owners release their slots. Neither the router nor its session-route owners
retain a core session or engine, so an engine-owned tool may retain the router
without creating an ownership cycle.

Immediately after core prompt or continuation admission returns its real `Turn`,
the native conversation registers that exact session/turn with
`McpSubmissionRegistry`, before provider polling or later route enrollment.
The returned non-clone registration belongs to `NativeConversationTurn`.
Admission failure unwinds it along with other acquired route/turn ownership and
does not return a usable native turn. Conversations without MCP injection retain
their existing behavior; constructing or dropping an unpolled prompt stays inert.

## Lookup and retirement

Tool and permission lookups match the full session ID, session incarnation and
turn ID. They reject missing, foreign, unpublished, retired and cancelled turns;
there is no current-session fallback. Call IDs remain input to the submission
registry's exact preparation checks, not a way to enroll or authorize a call.

Snapshots expose the exact shared registry only after revalidation and provide an
owned cancellation observer. Holding a snapshot, registry or observer does not
hold the native registration, session or engine alive. The registry independently
revalidates its scope before admission and effects; lookup itself grants nothing.

Routes are unpublished before durable terminal checkpoint finalization, normal
finish, turn drop, conversation drop and host lifecycle retirement. Actual core
turn cancellation invalidates lookup immediately. Retirement cancels observers
and drops submission state outside routing locks, permitting reentrant wakeups.
It does not claim that remotely accepted work was revoked.

Allocation identity prevents an old registration or retired owner from removing
a replacement route. A route also rejects re-enrollment of its last core turn.
As with core, session incarnations must be globally unique for their logical
lifetimes and turn sequences must not be rolled back. Arbitrary historical reuse
of every serialized identifier cannot be distinguished from fresh lookup data;
the router does not retain an unbounded tombstone history to accommodate it.

Transport/runtime generation ownership, exact submission proof binding, session
negotiation, catalog publication and execution remain separate responsibilities.
See [submission ownership](mcp-submission.md) and the canonical
[implementation plan](implementation-plan.md) for the integration boundary and
required feature gates.
