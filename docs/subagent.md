# Managed `subagent`

The provider-neutral tool validates complete managed-agent commands and results.
An explicitly injected `ManagedSubagentAuthority` owns native admission,
durable acceptance, scheduling, persistence and lifetime. Core performs no
ambient filesystem, process, environment, network, clock or thread operations.
The foreground-only API and its global counters are removed.

## Commands

The root is `{"command":{...}}`, selecting exactly one of these branches.
Every object rejects unknown fields. Optional fields are omitted, not null.

| Branch | Fields |
| --- | --- |
| `create` | Required `name`, `mode` (`one_off` or `persistent`); `prompt` required for one-off. Optional `model`, `effort`, `permission_mode`, `notifications`. |
| `inspect` | Required `id`, nonempty distinct `sections`; optional `cursor`, `limit`, `wait`. |
| `message` | Exactly `send:{id,content}` or `milestone:{name}`. A milestone belongs to the actual calling child/work, not a supplied target ID. |
| `relationship` | Required `action`, `id`; optional `parent_id`. Attach defaults to actual actor; detach forbids parent; reparent requires parent. |
| `configure` | Required `id` plus at least one of `name`, `model`, `effort`, `permission_mode`, `notifications`. |
| `lifecycle` | Required `id`, `action`: cancel, resume, close or reopen. |

Names/milestones are 1–128 UTF-8 bytes; model 1–256; prompt/message 1–65,536.
NUL is rejected. IDs contain 1–255 ASCII alphanumeric, dot, underscore or hyphen
characters, excluding dot and double-dot. Effort is `auto` or a 1–64-byte name
using ASCII alphanumeric, dot, underscore or hyphen. Explicit model/effort wins;
otherwise native uses admitted selected preferences.

Permission modes are ask/auto/yolo. Omission remains explicit `None` in the DTO:
native inherits the admitted parent policy and enforces same-or-stricter policy.
There is no decoder-default yolo authority. Children receive standalone prompts,
not inherited parent transcripts or grants.

Notifications default terminal completed/failed/cancelled on, started off,
milestones empty and stop conditions `[terminal]`. At most 32 distinct
milestones and eight distinct stop conditions are accepted. Conditions are
terminal/duration_elapsed. Positive signed-millisecond interval/duration values
are checked; duration requires interval and adds duration_elapsed if absent.
Native must also check deadline addition against its actual clock.
The `started` boolean is a deliberate modern extension to the pinned tool.
Each accepted work item freezes its policy; native notices feed the parent's
next-turn context, never an automatic idle turn or active-turn injection.

## Inspection and results

Sections are status, messages, tool_activity, events, configuration and
relationship. There is no additional history or list command. Messages includes
conversation history. Limits default to 50 and range from 1 to 100.
The complete selected page contains at most 100 message/history/event/tool items.
`v1:generation:offset` cursors use canonical checked unsigned integers; native
validates the exact generation and projection and reports gaps/restart-required.
Retained history and tool activity are pageable, not pinned first-page-only
projections. Retention is not a lifetime child-creation limit.

Wait requires `until:"settled"`, timeout_ms from 1 to 60,000, the status section
and no cursor; optional after_generation requires a strictly later generation.
Idle/interrupted/completed/failed/cancelled/archived are settled;
queued/running/awaiting_approval are not. Timeout returns an inspection with
wait_timed_out status, not durable cancellation. Native owns dependency-wait
admission, cycle detection, released execution quota and bounded waiters.

`ManagedSubagentResult` preserves the envelope: ok, operation_id, child_id,
status, error_code, retryable, requested and cursor. Status/error codes are closed
typed tags, never raw host diagnostics. Requested data is a receipt, inspection
or relationship approval. Receipt outcomes are created, message_queued,
relationship_changed, configured, lifecycle_changed and milestone_emitted;
they are not completed child answers. Inspection includes state/configuration/
relationships, messages/history/events/tool activity, generation/cursor,
truncation/gap and bounded source-error projections. Private policy and inherited
root-user evidence are not public message fields. Child text remains untrusted.

Tool arguments are bounded to 448 KiB serialized, 12 container levels and
512 JSON nodes, including worst-case escaping of the full prompt. Output is
bounded to 512 KiB; history fields are at most 16 KiB each and 32 KiB together. Native composition
must provide per-tool complete input/output publication when ordinary engine
transcript bounds are smaller; unrelated tool limits must not be enlarged.
Core checks JSON bounds before recursive decode and destroys rejected deep JSON
iteratively.

## Actual invocation and native authority

Public `ToolContext` IDs grant no authority. Core supplies an
`AdmittedToolInvocation` only through `Tool::execute_admitted`, after the exact
prepared capability and optional final permission admission succeed.
Its immutable arguments, registered name and private call allocation bind the
actual invocation. `SubagentTool` constructs `ManagedSubagentInvocation` only
from this envelope; direct structural execution fails closed.

`Session::witness()` and `Turn::witness()` expose weak opaque allocation
identities, without constructors or deserialization. Native registers an actual
principal against the session witness before execution and owns a turn-scoped
registration. It may route using IDs but must verify actual session/turn identity,
liveness, principal generation and same-or-stricter policy. Calling
`ManagedSubagentInvocation::claim(&TurnWitness)` consumes a unique call claim
once. Foreign/stale witnesses and repeat claims fail. Provider call IDs are not
claim identity. Proofs retain neither session nor runtime ownership.

Wrappers must forward the admitted envelope unchanged. The ordinary default
adapter discards its proof when invoking structural tools; it cannot be used
around a managed tool. Native privately constructs its admitted principal/run
lease only after checking the witness and authority. Core identity alone is
never a native permission or resource grant.

## Cancellation and durable ownership

Create/message acceptance is durable before execution; user cancellation is
durable before signalling. Native serializes each child FIFO and explicitly
resolves interrupted/approval-blocked heads. Restart never implicitly executes
interrupted work. Accepted children are owned by the outer native manager and
outlive the creating tool future/turn.

Mutating tool submissions use completion-wins-after-first-poll: core observes
their real result and persistence before a pending turn cancellation. Native
still owns irreversible settlement if that future is abandoned. Inspect/wait
remain normally cancellable. Submission/wait cancellation and host teardown
are not durable user cancellation; only an admitted lifecycle command requests
that mutation. Persistent cancellation returns idle; one-off cancellation is
terminal. Close archives/settles, not deletes; reopen does not implicitly retry.
The manager owns aggregate scheduling, residency, queue, byte and waiter budgets;
core has no foreground admission counters or detached worker loop.

## Native scheduling and actual settlement

The shared native scheduler validates independent execution, resident and waiter
limits. Defaults are 4 executing runs, 64 resident principals and 64 queued or
dependency-waiting runs. All limits are positive, execution cannot exceed
residency, and hard ceilings are 256 executions and 4,096 residents/waiters.
These are live-resource budgets, not lifetime creation or durable-history caps.
Ordinary provider `Pending` retains execution capacity. Acquisition and
reacquisition use one FIFO queue; a grant reserves capacity before its future is
observed, and new work cannot bypass existing queued work.

Only the native manager registers an actual core turn against a non-clone
resident lease and a strictly increasing work generation. Opaque weak run
references cannot be constructed from public IDs and retain no manager/runtime.
Private managed-call admission must bind the actual turn and generation before
using a run reference. Register before polling the provider, acquire before
execution, and never use a principal residency epoch as a work generation.

A dependency wait targets an exact registered run. Self-dependencies, cycles,
foreign or unregistered targets and exhausted waiter capacity fail before quota
release. Under one short lock, the scheduler reserves a waiter and records the
dependency before releasing caller execution capacity. Observation success,
error and timeout all queue for fair reacquisition before the tool returns;
their outputs remain private until then. Cancellation or abandonment unlinks
the exact waiter/grant and cancels the actual caller turn instead of permitting
quota-free continuation. An unpolled wait is inert. No generic pending future
is interpreted as a dependency, and no thread/task/timer is created per child.
Native observation futures must likewise use weak manager/runtime access;
wrapping an owning future does not erase its ownership.

Execution completion and actual worker settlement have separate non-clone
owners. Finishing, cancellation or run-owner drop releases execution capacity
and enters a bounded settlement lane, which progresses even when all execution
slots are occupied. Only the actual finalizer may complete its settlement owner
after turn/worker/TLS/process-reap obligations finish. Dropping a response
observer proves nothing about settlement. Dropping the settlement owner without
completion quarantines its resident slot; dropping a resident with unsettled
work does not make that capacity reusable. A settled resident can admit its next
FIFO generation; a retired settled resident releases its capacity.

Registry locks never cover asynchronous waits, cancellation callbacks or caller
waker clone/wake/drop operations. Cancellation and removed-value destruction
run after unlocking. This scheduler supplies admission and lifecycle primitives;
native manager composition owns durable acceptance, deadlines, scheduling polls
and actual finalizer custody.

## Source evidence

The pinned FX decoder is `src/tools/agent/subagent.zig`; typed validation and
bounds are in `src/core/subagent/domain.zig`. Result envelopes are in
`tool_result.zig`, encoding in `tool_host.zig` and inspection projections in
`manager.zig` under that subagent directory. The Rust contract intentionally
adds started notifications, complete durable history/tool paging, allocation-bound
identity and checked clock-compatible durations without retaining historical
operation-identity or foreground compatibility paths.
