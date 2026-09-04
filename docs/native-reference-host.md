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
system implementations. In particular, the complete host compiles on macOS,
but the production `semantic_search` and terminal foreground executor are
Linux-only. The private host catalog retains both tools on macOS:
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
provider, the web-search transport adapter, and the private vision worker. The
host retains the complete `LoadedNativeConfig`, including its observable
schema origin/version.

## Constructors

The root, transport, and MCP composition paths are:

| Roots | Transport | Constructor behavior |
| --- | --- | --- |
| Existing explicit workspace and session paths | Production AI Gateway HTTP | Opens and retains both roots, discovers the configured credential, and constructs the production transport |
| `PreparedNativeRoots` | Production AI Gateway HTTP | Consumes the already retained identity-checked roots without reopening their selected paths, then discovers credentials and constructs the transport |
| Existing explicit workspace and session paths | Injected `Arc<dyn AiGatewayTransport>` | Opens and retains both roots, skips credential discovery, and uses the supplied canonical `NetworkTarget` |
| `PreparedNativeRoots` | Injected transport | Consumes retained roots, skips credential discovery, and uses the supplied canonical target |
| Existing explicit roots | Injected transport and `Arc<dyn McpToolCatalog>` | Uses the custom transport path and advertises search plus exact next-round selection over the injected admitted MCP metadata and attached executable source; feature access remains an inert empty authority |
| Existing explicit roots | Injected transport, `Arc<dyn McpToolCatalog>`, and `Arc<dyn McpFeatureAuthority>` | Adds bounded exact server-qualified resource, prompt, and completion access through the separately injected read-only authority |
| Existing explicit roots | Injected transport and `Arc<dyn SubagentAuthority>` | Adds bounded foreground one-off delegation while MCP authorities remain inert |
| Existing explicit roots | Injected transport plus MCP catalog, MCP feature, and subagent authorities | Retains all three exact extensibility allocations without probing or polling them |

Every ordinary path injects an inert unavailable `SubagentAuthority`. A
separate explicit subagent injection seam accepts the same root/transport
composition plus one trusted authority allocation. Neither path probes or polls
the authority during construction.

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

Every successful host contains:

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
- one background-history reader over a separate clone of the same retained
  state-root descriptor and frozen canonical workspace identity. It creates no
  namespace or worker during composition, listing, or inspection, never
  initializes the lazy supervisor, and exposes only terminal's compact ordered
  list and exact-record projections. A separately injected delay adapter adds
  bounded record-only `wait` without process authority;
- default provider-neutral `EngineLimits` and the default no-op event sink;
- one explicit `Arc<dyn WebSearchDeadline>` for bounded web-search timing,
  reused through fixed category-only adapters for terminal's persisted-record
  wait and vision's capacity wait, cooperative filesystem checkpoints, and
  Gateway operation, subject to each tool's documented synchronous-system-call
  caveat; and
- the fixed tool catalog below.

The production AI Gateway target is `https://ai-gateway.vercel.sh` with the
default HTTPS port. A custom transport must receive the canonical target it
actually contacts; that value becomes both web-search permission identity and
the remote half of each composite vision capability. The vision worker reuses
the same configured model and `Arc<dyn AiGatewayTransport>` as the ordinary
provider and web search. The custom path is a trusted authority override and
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
`vision`, and `write_file`. The terminal lazy background starter receives one
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
