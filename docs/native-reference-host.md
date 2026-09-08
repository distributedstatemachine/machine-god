# Native reference-host composition

`NativeReferenceHost` is the maintained example of composing the
provider-neutral engine with native AI Gateway, persistence, prompt, root, and
tool implementations. It is a library boundary; the CLI uses it for bounded
one-shot requests but does not yet provide a full interactive agent UI. Current
milestone state is maintained only in the
[implementation plan](implementation-plan.md#current-delivery-state).

## Availability

The complete reference host is compiled when all of these are true:

- the target is Linux or macOS;
- the target is not WebAssembly; and
- `machine-god-native` enables `ai-gateway-http`.

Individual native contracts may expose portable injected seams or narrower
system implementations. The explicit terminal-options constructors support
foreground and interactive terminal execution on Linux and macOS. Constructors
without those options retain the legacy adapter: its foreground executor and
the production `semantic_search` remain Linux-only. That legacy catalog retains
both tools on macOS:
`semantic_search` and terminal `exec` return their fixed unsupported results
after strict preparation and permission, while terminal `start` uses the
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

Composition rejects a loaded configuration unless it selects exactly:

- permission mode `ask`;
- provider `vercel_ai_gateway`;
- transport `ai_gateway_http`; and
- credential source `environment`.

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
| Existing explicit roots | Injected transport and `Arc<dyn SubagentAuthority>` | Adds bounded foreground one-off delegation while MCP authorities remain inert |
| Existing explicit roots | Injected transport plus MCP catalog, MCP feature, and subagent authorities | Retains all three exact extensibility allocations without probing or polling them |

Every ordinary path injects an inert unavailable `SubagentAuthority`. A
separate explicit subagent injection seam accepts the same root/transport
composition plus one trusted authority allocation. Neither path probes or polls
the authority during construction.

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

The prepared-root constructors
`compose_ai_gateway_http_with_prepared_roots_and_conversation` and
`compose_with_ai_gateway_transport_and_prepared_roots_and_conversation` append
these options to the corresponding prepared-root inputs. Before engine
construction they attach the **same** `Arc<FileUndoTracker>` to `write_file`,
`edit_file`, `delete_file`, `rename_file`, and `copy_file`, using each tool's
existing builder and retained workspace descriptor. They never reopen the
selected workspace or state path to create undo authority. Preparation and
ordinary forward permission handling remain unchanged; denied mutations and
dropped unpolled tool futures do not acquire preimages or register inverses.

`with_terminal(NativeReferenceHostTerminalOptions)` also selects the complete
terminal path described below, including its blocking-worker requirements,
startup/archive directory preparation, limits, and owned cleanup. Without it,
conversation composition retains the legacy terminal and creates no terminal
startup/archive children. All existing constructors keep their prior behavior
and do not implicitly inject an undo tracker. MCP, subagent, provider, and
permission selections are unchanged.

The trusted host retains its own tracker handle for explicit inverse operations
and must clear or replace it at the intended conversation-lifetime boundary.
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
result publishers and `read_tool_result`. Large arguments are durably archived
before execution; later calls can page their original, session-incarnation-bound
contents. The complete terminal tool advertises all twelve actions; it does not
share the legacy background supervisor. Actual Engine, Session and lifecycle
handles retain the terminal host resource through `EngineBuilder::host_resource`.
Tools and pending operations retain only non-owning requesters. `into_engine`
preserves this ownership, and dropping the last real host handle requests
native shutdown even when a tool/turn future is retained.

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

## Composition graph

Every successful host contains the shared components below. Terminal-specific
bullets describe constructors without explicit terminal options; the complete
terminal selection described above replaces those legacy components.

- `AiGatewayProvider` over one shared `Arc<dyn AiGatewayTransport>`;
- `AskPermissionHandler` over an injected `Arc<dyn PermissionPrompter>`;
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
`mcp_features` uses only its explicitly injected read-only authority. It stamps
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
`subagent` performs only bounded foreground one-off delegation through its
injected authority. It receives no parent transcript, grants, dynamic tools, or
recursive subagent visibility; its complete boundary is defined by the
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
verification. On Linux, `semantic_search` uses only its retained workspace
identity; it does not use the provider, transport, or an embedding index. Its
macOS placeholder retains the clone only for catalog stability and never
inspects it.

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
  the production path;
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
- the selected production credential-source enum, if discovery ran.

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
- provider; or
- engine.

Display and debug output include only the stable stage, never a token, path,
environment value, endpoint diagnostic, operating-system error, or injected
component detail.

## Deferred composition

The reference host does not itself supply a full interactive CLI/TUI,
persistent grant policy, alternate provider or credential selections, remote
or packaged skill discovery and installation, production MCP transport,
authentication, protocol-driven catalog discovery, caching, subscriptions,
ACP or persistent/background subagent management, encrypted storage, non-Unix
root hardening, durable image
attachments, prompt images, or CLI image flags. Those additions must preserve
the crate ownership and authority boundaries in
[architecture.md](architecture.md) and [security.md](security.md).
