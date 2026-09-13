# ACP human interaction

ACP uses the native permission and human-input owners. Client replies are data,
not permission, execution, browser or persistence authority. Each pending reply
must retain its exact session incarnation, operation and request round; stale
replies cannot be rebound to a replacement session.

## Modern client-managed URLs

The native MCP presenter may explicitly select a client-managed URL endpoint.
This selection, not untrusted capability flags, enables URL support without a
local browser launcher. Client URL consent never invokes the native browser or
its recovery prompt, even if a browser launcher is also available. Native
interactive CLI presentation keeps its explicitly selected local-browser path.

Registration occurs only after native request admission and revalidation, before
presentation. It is bounded and nonblocking and does not submit anything by
itself. Each registration owns an exactly-once terminal observation. Only an
accepted URL is retained through the continuation; declining, cancelling or
dropping a pending answer abandons its registration. Answer acceptance and
browser handoff are never operation-completion evidence.

For model tools, successful terminal observation follows the final admitted
response and native archive publication. Human resource-read and prompt-get
actions retain the same custody until their final admitted response. Unresolved
input, exhausted continuation rounds and protocol failure are not successful
completion. Any early error, cancellation, stale authority or dropped operation
abandons its remaining registrations. Completion does not imply that the
remote tool's application-level result was successful.

At most 256 completion registrations can be retained per operation: eight
rounds of 32 requests. The ACP endpoint must independently bound its pending
correlations and queued output, discard never-submitted or stale registrations,
and emit modern `elicitation/complete` only for accepted, completed requests.
Callbacks enqueue without blocking; they do not perform I/O in destructors.
No legacy URL registry, inbound legacy completion or automatic retry is added.

`NativeAcpElicitationPresenter` adapts the existing native prompt bridge. Its
256-entry limit includes pending registrations and queued completion notices;
pre-submission request retention has a separate 8 MiB bound. Admission to the
wire output lane releases retained request bytes, keeping only compact host
correlation and the exact principal. IDs never repeat within the endpoint,
including same-session reactivation. Deactivation discards old registrations
and queued notices without publishing success. No request or answer is saved
to profile configuration or credentials.

See the [implementation plan](implementation-plan.md) for integration and
delivery gates; this contract is not a delivery-status ledger.
