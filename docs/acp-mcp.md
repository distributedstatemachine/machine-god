# ACP session MCP ownership

ACP `mcpServers` is an authoritative, ephemeral selection for each native session
incarnation. Omission and `[]` select no servers; `null` is invalid. New, load and
resume must each supply this selection to the session host. There is no profile
fallback, merge, saved credential lookup, temporary profile file, or persistence
of injected configuration, environment or Authorization bytes.

`mcp::ephemeral::NativeMcpEphemeralConfiguration` admits a raw array without effects.
It accepts modern stdio (omitted type or `stdio`) and Streamable HTTP (`http`),
not deprecated SSE transport. Server names use the native bounded alias grammar.
Every server is enabled and required. Unknown fields and duplicate JSON keys,
server names, environment names and case-insensitive headers are rejected.

Stdio requires an absolute command, an argument array and an `env` array of
`{name,value}` strings. Empty environment inherits the explicitly captured host
snapshot; nonempty environment replaces it. Values and arguments are literal,
with no shell splitting, substitution or new ambient environment lookup.
Literal argument/environment values preserve whitespace and newlines but reject
NUL, which native process arguments and environments cannot represent.
HTTP requires the existing HTTPS or explicit-port loopback HTTP endpoint grammar
and a `headers` name/value array. Resolved Authorization is accepted through
[resolved headers](mcp-headers.md), never through the profile header codec.
Transport-owned/reserved fields, ambiguous names and invalid value bytes fail
admission. The HTTP selection always uses configured authentication, with no
stored-authentication or OAuth-service authority.

The parser reuses the bounded strict configuration JSON reader: 1 MiB input,
64 servers, depth 8, 16,384 nodes, 512 KiB decoded strings, 16 KiB individual
strings, 4096-byte command/URL, 256 arguments/environment entries and 128 headers.
Private tagged ACP configuration identities are separately capped at 2 MiB in
aggregate. These exact secret-bearing identities are native permission-binding
inputs, not public state; no profile serialization is used. Configuration and
owner debug/error forms are redacted. No secure-erasure claim follows.

## Activation and cleanup

`NativeMcpEphemeralOwner` receives a fresh dedicated runtime, owned worker scope,
captured process/network authorities, monotonic clock and epoch, owner token,
peer lifetime, reserved tool names and finite budgets. Construction and unpolled
operations perform no clock, process, filesystem, network or worker acquisition.
The runtime admits exactly one weak ephemeral-owner binding, mutually exclusive
with its profile controller. Session hosts must use distinct runtime/context and
owner allocations; matching server names never grant cross-session authority.

`NativeReferenceHostMcpOptions::with_ephemeral_startup` selects these captured
inputs separately from profile activation. Host composition rejects profile
management, controller startup or authentication selection in either builder
order, before terminal acquisition. It composes the ephemeral owner from the
actual host runtime, registered tool names and existing worker scope, not a
parallel owner or temporary profile. `capture_ephemeral_startup` explicitly
captures the retained workspace descriptor, selected helper/environment and
optional DNS/TLS authority without selecting a credential or OAuth service.
Absent remote authority still permits an empty or stdio selection.

`mcp_ephemeral_owner()` returns that exact host-owned allocation. The host binds
its weak readiness check to each conversation: every prompt requires the exact
live publication, even when the authoritative server list is empty. This path
does not call profile authentication refresh. `mcp_deadline_after` explicitly
observes the selected MCP clock; callers do not substitute ambient clock values.

`replace` reserves one mutation and one retained generation before startup's first
effect. Every selected server must become ready and pass private runtime admission.
The full candidate is conditionally published against the exact previous
checkpoint; failure, stale work or cancellation leaves the old selection intact.
Successful publication wins cancellation that arrives after its commit. Old
generation cancellation and peer retirement happen with retained cleanup custody.
Empty replacement is a real empty publication, not an absent selection.

Readiness checks the exact live publication and every required peer. Runtime
demand activation also checks the ephemeral owner and cannot reach a profile
loader. Same-generation catalog refresh reserves a short handoff, commits without
an owner lock, and advances only the exact active generation's checkpoint.
Readiness and replacement reject a handoff in progress; a concurrent close never
resurrects the active selection. Failed refresh does not adopt a predicted view.

At most eight generations (normally four) include active, pending, retired and
retained successful receipts. A receipt pins its reservation; dropping it permits
reuse only after that generation's actual local cleanup settles. Startup and
runtime retain their own finite peer/catalog limits and active-plus-retired byte
budgets. Excess ownership is rejected before another startup effect.

`close` irrevocably cancels the session owner and generations and closes the
dedicated runtime; it is not proof of child/socket completion. `settle` uses a
separate cleanup token/deadline, drains retired peers and observes startup cleanup.
The host must poll or drop its pending replacement future, then drive settlement
and finally shut down/join its worker scope. Cancellation or timeout preserves
custody for a subsequent cleanup attempt. No detached driver, browser launch,
remote revocation, HTTP DELETE or reversal of application effects is implied.
The reference host exposes `settle_mcp_ephemeral` for that owned cleanup phase.
Both `close_mcp` and the actual engine host-resource lease invalidate the
ephemeral owner before terminal worker teardown, including after `into_engine`.
Retained owner accessors, tools and requesters cannot extend the engine lease.
Drop is a final cutoff, not evidence that explicit settlement succeeded.

The [native startup](mcp-startup.md) and [runtime publication](mcp-runtime-publication.md)
contracts continue to own transport negotiation, all-ready admission, descriptor
budgets, permission-bound calls, exact generation checks and local cleanup.
