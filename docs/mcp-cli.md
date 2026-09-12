# Native MCP command boundary

`machine_god_native::mcp::commands::McpCommand` parses human-invoked command
intent without opening files, starting servers, contacting a network, granting
permissions or opening a browser. Parsing is separate from the native runtime
and thin CLI integration. The implementation plan owns complete-feature delivery
status; a parser result alone does not establish production MCP availability.

## Command grammar

Input excludes the `/mcp` prefix. Verbs are case-sensitive. ASCII spaces and
tabs separate tokens; leading/trailing spaces and tabs are removed. Literal
control characters other than tabs are rejected before argument allocation.
Escaped control characters inside a JSON string remain argument data.

| Input | Native intent |
| --- | --- |
| empty | Summary |
| `list`, `path`, `reload` | Catalog listing, profile path, explicit reload |
| `add <server> <command> [args...]` | Add/replace a stdio configuration through a separate store authority |
| `remove <server>` | Remove an exact configured server |
| `auth <server> [--open]` | Authentication request with a distinct explicit browser-confirmation flag |
| `logout <server>` | Remove credentials through the separate authentication owner |
| `resource list <server>` | List resources |
| `resource templates <server>` | List resource templates |
| `resource read <server> <uri>` | Read the exact remaining URI string |
| `resource complete <server> <template> <argument> [value]` | Complete the exact template argument |
| `prompt list <server>` | List prompts |
| `prompt get <server> <prompt> [arguments-json]` | Get a prompt with an optional string-valued argument object |
| `prompt complete <server> <prompt> <argument> [value]` | Complete the exact prompt argument |

The grammar follows `src/builtins/mcp.zig` at the pinned upstream revision.
There is no shell interpolation, escape evaluation or quoted-argument grouping
for `add`; quotes and shell-looking text remain literal token bytes. Resource
reads and completion prefixes preserve interior spaces/tabs. Completion defaults
to an empty value; omitted prompt arguments default to an empty object.
Unknown verbs, missing required tokens and extra tokens on fixed-arity commands
are errors. Browser confirmation only records human intent; it is not itself
browser or network authority.

## Bounds and validation

The entire raw command is bounded to 128 KiB before scanning or copying.
Server aliases contain 1–128 ASCII letters, digits, `_` or `-`, matching the
native catalog and configured-server identity boundary. Executable tokens and
individual stdio arguments are at most 4 KiB; at most 256 arguments follow the
executable. Resource URIs/templates are at most 64 KiB. Prompt and argument
identities are at most 256 bytes; completion values are at most 4 KiB. Bounds
count UTF-8 bytes and are inclusive.

Prompt argument JSON is bounded to 64 KiB before decoding and must be one
complete object with at most 128 unique, nonempty keys of at most 256 bytes and
string values. Duplicate keys, non-string values and trailing JSON are rejected.
The parser requests strings directly rather than recursively constructing
arbitrary JSON values; deeply nested non-string values cannot cause recursive
owned-value destruction. Duplicate rejection and explicit finite limits are
intentional strictness beyond permissive upstream parsing.

Fixed parse errors distinguish invalid syntax from raw/token byte or argument
count limits. Invalid prompt-object structure and its internal entry/key limits
use the invalid-syntax category. Error and debug forms omit names, arguments,
paths, resource content and credentials.

Public command enums represent untrusted intent, not validated authority.
Native adapters must validate directly constructed variants, live configuration,
exact server/resource/prompt admission, aggregate canonical request limits and
permission policy before any effect. Feature result bounds and trust handling
remain governed by [MCP features](mcp-features.md); tool selection grants no
execution permission under [MCP selection](mcp-select-tool.md).

`McpFeatureRequest::try_from(McpFeatureCommand)` projects all seven human feature
forms through the existing feature request decoder and 64 KiB canonical request
budget. Direct enum construction is validated before copying fields into an
owned JSON map. Prompt arguments retain escaped control characters as data;
their count, key lengths and compact JSON bytes are checked separately. Empty
completion values and empty argument objects use the same canonical defaults as
the model-facing tool. This conversion creates a request, not a permission grant
or network call; live native feature authority and untrusted-result validation
remain required.

## Human input presentation

Authentication control receipts separately report confirmed credential persistence,
usability at the native observation, and the result of any full configured reload;
they do not claim a targeted reconnect or a current connection. Without `--open`,
the confirmation receipt prints the explicit `/mcp auth NAME --open` command.
Logout reports local removal independently from confirmed, unsupported,
unattempted or ambiguous remote revocation. Issuer mismatch remains rejection,
with a fixed instruction to edit the selected server's `oauth.issuer` and retry;
there is no issuer-override prompt. Output is bounded and terminal-escaped, never
includes an authorization URL or credential, and never automatically retries an
ambiguous result. Management-only hosts without native runtime/auth authority
still reject auth/logout as unavailable.

The interactive modal presents an explicitly queued native MCP elicitation,
including its selected server and exposed tool identity. Server messages, URLs,
field schemas, choices and answer previews are terminal-escaped. Source text is
paged in UTF-8-safe 8 KiB pieces within the existing 64 KiB output bound; long
accepted messages and schemas are not silently truncated. Each page and field
has its own acknowledged input epoch. Buffered answers for earlier pages cannot
answer a later field or approve the final submission.

Forms support string, number, integer, boolean, single-select and multi-select
fields. Native schema validation precedes advancing a field. Numeric input keeps
its original JSON lexeme; strings preserve spaces, `text <literal>` escapes UI
commands, and `json <quoted string>` permits escaped newlines or other characters.
Choices use displayed indices, with comma-separated indices for multiple choices.
`/default` explicitly selects a declared default; `/skip` omits only optional
fields. The complete answers are shown for a separate final `y` confirmation,
under an independent 128 KiB response bound. The native inbox independently
validates the entire response against the exact queued request.

`/decline` and `/cancel-input` return distinct protocol actions; `/cancel` retains
its existing whole-turn cancellation meaning. URL approval is explicit consent
only: this presentation code does not launch a browser, infer authentication
completion, or resubmit a tool call. Browser execution, exact continuation
authority remain native responsibilities. Legacy completion is not supported.
The shared input queue owns stale-token, cancellation and principal checks.
This presentation adapter does not itself activate a configured MCP server.

## Profile configuration codec

`machine_god_native::mcp::config` validates and owns an explicit profile
configuration independently of command parsing, filesystem storage and runtime
admission. `McpConfig` accepts the pinned top-level `mcp` object and retains
server order. `McpServerConfig` exposes immutable transport-specific getters;
its constructors and decoder grant no process, network or credential authority.

The codec supports stdio/local string or vector commands, ordered arguments,
`environment` over `env` precedence, and vector-command over separate-arguments
precedence. HTTP configurations retain exact URL strings, headers,
header-environment bindings and bearer-token environment names. OAuth retains
resource, issuer, client ID, client-secret environment name, client metadata URL
and scopes. Required/enabled flags and nonzero `u32` startup/operation timeouts
are preserved; defaults are 10,000 and 60,000 milliseconds. Stdio restart count
uses the full `u8` domain and defaults to one.

Decoding rejects duplicate JSON keys, including escaped duplicates, while
constructing its bounded intermediate data. Limits are 1 MiB input and canonical
output, 64 servers, 128-byte ASCII server aliases, JSON depth 8, 16,384 value
nodes, and 512 KiB of aggregate decoded keys and strings. Individual strings are
at most 16 KiB; executable and URL fields are at most 4 KiB. Collections permit
at most 256 arguments or environment bindings, 128 combined headers, and 64
OAuth scopes. Retained owned strings and collections are compacted.

Canonical encoding preserves server order and deterministically encodes object
fields. Accepted output is bounded and re-admitted against the codec's own
constraints. Insert rejects an existing alias; replace retains its original
position; new entries append. An invalid or aggregate-overflowing mutation
leaves the previous configuration unchanged. Removal affects only the exact
case-sensitive server identity.

Unknown or inactive transport fields, invalid environment names, duplicate
case-insensitive header identities, reserved/authentication headers and active
control strings are intentionally rejected rather than silently ignored.
Debug and error forms omit server identities, commands, header values and
credentials. No secret environment variable is resolved by decoding.

URL validation here is bounded structural validation, not complete URI or
network admission. The effect-bearing HTTP authority must use its established
URI parser and validate origin, DNS, endpoint and OAuth policy before effects.
The codec does not discover or mutate fx roots, change native settings schema,
create a profile directory, save credentials or connect to any server.

The [profile store](mcp-persistence.md) separately owns private `mcp.json`
observation and publication. Its save receipt is not a runtime reload result.
Profile configuration rejects explicit Authorization headers at the upstream
pin too. This is distinct from resolved transport headers, where generated
bearer/OAuth Authorization must be admitted. Runtime resolution must prefer
active OAuth credentials over the configured bearer environment variable;
OAuth configuration alone is not an active credential. Missing required header
environment values and duplicate resolved names are errors, not overwrite or
fallback instructions.
