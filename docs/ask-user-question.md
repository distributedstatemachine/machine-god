# Native `ask_user_question`

This document is the durable contract for the provider-visible native
`ask_user_question` tool. It covers one ordered batch of ordinary, bounded
questions through an explicitly injected `QuestionPrompter`. It does not
provide a terminal UI or approval escalation.

## Product boundary

One call contains one to four questions. Each question contains two to six
ordered options. Options guide presentation but do not constrain the returned
answer: a host may expose an `Other` path, and any bounded nonempty free-form
answer is valid. The number and order of answers must exactly match the prepared
batch.

Pinned fx also accepts `permission_request_id` for an action-bound approval
flow. Machine-god has no exact auto-denied continuation authority, so this tool
rejects `permission_request_id` whenever the field is present, regardless of
its value or type. Question, option, answer, and conversation text are never
authorization.

## Model-visible schema

The root is a strict object with exactly one allowed field:

```json
{
  "questions": [
    {
      "question": "Which implementation should proceed?",
      "options": [
        {
          "label": "Bounded native seam",
          "description": "Add only the injected library boundary."
        },
        {
          "label": "Defer the slice",
          "description": "Leave the native tool inventory unchanged."
        }
      ]
    }
  ]
}
```

Objects are strict at all three levels:

| Object | Required fields | Optional fields | Unknown fields |
| --- | --- | --- | --- |
| root | `questions` | none | rejected |
| question | `question`, `options` | none | rejected |
| option | `label` | `description` | rejected |

`permission_request_id` is a dedicated deferred-feature error, not an ordinary
unknown-field error. Duplicate JSON object keys are outside this tool's
boundary because core supplies an already-decoded value.

The schema advertises one to four questions and two to six options per
question. It does not advertise approval escalation, timeouts, defaults,
multi-select, option IDs, selected labels, or presentation properties.

## Canonicalization and terminal safety

Preparation visits questions and options in input order:

1. require the strict object and field type;
2. trim only ASCII space, tab, carriage return, and line feed from both ends;
3. reject an empty required question or label;
4. check the raw per-field byte limit;
5. encode terminal-unsafe scalars without truncation;
6. check rendered per-field and aggregate byte limits; and
7. build strict normalized arguments and check their compact serialized size.

An optional description must be a string. An empty description after ASCII
trim canonicalizes to absence; a nonempty description follows the same raw and
rendered checks. This is intentionally stricter than pinned fx, which discards
a non-string description.

Terminal-safe encoding preserves printable ASCII and printable valid Unicode.
It replaces ASCII C0 controls and `DEL` with lowercase `\xnn`. It replaces C1
controls `U+0080..U+009F`, `U+061C`, `U+200B..U+200F`, `U+2028..U+202E`,
`U+2060..U+206F`, and `U+FEFF` with lowercase `\u{nnnn}` using at least four
hex digits. JSON input is valid UTF-8. The encoder does not interpret ANSI
sequences, normalize Unicode, remove visible text, or truncate.

Labels must be unique within one question after trim and terminal-safe
encoding. Comparison is bytewise ASCII case-insensitive: `Yes` conflicts with
` yes `, and raw ESC conflicts with literal `\x1b`; non-ASCII case variants are
not folded. Duplicate questions and labels reused in different questions are
allowed.

The values supplied to `QuestionPrompter` are the exact normalized values kept
in prepared execution arguments. Direct `execute` accepts only a canonical
normalized value whose terminal-safe preimage also satisfies the raw-field and
incoming-serialization bounds. Direct execution therefore cannot widen the
ordinary preparation boundary.

## Fixed limits

All sizes are inclusive UTF-8 byte limits. `Raw` means the decoded string after
the ASCII-edge trim above; `rendered` means its terminal-safe representation.
Serialized sizes are compact `serde_json` bytes, including punctuation and
escaping.

| Resource | Inclusive maximum |
| --- | ---: |
| Incoming serialized arguments | 32,768 bytes |
| Questions | 4 |
| Raw/rendered question | 1,024 / 4,096 bytes each |
| Options | 6 per question; 24 total |
| Raw/rendered option label | 128 / 512 bytes each |
| Raw/rendered option description | 512 / 2,048 bytes each |
| Aggregate rendered presentation | 32,768 bytes |
| Serialized normalized arguments | 49,152 bytes |
| Complete pre-trim host answer | 4,096 bytes each |
| Aggregate complete pre-trim host answers | 4,096 bytes |
| Aggregate rendered answers | 16,384 bytes |
| Reachable serialized `ToolOutput` maximum | 41,102 bytes |
| Serialized `ToolOutput` defense-in-depth guard | 49,152 bytes |
| Default/hard simultaneous active prompts | 1 / 8 |
| Downstream callbacks per admitted prompt lifetime | 256 |

The incoming ceiling is checked by a remaining-budget-aware bounded
serialization pass before semantic traversal or string cloning. It stops as
soon as the budget is exceeded, including inside strings and object keys.
Raw-field checks occur after trim and before encoding. Rendered checks occur
during encoding. The normalized serialization check occurs last. Aggregate
presentation is the checked sum of normalized question, label, and present
description bytes and excludes container overhead; serialized normalized size
includes that overhead. No stage truncates.

Host-answer limits measure complete strings before trim or character scanning.
Each answer and their aggregate are checked before ASCII-edge trimming, which
prevents an arbitrarily large whitespace-only answer from causing unbounded
synchronous scanning. Checked arithmetic fails closed on overflow.

Legal question and answer bounds can produce at most 41,102 serialized output
bytes. The separate 49,152-byte complete-result check remains authoritative
defense in depth but is unreachable under the other limits.

## Authority and effects

The tool is read-like and reversible in metadata, but it is not a filesystem
read. Preparation uses core's explicit
`PreparedToolCall::without_authority` disposition. Core constructs no
permission request ID, emits no `PermissionRequested` or `PermissionResolved`
event, and does not invoke the permission handler for this call. Every existing
tool retains the permission-required default.

`without_authority` means no policy-governed authority; it does not claim that
the injected prompter lacks terminal, UI, or transport authority.
`PreparedToolCall::capability()` is total: permission-required calls return
their exact capability and explicit no-policy-authority calls return `None`.

The prompt seam is rootless. It performs no ambient filesystem, environment,
process, network, terminal, or runtime discovery. Any interaction authority is
supplied explicitly by the host through `QuestionPrompter`.

## Injected prompt lifecycle

`QuestionPrompter` is object-safe, `Send + Sync`, and executor-neutral. The
native tool supports owned and shared construction, including
`Arc<dyn QuestionPrompter>`. A prompt request owns the normalized ordered batch
and exposes read-only getters. Debug output is structural and excludes all
question, option, description, and answer text.

Calling `Tool::execute` creates an inert future. On first poll the future:

1. checks engine cancellation;
2. attempts fail-fast active-prompt admission; and
3. checks cancellation again immediately before invoking the prompter once.

Capacity exhaustion neither queues nor registers a capacity Waker. The default
active limit is one; explicit construction accepts one through eight. Zero or
more than eight fails construction with the fixed, data-free invalid-limits
error. Each tool instance owns an independent counter.

The tool owns the prompt future. Dropping an unpolled tool future does not
invoke the prompter. No thread, task, queue, channel, timer, runtime, retry, or
detached work is started. The tool sets no timeout. A conforming prompter keeps
interaction work owned by its returned future or completes its own cleanup on
drop; a stalled or blocking injected prompter can stall the call.

### Wake delivery and capacity

The prompt and cancellation Waker family shares one activity-backed notifier.
It provides one serialized, nonrecursive downstream-callback lane with at most
one callback in flight. Observation-aware coalescing preserves wake progress,
including a wake after its corresponding outer poll, without requiring
unrelated activity. Target clone, callback, and drop work always occurs outside
the state lock.

One state-owned budget covers the complete admitted prompt lifetime across
synchronous reentry, queue-only callback return plus queued polls, and later
external wakes. Calls 1 through 255 are ordinary. Before callback 256, the
notifier records sticky delivery exhaustion; callback 256 schedules the
terminal outer poll. That poll checks cancellation first, otherwise returning
the fixed nonretryable `Execution`/`ask_user_question_prompt_failed` error.
Further bind and wake attempts are suppressed until close. Callback panic does
not refund budget.

Prior callback target A is dropped before any replay target B is selected. The
then-current lifecycle, pending notice, and target are read only after A is
destroyed successfully. A reentrant close or replacement therefore wins; a
target-drop panic cannot wedge the lane or permit stale replay.

An open notifier holds a state permit. Every admitted callback takes a local
activity guard before leaving the lock. Close marks the notifier closed,
detaches target and state guard, retains a local close guard through out-of-lock
target destruction, then releases it before resuming any selected panic. An
already admitted callback independently keeps its guard through callback
return, target destruction, and lane settlement. The outer activity owner
covers prompt, cancellation waiter, registration, and cached-Waker teardown.

Closed supplied-Waker clones are inert: they deliver no callback and retain no
capacity. Fresh admission waits only for the outer activity owner and admitted
callback or close guards, never for all closed identities to be destroyed.
Moved cancellation callbacks retain the originating activity through callback
return, so outer-future drop cannot admit a replacement while an old callback
tail is still running.

### Cancellation and panic precedence

Engine cancellation is checked at first poll, immediately before prompt
invocation, after every ready prompt outcome, and after prompt/waiter/cached-
Waker teardown immediately before every direct return. Cancellation observable
in the same poll as an answer, user cancellation, unavailability, host failure,
or final registered-Waker destruction wins. No answer is published after
cancellation wins.

Release panic handling is `unwind` so product cleanup can settle notifier state
and preserve deterministic precedence. Opaque panic payload selection follows
these rules:

- ambient unwind precedes every cleanup panic;
- prompt-poll panic precedes activity cleanup;
- activity cleanup orders prompt drop, cancellation waiter, registration close,
  then registration drop;
- registration close precedes cached-Waker drop; and
- notifier callback precedes target drop.

All suppressed or nonselected opaque cleanup payloads are intentionally
forgotten before a selected panic resumes. Ordinary resources still drop.
Callback or target-drop panic clears lane flags, stale delivery closes, and
capacity is recoverable. A secondary target-drop payload may own the supplied
Waker; forgetting it is safe because closed Waker identities are inert and no
longer own capacity.

## Outcomes and output

The prompter returns one structured outcome:

- `Answered`, backed by a private fixed four-slot container that can hold zero
  through four strings;
- `Cancelled`, for explicit user cancellation; or
- `Unavailable`, for a noninteractive host.

The bounded answer container permits count-mismatch validation without letting
a malformed host transfer an arbitrarily large vector whose destructor runs in
the tool future. `Answered` must have exactly one value per question. Each
complete value is checked against the pre-trim individual and aggregate limits,
then ASCII-trimmed, required nonempty, terminal-safe encoded, and checked
against the rendered-answer limit. An answer need not equal an option label.

Success returns deterministic ordered JSON as non-error `ToolOutput.content`:

```json
[
  {
    "answer": "Bounded native seam",
    "question": "Which implementation should proceed?"
  }
]
```

Each object intentionally inserts `answer` and then `question`; array order is
question order. Questions are the exact normalized prompt strings. No options,
descriptions, IDs, timing, or host metadata are returned.

`Cancelled` returns `(user cancelled the question)` as successful string
content. `Unavailable` returns `(ask_user_question is only available in the
interactive shell; ask the user freeform instead)` as successful string
content. Neither sentinel is authorization.

For a non-cancelled ready prompt result, precedence is:

1. redacted prompter failure;
2. answer-count mismatch;
3. per-answer complete pre-trim byte limit in question order;
4. aggregate complete pre-trim answer limit;
5. ASCII-edge trim and empty-answer rejection;
6. terminal rendering and aggregate rendered-answer limit;
7. serialized result defense-in-depth guard; and
8. ordered success publication.

No partially validated or partially encoded answer array is returned.

## Fixed failures and redaction

| Condition | `ToolErrorKind` | Code | Message | Retryable |
| --- | --- | --- | --- | --- |
| Malformed shape, type, field, empty required text, or duplicate label | `InvalidInput` | `ask_user_question_invalid_arguments` | `ask_user_question arguments are invalid` | no |
| Incoming, field, presentation, normalized, answer, or result limit | `InvalidInput` | `ask_user_question_resource_limit` | `ask_user_question resource limit exceeded` | no |
| `permission_request_id` present | `InvalidInput` | `ask_user_question_permission_request_unsupported` | `ask_user_question permission escalation is not supported` | no |
| Invalid configured active limit | construction error | n/a | `invalid ask_user_question limits` | n/a |
| Active prompt limit full | `Unavailable` | `ask_user_question_busy` | `ask_user_question prompt capacity is exhausted` | yes |
| Prompter failure or delivery budget exhausted | `Execution` | `ask_user_question_prompt_failed` | `ask_user_question prompt failed` | no |
| Wrong answer count or malformed answer | `Execution` | `ask_user_question_invalid_response` | `ask_user_question prompt returned an invalid response` | no |
| Engine cancellation | `Cancelled` | `ask_user_question_cancelled` | `ask_user_question was cancelled` | no |

Argument precedence is serialized-input limit, root type, deferred
`permission_request_id`, other root keys, `questions`, then each ordered
question/option pipeline. Resource-limit failures at an exact check precede
later semantic checks. Errors retain no input, answer, question, label,
description, prompter diagnostic, session identity, or executor text. Debug
implementations are fixed or structural and never invoke user-defined `Debug`.

## Platform and host composition

The tool and injected seam use safe standard Rust, allocation, atomics, and
core futures only. They are exported by `machine-god-native` without an HTTP,
Unix, or non-WebAssembly feature gate and compile for the repository's native,
FreeBSD, and WASI library targets. No browser bridge or WASI terminal
interaction is supplied; those hosts inject a deterministic or unavailable
prompter.

`NativeReferenceHost` retains its existing Linux/macOS, non-WebAssembly,
`ai-gateway-http` boundary. Its constructors require an explicit shared
`QuestionPrompter`; they do not discover a terminal, TTY, environment variable,
file, or runtime for it. The sixteen-tool catalog is `ask_user_question`,
`copy_file`, `create_folder`, `delete_file`, `edit_file`, `file_info`,
`glob_files`, `grep_files`, `list_files`, `open_file`, `read_file`,
`rename_file`, `terminal`, `web_fetch`, `web_search`, and `write_file`.

## Regression evidence

The primary contract suite is
`crates/machine-god-native/tests/ask_user_question.rs`. It covers schema and
limit boundaries, terminal classes, canonical direct execution, cancellation
precedence, bounded answer storage, concurrency, drop/unwind cleanup, stale-
Waker suppression, autonomous wake progress, the lifetime-wide callback bound,
and redaction. High-value lifecycle regressions include:

- `completed_prompt_recovers_capacity_after_in_flight_callback_while_closed_clones_remain`;
- `replay_target_drop_panic_clears_lane_for_a_fresh_notification`;
- `replay_target_drop_close_suppresses_selected_replay_and_retains_capacity`;
- `callback_panic_precedes_panicking_replay_target_payload_drop`;
- `queue_only_wakes_share_one_prompt_lifetime_delivery_bound`;
- `cancellation_wins_after_the_terminal_queue_only_wake_is_enqueued`;
- `prompt_poll_panic_precedes_panicking_target_cleanup_payload`; and
- `prompt_drop_cleanup_precedence_survives_panicking_secondary_payloads`.

`ask_user_question_engine.rs` covers provider/orchestrator event and persistence
composition. `reference_host.rs` covers catalog and injected-host composition.
The locked release example
`examples/ask_user_question_release_panic_probe.rs` exercises ordinary prompt-
drop and ambient primaries, a secondary payload owning the supplied Waker,
stale-wake suppression, and fresh admission while closed identities remain.

## Pinned-fx relationship and deferrals

Pinned fx informed the question/option cardinalities, ASCII-edge trim,
case-insensitive label deduplication, terminal-safe presentation, ordered answer
JSON, cancellation and noninteractive sentinels, and the absence of answer-to-
option membership enforcement. Machine-god additionally enforces strict
objects, explicit byte and concurrency limits, bounded host outcomes, its own
terminal-safe answer encoding, and the authority boundary above. This is not a
byte-level or full fx-equivalence claim.

Deferred work includes approval continuation and broader permission modes; a
terminal, graphical, browser, remote, or CLI question UI; timeouts, detached or
background prompts, persistence/history/resume, and multi-process capacity;
multi-select/default choices; durable terminal actions; `vision`;
`read_tool_result`; Milestone 05 extension surfaces; benchmark workloads; and
product-performance or compatibility-promotion claims.

Zig remains only a pinned upstream fx build input. Machine-god is a Rust
product and neither ships nor executes Zig.
