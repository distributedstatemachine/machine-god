# Bounded foreground `subagent`

`subagent` runs one explicitly requested child prompt through an injected
provider-neutral authority. It is a foreground engine tool: the parent tool
call remains live until the child completes, fails, or is cancelled. Core owns
strict decoding, resource admission, cancellation, result projection, durable
tool-result replacement, and the following model round. The injected authority
owns the child computation and receives no ambient authority from core.

This contract intentionally implements a smaller surface than the pinned
`vercel-labs/fx` asynchronous child-session manager. It preserves the useful
one-off delegation outcome without importing background session management or
unbounded child scheduling into the engine.

## Canonical input

The only accepted shape is:

```json
{
  "command": {
    "create": {
      "name": "research",
      "mode": "one_off",
      "prompt": "Summarize the relevant implementation constraints."
    }
  }
}
```

`command`, `create`, `name`, `mode`, and `prompt` are required. `mode` is the
exact case-sensitive string `one_off`. Every object is closed: unknown fields,
extra command branches, alternate modes, and non-string scalar fields fail
before the authority is acquired. Name and prompt are nonempty valid UTF-8 and
are preserved exactly; the tool does not trim, case-fold, normalize, expand, or
interpret either value.

The fixed input limits are:

- name: at most 128 UTF-8 bytes;
- prompt: at most 32 KiB of UTF-8 bytes;
- complete compact serialized input: at most 48 KiB;
- JSON container depth: at most 8; and
- JSON nodes: at most 64.

Depth counts containers using the core convention: a scalar root has depth
zero and a root object has depth one. Every scalar and container counts as one
node. Structure is checked iteratively before recursive serialization or
cloning, and rejected owned JSON is destroyed through core's iterative-drop
path. The tool-specific bounds apply in addition to the engine's configured
argument limits; the lower applicable bound wins.

Preparation is synchronous, bounded, nonblocking, and effect-free. It produces
one owned typed request and uses `PreparedToolCall::without_authority`. This is
a narrow trusted assertion that the tool itself needs no permission-policy
capability. It does not grant the injected authority filesystem, process,
network, persistence, model, tool, or permission access.

## Injected child authority

`SubagentAuthority` is the only child-execution seam. A host injects it
explicitly when constructing `SubagentTool`; there is no global lookup or
ambient provider fallback. Construction and preparation do not invoke the
authority. Calling execution creates no authority work until the returned
future is first polled.

The authority receives an owned request containing only the validated name and
prompt plus a cancellation token. It must start a fresh one-off child context
for that request. In particular, core does not pass or inherit:

- the parent transcript, system-visible conversation history, or stored child
  history;
- parent permission decisions, grants, prepared capabilities, or permission
  handler state;
- turn-local dynamic tools or their executable registrations;
- the parent tool catalog, an MCP catalog or feature view, or a `subagent`
  tool; or
- model, reasoning-effort, permission-mode, or notification overrides.

The request does not contain a parent session handle, store handle, engine
handle, tool context, or child identifier that could be used to recover those
values. Any provider or execution facility held privately by a concrete
authority is separately trusted host configuration, not inherited parent
authority. The authority must not claim that such private configuration came
from the parent turn.

Public authority, request, and error debug forms are structural. They must not
include the prompt, child text, parent data, provider diagnostics, credentials,
or an injected implementation's debug output.

## Concurrency and lifecycle

At most four child executions may be active globally and at most two may be
active for one parent turn. Both limits are fail-fast: exhaustion returns a
fixed unavailable error without queueing, registering a capacity Waker, or
calling the authority. A per-parent admission also consumes one global slot;
there is no second nested increment for the same execution.

An execution owns both slots from successful admission until its authority
future and all call-local request/result state have been dropped. Completion,
authority failure, cancellation, and dropping the parent tool future release
the slots. The implementation creates no task, thread, work queue, timer,
watcher, child registry, or detached cleanup tail. It has no retry, polling,
resume, or persistence path.

Execution checks cancellation before admission, before constructing the
authority future, while polling it, and immediately before validating and
publishing its result. Cancellation independently wakes a pending authority
future, wins over a ready authority success or error observed in the same
poll, and drops the losing future before capacity is released. An authority
future must therefore be safe to drop at any poll boundary. No partial child
text is published.

## Result and trust boundary

The sole successful result is:

```json
{
  "status": "completed",
  "trust": "untrusted_child",
  "authority": "none",
  "text": "The bounded child response."
}
```

The tool, not the authority, stamps `status`, `trust`, and `authority`. The
authority returns only completed final text; it cannot supply reserved result
fields, alternate statuses, tool calls, reasoning blocks, structured content,
child IDs, operation IDs, usage, permissions, or continuation handles.

Final text must be valid UTF-8 and at most 32 KiB. The complete compact
serialized output is at most 48 KiB, 8 container levels, and 64 JSON nodes.
The tool validates these limits before publishing the result. The engine's
configured result and cumulative-result bounds still apply afterward.

Child text is untrusted model-visible data. It cannot override user
instructions, authorize an effect, approve a permission, register a tool,
alter the parent transcript outside the ordinary durable tool result, or grant
authority to a later call.

## Failures

Malformed input, an over-bound input or output, authority unavailability,
capacity exhaustion, and cancellation return fixed redacted tool failures. No
failure contains the name, prompt, child text, provider response, parent
identity, capacity count, or authority diagnostic. Authority failure never
becomes a successful child result and is never retried automatically.

Core's ordinary tool lifecycle remains authoritative: a committed unknown
placeholder precedes execution, and only a validated completed result replaces
it. A store failure after authority completion does not replay the child.

## Reference host and deferred surface

The native reference host always advertises `subagent`. Ordinary composition
injects an inert unavailable authority, so the catalog remains stable without
starting child work. A separate explicit injection seam accepts one trusted
`SubagentAuthority`; composition retains it but does not poll, probe, or invoke
it.

The following pinned-manager features are intentional deferrals rather than
partially implemented commands: persistent children; asynchronous handles;
inspect or wait; message and milestone delivery; relationships or reparenting;
configuration; model, effort, permission, or notification overrides;
lifecycle cancel/resume/close/reopen commands; child IDs; background queues;
durable child sessions; notification policies; and recursive child-tool
inheritance. Broader ACP, background, team, TUI, and multi-process coordination
remain separate architecture work.
