# Native ask permission handler

`AskPermissionHandler` is an executor-neutral native adapter from core's
`PermissionHandler` boundary to an explicitly injected `PermissionPrompter`.
The adapter owns no prompt presentation or input authority. A host may place a
terminal, graphical UI, remote approval service, or deterministic test double
behind the prompter, but none is selected implicitly.

## Public contract

```rust,no_run
use std::sync::Arc;

use machine_god_core::{BoxFuture, PermissionRequest};
use machine_god_native::{
    AskPermissionHandler, PermissionPromptDecision, PermissionPromptError,
    PermissionPrompter,
};

struct HostPrompter;

impl PermissionPrompter for HostPrompter {
    fn prompt(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async move {
            let _auditable_request = request;
            Ok(PermissionPromptDecision::Deny)
        })
    }
}

let owned = AskPermissionHandler::new(HostPrompter);
let prompter: Arc<dyn PermissionPrompter> = Arc::new(HostPrompter);
let shared = AskPermissionHandler::shared_prompter(prompter);
# let _ = (owned, shared);
```

The exact constructor and prompt surfaces are:

```rust,ignore
AskPermissionHandler::new(prompter: impl PermissionPrompter) -> Self
AskPermissionHandler::shared_prompter(
    prompter: Arc<dyn PermissionPrompter>,
) -> Self

PermissionPrompter::prompt(
    &self,
    request: PermissionRequest,
) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>>
```

`PermissionPromptDecision` is a closed structured host result:

| Prompt decision | Core decision returned by the adapter |
| --- | --- |
| `AllowOnce` | `PermissionDecision::Allow { scope: Once }` |
| `AllowTurn` | `PermissionDecision::Allow { scope: Turn }` |
| `AllowSession` | `PermissionDecision::Allow { scope: Session }` |
| `Deny` | `PermissionDecision::Deny { reason: "permission denied" }` |

The scope mapping records the host's decision faithfully. Neither core nor this
adapter caches positive grants or automatically authorizes a later request.
Any future identity-safe grant cache is a separate host policy feature.

## Request and audit boundary

For engine-driven calls, core constructs and bounds the complete
`PermissionRequest` before invoking the handler. Core validates the prepared
tool arguments and capability byte/depth/node limits, supplies validated
session, incarnation, turn, call, and permission identifiers, and fixes the
current tool risk and reason. Core also emits `PermissionRequested` with a
clone of that request before authorization.

The adapter passes its owned request directly and exactly once to
`PermissionPrompter::prompt`. It does not clone, mutate, serialize, truncate,
revalidate, or traverse the request, its capability, or its reason. The host
prompter therefore receives the same auditable value that core supplied to the
handler. A prompter must treat all request fields as potentially sensitive
host-facing data and must not mistake presentation truncation for a change to
the authorization input.

The core engine bounds a returned denial reason before staging the
host-facing `PermissionResolved` event. This adapter always returns the much
smaller fixed denial string. A denial never starts the tool. Prompt
infrastructure failure is not a denial and never becomes approval; it fails the
turn through core's permission-error path.

## Fail-closed diagnostics

`PermissionPromptError` is a public zero-data type. `new()` and `default()`
construct the same value, its display is exactly `permission prompt failed`,
and its debug output is exactly `PermissionPromptError`. A prompter cannot attach
terminal, UI, transport, path, credential, request, or operating-system text to
the error returned across this interface.

The adapter discards the prompt error value and constructs a core
`PermissionError` with only these constants:

| Constant | Exact value |
| --- | --- |
| `ASK_PERMISSION_PROMPT_ERROR_CODE` | `permission_prompt_failed` |
| `ASK_PERMISSION_PROMPT_ERROR_MESSAGE` | `permission prompt failed` |
| `ASK_PERMISSION_DENIED_REASON` | `permission denied` |

`PermissionError` has only a code and message, so this adapter makes no retry
classification claim. `AskPermissionHandler` debugging is exactly
`AskPermissionHandler { .. }`; it does not format the injected prompter.

## Configured permission patterns

`NativeConfiguredPermissionRules` is a pure, explicitly supplied ordered list
of `NativeConfiguredPermissionRule` values. It is separate from saved exact-action
rules and ephemeral capability grants: constructing or evaluating it grants no
authority and does not load config, persist policy, or change `AskPermissionHandler`.
`decide(&NativePreparedPermissionTarget)` returns the last matching
`Allow`, `Ask`, or `Deny`, or `None` when unresolved. The caller separately owns
multi-target aggregation, modes, grants and actual execution enforcement.

Rule constructors trim only space, tab, CR and LF from permission/pattern keys;
empty permissions are invalid and empty patterns remain meaningful. The complete
serialized ordered array, including each `permission`, `pattern`, and `action`
field, JSON escaping, commas and brackets, cannot exceed the existing 64-KiB
`MAX_CONFIG_BYTES`. Configured rules do not use the saved-rule 1,024-entry cap.

Matching follows pinned fx `permissions.zig` (`permissionNameForTool`,
`patternForRuleMatch`, and `evaluateRulesetForTool`) at
`b1774fbf6c7602b503026f96f6e960e946c692ef`. Categories match the pinned aliases
or actual tool name. `*` matches any bytes, including slashes; `?` matches one
byte, not a Unicode scalar. There are no character classes, escaping or regexes.
A literal `directory/**` also matches that directory itself. The evaluator is
nonrecursive and shares a 4,194,304-step budget across one target evaluation;
exhaustion returns `Limit`, never an earlier partial allow.

Prepared targets borrow the tool name, workspace, target string and explicit
`NativePermissionTargetKind`. Core tool-name validation, NUL checks, a 4-KiB
workspace bound, and a 73,856-byte framed-command target bound apply. The caller
must supply the already-prepared canonical path/command identity: construction
does not resolve paths, inspect files, infer target kinds or establish authority.
Path presentation is workspace-relative for the pinned path kinds and copy/rename;
other kinds retain their own presentation. Bash matching strips the prepared cwd
and `@fx-terminal-env:` length-framed shell identity; sandbox matching strips only
the environment identity. This projection is only for configured matching:
the original borrowed identity remains unchanged for exact grants and rules.

`web_fetch` is deliberately special: only the exact category and exact canonical
`domain:host` target/pattern match. Wildcard categories, wildcard domains, URLs,
ports, uppercase hosts and trailing-dot spellings cannot authorize it. Invalid
configured domain patterns stay inert and are counted by
`web_fetch_warning_count`; they do not become generic wildcard rules. No DNS or
URL normalization occurs. Constructors and diagnostics retain no ambient authority,
and debug/error formatting omits rule and target content.

## Polling, cancellation, and authority

Calling `PermissionHandler::authorize` only creates an inert future. An
unpolled future does not invoke the prompter. Its first poll moves the exact
request into one call to `prompt` and polls the returned future. The adapter
starts no task, thread, timer, channel, runtime, retry, or detached work.

Dropping an unpolled authorization future drops the retained request without
prompting. Dropping a pending authorization future drops the underlying prompt
future and its retained state. The adapter sends no separate cancellation
notification and cannot revoke work that the prompter detached. Consequently,
a conforming `PermissionPrompter` must keep prompt work owned by its returned
future, or arrange its own drop cleanup so no detached approval operation can
outlive that future. Core's cancellation wrapper obtains prompt cancellation by
dropping the handler future; there is no second permission-specific
cancellation token.

The adapter is executor-neutral and does not read terminal input, write terminal
output, inspect environment variables, access files, start processes, contact a
network, discover configuration, or select a runtime. All presentation,
interaction, scheduling, and any associated authority belong to the explicitly
injected prompter. This adapter does not provide a concrete prompter, wire the
CLI, change the configured `ask` mode, implement modes beyond `ask`, or persist
grant decisions.

## Saved exact-action rule values

`NativeSessionPermissionRules` is the pure schema-1 value codec for
`machine_god.session_permission_rules`. An absent entry is an empty set with
`next_generation = 1`. Ordered rules contain a stable positive `id`, exact
`key`, `display_identity`, `allow`/`deny` decision and per-rule `generation`.
Keys combine the `command`, `file_mutation` or `structured_tool` namespace,
recomputed SHA-256 digest and original canonical bytes. Equality checks all
three fields, not just the digest. Identities are opaque data, not commands.

At most 1,024 rules are admitted; canonical and display strings each have an
independent nonempty 4,096-byte UTF-8 bound. Decoding checks exact shallow shapes,
duplicate IDs/keys, generations and native record byte/node limits before
cloning. Missing fields, unknown versions, forged digests and overflow fail with
redacted errors; unrelated metadata is not recursively traversed.

`apply_set` and `apply_revoke` return a validated new value without mutating the
old one. New rules use the next generation as their ID; replacement retains the
ID and advances that rule's generation. Every mutation, including revoke,
advances the checked allocator. Expected generations are per rule, so editing
one rule does not stale an unrelated proposal. Missing/existing rule mismatches
and stale generations fail without eviction or partial mutation.

These values follow pinned `src/core/permissions/session_permission_state.zig`.
Decoding or constructing them performs no prompt, confirmation, persistence or
execution. They remain separate from capability grants and the authorization
handler; loading a saved `allow` value alone does not grant tool authority.
