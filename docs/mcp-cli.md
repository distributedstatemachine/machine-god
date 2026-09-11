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
