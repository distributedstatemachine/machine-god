# Native slash-command catalog and routing

The native slash catalog is a pure, allocation-free component. It declares
commands in the pinned `general`, `session`, `model`, `security`, `workspace`,
`agents` and `extensions` categories and the `/exit` alias. It parses command envelopes,
provides completion rows, and searches categorized help. It does not execute
commands, validate action-specific payloads, own a UI, or claim that the runtime
behavior of these categories is complete. Delivery state and milestone ownership
remain only in the [implementation plan](implementation-plan.md).

The source reference is `vercel-labs/fx` revision
`b1774fbf6c7602b503026f96f6e960e946c692ef`, particularly
`src/builtins/commands.zig`, `src/core/slash_commands/command_specs.zig`,
`src/core/slash_commands/command_router.zig`, `src/core/mods/registry.zig`,
and `src/core/app/input_submit_runtime.zig`.

## Registry and effect ownership

`native_slash_registry()` returns static `NativeSlashSpec` entries in the
pinned relative registry order. `NativeSlashCommand::spec()` returns one entry.
Descriptions and help grammar retain the pinned catalog wording, including the
upstream product name. They describe intended upstream actions, not proof of
implemented machine-god effects. Category labels use pinned presentation order.

| Category | Commands | Payload-bearing envelopes |
| --- | --- | --- |
| General | `/help`, `/clear`, `/status`, `/version`, `/quit` (`/exit`) | None |
| Session | `/new`, `/reset`, `/resume`, `/continue`, `/rename`, `/undo`, `/copy`, `/compact` | `/rename` |
| Model | `/model`, `/models`, `/fast` | `/model` |
| Security | `/permissions`, `/allowlist`, `/sandbox` | All three |
| Workspace | `/workspace` | `/workspace` |
| Agents | `/background` | `/background` |
| Extensions | `/skills` | `/skills` |

The payload is an unparsed remainder. For example, `/sandbox vercel`,
`/workspace add`, and `/permissions revoke nope` have valid command envelopes;
their concrete handlers must reject invalid actions or arguments before any
effect. A valid envelope is not an authorization decision or successful action.
The catalog intentionally does not duplicate handler-specific validation of
paths, titles, model IDs, permission identities, JSON, or allowlist targets.

`/help` is a no-payload command in the pin and opens the help menu. `/help search`
is not its search grammar: `native_slash_help(query)` is the separate search API.
Likewise `/resume` takes no session ID payload in this slash registry.

The full security grammar includes permission modes and reset, exact saved-session
`remember <allow|deny> <tool-name> <arguments-json>` and `revoke <rule-id>` forms,
effective/local/user allowlist views, local/user add/remove/reset operations,
and sandbox `os|none`. These remain concrete handler obligations even where the
pinned short help label omits rule-management forms. Catalog recognition and
completion must never stand in for permission persistence or enforcement.

## Raw command router

`route_native_slash(input)` returns `NativeSlashRoute`:

- `NotLocal`: no registered token in this catalog. This includes
  ordinary text, unknown or later-category commands, paths, case mismatches,
  and input with leading whitespace.
- `KnownInvalid { command }`: an exact known token followed by invalid separator
  whitespace, or a no-payload command followed by an argument.
- `Valid(NativeSlashInvocation { command, payload })`: a valid command envelope;
  the borrowed payload is trimmed only at surrounding ASCII space/tab bytes.

Command tokens and `/exit` are case-sensitive. The separator between token and
payload is one or more ASCII spaces or tabs, never CR/LF. No-payload commands
permit trailing spaces/tabs but reject CR/LF or any argument. Payload-bearing
commands accept the bare token with empty payload. Payloads are neither
shell-tokenized nor unquoted, and newlines inside a payload remain intact.

Invocation `requires_prompt_credential()` reports the pinned submission
preflight requirement: `/continue` always requires it; `/model` only when its
payload is nonempty after space/tab/CR/LF trimming. No other catalog command
requires this preflight. This is metadata, not credential acquisition.

## Separate submission policy

`resolve_native_slash_submission(input, context)` first left-trims spaces, tabs,
CR, and LF, as the pinned interactive submit path does. Explicit
`NativeSlashSubmissionContext` facts describe picker activity, dismissal, and
an optional visible completion index. The component never infers those facts
from a terminal or owns mutable picker state.

When a completion menu is explicitly visible and candidates exist, the selected
index wraps modulo the candidate count and its static replacement is routed.
Otherwise the left-trimmed original is routed. Known malformed commands remain
local `KnownInvalid` results rather than becoming provider prompts.

A raw unknown slash word becomes `UnknownLocal` only when it has no
space/tab/CR/LF and no second slash, the slash picker is active and not dismissed,
and no completion candidate exists. Unknown slash text with arguments and
absolute paths remain `NotLocal`. This policy only covers the supplied registry;
the embedding host must handle its other registered categories before treating
input as an ordinary prompt.

Paste expansion, direct-terminal commands, attachments and retained image
prefixes, credential acquisition, prompt history, rendering, and actual prompt
submission stay with their owning host components.

## Completion and help

`native_slash_completions(prefix)` consumes a raw prefix, without leading trim.
Command matches use exact, prefix, then substring rank; matching is case-sensitive
and stable within pinned registry order. Each primary command contributes at
most one row; aliases compete for its best rank, with the primary winning a tie.
Thus `/` includes `/quit` once, while `/ex` completes to `/exit`.

Argument rows use case-insensitive prefix matching in pinned table order:

- Sandbox: `os`, `none`.
- Permissions: `ask`, `auto`, `remember`, `revoke`, `yolo`, `reset`.
- Workspace: `list`, `add`, `remove`, `clear`.
- Allowlist: staged view/action/scope, target kind, reset scope, and the thirteen
  pinned tool-name suggestions, including local/user forms.

The pinned allowlist tool suggestions are not an exhaustive or authoritative
tool registry. They do not authorize a tool or promise it is implemented.
Allowlist stages preserve trailing spaces/tabs because those advance a stage;
other argument queries trim both ends. Argument rows have no category or
description. Their `label` is the current-stage suffix. `has_args` follows pinned
Tab behavior, including its omission on permissions remember/revoke rows.

`native_slash_completion_prefix(input)` separately performs composer left trim
and yields no prefix for no-argument commands followed by whitespace. It must
not be confused with the raw parser or used as action validation.

`native_slash_help(query)` returns static specs grouped General, Session, Model,
Security, Workspace, Agents, Extensions, preserving registry order within each category. Query tokens
split on space/tab/CR/LF and use AND semantics. Each token may match any of the
command, aliases, help grammar, description, or category label through
ASCII-case-insensitive substring search. Empty queries return all registered entries.
This is catalog search, not an ANSI rendering or interactive menu implementation.

## Bounds, redaction, and checks

Raw routing and submission reject inputs over 65,536 UTF-8 bytes before scanning.
Completion, composer-prefix extraction, and help reject inputs over 4,096 bytes
before scanning. Both bounds are inclusive and report fixed errors without input
reflection. Submission uses its independent 65,536-byte bound, including optional
completion resolution; it does not impose the smaller explicit query bound on a
valid long payload. This also preserves argument whitespace normalization for a
long submission containing many spaces before a short argument.

All catalog strings are static, input strings remain borrowed, and the APIs
allocate nothing. Iterators are fused and use fixed registry/argument-table bounds. Work is
bounded by the fixed registry, fixed argument tables, and input/query ceilings.
Invocation and iterator Debug output redact payload/query bytes.

Focused tests cover every token/alias and envelope, raw-versus-submit whitespace,
credential metadata, malformed/unknown/path routing, selected-completion wrapping,
ranking and alias deduplication, all argument stages, categorized AND-token help,
inclusive byte bounds, fused iteration, redaction, and zero allocations. Runtime
scenario evidence, freshly built CLI behavior, complete feature reviews, and
remote gates are separate requirements; this component alone satisfies none of
those delivery claims.
