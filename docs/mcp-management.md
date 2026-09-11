# Native MCP profile management

`mcp::management::NativeMcpManagementService` retains an explicitly selected
`Arc<NativeMcpConfigStore>`. Its constructor is inert: no profile loading,
directory creation, environment discovery or server startup occurs. The native
host must run synchronous `execute` calls within its owned worker lifecycle.
The service starts no workers and supplies no detached-task framework.

## Operations and authority

`Summary` and `List` load one bounded profile observation and return ordered
configured-server metadata: alias, transport kind, enabled and required flags.
This is not a live connection or health report. Executable arguments, environment
values, endpoints, headers and credentials are absent from this projection.
`Path` returns the selected `mcp.json` path without loading even an invalid file.
Missing profile listing is empty and creates nothing.

`Add` deliberately replaces an exact server alias with a default stdio
configuration, preserving an existing position or appending an absent alias.
It does not merge old remote settings, environment or flags. Defaults are enabled,
not required, 10,000 ms startup timeout, 60,000 ms operation timeout, empty
environment and one restart. Executable and arguments remain literal token data;
quotes, glob characters, semicolons and shell-looking expressions are not
interpreted or executed. `Remove` is exact and case-sensitive. Missing removal
and identical replacement preserve the store's observational no-op semantics.

Reload, authentication, logout and resource/prompt operations return
`RuntimeUnavailable` before any profile observation or copy of their fields.
They need separate admitted runtime authority, not a profile-management fallback.
This service grants no process, network, credential, browser or tool authority.

## Validation, cancellation and receipts

Public command enums are untrusted. `validate_command` and `execute` validate
supported direct variants before copies or filesystem observation. Aliases use
1–128 ASCII letters, digits, underscores or hyphens. Executable and argument
tokens are nonempty, contain no ASCII space or control character, and are each
at most 4 KiB, with at most 256 arguments. The minimal parser spelling, including
verb and separating spaces, must fit the command's 128 KiB aggregate budget.
The broader configuration codec's 16 KiB argument allowance does not bypass
this human-command boundary. Additional complete configuration and canonical
publication bounds remain owned by the codec and store.

Cancellation is checked before observation and again before starting a save
transaction. Once the bounded synchronous transaction starts, the service waits
for its result and retains its receipt even if cancellation arrives meanwhile.
Filesystem syscall latency is not a hard wall-clock deadline guarantee.
The service does not retry conflicts or ambiguous publication.

A `Saved` receipt carries the full store commit and independent
`McpManagementActivation::NotAttempted`. Confirmed publication does not mean a
server was started or reloaded. Post-rename uncertainty remains an `Ambiguous`
commit, not a cancellation error or claimed rollback; `receipt.failed()` reports
only this publication uncertainty. Reconciliation belongs to the caller.
All debug/error forms redact paths, aliases, arguments and configuration bytes.

The [profile store](mcp-persistence.md) owns descriptor-bound observation,
private-file policy, exact-snapshot publication and durability. The
[command contract](mcp-cli.md) owns grammar and configuration data admission.
The [implementation plan](implementation-plan.md) owns feature acceptance;
profile-management service tests alone do not establish complete MCP delivery.
