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
