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

## Native prepared policy targets

`NativePermissionTargetAuthority` receives a retained directory `File`, its
canonical absolute workspace spelling, and explicitly registered real native
tool implementations. Construction is inert. Its owned preparation future first
bounds the actual borrowed JSON, then checks canonical arguments and the exact
requested capability against that trusted implementation. Ordinary validation
calls contextual preparation with the exact request session, incarnation, turn
and invocation call ID, so scope-aware tools observe the same captured turn as
core preparation. This adds no permission grant or filesystem lookup. Unknown tools are
rejected; a registered name or risk hint alone cannot establish a builtin bypass.
The typed question-tool registration uses its prepared-presentation validator,
so a valid 1,024-byte raw question expanded to 4,096 terminal-safe bytes is not
rejected by applying the incoming-text limit twice. It acquires no prompt slot.
Likewise, typed grep registration validates its actual execution object, including
the canonical `include: null` default that the incoming grammar does not accept.
Dynamic tools require a separate trusted schema-bearing adapter. The host must
register implementations bound to the same workspace authority.

Ordinary arguments retain the core 65,536-node/64-depth envelope and a 1-MiB
encoded ceiling. Terminal preparation uses its existing complete prepared-byte
and node bounds instead. These are independent of the smaller reviewer packet
and context limits: inability to fit a review does not truncate execution identity.
All diagnostics omit arguments, paths and native failures.

Existing paths are walked from the retained root without following directory
symlinks. `file_info` preserves final-entry metadata semantics without opening
the selected file for content. Create-folder preparation observes missing entries
without creating them, evaluates target and parent separately, and grants no
Auto bypass for pinned sensitive paths. Prepared observations retain identities
and selected descriptors for explicit native revalidation. Revalidation is not a
core no-I/O admission guard or an atomic final-effect check. The five file
mutations instead project targets from `PreparedFileApproval`; their separately
retained file evidence remains responsible for final-effect validation.

Terminal preparation validates the private canonical envelope against the actual
`TerminalActionTool`, including host/call stamps, digest and repeated-probe
capability. It does not reinterpret that envelope as provider input. The native
terminal host attaches its actual captured resolver automatically: one owned
worker resolves the immutable default cwd and native path semantics, retains
the resulting directory descriptor, and selects the same foreground/start shell
used by execution. At most four permission-resolution workers run concurrently;
each checks a two-second first-poll deadline and cancellation around native
boundaries. Indivisible filesystem calls are not preempted. Dropping the future
cancels its work, and the real host's stop token and worker scope own shutdown
and completion; retaining a resolver does not keep the terminal runtime alive.

Typed terminal results expose selected shell, environment/selection digests and
cwd ownership. Semantic argument identity excludes private call/host stamps;
configured bash matching uses resolved cwd plus command text. Neither is a
direct-execution plan. Commands receive no fabricated read-only bypass. Auto
terminal bypass is exactly read, screen, list, or inspect without acknowledgement;
wait and other actions still need their ordinary policy. A standalone terminal
without a resolver cannot invent command cwd or resolve a workspace filter.

Multi-target evaluation preserves last-match semantics per target: any Deny
wins, otherwise any Ask remains Ask, Allow requires every target to allow, and
unmatched targets remain unresolved. These prepared outcomes do not grant,
persist, prompt, or execute; the native controller and final-effect adapters own
those responsibilities.

## Concrete native action preparer

`NativeToolPermissionPreparer` composes the target authority, file authority and
registry, exact live contexts, real reviewer, and an explicitly supplied owned
worker scope. Its constructor and unpolled preparation futures are inert. The
host binds its controller once through a weak reference and attaches both
controller and context registrations to the actual conversation. At most four
preparations/admissions are retained. Borrowed arguments and request capabilities
are bounded before copies; native target observation runs on owned workers.
Cancellation or dropping preparation cancels its work, while the supplied scope
retains actual worker completion. A retained context cannot revive authorization.

Saved and grant identities are separately owned, length-framed and bounded to
4,096 bytes without truncation. Oversized identities remain unavailable for reuse
without disabling once-only approval. Command keys include actual resolved cwd,
shell executable/profile, environment and selection digests, execution controls,
and the taken effective sandbox selection, excluding private call/host stamps.
File keys include the canonical workspace and complete existing content identity.
Read grants use the canonical workspace and target independently of read ranges;
live turn/session read grants satisfy configured write/edit disclosure checks.
Reset, turn closure and uncertain rule publication invalidate that lookup.

Automatic review borrows the original pending call and proven root context,
never canonical private envelopes as user intent. It receives complete prepared
file evidence or the actual resolved command; Ask/failure stays with the
controller's recoverable denial path and does not prompt a human. Workspace
write/edit bypass remains tool-specific and excludes pinned sensitive paths and
unresolved configured read disclosure. Unknown dynamic tools need a distinct
trusted schema-bearing adapter, not a guessed builtin identity.

All five mutation admissions retain the exact registry proof and live controller
policy through the existing final publication checks. Exact-turn cleanup retires
unclaimed proofs. Ordinary core admission retains bounded evidence and checks
live policy without native I/O; it does not claim an atomic target-check/effect
operation. The host must use the same workspace and registry allocations for
preparation and execution; file/target directory agreement is checked on the worker.

## Automatic permission reviewer

`AiGatewayPermissionReviewer` implements `NativePermissionReviewer` over an
explicitly injected `AiGatewayTransport` and `NativePermissionReviewClock`.
Construction and unpolled review futures are inert. A polled review owns one
15-second monotonic deadline, one transport attempt, and all startup/stream/timer
futures. Cancellation and deadline readiness are checked before and after work,
including same-poll completion and owned-state teardown. Dropping the review
releases those futures; it does not spawn a detached task or retry. Production
`TokioPermissionReviewClock` is available with `ai-gateway-http` on non-WebAssembly
targets and uses the host's existing Tokio runtime. The injected reviewer and
clock contracts remain available on WebAssembly, including all-feature builds;
enabling native HTTP features does not introduce a Tokio dependency there. CI
cross-compiles the all-feature library and portable reviewer tests for WASI in
addition to the no-default-feature unsupported-tool checks.

The dedicated wire model is always `zai/glm-5.2`, with required tool choice,
2,048 maximum output tokens and no inherited effort/fast controls. The complete
encoded request is bounded to 16 KiB. The pinned policy text is preserved from
fx `auto_classifier.zig` at `b1774fbf6c7602b503026f96f6e960e946c692ef`.
The existing Gateway codec handles bounded fragmented SSE, strict JSON and
streamed argument integrity. Its private reviewer finish mode accepts pinned
stop/length/other completions containing a valid decision; the ordinary provider
retains its stricter call/finish correspondence. Content filtering is a permanent
review failure, provider unavailability and retryable transport failures are
transient, and malformed
assessment output is invalid. None causes a retry or a fabricated assessment.

`NativeAutoPermissionReview` borrows the successful pending assistant message,
exact target call ID, explicit prepared action/targets and separately typed
`NativeAutoPermissionRootContext`. The latter accepts only a bounded 1,024-byte
canonical proven-root projection, strips historical permission feedback, and
does not infer provenance from a message role. Its caller must own the actual
user provenance. Only the exact uniquely identified pending call is forwarded;
assistant prose, JSON/image attachments and sibling calls are omitted. A synthetic
tool result explicitly says the call has not executed. Action evidence is
terminal-safe and XML-escaped inside the policy, never promoted to user authority.

Command, generic-tool, prepared-file and sandbox-widening actions have distinct
borrowed inputs. Missing required schemas or reactive restricted results reject
the review before transport. File review derives a complete delete/insert
presentation from borrowed pre/postimages without filesystem access or an LCS
allocation; byte counts, final-newline flags and absent/file/empty-directory
identity preserve distinctions between otherwise similar line views. Invalid
UTF-8 bytes are visibly escaped. Oversized packets or evidence that the pinned
secret detector would mask are rejected, not silently truncated or auto-approved. Selected call arguments
also have a 4,096-node/64-depth bound before bounded serialization.

Accepted output has exactly one `permission_decision` call and no non-whitespace
prose. Its object has exactly `risk`, `authorization`, `decision`, and `rationale`;
the rationale is nonempty and at most 240 UTF-8 bytes. Decisions are only `Allow`
or `Ask`; risk and authorization are informational and never veto an allow.
Public assessment construction enforces the same bounds for injected deterministic
reviewers. Debug/error formatting omits action, context, rationale and transport
content. This reviewer does not itself grant execution, persist rules, or prompt
a human: the native permission controller owns Auto recovery and admission.

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

## Owned file-approval preparation and execution

On Linux and macOS, `NativeFileApprovalAuthority::from_directory(File)` accepts
an explicitly supplied retained workspace directory for selected-file approval
reads. It never chooses a root from the environment or reopens a host root path.
`NativeFileApprovalRegistry::prepare(&authority, &request, invocation, cancellation)`
validates the concrete canonical arguments and exact requested capability before
bounded copying; its owned future reserves capacity and performs descriptor work
only when first polled. `Tool::prepare` remains effect-free. This additional
authority is separate from ordinary mutation authority and from undo tracking.

Preparation supports `write_file`, `edit_file`, `delete_file`, `rename_file`, and
`copy_file`. `PreparedFileApproval` owns the root, existing parent and selected
target/source descriptors, exact stable preimages, operation, arguments, and
expected resulting bytes. Missing targets differ from empty files; unreadable,
unsupported, unstable, symlink, and oversized observations fail closed rather
than producing a permissive unavailable snapshot. Missing parents are rejected,
never created. Delete accepts an explicitly verified empty directory using at
most two dot entries and one end/nonempty witness, without recursive traversal.
Edit uses the same bounded exact-one matcher and postimage builder as execution.
Copy and rename expose both source and destination; their destination must be
absent. Copy retains its complete source once, and borrows that same allocation
as its expected result.

Explicit workspace-context composition uses the exact active turn's retained
root for each endpoint. The reviewed paths remain canonical logical paths;
private descriptor-relative paths do not replace the review identity. Source
and destination may belong to different active roots, including identical
relative basenames. The endpoint adapter and permission preparer share the same
bounded logical/private path projection. Their observations and inverse-operation
locations retain each root independently. Cross-device rename remains an error, without a
copy/delete fallback. Missing or expired workspace contexts do not fall back
to primary-root authority. Ordinary scoped path observations likewise retain
their selected roots and expire with the owning turn.

Each complete preimage is bounded to 16 MiB; edit retains its existing 48-KiB
preimage/result bounds. A registry admits at most four retained preparations,
admissions, or executions. Its derived retained payload ceiling is four times
16 MiB plus 256 KiB for bounded arguments, paths, identities, and write/edit
results per slot. Each file read/compare has at most 4,096 native calls, at most
16 cumulative interrupted results, and an exact-size overflow witness. Copies
of borrowed preview data made by a caller are that caller's responsibility.

Borrowed `kind`, `tool_name`, `target_path`, `source_path`, `preimage`,
`source_preimage`, and `postimage` accessors support complete review evidence.
No preview is silently masked, truncated, or decoded lossily by this component.
`saved_rule_key()` constructs a length-framed UTF-8 content identity containing
the operation, exact argument digest, canonical endpoints, distinct preimage
states/digests, source digest, and expected-result digest. It deliberately excludes
runtime device/inode values. A content-equivalent replacement may therefore
match a saved rule only after fresh preparation; it cannot reuse the old runtime
approval. A key exceeding the independent saved-rule 4,096-byte limit is unavailable
for saving, without weakening or truncating the one-shot approval.

`PreparedFileApproval::admit(Arc<dyn NativeFileApprovalPolicy>)` constructs a
one-shot `NativeFileApprovalAdmission` for core's execution-admission seam. The
injected policy performs only bounded synchronous checks. Core consumption checks
that policy and transfers the exact generation's proof into the ready registry.
An explicitly configured tool's `with_file_approvals(Arc<Registry>)` requires a
ready proof: missing, denied, stale, foreign-workspace, mismatched-argument, and
already-claimed proofs fail closed. Registry identity includes session,
incarnation, turn, permission-request ID, actual tool name, and call ID, with a
checked generation allocator. Concurrent ambiguous call-ID reuse is rejected;
later sequential reuse is supported. Drop removes only its own exact generation.

Execution-future construction captures only a read-only ready-route identity and
generation stamp, without claiming it or doing filesystem work. An old unpolled
future cannot consume a newer permission request that reuses the call ID, nor can
a future constructed without approval acquire a later grant. Inside the inert
execution future, the tool claims that exact proof once before undo,
staging, or mutation. The claimed proof and live policy remain owned through the
effect. All five tools rewalk and compare retained parents/targets and complete
preimage contents at their final mutation checkpoint. Write/edit/copy also bind
the actual staged pathname and descriptor, ordinary mode, and complete bytes to
the approved result. The live policy is checked again immediately before the
publication, unlink, or rename. No registry mutex is held across policy callbacks,
prompts, descriptor I/O, or mutation. No undo mutex is acquired during preparation
or held across a prompt. Standalone tools without registry injection retain their
previous non-approval behavior.

The host must invalidate its live exact-turn policy and call
`close_turn(session, incarnation, turn)` on completion/drop, including an
admitted call whose execution future was never polled. Cleanup removes only that
turn's unclaimed entries; a claimed execution checks the separately invalidated
policy. A checked closure epoch also rejects preparation futures constructed
before closure and first-polled afterward, conservatively including an unrelated
turn closure. There are no detached workers or unbounded closed-turn tombstones.
Cancellation and drop release owned work without publishing a mutation; after
the native irreversible syscall, existing success/uncertainty semantics remain.

Errors and debug output omit paths, bytes, descriptor identities, OS diagnostics,
and policy text. Tool-side rejection is the fixed nonretryable
`file_approval_failed`; this component does not implement prompts, persisted rule
updates, reviewer transport, or host lifecycle wiring. Revalidation is not a
portable atomic filesystem/policy compare-and-swap: another actor can still act
between the final check and syscall, or after observed success. Existing native
postcommit verification and ambiguity handling remain authoritative.

## Native permission controller

`NativePermissionGovernedTool` is an explicit host wrapper over an exact trusted
tool allocation and the hosting engine's limits. It routes standalone
no-authority preparations through an exact tool-name/call-ID/prepared-arguments
capability, so configured policy can govern them. It bounds normalized JSON
before the extra capability copy and leaves existing concrete capabilities
unchanged. Complete-input/output limits, owned argument archives, extended tool
execution and prepared completion-wins cancellation semantics remain those of
the underlying tool. Standalone tools are unchanged; the wrapper does not grant
permission or classify an action as read-only.

`NativePermissionController` composes explicitly injected action preparation and
human prompts. Unlike the legacy adapter, it requires the actual prepared
invocation and a live exact-session/incarnation/turn registration. Native
conversations can register the controller; the runtime captures its mode and
configured patterns when taking a queued job, before awaiting core reservation.
Saved exact rules stay live, and are not copied into the taken-job snapshot.
The snapshot also retains configured sandbox mode separately from its effective
mode: Yolo selects effective None without replacing the configured preference.
Changing sandbox selection affects future taken jobs only; reset preserves that
preference. Neither selecting nor observing these values establishes OS launch
enforcement.
The controller does not itself supply file descriptors, an Auto transport or an
OS sandbox. Those remain concrete native adapter/host responsibilities; selecting
a mode alone is not evidence of sandbox enforcement.

Runtime-bound permission owners share the conversation's Open/Quiescing/Retired
admission gate. Policy snapshots, mode/sandbox changes, reset, rule proposals and
newly polled rule operations reject unavailable ownership with redacted errors.
A confirmed saved-rule operation retains its admission permit through the whole
CAS/reconciliation future, including failure and drop. Quiescence therefore
waits for already admitted operations; releasing an unsuccessful quiescence does
not reset policy or discard pending observations. An admitted turn may finish
binding its exact policy and editor while quiescing using its private permit.
No retained handle or previously unpolled future can reopen a retired owner.
Attaching the runtime gate rejects already admitted standalone controls instead
of moving their ownership mid-operation. Both standalone control admission and
the shared runtime gate are bounded to 256 concurrent permits.

Successful retirement explicitly detaches model, permission, review-context and
observation routes even when old owner aliases remain alive. Removal matches
the exact registration allocation, so an old retirement/drop cannot remove a
fresh registration for the same session and incarnation. Old execution proofs
and review contexts remain unavailable. Detaching observations is not a save
receipt: pending facts and late settlements stay with their old owner, and
durable session history is not deleted.

Native action-preparation failures produce a fixed recoverable denial for
replanning, without a prompt, grant or execution. A missing selected target
therefore does not abort an otherwise healthy conversation. Unknown/closed
session routes, invalid canonical policy and failed prompt infrastructure
remain permission-service failures; this distinction never authorizes a
partially prepared action.

Configured denial wins before saved denial; saved allow and an existing exact
capability grant can satisfy configured ask. Unresolved configured ask requires
the human prompter. Remaining unresolved Auto actions use the prepared action's
reviewer; Ask or review failure is a recoverable denial for replanning, without
an immediate human fallback. Tool-specific proven bypasses belong to the trusted
preparer, not generic risk hints. Yolo skips ordinary configured/saved policy;
file mutations still check saved exact denial.

The owner retains at most 1,024 exact grants per session and routes at most 64
live sessions. Turn grants retire on exact-turn closure; session grants survive
turns but never restoration. Reset clears grants and invalidates pending ordinary
approvals before returning, preserves saved rules, and changes only future jobs'
mode to Ask. An already taken Yolo job keeps its captured mode. Core always
receives a once-scoped result and a consumed execution proof; the native owner,
not core, owns reuse. Closing/cancelling the actual core turn invalidates proofs
even if its native registration has not yet been dropped.

Saved-rule edits have an opaque owner-bound, single-use proposal and an expected
per-rule generation plus the owner's reset epoch. Hosts must obtain explicit human confirmation separately
before consuming a proposal. Active-turn writes use the turn-owned metadata
editor; idle writes use ordinary core metadata CAS. Neither writes directly to
the session store. Rule publication invalidates old proofs before awaiting the
store, and failure/drop leaves authority blocked until authoritative
reconciliation. Reconciliation can admit a new proof, but cannot revive one
invalidated by the attempted change. Transcript and unrelated metadata edits
are preserved. These are publication outcomes, not implicit retries or success
receipts after an uncertain save.

The final proof checks current canonical saved rules without cloning the
transcript. Native file adapters must retain it through their final mutation
checkpoint, alongside descriptor/preimage identity, rather than treating core
tool entry as the final effect boundary. A synchronous check is not an atomic
check-and-effect guarantee against a subsequent policy or filesystem race.
On Linux and macOS, `NativePermissionExecutionProof` implements
`NativeFileApprovalPolicy` directly for `PreparedFileApproval::admit`. The
preparer's exact-turn cleanup hook runs outside controller locks after the
registration is invalidated, allowing the file registry to retire unclaimed
approvals without keeping the controller or core turn alive.

## Owned interactive prompt bridge

`NativeInteractivePromptBridge` supplies shared permission and contextual
question prompters plus one uniquely owned `NativeInteractivePromptInbox`.
It acquires no input, output, thread, task, timer, environment, or session
authority. The host routes its existing input owner to this inbox.

The inbox explicitly activates an exact session/incarnation. Every activation
advances a checked scope generation, including reactivation of the same owner.
Prompt construction only observes that scope; first polling performs bounded
validation and FIFO admission. An old unpolled future cannot inherit a new
binding. Tokens combine the scope, a checked request generation, and inert
bridge identity. Requests retain exact permission request/turn identities or
the actual question `ToolContext`; reused call IDs cannot revive old tokens.

One prompt is displayed at a time. Polling returns its same token until
answered, cancelled, or dropped; empty observations do not self-wake. Replies
must match the displayed unanswered token and response type. All four permission
choices are preserved. Explicit cancellation returns Deny for permissions and
Cancelled for ordinary questions. AllowSession is an in-memory grant, not a
saved rule; saved-rule management keeps the separate confirmed controller API.

Limits allow one through eight outstanding entries and one through 67,108,864
aggregate request payload bytes; defaults are eight and 8,388,608. Replied but
unconsumed entries still count. Permission size uses bounded compact
serialization after depth-64/node-65,536 JSON validation; question size counts
normalized presentation and identity UTF-8 bytes. Bounded entry/question/option
counts cover structural overhead. Question responses independently retain the
existing 4,096-byte aggregate raw-answer bound per outstanding request. No input
is truncated. Payloads are not cloned during admission or included in debug or
error text; rejected and unpolled permission JSON is dropped iteratively.
Immutable views retain bounded payloads but no admission or grant; caller-held
views are outside outstanding-entry accounting.

Future drop removes only its exact registration. `deactivate` retires the scope
while allowing later activation; close and inbox drop permanently close it.
Queued, displayed, and already-replied entries are invalidated on retirement.
Ready responses are checked again after cleanup callbacks. Waker clone, wake,
drop, and payload cleanup run outside state locks. Hosts must pin input framing
to the displayed token and discard obsolete fragments instead of applying them
to a replacement prompt after a transition.

### Interactive saved exact rules

The interactive CLI adds `a` (propose saved allow) and `d` (propose saved deny)
only when the native preparer supplied an exact saved-rule identity. Existing
`y`/`yes`, `t`, `s`, and `n`/`no` choices retain once, turn, session, and deny
semantics. Selecting `a` or `d` does not grant, deny, or save anything by itself.
A second, separately flushed confirmation displays the pending request again
and requires `yes`; `no`, `n`, or `/cancel` dismisses the proposal and denies
the pending request. Buffered input from the earlier prompt cannot confirm
the newly displayed page. EOF, quit, signal, and session retirement invalidate
unconfirmed prompt authority.

`PermissionPrompter::prompt_with_rule` is a source-compatible contextual method;
its default calls the ordinary prompt without saving. The opaque
`NativePermissionRulePrompt` originates only from the prepared native action.
The inbox proposes against its current displayed, unanswered token; the CLI
never builds canonical keys from display strings, capability JSON, or model
prose. The proposal retains weak exact session/turn ownership, reset and rule
epochs, and prompt lifetime. Reply, cancellation, future drop, or inbox retirement
invalidates it. Publication rechecks these guards immediately before arming
the existing metadata CAS. One extra canonical identity is covered by a bounded
4,352-byte prompt accounting allowance. Native debug output omits its content.

Confirmation submits the existing native control operation, which progresses
while the original prompt remains pending. A success receipt means the saved
rule was published; it does not renew the original execution proof. The CLI
denies that pending request after the save receipt. A later explicit request
must undergo fresh native preparation before the saved allow/deny can apply;
no automatic retry is introduced. Failed or interrupted publication keeps the
existing native uncertainty semantics, not an assumed rollback or success.
Already accepted saves remain native-owned through cancellation and shutdown.

`/permissions rules [offset]` lists the active session's saved exact rules with
stable IDs, decision, kind, and escaped display identity. Output is bounded;
an omission row supplies the next zero-based offset. `/permissions revoke <id>`
proposes revoking that exact canonical ID and also requires a separately flushed
`yes` confirmation. Missing IDs, malformed rules, changed generations, and reset
epochs fail closed. Display identities are never lookup keys. The active session
metadata is the source; this command does not edit user/workspace configuration.
These machine-god command forms are intentionally distinct from configured
`/allowlist` patterns and volatile session grants. `/permissions reset` retains
saved rules, but invalidates proposals opened before that reset.

During an acknowledged permission/question prompt or saved-rule confirmation,
the exact `/permissions ask`, `/permissions auto`, `/permissions yolo`, and
`/permissions reset` forms remain available. They require input bound to that
currently displayed epoch: pre-display input, stale buffered lines, and extra
arguments cannot acquire policy-command authority. Mode changes affect future
taken jobs, not the active job's captured policy. Reset also retires old inbox
tokens and unconfirmed proposal pages; a later old answer cannot recreate a
grant or save. It does not claim rollback of an already admitted publication.

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

## Native automatic-review context

An explicitly shared `NativePermissionContexts` registry attaches through
`NativeConversation::with_permission_contexts`. It holds at most 64 weak routes
for exact session incarnations. The production preparer reads `snapshot(request)`
only during that request's authorization. The returned immutable context pairs
the actual taken model and permission-policy snapshots with the original pending
provider call and its canonical message/block position. Call IDs are not searched
or parsed; prepared action arguments and archived argument projections remain
separate. Core's scoped `Session::permission_invocation_snapshot` exposes only
this current call and conveys no root-user authority.

Native admission writes bounded index provenance in
`machine_god.permission_root_provenance` atomically with the actual user prompt
and turn reservation. Canceled queued input and unpolled prompt futures add
nothing. Continuation uses its proven original request; resumed sessions without
provenance remain unknown. Stored user-role messages, assistant prose, tool
results, images, attachments and repository text are never inferred as trusted
requests. This metadata records provenance, not permission grants.

The root projection follows pinned first/current/recent labels and omission
counts within 1,024 bytes, masking pinned secret spans and escaping terminal
controls before UTF-8-safe head/tail reduction. Each admitted raw request is
bounded to 256 KiB; provenance retains only current/first indices, at most 32
recent indices and an omitted count, not duplicate transcript text. Invalid
provenance or bounds fail with redacted errors. The shared masking detector does
not change the reviewer's rejection of incomplete action evidence.

Context lookup performs no I/O, and snapshots retain only the single pending
call and bounded taken-turn context. Cancellation, authorization return/drop and
native finalization retire access; retaining a snapshot cannot keep a session
or turn alive. Hosts must honor `is_live()` and use `trusted_root_context()`;
missing or retired context cannot substitute an authorization reason or a
generic history-role projection. Queued jobs capture model and policy when
taken, so later selection changes cannot rewrite an active review.
