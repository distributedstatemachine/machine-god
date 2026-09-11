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
after checking that it is inactive and that native metadata, checkpoint, context
and model preference fields are valid. It performs no effects and invents no
missing historical facts.
These operations neither restore permission grants nor reconstruct volatile
file-undo history. The host must scope those resources independently.

### Clipboard reply observation

Conversation and runtime `record_snapshot()` wrappers retain the core's immutable
`Arc<SessionRecord>` without cloning transcript or metadata payloads. This is a
canonical-memory observation, not a new store receipt or uncertain-write
reconciliation. Later commits do not retarget an already captured snapshot.

`NativeClipboardReplySelection` selects from one such snapshot without effects.
Construction does not scan history. Each `next_step` visits at most 256
message/block positions and incrementally copies at most 4,096 text bytes. It
chooses the newest assistant message with no tool-call blocks and nonempty text,
concatenating all text blocks in original order with no separators or rendering.
Whitespace, Markdown and control bytes remain exact clipboard data. Other roles,
JSON and tool payloads are not reply text. Group lifecycle state does not define
reply kind: a canonically saved final assistant remains eligible even when later
native metadata finalization fails and recovery marks its group interrupted.

Eligibility and the `MAX_FILE_SESSION_BYTES` payload cap (8,651,165 bytes) are
checked before allocating reply text. An oversized latest eligible reply returns
`ResourceLimit`, never an older fallback. The final safe conversion to `Arc<str>`
separately copies at most that cap once, with at most twice the cap in logical
owned payload during conversion, excluding allocator overhead and the retained
snapshot. `Progress`, `Selected(Arc<str>)`, `Empty` and resource failure are
explicit; terminal results are fused and debug output is redacted. Selection
itself never invokes a clipboard backend, model, tool or metadata writer.

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

## Checkpoint-bound skill context

`prompt_with_skill_context(prompt, context, optional_model, now_ms)` accepts
already materialized `NativeSkillPromptContext` from an explicitly composed
native host. Construction validates at most 65,536 UTF-8 text bytes; it does not
discover, read or authorize a skill. The borrowed admission future is inert
before polling. The optional model has the same atomic snapshot semantics as
`prompt_with_model`.

Core projects the context in its fixed untrusted-advisory wrapper as a separate
text block on this exact latest user message for every provider round.
Canonical prompt text and permission
provenance remain unchanged. Native stores the inert text atomically with the
checkpoint under `machine_god.skill_prompt_context`: exactly `schema_version`
(`1`), `turn_sequence`, `first_user_message`, and `text`. The latter two identity
fields must match the validated checkpoint; absent legacy metadata means no
context. Unknown fields, malformed values, oversized text, orphaned context,
and mismatched attempt/user identities fail validation before provider work or
new publication. Validation checks shape and length before copying context.

Explicit continuation copies the admitted bytes, updates the attempt identity
in the same reservation, and never rescans a skill source. Cancelled, failed or
dropped turns retain the context alongside recovery evidence. Normal completion
removes both checkpoint and context in one finalization save. Every new prompt
replaces or clears the preceding context, including when abandoning paused work.
Model changes, rename and compaction do not alter these inert bytes. They confer
no path, process, filesystem or permission authority after restart.

The 65,536-byte text ceiling is not a guarantee that a composed turn fits:
core independently checks total serialized session metadata (256 KiB by default,
including JSON escaping and unrelated entries) and provider payload limits.
Rejected or uncertain saves retain the existing reconciliation semantics;
there is no separate context writer, destructor save or automatic retry.

## Durable model preferences and job snapshots

`model_preferences` observes saved requested settings under idle admission;
missing historical settings remain `None`. `set_model_preferences(value, now_ms)`
is borrowed and inert before polling. It holds admission through one exact-CAS
metadata save, updates the native timestamp, and preserves checkpoints, context,
canonical history, incarnation, allocator and unrelated metadata. It returns the
saved revision, including when explicitly saving an unchanged value. Malformed
saved preferences and time regression fail without overwriting prior state.

`prompt_with_model(prompt, snapshot, now_ms)` and
`continue_turn_with_model(options, snapshot, now_ms)` publish the snapshot's
requested preferences in the same reservation as the checkpoint, fresh turn
allocator and optional user input. There is no preferences/input publication
gap. The immutable [model snapshot](model-preferences.md) installs its effective
controls in core inference options, pinned across every provider/tool round.
Continuation still requires a checkpoint and appends no duplicate prompt; its
explicit snapshot represents the newly admitted job's current selection.

The original `prompt` and `continue_turn` methods keep caller-supplied inference
options and do not infer defaults or silently apply saved preferences. Both
paths reject malformed saved preferences before admitting provider work.
Resume validates and exposes saved preferences; applying process overrides is
the runtime owner's responsibility, not an inferred history mutation.

These APIs persist only the session. `NativeConversationRuntime` accepts
queued/future preference changes while a job is active, then flushes session
preferences under idle admission; `Busy` is not a successful deferred save.
User-default settings are a separate write target with an independent outcome.
No API bypasses core's active lease by writing the underlying store directly.
Failure, dropped-save uncertainty and cross-process conflicts use the same core
reconciliation contract as other native metadata mutations.

## Native queue and selection runtime

### Exact-turn workspace scopes

An explicitly bound conversation shares the host's workspace authority manager
and `NativeWorkspaceContexts`, independently of permission-controller or review
composition. Construction only registers bounded process-local state. On first
polled admission, direct prompts and continuations capture an immutable scope;
the runtime captures it when taking a queued job, alongside that job's model and
policy selection. Enqueueing and constructing an unpolled start future capture
nothing. Later workspace publication affects future taken turns, never an
already taken turn or its subsequent tool rounds.

The registry retains at most 64 weak exact session/incarnation routes, with one
live turn registration each. Tool preparation and permission preparation can
look up the same pinned scope using their actual core turn context, including
before permission authorization begins. Lookup performs no filesystem access,
descriptor duplication, or mutable-manager read. Matching IDs selects native
workspace context only: it proves neither user provenance nor policy approval.
Tools still require their ordinary preparation, authorization, cancellation, and
descriptor-relative descendant checks. Unbound legacy conversations retain
their existing behavior; no ambient workspace is inferred.

The returned native turn owns registration through its active work. Completion,
cancellation, error, drop, and lifecycle retirement make old lookups unavailable;
retained context handles cannot revive registration. Explicit continuation gets
a fresh turn identity and a newly captured scope. No descriptor or scope is
written into core state or session metadata. Failed or abandoned startup cannot
leave a live registered route.

The native workspace-control lease rejects active or queued runtime work and
also retains conversation admission, fencing direct prompt/continuation aliases.
The actual owner keeps this opaque lease through publication and worker cleanup;
quiescence cannot settle until it is released. The lease itself performs no I/O
and does not spawn work. It does not imply that CLI or tool routing is composed.

### Queue and model selection

`NativeConversationRuntime` owns one conversation, a FIFO of pending inputs and
the current requested model preferences. Construction reads validated canonical
memory only. Saved preferences replace ordinary startup defaults; an explicitly
provided process model override changes only the restored model, preserving
saved effort and fast requests. Absent historical preferences stay absent until
an explicit flush or job admission saves them. No worker is spawned.

`enqueue(Prompt)` validates and owns input without calling a provider or writing
the store. Pending inputs share the current runtime selection, so
`set_model_preferences` updates all pending/future jobs in constant queue-length
time. It returns an acceptance generation, not a persistence receipt. On first
poll, `start_next(now_ms)` takes one FIFO entry and copies current preferences
and explicitly supplied catalog capabilities into an immutable model snapshot.
Later preference or catalog changes cannot alter that taken job. Unsupported or
missing capabilities do not infer controls from a model ID. Hosts must supply a
completed catalog observation, not a retained value from a still-loading cache.

Admission uses `prompt_with_model` or `continue_turn_with_model`; requested
preferences are saved atomically with the checkpoint and effective controls reach
every primary provider round. Other per-input inference options survive.
An unpolled start future leaves the queue unchanged. Once taken, failed/dropped
admission is never automatically requeued: a save may have published. The runtime
marks model persistence uncertain until a reservation or explicit flush succeeds.
The returned `NativeConversationRuntimeTurn` retains runtime admission through
native finalization and drops native/core work before releasing that lease.
Dropping the runtime clears pending inputs but does not invalidate a separately
owned active turn. `cancel_queued` and `clear_queued` affect pending input only.

### Quiescence and irreversible retirement

`begin_quiescence()` synchronously fences new runtime work and requests active
turn cancellation. It returns an owned, non-cloneable `NativeRuntimeQuiescence`;
another simultaneous fence is rejected. Runtime status distinguishes `Open`,
`Quiescing`, and `Retired`. Queue insertion, model/catalog mutations, and
first-polled persistence/admission futures reject a closed fence. During
quiescence or retirement, `cancel_queued` returns false and `clear_queued` returns
zero without changing queued input. Read-only record and model observations
remain available. `set_model_catalog` now reports admission failure explicitly.

The guard's inert `wait_idle(&mut self)` installs at most one bounded waiter.
It waits for owned operations to finish or drop, including native checkpoint
finalization, confirmed rule edits, and the entire independent user-default save.
The shared gate admits at most 256 overlapping permits; the limit rejects new
work without evicting existing ownership. Cancellation and waiter callbacks run
outside state locks. No worker is detached and the guard does not poll another
owner's turn. The caller must keep driving pending admission and the active turn.
A reservation completing after the fence is cancelled before provider entry.

The guard's synchronous `selection_snapshot()` rejects outstanding permits with
`Busy`. Once drained, it copies actual current model preferences, shares the
current catalog, and captures the current permission mode, sandbox preference
and configured rules through the exact bound guard. It reads settled state, not
a pre-fence cache; already-admitted saves and selection changes must settle
first. Public policy controls remain closed. This bounded, redacted snapshot
contains no grants, saved exact rules, history or persistence receipt, and does
not revive routes. Stale or retired guard ownership cannot obtain a snapshot.

Idle means ownership release, not a successful persistence receipt: dropping a
turn can release a pending, publication-uncertain finalizer. An interactive
transition must handle its native terminal/persistence outcome separately.
Dropping an uncommitted guard reopens only its exact generation, preserving
queued inputs and accepted settings; it cannot undo cancellation already sent.
Keep one guard while replacing a pending selection with a newer request.

After the candidate has been prepared and the current work settled, consuming
`retire()` requires zero outstanding permits, permanently closes admission,
detaches the old native/model routes, and discards only never-taken queued input.
Old runtime and permission aliases cannot regain mutation or admission authority.
Returning to the same session may register fresh routes while old aliases still
exist; dropping an old registration cannot remove its replacement. A failed busy
consuming retirement releases its guard and preserves queued input. The borrowed
`try_retire(&mut self)` instead retains its fence on failure, allowing a transition
owner to retain confirmed receipts and keep admission closed while handling an
outstanding operation. Success is irreversible under either API; a retained
retired guard cannot retire again or reopen admission when dropped. Retirement does not
delete canonical history, flush uncertain observations, close a shared engine,
or revoke independently supplied core session handles.

### Interactive session ownership

`NativeInteractiveSession::open` creates the initial runtime through the exact
reference host. It requires that host's complete terminal, undo, model-route and
observation allocations, and checks the explicit workspace against its retained
canonical workspace label. Workspace defaults remain separate from saved session
preferences. Startup-only model override does not become a fresh-session default.

The owner retains admission, native turn, candidate preparation and terminal
commit futures. `poll_progress(cx, now_ms)` supplies explicit time and never
consumes a result: `take_presentation` and `take_outcome` are separate bounded
lanes. One presentation event may pause ordinary streaming. Transition/shutdown
draining discards obsolete intermediate presentation and continues native
finalization without waiting for stdout; terminal errors and receipts remain
retained control outcomes. A dropped outer poll wrapper does not drop started
work. Work per poll is bounded to 32 progress steps.

Clear/new/reset generate fresh persisted IDs. Clear/new carry terminal resources;
reset and live resume stop/forget first, then hand off explicitly retained
indeterminate resources. Resume selects latest or an exact validated ID and
adopts the candidate before old terminal effects. Exact same-principal resume is
a checked no-op preserving runtime identity, queue and undo. Fresh runtimes use
explicit workspace model defaults; resume restores candidate preferences. The
settled current permission selection and shared catalog carry without live grants.

Requests coalesce until terminal work first polls. Already-started preparation
still settles and reports superseded candidate publication; it is not rolled
back. Later requests wait behind a started terminal commit's receipt. Quiescence
cancels current work and drains its actual native finalizer. Selection,
composition or undo-reservation failure preserves old queue/settings/undo;
already requested cancellation is not undone. All fallible composition and undo
reservation precede old terminal effects; undo clears only after confirmed
terminal success and exact old-route retirement.

Uncertain terminal results or failures after a reset receipt retain a fenced
candidate, old guard, undo reservation and reset receipt. They never reopen old
admission or automatically replay reset. Shutdown drives owned work, retires
routes, and preserves unconsumed errors; `shutdown_error` distinguishes failed
shutdown from `is_closed`. The caller then drops the owner and its actual host
handles once, using the host's existing completion observer. Dropping the owner
without shutdown is abandonment, not a cleanup or persistence receipt.

The interactive owner also retains one explicitly requested durable control and
one independent `take_control_outcome` receipt. Rename, compact, session model
save, session-plus-user-default model save and explicitly confirmed permission
rule changes call the existing runtime/controller operations on the captured
exact runtime. They remain inert until progress polls them. Rename retains only
the existing validated, trimmed 240-byte title. Continue instead performs the
existing bounded synchronous empty-queue/checkpoint admission and immediately
returns a queued-job receipt; it does not claim provider work or persistence.
Control IDs are checked before queue mutation and never wrap.

`UndoLast` uses the exact host-injected file tracker on the existing host-owned
terminal/archive worker scope. It is admitted during active turns without
cancelling them, and retains exact-runtime lifecycle admission inside the worker
through inverse execution. No filesystem inverse runs on the polling thread.
The owned control survives dropped outer polling wrappers, blocked presentation,
transition requests, cancellation and shutdown. The typed `Undone` receipt keeps
`Empty`, `Restored` and `Removed` distinct; `Undo` errors preserve changed targets,
busy/unavailable admission, non-undoable preimages and ambiguous publication.
Neither success nor failure rewrites canonical messages or invokes a model/tool.
Response readiness confirms the inverse result, not worker-thread destruction;
the host's existing completion observer separately joins all enrolled work.
Dropping the owner itself abandons its receipt but cannot release an executing
worker's lifecycle permit. Ambiguous results are never replayed automatically.

`Skills` controls retain the exact host-selected `NativeSkillsService`, captured
session working directory and a private cancellation token. Synchronous catalog
and managed effects run only in the host-owned worker, retaining a file-control
lifecycle permit through actual completion even if the response future is
dropped. Receipt readiness and worker-thread completion remain separate. Direct
enum commands are bounded before retention and domain-validated before effects.
Catalog, managed-path and per-item mutation receipts remain typed; partial,
rolled-back, unattempted and uncertain batches are failures, never empty success.
Show does not materialize skill bodies. Recovery IDs survive managed errors.
Cancellation signals the owned command without dropping its result or inferring
rollback; controls and shutdown retain the existing receipt-settlement rules.

Accepted controls settle before quiescence, cancellation or advancement of an
actual turn. In particular, an active-turn permission editor remains live until
its confirmed save settles; a later transition or shutdown cannot invalidate it
by closing the turn first. The owner does not reopen a deactivated human prompt
or infer confirmation from a proposal. Input/presentation wait cancellation must
not be connected to an already explicitly confirmed control's publication.

The independent control lane progresses even with occupied presentation. Its
unread receipt blocks new queued-prompt admission, not transition/finalizer or
shutdown progress. New controls require both the prior control receipt and prior
lifecycle outcome to be consumed, and are rejected while switching/shutting down.
An accepted save is retained through dropped outer polling wrappers. It is not
superseded by a newer transition. Failure or partial model-target failure rejects
the latest pending transition before undo reservation or terminal effects; the
exact control receipt and rejected-transition outcome remain separate. A failed
unread control receipt also rejects later transition requests. Shutdown still
settles native work and preserves that receipt without claiming rollback.

`set_model_preferences` on the interactive owner is a synchronous selection
change returning an accepted generation, not a persistence receipt. It shares
control admission checks. Session save outcomes preserve `Unchanged`, `Deferred`
and `Saved`; composite model saves preserve both independent target results.
Failed or uncertain publications are never retried automatically or described as
unsaved merely because the caller received an error. Direct retained runtime
aliases retain their separate runtime contract until actual quiescence begins.

`request_cancel` retains cancellation of the currently owned admission/turn;
it does not replace the session or discard untaken queued input. An already
accepted control finishes first, preserving its active-turn metadata editor.
The owner then cancels the actual turn handle and drains its finalization even
with occupied presentation. Acceptance is not a settled cancellation receipt;
the ordinary typed turn outcome remains authoritative.

### Runtime controls and persistence

Runtime `rename`, `compact`, and `set_max_history_turns` expose the conversation's
existing durable operations through the same admission lease as queued jobs and
preference saves. They are borrowed, inert-before-poll futures. A pending save
excludes job admission and other durable controls through completion or drop;
native exact-revision and uncertain-publication semantics remain unchanged.
Queued inputs are retained and use the newly saved context when later admitted.
Selection changes and queue edits remain available while publication is pending;
these controls neither persist nor clear the dirty model-preference generation.
`context_preferences` and `paused_turn` are idle observations of canonical state,
not cross-process snapshots or permission to replay historical effects. All five
operations reject active runtime work, including the native finalization gap.
The same admission rule covers `history`, `record_history_file`,
`flush_history_observations`, and `set_history_background`. Their durable observations preserve queued inputs and
pending model selection; an observation save is not a model-preference flush.

`enqueue_continuation` requires an idle, empty queue and a valid paused checkpoint.
It rechecks the empty queue after checkpoint observation, and the taken job
rechecks that checkpoint's sequence. A racing prompt or changed checkpoint
cannot silently redirect the continuation. It captures current selection when
taken, not settings from the interrupted attempt. Recovery route/fast-downgrade
and attempt-budget policy remain separate from this queue's ownership.

`flush_model_preferences(now_ms)` is inert before polling and performs at most
one idle session save. `Unchanged` means the canonical in-memory generation was
already saved; `Deferred` means dirty settings could not be saved while runtime
work is active. `Saved` names the captured generation and exact session revision.
A selection accepted during the save remains current and pending, even when the
older save succeeds. Failure/drop never rolls back accepted runtime settings or
claims successful persistence. A subsequent job admission saves its current
generation; when no job follows, the host must explicitly flush after settling
or dropping the active turn. This is not a user-default settings write and does
not claim cross-process state following an ambiguous core persistence outcome.

`persist_model_preferences(user_store, now_ms)` explicitly attempts both targets
for one runtime generation. The borrowed future is inert until first poll,
when it captures the current requested settings and session-save decision under
one state lock. It reads a bounded user-config snapshot before awaiting any
session save, then attempts publication against those exact bytes. A user-load
failure does not suppress the session attempt; a session error, `Unchanged`, or
`Deferred` does not suppress the independent user-default attempt. Each target
retains its existing conflict and ambiguous-publication semantics. No retries,
rollback of accepted selection, detached writer or implied all-or-nothing
transaction is added.

The returned outer `Result` reports lifecycle admission failure before either
target is touched. Once admitted, `NativeModelPreferenceCommit` retains the two
independent target outcomes, and the lifecycle permit covers both attempts even
when the session target is deferred, unchanged, or fails.

`NativeModelPreferenceCommit` reports the captured `generation`, separate
`session` and `user_defaults` results, and the exact published user config on
user success. Runtime status exposes `model_preferences_generation` so the host
can distinguish an older completed save from the current selection. Generations
are scoped to this runtime, not durable or cross-process revisions. A change
accepted while a save is pending stays current and pending; both completed
targets still name the earlier captured selection. A concurrently changed user
file causes a conflict instead of being replaced using a late fresh snapshot.

Dropping a pending combined operation releases session admission and its owned
user snapshot without starting a later user write or returning a partial receipt;
a session save may still be publication-uncertain under core's existing contract.
An active turn defers only the session target. When it settles, the host must
explicitly flush or admit the next job to persist pending session settings.
User-file success is not evidence that session metadata changed, and session
success is not evidence that defaults for future sessions changed. The user
store's explicit directory authority and bounded synchronous I/O contract are
defined in [configuration](configuration.md#schema-v4-and-durable-user-model-defaults).

Native queue bounds are 64 pending entries, 256 KiB prompt text per entry,
64 KiB serialized inference options per entry and 4 MiB aggregate pending text
plus serialized options. The active job is separately bounded and owned. Queue
validation reuses the existing iterative JSON measurer with the core safe depth
bound; rejected nested JSON is destroyed iteratively, including continuation
options. Core still applies its independent configured limits during admission.
Queue IDs are process-local, monotonic, non-reused values, distinct from core
turn IDs. Exhaustion and limit errors do not silently evict pending work.

The queue/current-selection and resume rules follow pinned
`src/core/agent/worker_runtime.zig:728`, `:1006`, `:1161`, `:1180`, `:1194`,
`:4034`, and `src/core/app/app_session_runtime.zig:4831`. Independent persistence
attempts follow `app_session_runtime.zig::commitRuntimePreferences`. The CLI
input loop must compose these native ownership APIs and render each outcome.

## Secondary-worker model routing

`NativeConversationModelRoutes` is an explicit, bounded table shared with the
host's search tool. `NativeConversationRuntime::new_with_model_routes` registers
the exact session ID and incarnation, backed by a weak reference to the same
current-selection state used by the queue. A table admits at most 64 live
registrations; duplicates and capacity exhaustion fail without replacing any
existing route. Missing incarnations return no snapshot. Lookup copies only
the bounded model ID and invokes no provider, catalog or persistence operation.
Source observation and last-reference destruction occur outside the table lock.

The runtime owns registration; a returned active turn also retains it through
native finalization. Dropping the runtime clears queued inputs but preserves
routing for its independently owned turn. Once the last registration owner
settles or drops, the slot is removed and reusable. The table cannot keep a
conversation or its engine alive. IDs are not permission grants: the tool still
uses core's actual execution context after ordinary preparation and approval.

Search captures current selection at tool-entry time, not the main job's older
admission snapshot. This deliberate distinction follows the pinned interactive
implementation, whose live selection can change during an active response.
The captured search model stays fixed while waiting for capacity and streaming.
Vision always uses its dedicated Gemini worker. Neither secondary worker inherits
effort or fast controls; those continue to apply only to main provider rounds.
The exact worker contracts are in [search](web-search.md) and [vision](vision.md).

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

## Typed historical facts

`NativeConversationHistory` reads the reserved
`machine_god.conversation_history` entry. An absent entry means unknown facts,
not a transcript-derived success or failure. Schema 1 contains exactly
`schema_version` and `groups`. Each sparse, ordered group contains exactly
`first_user_message`, `turn_sequence`, `state`, `files`, and nullable `background`.
Indices reference real canonical user messages; positive attempt sequences are
already reserved and strictly increase between groups. Native adoption also
validates consistency with the active checkpoint. A `running` fact cannot
silently survive without its matching running checkpoint.

Native admission saves `running` with the input, allocator and checkpoint in the
same prepared transaction. Finalization saves `completed`, `cancelled`, or
`failed` with the corresponding checkpoint transition before forwarding the
terminal outcome. Dropping a turn starts no finalizer and invents no terminal
reason: its published `running` fact remains. Starting a subsequent user group
marks that known unfinished predecessor `interrupted`. Explicit continuation
keeps the same user boundary and attachments while replacing the current attempt
sequence and outcome; it does not create a duplicate user history item. A legacy
native checkpoint can establish unfinished work, but absent earlier facts are
not reconstructed from assistant text or tool results.

`record_history_file(first_user_message, turn_sequence, evidence, now_ms)` and
`set_history_background(first_user_message, turn_sequence, observation, now_ms)`
are explicit trusted-host observation saves, not tool or process operations.
They require an exact known historical attempt, including when an asynchronous
observation targets an earlier group. Continuation changes that identity, so a
late observation for the previous attempt fails instead of attaching itself to
new work. The borrowed futures are inert, hold admission through the exact-CAS
save, preserve canonical messages and unrelated metadata, and keep existing
uncertain-publication semantics on failure/drop.

File observations contain exactly `path`, `action`, `stale`, nullable `source`,
nullable `new_path`, `status`, and `model_view_covers_full_file`. The action
vocabulary is `read`, `write`, `edit`, `delete`, `rename`, `copy`, `search`, `list`,
or `unknown`. A source names a canonical assistant-message and content-block index
plus the exact validated call ID and tool name. Indexed validation checks the
actual call in the same user group before publication and on adoption, without
cloning its arguments. Reused call IDs in different rounds remain distinct.
Known call sources follow canonical order; updates retain their position and
duplicate or out-of-order new source identities fail without mutation.

An explicitly unknown observation has no source, no destination, unknown status
and no full-file claim; only those observations use a path/action upsert key.
Per-call observations retain distinct reads even when paths and actions repeat.
Their status is explicit `unknown`, `success`, or `failure`; only a successful
read can claim a full-file model view. Successful writes, edits, deletes and
renames mark earlier reads of the source path stale. Copies affect earlier reads
of their destination, not their source; renames also affect the destination.
Failed or unknown mutations do not establish new staleness, and later reads do
not inherit a predecessor's stale flag. These are historical observations, not
new file inspection or confirmation of an uncertain effect.

Background observations contain exactly `log_path`, nullable `url`, and
`expect_url`; `None` explicitly clears the observation. Paths and URLs are
descriptive text, never authority or proof of present file/process state.
Strings reject empty/NUL-bearing values and retain the existing 4,096-byte path
and 2,048-byte URL bounds. Validation checks shallow shapes, aggregate native
JSON-node and serialized store-byte bounds before cloning text. Mutation failures
leave the previous value unchanged; debug and error output omit sensitive facts.

Native turn outcomes are produced automatically. Background facts require
explicit host observations. File producers are explicitly configured as below;
facts are not inferred from arbitrary output JSON, unknown-result placeholders,
or unrelated metadata.
These facts also do not reconstruct file-undo authority or permission grants.
The context consumer and its bounded display rules are described in
[native context](conversation-context.md).

Full terminal `start`, `monitor`, and `close` return ordinary tool results and
continue the model round; they do not manufacture legacy `log_path`/`url` history
attachments or end the turn early. This follows pinned
`src/core/tooling/tool_runtime.zig::toolExecutionResultFromDispatch` and
`src/tools/terminal/terminal.zig::resultFromCompletion`. The legacy
`background_command` completion field has no production producer in that revision.
Terminal session identity and current monitor observations remain in the terminal
host's typed records, not inferred from arbitrary result JSON. Late URL notices
likewise do not rewrite launch-time history facts. A composed-host regression
exercises real start, custom-monitor admission in a later turn, and close, with
ordinary provider continuation and no invented legacy attachment.

### Automatic file observations

The conversation and prepared reference host share one explicit
`Arc<NativeConversationObservations>`. `with_observations` registers the exact
session/incarnation in a table of at most 64 weak live routes. Missing or
mismatched routes reject wrapped execution before constructing the underlying
tool future. Legacy unconfigured hosts preserve their existing behavior.

The configured native adapters cover `read_file`, `list_files`, `glob_files`,
`grep_files`, `write_file`, `edit_file`, `delete_file`, `rename_file`, and
`copy_file`. They use those concrete tools' normalized prepared path fields,
not name-based inference about arbitrary tools. Before forwarding `ToolStarted`,
the native stream binds its canonical source with core's cursor-based locator.
Core's observer acknowledgement still precedes execution and permission guards
remain authoritative. Denied or never-started calls create no execution facts.

Each adapter reserves bounded storage and an `unknown` attempted fact before
constructing or polling the underlying execution. Actual returned success or
failure settles that fact; dropping an unfinished execution preserves unknown,
not success. Result observations survive a hidden `ToolFinished` event, including
observer failure or cancellation after execution. Successful reads claim full
model coverage only when their whole output remains inline under the exact
Gateway projection predicate, with no archived persistence override, and the
exact paired successful result has reached canonical history. A failed result
save preserves observed read success without claiming model visibility. Mutations
use the history codec's chronological staleness rules, never undo receipts as
proof of successful tool completion.

Finalization merges the batch in the same save as the terminal history state.
Dropping or failing that save retains pending facts in the conversation owner.
An idle `flush_history_observations(now_ms)` explicitly saves them without
starting provider/tool work; an empty batch returns `None` without a save.
The next admission also merges pending facts before advancing a continuation's
attempt identity or building compacted provider context. Explicit history saves
merge pending facts first. Only successful publication acknowledges a batch;
versioned acknowledgement cannot erase a later settlement, and reconciliation
does not downgrade already-published status or staleness.

Pending storage is bounded by the native record byte/node envelopes and fails
before effects if exhausted. There are no detached writers or destructor saves.
Pending observations are process-local until confirmed publication: losing the
last owner or the process before that save can lose pending facts, without
erasing the already-saved canonical transcript or implying safe effect replay.

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
