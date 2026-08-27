# Native `ask_user_question`

Status: **CYCLE 2 LOCAL GATE GREEN — FORMAL REPLACEMENT REVIEW, REMOTE
WORKFLOWS, AND DELIVERY PENDING**.

Bounded Milestone 03 slice 35 starts from exact delivered base
`5846799b665d62fc8301b33520da5cda33e850b3`. The comparison input is pinned
fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef`. This first slice asks only
ordinary, bounded questions through an explicitly injected, rootless
`QuestionPrompter`. It does not provide a terminal UI or approval escalation.

## Local implementation checkpoint

Core no-authority component `de1ce26`, frozen contract `13cd366`, contract
correction `399f960`, initial production component `b24a673`, and initial
evidence `a76818e` formed the rejected cycle-1 candidate. Cycle-2 independent
evidence component `c77b336a378b349f51eaddc60cb342f805fd7e21` is integrated
as `0dd1128b914b00f15a17be3cbf2b6f7edccf605b`; production component
`9d2e0f234fd96beb2b2ce5b7dd5a6c123905fbf6` is integrated as
`47e9505f463b5ca9f4f418198022a4805757621b`. Cycle-1 finding documentation
composes with both at exact behavior head
`c8718c60ead54b4e66916cecb1d382c1e8f82934`, tree
`c27463b76607ae048363327e163c2077e296b898`.

That exact composed head passes the complete exact-Rust-1.94.1 local gate,
including all four required workspace commands and 55 focused tests: 26 direct
tool, one engine, 15 configuration, three root-selection, nine reference-host,
and one reference-host lifecycle test. This checkpoint establishes
implementation and local regression/delivery evidence only. It does not
establish a formal replacement-review outcome, remote CI, benchmark evidence,
integration, delivery, product performance,
compatibility promotion, or fx equivalence. No release-binary prompt exercise
applies because this library-only slice adds no CLI prompt UI.

Formal cycle 1 reviewed exact candidate
`6c54ec3bf2c23983f14b0a4edeac723321a97900`, tree
`bea90245a559e8e223cc5bb45e0ddfa15e426ee6`, and rejected it. The
deduplicated result was 0 blocker, 1 high, 3 medium, and 3 low findings.
Cycle 2 implements and locally proves remediation for every accepted finding.
The cycle-2 tree is ready for immutable same-SHA review; no formal replacement-
review outcome exists yet.
The detailed outcome, remediation, and local-gate evidence are in the
[`review ledger`](reviews/m03-ask-user-question-review-01.md).

## Product boundary

`ask_user_question` is a provider-visible native tool for one ordered batch of
one to four blocking decisions. Each question has two to six model-supplied
options. Options guide the user but do not constrain the returned answer: a
host may expose an `Other` path, and any bounded nonempty free-form answer is
valid. The number and order of answers must exactly match the prepared batch.

Pinned fx also accepts `permission_request_id` to enter an action-bound
approval flow. Machine-god's current configuration has only ask-mode
permission handling and no exact auto-denied continuation authority. This
slice therefore rejects `permission_request_id` whenever the field is present,
regardless of its value or type. It never treats question, option, answer, or
conversation text as authorization.

The tool is read-like and reversible in product metadata, but it is not a
filesystem read and does not require permission. Effect-free preparation uses
core's explicit `PreparedToolCall::without_authority` form. Core consequently
constructs no permission request ID, emits no `PermissionRequested` or
`PermissionResolved` event, and never invokes the permission handler for this
call. Argument validation, cancellation, tool events, result limits,
persistence, and recovery remain unchanged. Every existing tool retains the
permission-required default.

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

`permission_request_id` is not an ordinary unknown-field case. Its presence
returns the fixed deferred-feature error below. JSON values have already been
decoded by core, so duplicate JSON object keys are outside this tool's
observable boundary; strictness applies to the resulting object.

The schema advertises one to four questions and two to six options per
question. It does not advertise `permission_request_id`, a timeout, default,
multi-select, option ID, selected label, or CLI presentation property.

## Fixed limits

All byte counts are UTF-8 bytes. `raw` below means the decoded string after
trimming only leading and trailing ASCII space, tab, carriage return, and line
feed. `rendered` means the terminal-safe representation described in the next
section. Serialized sizes are compact `serde_json` bytes, including JSON
punctuation and escaping.

| Resource | Inclusive maximum |
| --- | ---: |
| Incoming serialized arguments | 32,768 bytes |
| Questions | 4 |
| Raw question text | 1,024 bytes each |
| Rendered question text | 4,096 bytes each |
| Options per question | 6 |
| Total options | 24 |
| Raw option label | 128 bytes each |
| Rendered option label | 512 bytes each |
| Raw option description | 512 bytes each |
| Rendered option description | 2,048 bytes each |
| Aggregate rendered presentation text | 32,768 bytes |
| Serialized normalized prepared arguments | 49,152 bytes |
| Raw answer | 4,096 bytes each |
| Aggregate raw answers | 4,096 bytes |
| Aggregate rendered answers | 16,384 bytes |
| Serialized `ToolOutput` | 49,152 bytes |
| Default simultaneous active prompts per tool | 1 |
| Hard simultaneous active prompts per tool | 8 |

Aggregate presentation text is the checked sum of every normalized question,
label, and present description, excluding vector and object overhead. The
separate normalized-argument serialization check includes that overhead and
JSON escaping. Neither ceiling substitutes for the other.

The raw question total is at most 4,096 bytes. The raw answer total is also at
most 4,096 bytes. Terminal encoding expands one raw byte by at most four
rendered bytes, and compact JSON needs at most one additional escape byte per
rendered byte. The final 49,152-byte result check is nevertheless authoritative
and includes the complete `ToolOutput` envelope. Overflow uses checked
arithmetic and fails rather than truncating.

The input ceiling is checked by a bounded serialization pass before semantic
traversal or string cloning. The rejected cycle-1 candidate scanned a complete
oversized JSON string value or object key before testing the remaining 32/48
KiB budget. Cycle 2 makes string and key accounting remaining-budget-aware and
stops as soon as the applicable ceiling is exceeded. Per-field raw ceilings
are checked after trim and before terminal encoding. Rendered
per-field and aggregate ceilings are checked during encoding. The normalized
serialized ceiling is checked last. An input can fit 32 KiB and still fail a
later rendered or normalized ceiling; no field is silently shortened.

## Canonicalization and terminal safety

Preparation visits questions and options in input order and applies this exact
pipeline:

1. require the strict object and field type;
2. trim only ASCII `0x20`, `0x09`, `0x0d`, and `0x0a` from both ends;
3. reject an empty required question or label;
4. check the raw per-field byte limit;
5. encode terminal-unsafe scalar values without truncation;
6. check the rendered per-field and aggregate byte limits; and
7. build the strict normalized arguments and check their compact serialized
   size.

An optional description must be a string when present. After ASCII trim, an
empty description canonicalizes to absence; a nonempty description follows the
same raw/rendered checks. This is intentionally stricter than pinned fx, which
discards a non-string description.

Terminal-safe encoding preserves printable ASCII and printable valid Unicode.
It replaces ASCII C0 control bytes and `DEL` with lowercase `\xnn`. It replaces
C1 controls `U+0080..U+009F`, `U+061C`, `U+200B..U+200F`,
`U+2028..U+202E`, `U+2060..U+206F`, and `U+FEFF` with lowercase
`\u{nnnn}` using at least four hex digits. JSON strings are valid UTF-8, so
invalid UTF-8 cannot reach this stage. The encoder never interprets ANSI
sequences, applies Unicode normalization, or removes visible text.

Labels must be unique within their question after trim and terminal-safe
encoding. Comparison is bytewise ASCII case-insensitive: `Yes` conflicts with
` yes `, and raw ESC conflicts with the literal rendered text `\x1b`, while
non-ASCII case variants are not folded. Duplicate questions and labels reused
in different questions are allowed.

The normalized values supplied to `QuestionPrompter` are the exact normalized
values retained in prepared execution arguments. Direct `execute` accepts only
a canonical normalized value with an incoming preimage satisfying the same
raw-field and incoming-serialization bounds as preparation. The rejected
cycle-1 candidate admitted a printable 4,096-byte question that preparation
would reject at the 1,024-byte raw limit. Cycle 2 decodes a terminal-safe
preimage, rechecks its raw and canonical rendering bounds, and verifies the
complete incoming preimage under 32 KiB before invoking the prompter.

## Injected prompt boundary

`QuestionPrompter` is an object-safe, `Send + Sync`, executor-neutral host
boundary. The native tool supports owned and shared construction, including an
`Arc<dyn QuestionPrompter>`. The prompt request owns the complete normalized
ordered batch and exposes read-only getters. Its `Debug` output is structural
and does not include question, option, description, or answer text.

Calling `Tool::execute` creates an inert future. On first poll it checks
cancellation, attempts one fail-fast active-prompt admission, and only then
invokes the prompter exactly once. Capacity exhaustion does not queue, register
a capacity Waker, or invoke the prompter. A successful permit is held until the
tool returns or its future is dropped.

The prompt future remains owned by the tool future. Dropping an unpolled tool
future invokes no prompt. Dropping a pending future drops the prompt future and
releases its permit. The adapter starts no thread, task, channel, timer,
runtime, retry, or detached work. A conforming prompter must keep interaction
work owned by the returned future or perform its own complete drop cleanup.

There is no tool timeout. The host/executor may apply an outer deadline, but
this slice neither accepts nor starts one. A stalled or blocking injected
prompter can therefore stall its call; that violates the injected boundary and
cannot be repaired by the portable adapter.

The default active-prompt limit is one. An explicit constructor may select
one through eight. Zero and values above eight fail construction with a fixed,
data-free invalid-limits error. Each tool instance owns its counter; there is
no process-global registry.

## Outcomes, answers, and precedence

The prompter returns one of three structured outcomes:

- `Answered` with an ordered vector of strings;
- `Cancelled` for an explicit user cancellation; or
- `Unavailable` for a noninteractive host.

`Answered` must contain exactly one answer per prepared question. Each answer
is ASCII-trimmed, must remain nonempty, and is checked against the per-answer
and aggregate raw limits. Machine-god then applies the same terminal-safe
encoding and aggregate rendered limit used above. It deliberately does not
require an answer to equal an option label. This admits a bounded `Other`
answer. The only claimed parity with pinned fx's answer codec is the absence
of option-label membership enforcement; machine-god additionally ASCII-trims
answers, rejects empty answers, enforces raw/rendered/result bounds, and
applies its own terminal-safe encoding.

Successful answers produce deterministic ordered JSON as the `content` of a
non-error `ToolOutput`:

```json
[
  {
    "answer": "Bounded native seam",
    "question": "Which implementation should proceed?"
  }
]
```

The representation intentionally inserts `answer` and then `question` for each
object; array order equals input question order. This order does not depend on
the selected `serde_json::Map` implementation, lexical map behavior, or
feature unification. The rejected cycle-1 candidate inserted `question` before
`answer` and happened to serialize in the documented order only with the
current lexical-map dependency behavior. Cycle 2 expresses and tests the
intended insertion directly. Questions are the
exact normalized strings shown to the prompter. No option, description,
internal ID, timing, or host metadata is returned. Pinned fx emits the same two
object members in the opposite textual key order; JSON object order is not
semantic, so this slice makes no byte-level fx result claim.

An explicit user `Cancelled` outcome returns the successful string content
`(user cancelled the question)`. `Unavailable` returns the successful string
content `(ask_user_question is only available in the interactive shell; ask the
user freeform instead)`. These sentinels are not answer arrays and cannot be
misread as authorization.

Engine cancellation has precedence at first poll, immediately before prompt
invocation, and after every ready prompt outcome before interpretation. The
rejected cycle-1 candidate checked cancellation and then cloned up to 16 KiB of
question presentation text before invoking the prompter. Cycle 2 adds the
adjacent pre-invocation cancellation check after that last intervening work,
so observable cancellation prevents UI invocation.
Cancellation that is observable in the same poll as answers, user
cancellation, unavailability, or host failure wins and returns the fixed
cancelled tool error. After cancellation wins, no answer result is published.

For a non-cancelled ready result, precedence is:

1. redacted prompter failure;
2. answer-count mismatch;
3. per-answer raw validation in question order;
4. aggregate raw answer limit;
5. terminal rendering and aggregate rendered-answer limit;
6. serialized result limit; and
7. ordered success publication.

No partially validated or partially encoded answer array is returned.

## Fixed failures and redaction

| Condition | `ToolErrorKind` | Code | Message | Retryable |
| --- | --- | --- | --- | --- |
| Malformed shape, type, field, empty required text, or duplicate label | `InvalidInput` | `ask_user_question_invalid_arguments` | `ask_user_question arguments are invalid` | no |
| Incoming, field, presentation, normalized, answer, or result limit | `InvalidInput` | `ask_user_question_resource_limit` | `ask_user_question resource limit exceeded` | no |
| `permission_request_id` present | `InvalidInput` | `ask_user_question_permission_request_unsupported` | `ask_user_question permission escalation is not supported` | no |
| Invalid configured active limit | construction error | n/a | `invalid ask_user_question limits` | n/a |
| Active prompt limit already full | `Unavailable` | `ask_user_question_busy` | `ask_user_question prompt capacity is exhausted` | yes |
| Prompter failure | `Execution` | `ask_user_question_prompt_failed` | `ask_user_question prompt failed` | no |
| Wrong answer count or malformed answer | `Execution` | `ask_user_question_invalid_response` | `ask_user_question prompt returned an invalid response` | no |
| Engine cancellation | `Cancelled` | `ask_user_question_cancelled` | `ask_user_question was cancelled` | no |

Argument precedence is serialized-input limit, root type, deferred
`permission_request_id`, other root keys, `questions`, and then each ordered
question/option pipeline. Resource-limit failures at a field's exact check take
precedence over later semantic checks. Errors retain no input, answer,
question, label, description, prompter diagnostic, session identity, or
executor text. Tool, request, prompter-error, limits-error, and prompt-outcome
debugging is fixed or structural and never invokes user-defined `Debug`.

## Platform and host composition

The tool and injected seam use safe standard Rust, allocation, atomics, and
core futures only. They are exported by `machine-god-native` without an HTTP,
Unix, or non-WebAssembly feature gate and must compile on the repository's
native, FreeBSD, and WASI library targets. The slice supplies no browser bridge
or WASI terminal interaction; a caller must inject a portable deterministic or
unavailable prompter there.

The production `NativeReferenceHost` remains gated to its existing
Linux/macOS, non-WebAssembly, `ai-gateway-http` boundary. Its constructors gain
an explicit shared `QuestionPrompter`, register `ask_user_question` first in
the alphabetical tool catalog, and do not discover a terminal, TTY,
environment variable, file, or runtime for it. The exact sixteen-tool order is
`ask_user_question`, `copy_file`, `create_folder`, `delete_file`, `edit_file`,
`file_info`, `glob_files`, `grep_files`, `list_files`, `open_file`, `read_file`,
`rename_file`, `terminal`, `web_fetch`, `web_search`, and `write_file`.
Thirteen are descriptor-backed; the question and web tools are rootless.

## Pinned-fx relationship and deferrals

Pinned fx supplied the 1-4/2-6 schema, question/label ASCII trimming,
case-insensitive label deduplication, terminal-safe presentation, ordered
answer JSON, cancellation sentinel, noninteractive sentinel, and a result
codec that does not require answers to match option labels. Machine-god uses a
different stricter answer boundary: it trims and rejects empty answers,
applies explicit byte bounds, and terminal-safe encodes them. This slice also
adds strict unknown-field/type validation and explicit resource/concurrency
bounds. It intentionally omits fx's `permission_request_id` approval
escalation.

Deferred work includes:

- `permission_request_id`, auto-denied continuation, approval revalidation,
  grant caching, and permission modes beyond the delivered ask-only policy;
- a concrete terminal, graphical, browser, remote, or CLI question UI;
- timeout, detached or background prompts, prompt persistence/history, resume,
  notification, and multi-process capacity;
- multi-select, default choices, answer membership enforcement, and unbounded
  open-ended input;
- durable terminal actions, `vision`, `read_tool_result`, Milestone 05 skills,
  MCP, ACP, and subagent surfaces; and
- benchmark-workload changes, compatibility promotion, product-performance,
  or fx-equivalence claims.

Zig remains only the pinned upstream fx build input. Machine-god remains a Rust
product and neither ships nor executes Zig.
