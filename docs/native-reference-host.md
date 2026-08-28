# Native reference-host composition

`NativeReferenceHost` is the maintained example of composing the
provider-neutral engine with native AI Gateway, persistence, prompt, root, and
tool implementations. It is a library boundary; the current CLI does not yet
turn this complete composition into an interactive agent UI. Current milestone
state is maintained only in the
[implementation plan](implementation-plan.md#current-delivery-state).

## Availability

The complete reference host is compiled when all of these are true:

- the target is Linux or macOS;
- the target is not WebAssembly; and
- `machine-god-native` enables `ai-gateway-http`.

Individual native contracts may expose portable injected seams or narrower
system implementations. In particular, the complete host compiles on macOS,
but the production terminal system executor is Linux-only. The private host
catalog retains `terminal` on macOS and returns its fixed unsupported result
after strict preparation and permission, before cwd lookup or process creation.

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

There are four composition paths:

| Roots | Transport | Constructor behavior |
| --- | --- | --- |
| Existing explicit workspace and session paths | Production AI Gateway HTTP | Opens and retains both roots, discovers the configured credential, and constructs the production transport |
| `PreparedNativeRoots` | Production AI Gateway HTTP | Consumes the already retained identity-checked roots without reopening their selected paths, then discovers credentials and constructs the transport |
| Existing explicit workspace and session paths | Injected `Arc<dyn AiGatewayTransport>` | Opens and retains both roots, skips credential discovery, and uses the supplied canonical `NetworkTarget` |
| `PreparedNativeRoots` | Injected transport | Consumes retained roots, skips credential discovery, and uses the supplied canonical target |

The explicit-path constructors require the trusted host to choose disjoint
workspace and session roots; those constructors do not prove identity or
ancestor disjointness. The prepared-root path performs the stronger
root-selection checks defined in [native-root-selection.md](native-root-selection.md).

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
- default provider-neutral `EngineLimits` and the default no-op event sink;
- one explicit `Arc<dyn WebSearchDeadline>` for bounded web-search timing,
  reused through a fixed category-only adapter for vision's capacity wait,
  cooperative filesystem checkpoints, and Gateway operation, subject to the
  synchronous-system-call caveat in the [vision contract](vision.md); and
- the fixed tool catalog below.

The production AI Gateway target is `https://ai-gateway.vercel.sh` with the
default HTTPS port. A custom transport must receive the canonical target it
actually contacts; that value becomes both web-search permission identity and
the remote half of each composite vision capability. The vision worker reuses
the same configured model and `Arc<dyn AiGatewayTransport>` as the ordinary
provider and web search. The custom path is a trusted authority override and
reports no discovered credential source.

## Tool catalog

The engine registers exactly eighteen tools in deterministic alphabetical
order:

1. `ask_user_question`
2. `copy_file`
3. `create_folder`
4. `delete_file`
5. `edit_file`
6. `file_info`
7. `glob_files`
8. `grep_files`
9. `list_files`
10. `open_file`
11. `read_file`
12. `read_tool_result`
13. `rename_file`
14. `terminal`
15. `vision`
16. `web_fetch`
17. `web_search`
18. `write_file`

Fourteen tools use one retained workspace identity. `glob_files` consumes the
original descriptor; the other thirteen workspace tools, including `terminal`
and `vision`, receive identity-preserving clones. `ask_user_question` and
`web_fetch` are rootless. `read_tool_result` uses the engine's exact
session-store allocation and has no workspace authority. `web_search` is backed
by the configured AI Gateway network target and shared transport rather than a
workspace descriptor. `vision` combines its retained workspace identity with
that target in one disclosure capability and uses the shared transport only
after approval and descriptor-relative image verification.

Catalog membership does not imply that every platform can complete every
effect. Each tool still performs strict preparation, permission handling when
required, direct argument revalidation, and its documented platform check.

## Construction effects

Construction is synchronous. It creates no Tokio runtime, sends no model or
network request, polls no permission/question prompt, touches no session
record, and starts no persistent background work. Depending on the constructor
and enabled tools, it does perform the documented bounded setup needed to own
later authority:

- open and retain existing root descriptors, or consume prepared descriptors;
- discover and validate the two supported credential environment sources on
  the production path;
- snapshot the bounded process environment used by `terminal`;
- construct provider, web-search, and private vision adapters over the shared
  transport;
- construct `web_fetch`, including its bounded native resolver/entropy setup.

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
- credential;
- HTTP transport;
- web-fetch transport;
- web-search transport;
- vision configuration (`VisionConfig`);
- vision transport (`VisionTransport`);
- terminal configuration;
- provider; or
- engine.

Display and debug output include only the stable stage, never a token, path,
environment value, endpoint diagnostic, operating-system error, or injected
component detail.

## Deferred composition

The reference host does not yet supply a full interactive CLI/TUI, persistent
grant policy, alternate provider or credential selections, MCP/ACP/skill or
subagent infrastructure, encrypted storage, non-Unix root hardening, durable
image attachments, prompt images, or CLI image flags. Those additions must
preserve the crate ownership and authority boundaries in
[architecture.md](architecture.md) and [security.md](security.md).
