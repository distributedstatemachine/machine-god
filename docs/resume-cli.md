# Explicit-session `resume` command

Prompt-bearing `machine-god resume` continues one existing durable session with one bounded,
noninteractive prompt. It exposes native conversation resume and runtime
admission through the same streaming request path as [`ask`](ask-cli.md); it is
not a session picker, interactive shell, replay, reset, migration, or recovery
command.

## Grammar

The noninteractive form is:

```text
machine-god resume <id> [--] <prompt...>
```

`<id>` is one explicit session ID: 1-128 ASCII bytes containing only letters,
digits, `-`, `_`, `.`, or `:`, and with a first byte other than `-`. The core
`SessionId` alphabet itself remains unchanged; rejecting a command-position ID
that begins with `-` is an intentional CLI parser tightening because that token
is option-like. The exact token `last` is reserved for
interactive latest selection and cannot name an exact resume target. One or more Unicode prompt
arguments follow the ID and are joined with one ASCII space into exactly one
prompt. A single `--` after the ID ends option recognition and permits the first
prompt part to begin with `-`; it is not part of the prompt.

The joined prompt must contain at least one byte other than space, tab, CR, or
LF, contain no NUL byte, and contain at most 256 KiB of UTF-8. Join accounting
is checked before allocation. Invalid IDs or explicitly supplied blank prompts,
non-Unicode input, oversized prompts, misplaced delimiters, extra-invalid
arguments, and unsupported options use the global invalid-arguments diagnostic
and exit `2`.

Grammar and prompt validation complete before configuration, current-directory,
state-root, credential, runtime, session-store, or network effects. Standard
input is never read by this prompt-bearing form. There is no picker, alias,
`--resume`, `--resume-last`, `--continue`, short flag, JSON form, or interactive
mode in this prompt-bearing form. Separately, `resume`, `resume last`, and
`resume <id>` with no prompt start the [interactive host](cli.md#interactive-ownership)
using validated latest/exact selection. No synthetic user prompt is added.

## Native composition and authority

On Linux and macOS, a valid request starts the same owned signal guardian used
by `ask`, then captures the process-native environment, loads the current
strict configuration, captures the current workspace, prepares the selected
workspace and state roots, validates one inference credential, and awaits a
completed rich-model catalog observation before acquiring the complete terminal
host. It reuses that same acquired credential for inference. The catalog's
failed-observation fallback, setup-signal behavior, and bounded authority are
the same as [`ask`](ask-cli.md#native-composition).
It resumes `<id>` through `NativeConversation` over the host's retained
`NativeSessionLifecycle`, then enqueues exactly one new prompt through
`NativeConversationRuntime` on the engine-canonical session.

Resume accepts only a valid current-schema record under the state root selected
for this invocation. Missing, corrupt, future-schema, incompatible-incarnation,
or otherwise unloadable state fails; the command does not import, migrate,
reset, repair, skip, or rewrite it as a convenience. The session ID is never
selected from a directory listing, and this path does not allocate a new
session identity.

The stored transcript is provider-neutral. The continued turn deliberately
uses the provider, transport, credentials, engine limits, permission
adapter, tool catalog, current workspace, and other authority from the current
invocation's configuration and composed host. It does not resurrect historical
credentials, configuration, workspace authority, pending prompt UI, ephemeral
permission grants, or external tool effects from the durable record. Confirmed
saved exact rules are validated and evaluated against newly prepared actions;
historical allow decisions alone do not become execution authority.

Saved session model/effort/fast preferences override the current configuration's
ordinary defaults. If historical preferences are absent, configuration supplies
the runtime's startup selection without claiming those values were historical.
This grammar supplies no explicit process-model override. The completed catalog
determines effective controls at admission; unsupported controls are omitted on
the wire while requested preferences remain preserved in the session. Ordinary
`resume` does not write user defaults.

Historical workspace, origin, and creation time remain unknown when absent;
the current directory, file mtime, session ID, and current clock do not
invent those facts. Current invocation workspace authority is separate from
historical workspace metadata. Native context preferences select provider views
without deleting canonical history, and existing confirmed transcript evidence
is not automatically replayed as external effects.

As with `ask`, this noninteractive host enforces configured mode, patterns and
saved exact rules through newly attached native permission/context routes.
Permitted actions may execute; remaining human-prompt requirements are denied
per request. `ask_user_question` receives its fixed unavailable outcome. Neither
path reads standard input or starts detached interaction.

Targets other than Linux and macOS return the fixed operational failure for
valid grammar without attempting the complete native reference-host
composition. This is deterministic unsupported behavior, not partial support.

## Streaming, durability, and exits

Assistant text uses the `ask` command's output bridge and lifecycle contract.
Only `TextDelta` payload bytes are written to standard output, in event order,
without styling, a forced newline, or whole-answer buffering. Reasoning, usage,
lifecycle events, the session ID, provider diagnostics, permission details,
tool calls, and tool results are not printed. Successful terminal completion
flushes acknowledged output.

- A completed turn exits `0` after native checkpoint finalization succeeds and
  all preceding assistant bytes are written. Finalization failure is an
  operational failure, not successful terminal completion.
- Invalid grammar exits `2` with the global invalid-arguments diagnostic.
- Configuration, root, credential, composition, session-load, provider,
  engine, terminal-event, and runtime failures exit `1` with the fixed redacted
  diagnostic `machine-god resume: request failed` followed by LF.
- Standard-output failure cancels or drops the owned turn, exits `1`, and uses
  the same fixed output diagnostic as `ask`.
- `SIGINT` and `SIGTERM` use the `ask` signal guardian, cleanup, first-signal
  precedence, and exit codes `130` and `143`. The same 100 ms absolute
  post-cleanup acknowledgement deadline applies to an outstanding write and
  any following flush.

Partial assistant bytes already acknowledged before a later failure are not
retracted. No diagnostic may include the prompt, session ID, credential, path,
provider data, tool data, configuration value, or operating-system detail.

A turn persists through the native conversation and core session contracts.
Admission atomically reserves the checkpoint and requested model preferences
with the new prompt against its prepared revision. Once the resumed turn has
started, its user message may already be durably appended before a later
provider, engine-output, standard-output, signal, or final presentation
failure. Such a failure does not roll back the durable user turn or any
earlier committed transcript prefix. The next explicit inspection or resume
observes whatever current record successfully committed.

The command makes no cross-process serialization claim. Process-local engine
state converges same-incarnation resumes only within one composed host, while
the file store's advisory lock and compare-and-swap fence individual durable
operations. Another cooperating process may load the same revision and begin
work concurrently. Native prepared prompt reservation is pinned to the exact
revision: a conflict fails closed rather than reloading another process's
prefix, merging the new prompt, or automatically retrying admission. A later
assistant commit whose transcript has diverged fails closed instead of merging
assistant results. The CLI neither
holds a process-wide session lease for the complete provider turn nor excludes
processes that ignore the store protocol.

## Resource and compatibility boundary

The prompt, assistant text, provider rounds, events, tool calls/results,
transcript, metadata, and structural JSON retain the bounds documented for
[`ask`](ask-cli.md), the [core engine](core-api.md), and the
[session lifecycle](native-session-lifecycle.md). The initial lifecycle resume
loads at most one current-schema record within the file-store cap. Subsequent
prompt reservation, checkpoint finalization, and assistant persistence use the
native/core store contracts; compare-and-swap validation may reread the bounded
record, but prepared reservation does not retry conflicts. The command may
create the record's permanent lock sidecar. It adds no directory enumeration or
ID-generation retry. It reuses `ask`'s bounded scoped worker and owned signal
guardian, so no thread or task remains detached. Provider work retains its
existing bounds, but the inherited file store has no attempt or wall-clock
ceiling for advisory-lock acquisition, filesystem latency, or retries after
`EINTR`.

The pinned upstream fx surface accepts implicit-last selection, aliases,
recording, and interactive continuation forms. Machine-god intentionally
implements only the explicit-ID, one-prompt scenario above. Matching the
observable ability to continue a selected session is scenario compatibility;
it is not grammar, option, presentation, persistence-format, concurrency, or
performance equivalence.

## Native validated-resume library admission

`prepare_native_session_resume(lifecycle, target, workspace, now_ms)` is a
separate Linux/macOS library primitive for validated session selection. It does
not change the command grammar above or deliver interactive picker, migration,
or recovery behavior by itself. The borrowed future is inert before polling;
workspace spelling and time are explicit caller inputs, not environment or clock
lookups. Workspace input uses the native catalog's bounded normalized absolute
Unix path contract and confers no filesystem or tool authority.

`NativeResumeTarget::Latest` scans the exact file-store allocation retained by
the supplied lifecycle, selects only records with the explicit workspace
association, and ranks authoritative update time with descending-ID ties.
It refuses incomplete scans, unknown eligible activity, and corruption rather
than selecting a potentially older fallback. Missing eligible state is a fixed
`NotFound`. Latest never changes workspace association. The catalog's directory,
aggregate-byte, per-record and structural limits remain authoritative; there is
no index, inferred mtime ordering, or historical association manufactured from
the current process directory.

`NativeResumeTarget::Exact(id)` uses independent by-ID catalog observation and
does not depend on enumeration or display truncation.
`NativeResumeTarget::Observed(NativeObservedSession::from_entry(&entry))`
instead pins the ID, incarnation and revision of a previously displayed catalog
row. It retains none of the row's presentation data. Preparation observes that
exact ID again and checks all three fields before replay, canonical load or
workspace publication. A deleted row or changed tuple returns `Conflict`, with
no fallback to a newer incarnation/revision and no metadata write. The bounded
tuple exposes trusted-host getters and has redacted `Debug` output. This is the
selection fence for a picker, not a picker UI or a persistence lease.

All forms capture the selected incarnation and revision, replay and validate the complete native
conversation metadata, then use the engine's exact-revision guarded load. A
changed revision or incarnation fails before canonical reconciliation; the
operation does not silently follow a replacement. Busy candidates, invalid
checkpoint/context/history/model metadata, invalid saved exact permission rules,
and ordinary load failures do not
trigger new-session creation, provider work, tool calls or automatic repair.

Exact and observed selection explicitly persist a requested workspace-association
change through `rebind_native_session_workspace` before returning the candidate. Its
exclusive exact-revision mutation preserves canonical transcript, allocator,
incarnation, unrelated metadata and unknown historical origin/creation facts.
An unchanged association uses the reconciled no-save path without advancing
revision or timestamp. Association publication is descriptive metadata, not
permission grants or additional-root authority. Rebinding failures are returned
without retries; persistence failure or dropping a polled save can follow
publication, so neither implies that the association stayed unchanged.

The opaque `NativePreparedResume` owns one engine-canonical session, the retained
store, and the selected identity/revision. Its getters expose that bounded
identity to the trusted host; `Debug` and errors are redacted. Consuming
`adopt()` is inert until polled, then checks live identity/busy state, rereads
the exact durable record, compares its complete canonical snapshot, and performs
the core's exclusive revision/uncertain-save check before validating and returning
a `NativeConversation`. Changes after preparation fail rather than silently
adopting a newer target. Dropping a prepared candidate or unpolled adoption starts
no work and does not undo an already published explicit rebind.

Adoption is a checked observation, not a cross-process session lease. Another
writer can act after the final check; subsequent prompt and metadata admission
retain their ordinary exact-CAS fences. The caller must install a successful
candidate under its own idle runtime-ownership boundary and attach current host
permission, context and observation routes. Preparation/adoption do not receive,
replace, cancel, clear queues in, or shut down the caller's current runtime.
Failures therefore return to that owner without an implicit runtime transition.
When the selected ID is also the current session, its explicitly requested
rebind can still be confirmed or publication-uncertain before a later failure;
this is not a promise to roll back shared canonical metadata. The existing
canonical-load reconciliation rules likewise remain in force for validated
same-incarnation state. Runtime ownership, pending input and selected model are
not replaced by either operation.
These operations retain the existing synchronous store-I/O and lock-latency
contract and spawn no task, thread, retry worker or background lifecycle action.

The distinction between latest without rebind and explicit selection with
persisted rebind follows pinned `src/core/session/session_store.zig:1093–1150`
and `:3093–3199`.
Foreign import, native migration, recovery copies, interactive selection and
their presentation remain separate requirements, not silent fallback paths in
this primitive.
