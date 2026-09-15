# Native reference-host composition

`NativeReferenceHost` is the maintained example of composing the
provider-neutral engine with native AI Gateway, persistence, prompt, root, and
tool implementations. It is a library boundary used by both the one-shot CLI
and its long-lived interactive session owner. Current
milestone state is maintained only in the
[implementation plan](implementation-plan.md#current-delivery-state).

The native host groups its engine, concrete session store/lifecycle, worker and
terminal services, and shared permission/model/observation registries in one
`NativeHostServices` allocation. Sharing this execution domain does not copy a
principal's mutable workspace selection, permission grants, undo history or
ephemeral MCP owner. Reverse runtime routes remain weak; the service allocation
must not own a manager which in turn owns conversations using that engine.

`NativeReferenceHostConversationOptions::with_managed_agents` explicitly selects
the managed-agent clock and outer ownership assembly. It requires complete
terminal, permission, workspace, model-route, observation and undo selections
before consuming prepared roots. The assembly shares the selected undo budget
and existing tool archive; it creates no journal and admits no model work during
construction. An absent MCP profile still supplies an inert, independently
instantiable empty MCP seed. The shared engine receives only weak mailbox and
principal MCP routes, including builtin permission preparation; an unregistered
conversation cannot fall back to host-global authority. The outer driver must
enroll actual parent/child conversations before execution and retain the manager
separately from `NativeHostServices`. Keeping the engine alone does not retain
the outer assembly or revive its routes.

`NativeReferenceHost::open_managed_agents` opens an explicitly supplied private
journal descriptor and transfers the assembly once to `NativeManagedAgents`.
Unpolled opening is inert; validation or journal-open failure preserves the
original assembly. The outer owner exposes bounded resident projections,
explicit reconciliation retry and caller-polled progress/shutdown. It uses the
same engine and worker scope, not another execution domain. Foreground streams
must be co-polled independently of display output.

`open_workspace_managed_agents` instead receives the explicit private state
directory descriptor and derives a private workspace/origin journal namespace
on the existing owned workers. It hashes the captured workspace identity, never
reopening that path as authority. Unpolled preparation is inert; existing
nonprivate directories and symlinks are rejected. Reopening the same namespace
does not bypass its journal's exclusive owner lease.

Managed construction creates no host-global MCP runtime or context route.
Configured child seeds stay in shared services, while the parent-only seed
stays with the outer owner. Actual parent enrollment creates its own MCP
runtime, contexts, permission bundle and optional ephemeral owner. Parent-only
ephemeral selection is preserved, not copied to children; every instantiated
owner has independent cancellation. Actual foreground enrollment preserves
saved model preferences, while child work keeps its explicitly captured choice.

`NativeInteractiveSession::open_managed` consumes that explicitly selected host
and private journal descriptor. Initial preparation applies any process model
override after saved preferences; later new/resume transitions use their normal
defaults and saved selection. The native owner enrolls and retains foreground
candidates through supersession, terminal handoff and uncertain outcomes. MCP
human controls resolve the selected foreground's own runtime/controller, never
a managed host-global fallback. Child progress is polled before presentation or
control backpressure, and interactive shutdown is not complete until the outer
manager has settled all child and foreground custody.

`NativeManagedInteractiveStartup` owns that manager before selection, without
allocating a provisional conversation. Opening is caller-polled and transfers
the interactive owner once; failed selection retains the manager for explicit
retry. Signal cancellation retains the original preparation future through MCP
and admission cleanup, including a session whose successful opening raced with
shutdown. The caller must settle startup before tearing down input or terminal
resources. Its constructor rejects a different actual host-service allocation
and returns the original manager for cleanup.

Managed parent preparation starts its own configured `All` MCP selection under
an actual journal-retaining admission cohort. An empty child creates no MCP
connections: its first durably accepted work item initializes configured servers
inside that work's native admission, before provider execution. Failed activation
is not automatically retried by another prompt; explicit reload remains the
repair path. Request-scoped ephemeral servers still never flow to children.
Recoverable initial activation failures settle their failed generation and
original admission without closing the selected controller or credential
service. The interactive owner remains available for `/mcp` repair and exposes
the fixed startup failure; a successful explicit reload clears it. Readiness
still blocks provider execution until the required selection is usable.
Supersession or shutdown signals only the uncommitted candidate's startup token;
the original future remains driven through its receipt and owned peer cleanup.
A replacement receives a fresh token. Failed initial terminal activation also
drives the already-enrolled manager's cleanup before returning its error.

Interactive workspace controls use the actual foreground's independent selection,
not the host's original defaults. New/resume preparation captures the settled
source selection under its quiescence guard and forks it for the candidate.
Later edits cannot mutate the retired source or another principal's selection.

`NativeReferenceHostManagedOptions::with_prompt_inbox` explicitly binds runtime
preparation to the existing bounded interactive inbox. A weak native registrar
enrolls each actual parent/child session before execution; its registration lease
stays with that runtime's cleanup resources, not the displayed selection. Closing
one principal invalidates only its pending requests and unconsumed answers.
Parent replacement cannot retire a hidden child's approval. Dropping the sole
inbox rejects later preparation instead of recreating an input or prompt owner.
`manages_prompt_inbox` checks the exact selected inbox allocation. Presentation
uses native registrations without taking a duplicate lease; a foreign inbox is
an error, not permission to create another prompt route.

Foreground quiescence keeps ordinary prompts and saves fenced while granting
only exact original notice cleanup a weak, generation-bound continuation route.
Source acknowledgement and confirmed outbox removal precede irreversible
retirement; raw saved outboxes and uncertain cleanup remain blockers. Transition
preparation also waits for the original managed run/admission cleanup, not only
the end of the visible stream. Dropping a reversible guard invalidates its
continuation route and allows ordinary cleanup again; a replacement generation
cannot reuse the old route.

## Availability

The complete reference host is compiled when all of these are true:

- the target is Linux or macOS;
- the target is not WebAssembly; and
- `machine-god-native` enables `ai-gateway-http`.

Individual native contracts may expose portable injected seams or narrower
system implementations. The explicit terminal-options constructors support
foreground and interactive terminal execution on Linux and macOS. Constructors
without those options retain the legacy terminal adapter: its foreground
executor remains Linux-only, while production `semantic_search` scans retained
workspace roots on Linux and macOS. That legacy catalog retains terminal `exec`
on macOS with its fixed unsupported result after strict preparation and
permission, while terminal `start` uses the
Linux/macOS background helper, terminal `read` uses the lazy starter's shared
process-local same-incarnation output registry, terminal `signal` uses its
identity-checked live native-control registry and bounded blocking executor,
with Linux ancestry-tree plus original-group delivery and macOS original-group
delivery, and
terminal `list`, `inspect`, and `wait` use its separately injected
descriptor-confined persisted-record reader without process authority. `wait`
receives its monotonic delays through
the same runtime-paired deadline authority used by web search and vision.

## Required selections

Composition requires these provider selections:

- provider `vercel_ai_gateway`;
- transport `ai_gateway_http`; and
- credential source `environment`.

Legacy constructors require permission mode `ask`, sandbox `none` and no
configured permission rules. They reject policy they cannot enforce. Explicit
conversation permission options compose `ask`, `auto` and `yolo`, configured
patterns and saved exact rules through the native controller described below.

The validated configured model is retained and used by the ordinary AI Gateway
provider and the legacy fixed-model web-search transport adapter. Explicit
conversation routing lets search capture the current session selection;
vision instead always uses dedicated `google/gemini-2.5-flash`. The
host retains the complete `LoadedNativeConfig`, including its observable
schema origin/version.

The legacy host configures both ordinary Gateway tool-argument decoding and engine
preflight with the terminal's bounded canonical argument envelope (417,865
bytes). This admits a full 64 KiB command plus cwd and JSON escaping without
truncating the command or its process-permission identity. Individual tools
still enforce their own semantic limits. Generic `EngineLimits` and
`AiGatewayLimits` defaults remain unchanged; response, chunk, result and total
request limits remain independently bounded.

The explicit terminal-options constructors instead retain ordinary Gateway and
core argument/transcript defaults and select a trusted terminal-only input
override (16,378,880 bytes and 2,308,160 JSON nodes). Complete inputs and outputs
have independent per-turn aggregate budgets, each admitting one maximum
terminal payload; multiple smaller calls share those budgets. The output
aggregate is the terminal's encoding-derived complete-output ceiling. These
bounds do not enlarge inline transcript references or unrelated tools.

## Constructors

`NativeReferenceHostTerminalOptions` is the explicit, data-only terminal
launch-selection seam. It requires a trusted machine-god CLI helper path,
an optional frozen account-shell path, and a bounded environment snapshot;
`with_tmux` selects an optional tmux executable. Helper and tmux paths must be
absolute Unicode paths of at most 4,096 bytes without NUL. Environment
validation shares the native transport's limits and duplicate-key rejection;
cloning options shares the validated environment allocation. Debug and errors
remain redacted. Construction does not open or probe executables, search
`PATH`, infer `current_exe`, inspect `SHELL`, snapshot ambient environment, or
query the account database. A missing account selection stays missing; an
explicit unsupported account shell follows the existing platform fallback
when a request later resolves it. These options alone neither construct a
host nor change an existing tool catalog. Trusted CLI code can explicitly
select its own helper-capable executable; library embeddings must not mistake
their executable or test runner for that helper.
On macOS this explicit capability also supplies the fixed private process-
inventory service used by terminal-session cleanup. Its inert registration is
shared across the host's transports; native startup owners prepare and lease
the helper under their existing deadlines. Configuration cannot retain an idle
helper or delay host completion, and unresolved cleanup remains owned. Inventory
does not grant process authority by itself; see
[ADR 0004](decisions/0004-macos-process-inventory-helper.md).

When conversation options select workspace authority, the complete terminal
host receives those same exact turn contexts whether or not native permission
options are also selected. Working-directory admission therefore uses the
captured primary/additional roots and state exclusion; omitting permission
composition does not revert terminal cwd resolution to the primary-only host.
A conversation must attach the host's workspace contexts before tool execution.
Hosts without workspace selection retain their existing terminal composition.

The root, transport, and MCP composition paths are:

| Roots | Transport | Constructor behavior |
| --- | --- | --- |
| Existing explicit workspace and session paths | Production AI Gateway HTTP | Opens and retains both roots, discovers the configured credential, and constructs the production transport |
| `PreparedNativeRoots` | Production AI Gateway HTTP | Consumes the already retained identity-checked roots without reopening their selected paths, then discovers credentials and constructs the transport |
| Existing explicit workspace and session paths | Injected `Arc<dyn AiGatewayTransport>` | Opens and retains both roots, skips credential discovery, and uses the supplied canonical `NetworkTarget` |
| `PreparedNativeRoots` | Injected transport | Consumes retained roots, skips credential discovery, and uses the supplied canonical target |
| `PreparedNativeRoots` and explicit terminal options | Production HTTP or injected transport | Adds the complete twelve-action terminal host, dedicated durable input/result archive, and host-handle-owned native lifetime |
| `PreparedNativeRoots` and explicit conversation options | Production HTTP or injected transport | Shares the caller's exact file-undo tracker across all five file mutation tools; optional terminal selection uses the same complete-terminal path |
| Existing explicit roots | Injected transport and `Arc<dyn McpToolCatalog>` | Uses the custom transport path and advertises search plus exact next-round selection over the injected admitted MCP metadata and attached executable source; feature access remains an inert empty authority |
| Existing explicit roots | Injected transport, `Arc<dyn McpToolCatalog>`, and `Arc<dyn McpFeatureAuthority>` | Adds bounded exact server-qualified resource, prompt, and completion access through the separately injected read-only authority |
| Existing explicit roots | Injected transport and `Arc<dyn ManagedSubagentAuthority>` | Adds allocation-bound managed commands while MCP authorities remain inert |
| Existing explicit roots | Injected transport plus MCP catalog, MCP feature, and subagent authorities | Retains all three exact extensibility allocations without probing or polling them |

Every ordinary path injects an inert unavailable `ManagedSubagentAuthority`. A
separate explicit subagent injection seam accepts the same root/transport
composition plus one trusted authority allocation. Neither path probes or polls
the authority during construction.

Complete-terminal composition shares its owned argument/result archive with the
managed tool. The Gateway override admits up to 448 KiB of managed arguments;
the concrete tool validates the complete command and publishes lossless input
and output references when inline transcript limits are exceeded. Permission
wrappers forward the unchanged actual-call proof. These per-tool limits do not
widen ordinary tools, and archive publication does not admit child execution.

Workspace mutation tools select undo history from the original conversation's
live turn scope, never from the currently displayed principal or a shared tool
field. Workspace registration checks actual session/turn allocation witnesses;
reusing public IDs cannot attach a foreign turn. A scope retains its original
tracker for settlement but targeted retirement prevents new use or rebinding.

`compose_ai_gateway_with_prepared_roots_and_conversation_and_credential_and_transport`
accepts an already discovered credential and a trusted one-shot transport factory.
It shares the production prepared-host construction path, retains the exact OIDC
or API-key source observation, and moves the token into the factory only after
the ordinary selection and retained-root checks. Factory configuration errors
retain the fixed `HttpTransport` stage. The caller supplies the canonical target
actually contacted by the returned transport and owns any factory effects; the
host never polls that transport during construction. Existing production
constructors continue selecting the pinned HTTP endpoint and default limits.
This programmatic injection adds no CLI flag, configuration or environment
endpoint override and does not bypass credential discovery in CLI acquisition.

Complete-terminal compositions also bind `web_fetch` resolver discovery to that
host's existing worker scope. Construction does not read system DNS configuration
for this unused tool: the first admitted hostname fetch starts one shared,
failure-retaining snapshot. Public IP literals need no resolver discovery.
Cancellation stops a fetch's wait, while actual discovery remains owned through
host finalization. Standalone compositions without a worker-owning terminal
resource retain synchronous construction-time capture. See
[web-fetch DNS ownership](web-fetch.md#dns-and-destination-confinement).

`NativeReferenceHostConversationOptions::new(Arc<FileUndoTracker>)` is an
explicit trusted-host authority choice, not a passive display preference. It
grants the five file mutation tools bounded preimage **read** authority and
retains their inverse-mutation authority in the caller's process-local tracker.
An ordinary write, delete, or rename permission decision does not by itself
grant those reads. Embeddings must separately choose this additional authority;
configuration, ordinary constructors, and terminal-only options do not enable it.
The constructor and its clones only retain the exact shared allocation: they
capture no file state, clear no history, and perform no inverse operation.

`with_model_routes(Arc<NativeConversationModelRoutes>)` optionally attaches
the same explicit routing allocation to web search. The trusted host registers
each runtime with `NativeConversationRuntime::new_with_model_routes` using
that allocation. Search snapshots current selection for its exact session
incarnation before capacity waiting; an unregistered context fails before
transport instead of falling back to the configured model. Composition never
reads the selection or registers sessions itself. Existing constructors without
this option retain fixed configured search models. Vision remains independent;
neither worker inherits main-turn effort or fast mode. See
[native conversation](native-conversation.md#secondary-worker-model-routing).

`with_observations(Arc<NativeConversationObservations>)` optionally shares the
native file-history registry with `read_file`, `list_files`, `glob_files`,
`grep_files`, `write_file`, `edit_file`, `delete_file`, `rename_file`, and
`copy_file`. Attach each created or resumed `NativeConversation` to that same
allocation with `with_observations` before admitting a turn. The adapters retain
the tools' existing preparation, permission, cancellation, and execution
contracts; native conversation finalization owns history publication. Composition
does not collect observations or write session history itself. Constructors
without this option retain their existing unwrapped tools.

`with_skills(Arc<NativeSkillsService>)` selects explicit human-invoked catalog
and optional managed-write authority. It requires terminal options so admitted
commands use the complete host's owned worker scope; missing terminal selection
fails validation before prepared-root consumption. Both prepared composition
paths preserve the same supplied service. `skills()` observes that capability
without discovery or execution. It is not implicitly copied to a replacement
host or constructed from ambient roots, and it does not widen model-facing
`skill` or `install_skill` permissions. See [skills CLI](skills-cli.md).

`with_mcp_management(Arc<NativeMcpManagementService>)` separately selects a
native profile-management service. Both prepared composition paths retain that
exact allocation and require complete terminal options for collected control
workers. The getter is inert; selection does not load configuration, connect a
server or change model-facing MCP authority. Human profile controls acquire the
accepted conversation's file-control fence only on first poll and preserve
publication receipts through cancellation and cleanup. Skills and MCP profile
controls share the same private owned-operation wrapper. Configuration saves
and runtime activation remain independent facts; see [MCP management](mcp-management.md).

`with_mcp_contexts(Arc<NativeMcpContexts>)` independently selects one shared,
weak exact-session/turn router. Both prepared composition paths preserve that
allocation; `configure_conversation_mcp` attaches a created or resumed native
conversation before turn admission. Interactive transitions and noninteractive
ask/resume use this same hook. Production CLI startup selects the router only
when an explicit native MCP profile was selected. Registration opens no server
and grants no tool permission. Hosts without this option remain unchanged.
Conversation finalization and retirement invalidate retained turn routes; a
router or snapshot cannot keep the conversation or engine alive.

`with_mcp_runtime(NativeReferenceHostMcpOptions)` additionally composes the real
native MCP runtime and archived tool executor. Its inert options select one
exact context router, injected clock and bounded runtime limits. Complete
terminal and native permission options are required; a separately supplied
context router must be the same allocation. Missing required options and
mismatched routers fail before prepared roots are consumed; the runtime validates
its finite limits during composition. Both prepared production and injected-transport
constructors preserve this contract; existing generic extension constructors
retain their supplied authorities unchanged.

`NativeReferenceHostMcpOptions::with_form_responder` retains the actual human
presentation endpoint and supplies it to the concrete executor before its runtime
policy is selected. The native human feature path retains the same endpoint,
not a second inbox or an ambient presenter. Without that endpoint, form support is not advertised.
Selecting it is inert and does not advertise URL completion support.
The interactive CLI selects its existing prompt bridge for this endpoint;
noninteractive ask/resume do not select a human presenter.

`with_background_url_opener(executable, captured_environment)` separately retains
explicit desktop authority. Both prepared composition paths require complete
terminal options and bind the opener once to that terminal's actual worker scope,
before selecting the MCP executor. No executable inspection, launch or environment
capture occurs during binding. Invalid optional environment authority leaves the
opener unavailable. The immutable host opener supplies the executor's URL launcher,
human resource/prompt continuation, interactive background controls and interactive authentication with one shared
admission through direct-child reap; it also works without MCP selection. Sessions
inherit that opener and reject a second session selection before preparing a
conversation, including when the first selection failed optional validation. A
session-only selection remains supported for hosts without a desktop selection,
but does not retrofit executor support. The production interactive CLI
captures desktop authority once before host acquisition; one-shot ask/resume
select none. Launcher availability never supplies consent or continuation proof.

Human feature commands derive their presentation owner from the exact retained
conversation's session and incarnation after native control admission. They do
not fabricate a model turn or tool grant. Resource reads and prompt gets can
collect modern input through the shared bridge while retaining their original
server, descriptor and command authority. The native runtime releases the peer
lane during human waits, but retains the bounded operation slot and checks
retirement before continuing; the CLI only projects prompts and owned receipts.

With the native `mcp-http` feature, `mcp::clock::TokioMcpClock` is an explicit
production clock selection for both runtime/startup and HTTP deadlines. Its
unpolled timers are inert and use the host's existing Tokio runtime on poll;
they create no separate runtime, detached task or worker owner.

`NativeReferenceHostMcpOptions::capture_startup` is the explicit effectful
production capture boundary, separate from inert options construction. On the
caller's owned startup worker it captures system DNS configuration and secure
query entropy, and duplicates the already retained workspace descriptor without
reopening its pathname. It reuses the terminal's exact selected helper executable
and validated environment. Stdio selects the captured-execution entrypoint for
its exec-descriptor handshake and persistent pipe input, not the PTY entrypoint
or its terminal-device handshake. Capture registers one shared inert macOS
inventory helper and selects bundled TLS roots without ambient certificate files
or proxy settings.
The runtime, controller and network share one monotonic clock; actual host and
configuration cancellation own peer lifetime, while each request remains bounded.
Capture starts no peer, process, browser, worker or separate executor. Missing
DNS or entropy authority remains unavailable for remote peers; it does not
prevent empty profiles or stdio startup. Remote startup reports that absence
under its normal required/optional server policy, without later ambient capture
or fallback. Invalid process/root or bundled trust selection fails with a
redacted MCP configuration error. Capture also selects native OAuth entropy and
wall-clock expiry authority; it does not generate a verifier or inspect tokens.
When composed with the selected MCP management profile, authentication uses that
profile's separate `mcp-credentials.json`, the exact captured network and the
existing host worker scope. Missing DNS still permits local status/removal, but
OAuth network attempts fail without a resolver fallback. Human presentation and
browser-launch endpoints remain separate explicit selections.

ACP session hosts instead use `capture_ephemeral_startup` or the inert
`with_ephemeral_startup(NativeReferenceHostMcpEphemeralStartupOptions)` selection.
Capture takes the admitted transport requirement: empty/stdio selections skip
network inputs, literal endpoints use literal-only resolution, and hostname
endpoints retain system-DNS capture.
That path reuses retained transport capture but never selects profile management,
stored authentication or an OAuth service; conflicting profile selections fail
before terminal acquisition in either builder order. The actual host composes a
dedicated ephemeral owner over its runtime, contexts, fixed names and workers.
Client omission or an empty array requires an explicit empty publication, not
profile fallback. `configure_conversation_mcp` attaches the exact weak ephemeral
readiness check at every prompt admission without profile-auth refresh.
`mcp_ephemeral_owner` returns the same allocation, and `mcp_deadline_after`
explicitly observes the selected clock. `close_mcp` and engine-resource drop
invalidate it before terminal shutdown, while `settle_mcp_ephemeral` drives
startup and peer cleanup before worker joining. See [ACP MCP](acp-mcp.md).

The concrete executor receives the same archive adapter allocation as terminal
input/result publishers and `read_tool_result`, including its existing quota
owner and native worker scope. It does not create another archive directory,
adapter or independent quota. The runtime supplies the exact catalog shared by
`mcp_search_tools` and `mcp_select_tool`. Native MCP permission preparation wraps
the actual builtin target authority and preparer, uses the same reviewer and
controller, and resolves non-builtin names only through the live exact-turn
runtime. Unknown, foreign or retired names cannot fall back to builtin approval.
The native `mcp_features` registration uses this same runtime and archive adapter.
All seven resource/prompt actions retain exact-turn authority, complete untrusted
JSON and durable paging receipts; unresolved input-required responses finish the
turn without replay. The registration holds only a weak runtime reference.
Generic extension constructors retain their injected feature authority and
portable bounds; no contextless fallback is added.

`mcp_runtime()` returns this exact runtime without activating servers.
`reserved_tool_names()` borrows names captured from the engine's successful
fixed registrations, without cloning schemas or polling an extension catalog.
Host assembly copies only the builder's already captured names, before building
the engine; it does not reacquire complete specifications for this inventory.
After terminal-resource acquisition, failed or unwound synchronous assembly
releases its actual owners and joins that new worker scope on the constructor's
caller worker. Successful assembly transfers that obligation to the returned
host. Drop invalidation alone is not treated as completed construction cleanup.
`with_controller_startup` additionally selects the native profile controller;
it requires the exact runtime clock allocation and an explicitly selected MCP
management service. Selection mismatch is rejected before terminal acquisition.
`mcp_controller()` returns the controller composed before engine construction
from that management service, runtime, fixed names and existing worker scope,
with four retained generation slots, reusing the selected configuration store
and existing worker scope.
`mcp_authentication()` shares the exact optional profile credential service without
reading or refreshing it. Each loaded startup/reload configuration selects stored
credentials for its remote servers, unless an explicit per-server authentication
selection overrides that default; profile saves do not activate credentials.
The actual engine resource lease closes both controller and runtime, including
after `into_engine`; retaining controller accessors cannot extend that lifetime.
Startup/reload candidate construction and atomic publication remain explicit
caller-polled controller operations. Options and host construction do not connect MCP peers or
perform authentication, discovery, browser launch or application requests.
The production CLI polls configured `AskStartup` before one-shot ask/resume
admission, and configured `All` before interactive session/picker admission.
These operations use the already owned Tokio runtime and signal receiver;
signals cancel startup while its native completion remains polled. No separate
startup executor, aggregate timeout or absolute session lifetime is introduced.
`close_mcp()` closes the selected controller before runtime invalidation. Its
separate `settle` operation must run while host workers are still owned, before
the host's final worker join. Runtime-only hosts retain explicit `drain_mcp`
behavior; a peer drain alone is not controller-job settlement.
Controller close cuts off its selected auth service as well. Settlement polls
auth-operation/worker observation and peer cleanup together, attempting both even
if one fails; an aggregate success requires both. Caller-owned auth network or
browser futures must first finish or be dropped by their command owner. Retaining
an auth accessor cannot extend the engine's lifetime or create another worker.
Both CLI paths explicitly settle that controller before dropping the host and
joining its workers, including startup failure and unwind. MCP cleanup uses a
fresh token and a separate bounded 30-second window. Cleanup failure cannot
skip the host join, interactive input restoration, or recording settlement.

The engine's host-resource lease invalidates MCP before terminal worker shutdown,
including when the host is consumed with `into_engine`; retained tools or
requesters do not extend that lease. For deliberate cleanup, call `close_mcp()`
and await bounded `drain_mcp(deadline, cancellation)` while the host still owns
its workers. Closing alone is not socket completion, child reaping or remote
session deletion. Drain observations report local owned completion, and
cancelled/unfinished cleanup remains retained for a later owned attempt. Drop
is last-resort invalidation, not a claim that this explicit drain completed.

For non-workspace file mutations, the history wrapper captures the trusted
backend's existing read-only approval ticket when the outer execution future
is constructed. It retains both success and denial outcomes; a later grant
cannot revive an older future. Construction opens no file, claims no approval,
reserves no history, and does not construct the inner execution future.
After polling successfully reserves the exact history observation, the backend
consumes that captured ticket and retains the ordinary preimage, revocation,
cancellation and final-effect checks. Dropped unpolled futures and failed
history admission leave the grant and filesystem untouched. This binding is
native-only and limited to the five concrete mutation backends; arbitrary
wrapped tools remain fully deferred until polling.

`with_workspace(NativeWorkspaceAuthority, Arc<NativeWorkspaceContexts>)` selects
captured workspace routing for reads, metadata, folder creation, enumeration,
grep, vision and the five file mutations. This option is independent of permission
mode. During
prepared-root composition, the primary identity and both retained directory
descriptors must match the selected authority. State selections may use different
path aliases; the actual state-directory identity, not its spelling, must match.
An authority prepared while state was absent must be prepared again with the
explicit existing state descriptor before constructing this host.

`NativeWorkspaceAuthority::fork_selection()` explicitly allocates an independent
mutable selection manager over the admitted immutable snapshot. It shares retained
descriptors and provenance without opening paths; later parent/child installs do
not affect one another. Prepared installs remain bound to their exact manager,
even when snapshots and generation numbers match. Ordinary `clone()` continues
to share one manager and must not be used to isolate a principal. Forking a
workspace copies neither permission grants nor undo history.

Call `host.configure_conversation_workspace(conversation)` before admitting each
created or resumed conversation. The native interactive owner performs this
attachment during initial startup and subsequent session transitions. The exact
registry is shared by the actual
registered tools and, when selected, native permission preparation. Missing or
expired registrations cannot fall back to primary-root execution. Permission
and execution use the same logical mutation projection and retain each endpoint
independently; the injected undo tracker keeps those endpoint identities.
History retains the same qualified logical paths and cross-root destinations.
For scoped mutations, observation reservation follows the outer execution-ticket
stamp and endpoint claim; history cannot let an old unpolled call acquire a
later approval. The original history adapter remains inert before polling.

`host.workspace_service(store)` retains the same workspace authority and the
complete-terminal host's actual worker scope, without loading settings or
starting an operation. It returns `None` when either selection is absent.
Interactive callers use the service's runtime-bound operations so queued or
active turns cannot be bypassed. Selected native terminal permission policy also
uses the same exact scope: `Os` captures all active retained roots, while `None`
and effective Yolo require a live scope without acquiring OS roots. Final launch
checks retain that scope; independently installed monitor grants retain their
authorized roots after the original turn ends. Scoped semantic search, terminal
cwd selection and top-level/interactive launch selection require their separate tool
composition; selecting the workspace option alone does not complete that surface.

`with_permissions(NativeReferenceHostPermissionOptions)` opts into native
permission preparation, selected-file preimage reads and mode/rule enforcement.
The options retain an explicitly supplied `NativePermissionContexts` registry
and reviewer clock. `with_sandbox_executable(File)` separately supplies the
retained system launcher; configuration cannot open it. Construction is inert.
This path requires complete terminal options for the shared native worker and
launch lifetime, and rejects missing selection before preparing namespaces.

Call `host.configure_conversation_permissions(conversation)` before admitting
each created or resumed conversation. It binds that exact incarnation to the
host's controller and context registry, using the configuration's initial policy;
the native runtime subsequently captures taken-job policy. Saved rules are
validated, not converted to restored grants. Direct engine sessions without the
required routes cannot execute permission-governed calls.

`host.model_routes()` and `host.observations()` return the exact optional shared
registries injected into search and file tools. The host retains these
allocations so later conversation composition cannot substitute a disconnected
registry. Access does not register a session, inspect files or publish history;
hosts without those explicit selections return `None`.
`host.workspace_root()` is the canonical workspace association captured during
composition. It does not reopen or revalidate that pathname after external
renames or replacements; the tools' actual authority remains descriptor-owned.

`configure_conversation_permissions_with_policy(conversation, policy)` uses the
same exact routes with a trusted host's explicit current selection instead of
reapplying configuration defaults. It rejects hosts without native permission
composition. It does not restore grants or bypass saved-rule validation.

The host registers exact tool allocations for canonical preparation, including
typed grep, question and complete-terminal validators. The five mutation tools
share one file-approval registry and retain final-effect proof checks; file-history
instrumentation and tool archives remain intact. Automatic review uses the same
explicit Gateway transport with its dedicated fixed model and clock. The terminal
policy and preparer bind weakly to the controller, avoiding ownership cycles.
Configured sandbox selection follows the live turn into native launch; missing
or unsupported Os authority fails without an unsandboxed fallback. Legacy hosts
retain their prior tools and `AskPermissionHandler`.

The prepared-root constructors
`compose_ai_gateway_http_with_prepared_roots_and_conversation` and
`compose_with_ai_gateway_transport_and_prepared_roots_and_conversation` append
these options to the corresponding prepared-root inputs. Before engine
construction they attach the **same** `Arc<FileUndoTracker>` to `write_file`,
`edit_file`, `delete_file`, `rename_file`, and `copy_file`, using each tool's
existing builder and retained workspace descriptor. They never reopen the
selected workspace or state path to create undo authority. Undo instrumentation
preserves forward permission handling; denied mutations and dropped unpolled tool
futures do not acquire undo preimages or register inverses. The opt-in permission
adapter's separately authorized approval evidence is distinct from undo history.

`with_terminal(NativeReferenceHostTerminalOptions)` also selects the complete
terminal path described below, including its blocking-worker requirements,
startup/archive directory preparation, limits, and owned cleanup. Without it,
conversation composition retains the legacy terminal and creates no terminal
startup/archive children. All existing constructors keep their prior behavior
and do not implicitly inject an undo tracker. MCP, subagent, provider, and
permission selections are unchanged unless permission options are also supplied.

`host.undo_tracker()` returns the exact injected allocation shared by all five
mutations, or `None` when no conversation tracker was supplied. The reference
host retains one additional share; access does not create authority or clear
history. The trusted host must clear or replace that tracker at the intended
conversation-lifetime boundary.
Dropping a reference host neither performs undo nor clears independently retained
tracker history; that history can retain descriptors and bounded snapshots after
the engine is gone. It is not persisted or reconstructed on session resume.
The tracker limits, unavailable-preimage markers, uncertainty barriers,
postimage checks, cancellation rules, and filesystem race caveats remain those
in the [file-undo contract](file-undo.md). Reference-host composition does not
strengthen those guarantees or turn the tracker into a transaction journal.

`compose_ai_gateway_http_with_prepared_roots_and_terminal` and
`compose_with_ai_gateway_transport_and_prepared_roots_and_terminal` append
`NativeReferenceHostTerminalOptions` to their existing prepared-root
counterparts. Call these synchronous constructors on a blocking worker. They
capture the selected state path before consuming the retained roots, verify
its canonical spelling against the descriptor, and use the existing bounded
private-directory preparation for fixed `terminal-startup` and
`tool-result-archive` children. They prepare the archive's private lock but
start no shell, terminal session, profile-owner worker, network request, or
hidden async runtime during composition. The terminal profile initializes
lazily on the first authorized native request. Existing unsafe children and
symlinks fail without repair; terminal/archive preparation errors use the
redacted terminal-configuration category.

One archive allocation and adapter are shared by terminal's complete input and
result publishers, `read_tool_result`, and the explicitly selected native MCP
executor. Large arguments are durably archived
before execution; later calls can page their original, session-incarnation-bound
contents. The complete terminal tool advertises all twelve actions; it does not
share the legacy background supervisor. Actual Engine, Session and lifecycle
handles retain the terminal host resource through `EngineBuilder::host_resource`.
Tools and pending operations retain only non-owning requesters. `into_engine`
preserves this ownership, and dropping the last real host handle requests
native shutdown even when a tool/turn future is retained.

`terminal_lifecycle_requester()` exposes the complete terminal host's explicit
session-transition authority to its trusted native owner. It shares the existing
owner thread for activation, access handoff and current-workspace reset, not a
new supervisor. The requester and unpolled operations do not retain the host's
resource lifetime; retain the actual reference host or engine while driving them.
Moving the host into its engine preserves that authority, but after the last real
host handle closes, a retained requester cannot restart it. Legacy composition
returns `None` without initializing its lazy background supervisor. This getter
does not select or switch a conversation; transition operations retain the
durability, ownership and uncertainty contract in [terminal](terminal.md).

`terminal_background_requester()` exposes non-owning listing, exact/latest
selection, inspection, bounded durable output and graceful close through the same
terminal owner. Each selected target retains its exact access generation;
handoff and final host shutdown revoke it. Unknown owners cannot create routes
by listing. Interactive background controls compose these requesters with an
optional explicitly captured URL launcher bound to the host's control workers,
not another supervisor or process-lifetime vote. Saved records and IDs never
restore live authority; details belong in [background commands](background-cli.md).

The explicit-path constructors require the trusted host to choose disjoint
workspace and session roots; those constructors do not prove identity or
ancestor disjointness. The prepared-root path performs the stronger
root-selection checks defined in [native-root-selection.md](native-root-selection.md).
Because the composed background writer shares the retained state-root identity,
successful composition read-only checks that root against the background
store's owner-private mode and supported macOS ACL contract. That check creates
no namespace and starts no worker. The workspace must also have a canonical
Unicode absolute path; composition binds that spelling to the retained
descriptor before exposing terminal `start`.

Production composition opens the workspace and session roots before credential
discovery. A failure never causes a fallback to a different provider,
transport, permission mode, credential source, or model.

`compose_ai_gateway_http_with_prepared_roots_and_conversation_and_credential`
accepts the same conversation-constructor inputs, replacing the environment
snapshot argument with an owned `DiscoveredAiGatewayCredential`. This explicit
startup seam consumes a previously acquired token into the ordinary inference
transport and retains its OIDC/API-key source. A host may first borrow that
credential through
`AiGatewayModelCatalogHttpTransport::with_discovered_credential` to obtain
authenticated model capabilities before constructing terminal authority.
Credential acquisition and any catalog request belong to that trusted startup
owner; composition itself neither reacquires credentials nor requests a catalog.
The ordinary inference discovery API still rejects missing credentials, so
callers can perform that check before granting catalog network authority.
All prepared-root paths share the same internal construction stages; existing
environment-taking constructors still consume roots and prepare memory before
discovering credentials. The acquired path preserves loaded configuration,
undo/model-routing allocations, optional terminal behavior, and fixed errors.

## Composition graph

Every successful host contains the shared components below. Terminal-specific
bullets describe constructors without explicit terminal options; the complete
terminal selection described above replaces those legacy components.

- `AiGatewayProvider` over one shared `Arc<dyn AiGatewayTransport>`;
- `AskPermissionHandler` over an injected `Arc<dyn PermissionPrompter>`, or the
  explicitly selected native controller/preparer/reviewer composition;
- `AskUserQuestionTool` over an injected `Arc<dyn QuestionPrompter>`;
- one concrete `Arc<FileSessionStore>` shared exactly, through the same erased
  `Arc<dyn SessionStore>`, with the engine, `ReadToolResultTool`, and
  `NativeSessionLifecycle`;
- one `MemoryTool` over an identity-preserving clone of that store's retained
  state-root descriptor, without access to session-record APIs;
- one `McpSearchToolsTool` and one `McpSelectTool` over the exact same inert,
  explicitly injected catalog allocation; ordinary constructors share an empty
  ready catalog, while the MCP-aware constructor accepts host-admitted metadata
  and attached executable registrations;
- one `McpFeaturesTool` over an inert, explicitly injected read-only feature
  authority; ordinary and catalog-only constructors share an empty authority,
  while the complete MCP seam retains the caller's exact allocation;
- one provider-neutral `SubagentTool` over an explicitly injected authority;
  ordinary constructors share an inert unavailable authority, while the
  subagent-aware seam retains the caller's exact allocation;
- one bounded lazy background starter over identity-preserving workspace and
  state-root descriptors, retained by `terminal` for noninteractive `start`;
  its environment is fixed and independently identified for process
  permission, while one supervisor is initialized and reused only after the
  first permitted start is polled;
- one process-local output reader sharing that exact lazy starter allocation and
  therefore its exact capture registry. Reading does not initialize the
  supervisor, and exact session plus session-incarnation ownership prevents a
  display ID from granting output authority;
- one process-local signal controller sharing the lazy starter's exact control
  registry and blocking-worker allocation. It binds delivery to the current
  session incarnation, never treats the displayed PID as authority, and does
  not initialize the supervisor merely to reject an unknown target;
- one input writer sharing the lazy starter's exact input registry and existing
  blocking pool. Writes require opt-in piped stdin and the current session
  incarnation. Unknown writes do not initialize the supervisor. Nonblocking
  writes preserve accepted-byte receipts, and explicit EOF closes only stdin;
- one background-history reader over a separate clone of the same retained
  state-root descriptor and frozen canonical workspace identity. It creates no
  namespace or worker during composition, listing, or inspection, never
  initializes the lazy supervisor, and exposes only terminal's compact ordered
  list and exact-record projections. A separately injected delay adapter adds
  bounded record-only `wait` without process authority;
- provider-neutral limits selected as described above and the default no-op event sink;
- one explicit `Arc<dyn WebSearchDeadline>` for bounded web-search timing,
  reused through fixed category-only adapters for terminal's persisted-record
  wait and vision's capacity wait, cooperative filesystem checkpoints, and
  Gateway operation, subject to each tool's documented synchronous-system-call
  caveat; and
- the fixed tool catalog below.

The production AI Gateway target is `https://ai-gateway.vercel.sh` with the
default HTTPS port. A custom transport must receive the canonical target it
actually contacts; that value becomes both web-search permission identity and
the remote half of each composite vision capability. The dedicated vision
worker shares the same `Arc<dyn AiGatewayTransport>` as the ordinary
provider and web search, but not their model selection. The custom path is a trusted authority override and
reports no discovered credential source.

## Tool catalog

The engine registers exactly twenty-six tools in deterministic alphabetical
order:

1. `ask_user_question`
2. `copy_file`
3. `create_folder`
4. `delete_file`
5. `edit_file`
6. `file_info`
7. `glob_files`
8. `grep_files`
9. `install_skill`
10. `list_files`
11. `mcp_features`
12. `mcp_search_tools`
13. `mcp_select_tool`
14. `memory`
15. `open_file`
16. `read_file`
17. `read_tool_result`
18. `rename_file`
19. `semantic_search`
20. `skill`
21. `subagent`
22. `terminal`
23. `vision`
24. `web_fetch`
25. `web_search`
26. `write_file`

Seventeen tools use one retained workspace identity. `glob_files` consumes the
original descriptor. The other sixteen workspace tools receive
identity-preserving clones: `copy_file`, `create_folder`, `delete_file`,
`edit_file`, `file_info`, `grep_files`, `install_skill`, `list_files`,
`open_file`, `read_file`, `rename_file`, `semantic_search`, `skill`, `terminal`,
`vision`, and `write_file`. On legacy paths, the terminal lazy background starter receives one
additional clone of that same workspace identity; it is not another catalog
tool. Terminal listing and inspection receive a separate retained state-root
clone, not ambient cwd or environment discovery. Terminal wait reuses that
exact reader and the shared deadline authority; none initializes or calls the
starter. Terminal output read reuses the starter allocation itself without
initializing it and has a separate four-read limit. At most four terminal lists
may be active. Terminal signal likewise reuses the starter allocation, runs
bounded Linux process-tree traversal or one macOS original-group delivery on
its existing blocking pool, and has a separate four-signal limit. These limits
are independent of terminal's
foreground-execution, read, and wait admission limits.
`ask_user_question`, `mcp_features`, `mcp_search_tools`, `mcp_select_tool`,
`subagent`, and `web_fetch` are rootless.
`mcp_features` uses its explicitly selected native runtime or injected read-only
authority. It stamps
all returned resource and prompt data as untrusted and grants no permission or
execution authority; its complete boundary is defined by the
[MCP features contract](mcp-features.md).
`mcp_search_tools` acquires only one bounded point-in-time metadata snapshot
when executed, never during composition; its injected boundary and intentional
protocol/discovery deferrals are defined by the
[MCP search contract](mcp-search-tools.md). `mcp_select_tool` shares that exact
catalog allocation and exact-selects one attached executable registration for
advertisement on the next model round. The overlay remains turn-local as
defined by the [MCP selection contract](mcp-select-tool.md). `read_tool_result` uses the
engine's exact session-store allocation and has no workspace authority.
`subagent` accepts managed commands through its injected authority, which must
validate the actual admitted turn and own durable child lifetime. Public context
IDs and structural execution grant no authority. Children receive no inherited
parent transcript or grants; the complete boundary is defined by the
[subagent contract](subagent.md).
`memory` uses a clone of the retained state-root identity but has no workspace
or session-record authority; its fixed files and permission boundary are
defined by the [memory contract](memory.md).
`skill` reads only an explicitly selected workspace-local UTF-8 resource after
an exact filesystem-read decision. It treats the bytes as opaque model-visible
content and neither parses nor executes them; its bounds and confinement are
defined by the [skill contract](skill.md).
`install_skill` copies one bounded local source tree into one absent managed
destination after an indivisible custom-capability decision; its confinement
and atomic publication boundary are defined by the
[install contract](install-skill.md).
`web_search` is backed by the configured AI Gateway network target and shared
transport rather than a workspace descriptor. `vision` combines its retained
workspace identity with that target in one disclosure capability and uses the
shared transport only after approval and descriptor-relative image
verification. On Linux and macOS, `semantic_search` uses only its retained
workspace authority; it does not use the provider, transport, or an embedding
index. Both scanners retain bounded descriptor-relative traversal. The macOS
reader exposes each directory refill so that the same scanner budget can charge
it explicitly; see [ADR 0005](decisions/0005-macos-directory-reader.md).

Catalog membership does not imply that every platform can complete every
effect. Each tool still performs strict preparation, permission handling when
required, direct argument revalidation, and its documented platform check.

## Construction effects

Construction is synchronous. It creates no Tokio runtime, sends no model or
network request, polls no permission/question prompt, and touches no session
record. Depending on the constructor and enabled tools, it does perform the
documented bounded setup needed to own later authority:

- open and retain existing root descriptors, or consume prepared descriptors;
- clone the retained state-root descriptor once for `memory`, without reading
  or creating memory state;
- discover and validate the two supported credential environment sources on
  the ordinary production path, or consume the explicitly acquired credential;
- snapshot the bounded process environment used by `terminal`;
- retain the exact workspace and state-root descriptors plus the fixed
  background environment identity after a read-only owner/mode/ACL suitability
  check; background namespace reconciliation and atomically admitted
  fixed-capacity worker creation are deferred to one shared, single-shot
  initialization on the first permitted `start` poll;
- construct provider, web-search, terminal-wait, and private vision adapters
  over the shared transport and deadline authority;
- on the explicit conversation path, retain five shares of the caller's exact
  undo tracker without snapshots, inverse work, or implicit history reset;
- construct `web_fetch`, including its bounded native resolver/entropy setup.

Composition does not poll or snapshot the MCP catalog and does not call the MCP
feature or subagent authority.

Later provider, web-fetch, web-search, and vision polling requires a compatible
host-owned Tokio runtime with the capabilities stated by their contracts. The
host does not create or hide that runtime.

## Retained observations

The host exposes read-only accessors for:

- the provider-neutral `Engine`;
- the exact concrete `Arc<FileSessionStore>`;
- `NativeSessionLifecycle` over that same engine/store pair;
- the retained `LoadedNativeConfig`; and
- the selected production credential-source enum, including when acquisition
  preceded construction through the explicit acquired-credential seam.

It exposes no bearer-token getter, raw environment value, root path, descriptor,
transport internals, prompt state, or provider response. `into_engine` consumes
the host and returns the configured engine.

## Failure boundary

Construction failures are fixed, redacted stage categories:

- unsupported selection;
- workspace root;
- session store;
- memory construction;
- credential;
- HTTP transport;
- web-fetch transport;
- web-search transport;
- vision configuration (`VisionConfig`);
- vision transport (`VisionTransport`);
- terminal configuration;
- background-supervisor configuration;
- provider;
- permission configuration (`PermissionConfig`);
- MCP configuration (`McpConfig`); or
- engine.

Display and debug output include only the stable stage, never a token, path,
environment value, endpoint diagnostic, operating-system error, or injected
component detail.

## Interactive clipboard ownership

`NativeInteractiveSessionOptions::with_clipboard` retains explicit executable
and environment inputs without effects. Opening the interactive owner binds
the optional clipboard capability to the verified workspace and the exact
reference host's control-worker scope. Missing or invalid clipboard authority
does not prevent interactive startup; an eligible copy reports its fixed
availability error. Empty history succeeds without invoking the backend.

`request_copy` captures one immutable canonical snapshot and its exact source
principal at acceptance. Incremental selection and the clipboard response have
one independent operation slot and one retained outcome slot. An unread copy
receipt blocks another copy, not model admission, active inference, durable
controls, or session transitions. Each owner poll performs at most one selection
step or response poll before continuing its other work. Copied text never
becomes a prompt, transcript mutation, or terminal output payload.

Transitions and shutdown cancel the copy's private token immediately. They
discard unfinished selection and retain a cancellation receipt; a started
backend response remains polled independently without delaying the transition.
Every receipt retains its original source even after a different session is
active. Ordinary turn cancellation does not cancel clipboard work. Errors do
not imply that an external clipboard was unchanged.

`has_pending_copy` remains true while an accepted copy needs polling, including
after the session reports closed or a shutdown failure. The presentation host
must continue polling until it is false and transfer the retained
`take_copy_outcome` result before switching to native-free final output. A copy
response is not a child-reap or worker-join receipt: the full reference-host
completion still covers actual worker and deferred child cleanup, independently
of blocked presentation output. Dropping a started response requests backend
private cancellation rather than cancelling any caller's token.

## Deferred composition

The reference host does not itself supply a full interactive CLI/TUI,
alternate provider or credential selections, remote
or packaged skill discovery and installation, automatic MCP transport
activation, authentication, protocol-driven catalog discovery, caching, subscriptions,
ACP or persistent/background subagent management, encrypted storage, non-Unix
root hardening, durable image
attachments, prompt images, or CLI image flags. Those additions must preserve
the crate ownership and authority boundaries in
[architecture.md](architecture.md) and [security.md](security.md).
