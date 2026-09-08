# Native conversation ownership

`NativeConversation` owns one live core session on Linux/macOS. It coordinates
turn admission with native checkpoint finalization, while the reference host
retains provider, prompt, workspace, terminal and persistence authority. It is
not a terminal input loop or a second transcript store. Its debug and error
surfaces are redacted; returned records and events intentionally expose their
contents to the trusted host for presentation.

`create` borrows a `NativeSessionLifecycle` and accepts typed
`NativeSessionMetadata`. The first durable record contains that metadata at
revision 1; there is no empty-record/metadata-update gap. `resume` loads the
current record through the same lifecycle. `from_session` adopts a live session
after checking that it is inactive and that native metadata/checkpoint/context fields
are valid. It performs no effects and invents no missing historical facts.
These operations neither restore permission grants nor reconstruct volatile
file-undo history. The host must scope those resources independently.

## Turn admission

`prompt(Prompt, now_ms)` and `continue_turn(InferenceOptions, now_ms)` return
borrowed, inert-before-poll futures. The timestamp is an explicit host clock
observation; the owner reads no clock, environment or filesystem itself. Native
metadata validation retains its time-regression rule. Arbitrarily nested
inference metadata is destroyed iteratively even if the future is unpolled or
rejected before core takes ownership.

On first poll, native admission excludes another owner operation until the
returned stream settles or is dropped. Core's shared turn lease also excludes
competing canonical handles. Native derives the next checkpoint from the same
record revision used for `SessionTurnPreparation`. Core publishes checkpoint
metadata, the fresh turn allocator and optional user message in one exact-CAS
save, before returning a stream or invoking the provider. Unrelated metadata,
incarnation and canonical history are retained. Stale preparation is rejected,
not silently retried against a newer record.

`rename(title, now_ms)` uses the same native admission lease and the existing
exclusive title mutation. It preserves paused checkpoints and cannot enter the
gap between core completion and native finalization. A returned revision proves
title persistence, not a process-only display update.

## Durable context selection

`context_preferences` observes the reserved native context value under the same
idle admission check. `compact(now_ms)` and
`set_max_history_turns(maximum, now_ms)` are borrowed, inert-before-poll futures.
They hold native admission through an exclusive, exact-revision metadata save,
including the finalization gap of other native turns. They update the explicit
native timestamp and preserve unrelated metadata, paused checkpoints, the turn
allocator and every canonical message and archive receipt.

Manual compaction saves only the cursor of the final logical user group. That
group includes every following assistant/tool round and no-input continuation,
even if unfinished. Zero/one group or an unchanged cursor returns `false`
without a timestamp change or store write. Successful cursor publication returns
`true`; persistence errors never masquerade as a process-only successful cut.
Setting the automatic limit persists its explicit value and returns the saved
revision, even when that value is unchanged. Zero disables automatic compaction
but preserves an existing manual cursor.

Prompt and continuation admission derive the provider projection from the same
record revision as their checkpoint reservation. Core pins that selection across
the new turn and still applies its ordinary resource bounds. Missing or explicitly
zeroed preferences with no manual cut retain the original full-history path;
they do not impose new native projection bounds or tool-group requirements on
legacy core records. Nonzero selections use the bounded native context contract.
Malformed preferences, unsupported schemas, stale cursors and invalid selected
history fail before provider work or new checkpoint publication. Adoption and
idle preference observations validate selections without building summary text.

No summary is persisted as canonical history or granted instruction authority.
Failed or dropped metadata futures obey core's existing uncertain-save
reconciliation rules; observations and no-op outcomes remain canonical-memory
observations, not proof of cross-process durable state after uncertainty. Resume
restores the saved cursor and automatic limit; it does not reconstruct discarded
process-local resources. The exact grouping, summary, byte limits and remaining
typed-history integration are in [native context](conversation-context.md).

## Checkpoint schema

The native checkpoint entry is `machine_god.conversation_checkpoint`. Schema 1
has exactly four scalar fields:

| Field | Meaning |
| --- | --- |
| `schema_version` | Exactly `1` |
| `turn_sequence` | Positive sequence of the latest admitted core turn |
| `first_user_message` | Index of the user message that starts the unfinished logical group |
| `state` | `running` or `paused` |

Decoding is shallow and bounded. The sequence must be exactly one less than the
record's next allocator value. The indexed message must be a user message with
no later user message. Unknown fields, versions, states or stale indices are
errors, not repair instructions. The checkpoint contains neither duplicated
transcript text nor executable tool instructions. It is not a permission grant.

## Completion, cancellation and interruption

`NativeConversationTurn` forwards ordinary core events with their original
identity and ordering. On a terminal outcome, it releases the finished core
stream and performs an exclusive metadata save before forwarding that outcome:

- A normal non-cancelled completion removes the checkpoint. A normal output
  token limit is not treated as an interrupted recovery attempt.
- Cancellation, a failed turn or an event-stream error retains a `paused`
  checkpoint over the existing canonical evidence.
- A failed final metadata save reports a redacted failure instead of claiming
  successful native finalization. Existing assistant/tool results are not erased.

The native admission lease remains held throughout that final save, including
when core's turn lease has already been released. Thus another native prompt
cannot enter the finalization gap. Core event sinks observe core outcomes before
native finalization; they are not evidence that this extra save succeeded.

Dropping the stream cancels and drops its owned core work and any pending
metadata future. It starts no detached finalizer. The previously published
checkpoint remains available for recovery, possibly still marked `running`.
Dropping the conversation owner does not invalidate a real returned turn handle;
that turn retains its own session/resource lifetime until it settles or drops.

Persistence failures can follow publication. Core records uncertain prepared or
metadata saves and requires authoritative reconciliation before later mutation.
The host must resume/reconcile after an ambiguous result before describing its
final durable state. A stale prepared native retry can return `Conflict` after
that reload rather than overwrite newer state. `record` and `paused_turn` are
observations of current canonical in-memory state, not cross-process snapshots.

## Explicit continuation

`paused_turn` is unavailable while the native owner or core session is busy.
Otherwise it observes a valid checkpoint, including a `running` checkpoint left
after interrupted work, and reports whether the retained group includes unknown
tool-result markers. It invokes no provider, tool or permission handler.

`continue_turn` requires that checkpoint. It atomically replaces the checkpoint
with a fresh sequence while reserving a new core turn, preserves the original
user message and all later confirmed/unknown tool evidence, and does not append
another copy of the prompt. The explicit operation starts with the ordinary
fresh core per-turn budget and host-supplied inference options. It neither
automatically retries providers nor persists/restores provider retry budgets.
Those retry and model-preference policies belong to the surrounding native host.

Historical tool calls are never dispatched by continuation. Only new provider
requests can invoke tools, through the current tool catalog and fresh permission
decisions. Unknown results remain unknown: a lost receipt is not proof that an
effect did not execute. The UI must present that distinction before the user
chooses to continue. Full history remains available through `record` and the
ordinary store/archive readers.

See [core API](core-api.md) for prepared-turn admission and canonical leases,
[native lifecycle](native-session-lifecycle.md) for initial publication and
resume, and [file undo](file-undo.md) for separately scoped filesystem inverses.
