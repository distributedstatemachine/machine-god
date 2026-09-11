# Native MCP tool submission

`machine_god_native::mcp::submission` is an effect-free native ownership boundary
for prepared `tools/call` requests. It neither opens a transport nor grants
startup, process, network, configuration, authentication, or tool permission.
Trusted native composition supplies a live session/turn, an admitted immutable
runtime binding, and the concrete `NativePermissionExecutionProof` obtained for
the exact prepared invocation. There is no optional-proof or boolean bypass,
serialized authority token, or global current-session fallback.

## Exact preparation and admission

An explicit `McpSubmissionRegistry::register_turn(&Session, &Turn)` checks the
session/incarnation and live turn. Its non-clone registration belongs to the
native turn owner, which must retain exactly one registry for that scope and
drop the registration on turn closure. This low-level registration is trusted
composition, not a model-callable operation or a permission grant.

`prepare` accepts only an exact existing `Capability::Tool`: its tool, call and
canonical arguments must match the supplied `PermissionInvocation`, and its
session/incarnation/turn must match the registry. The existing bounded,
duplicate-key-rejecting protocol parser admits only a JSON-RPC request with
`method: "tools/call"`, an explicit non-null request ID, and exactly `name` and
`arguments` parameters. The remote name must match the runtime binding and the
arguments must equal the canonical invocation. Extra envelope/parameter fields,
control methods, notifications, malformed or ambiguous input are rejected.
JSON object key order is immaterial; exact admitted payload bytes and wire ID
are retained. Runtime transport correlation must independently allocate wire
IDs uniquely across its outstanding requests.

The input is one JSON payload without literal CR/LF framing. The direct writer
guard submits those exact bytes followed by exactly one LF. Callers must not
append another newline. Payload preparation is bounded before copying; it does
not reserve a slot until first poll. An unpolled or pre-cancelled future reserves
nothing.

Each turn follows `Reserved -> Ready -> Claimed`. The prepared value owns its
reservation generation. Dropping it or its unconsumed core admission releases
only that reservation. Binding a concrete proof does not publish a ready route:
core must consume `PermissionExecutionAdmission`, which revalidates the native
proof and live turn/runtime/cancellation before publishing. The trusted native
permission-action preparer must bind the proof received for this exact action,
not a proof obtained for another action.

Execution captures its ready generation when `claim` constructs the future;
claiming occurs only on first poll. A missing, reserved or mismatched ticket
stays invalid even if an admission/replacement is published before polling.
Session, incarnation, turn, call, tool, canonical arguments and runtime allocation
must all match. Successful claim leaves a turn-lifetime tombstone, including
after a submission is dropped without writing. Concurrent duplicate reservations,
reused permission-request IDs, duplicate claims and claimed-call replay fail
closed. No API replaces request bytes after permission.

## Runtime identity and bounds

`McpSubmissionRuntimeBinding` retains immutable server identity, exposed and
remote tool names, exact admitted configuration/schema snapshots, and resolved
authentication identity bytes. These are identity evidence supplied by trusted
native composition, not schema validation or authentication authority by
themselves. Debug and errors omit all contents; no serde implementation exists.
Retention is not encryption or secure erasure.

`McpSubmissionRuntimeOwner` mints monotonically increasing generations and
retains the active allocation. Installing a completed replacement retires the
old allocation before waking its waiters. Failed installation preserves the
old generation. Retirement or owner drop invalidates all queued/unsubmitted
requests retaining that runtime. Equal numeric generations from different
owners are not interchangeable: claims require the exact retained allocation.
Both runtime and reservation counters reject exhaustion instead of wrapping.

Bounds are 64 reserved/ready/claimed slots per registered turn, 64 KiB canonical
arguments, 4,096 argument nodes (object keys charged) and depth 64, 128 KiB exact
JSON payload, 128 KiB serialized permission request, and 1 MiB aggregate bytes
per immutable runtime binding. The framing adds one LF byte. Runtime server and
remote tool names are limited to 128 and 256 bytes respectively. Protocol JSON
uses an 8,192-node/depth-64 envelope limit. Tombstones count against the turn
slot bound; closing the turn clears them. Owners must separately bound the
number of session registries/runtime lineages and unpolled futures they retain.

## Queueing and the final writer boundary

`McpSubmission` is non-clone and retains the concrete proof, exact request and
runtime while waiting for an exclusive writer permit. Queue admission must be
raced against `cancelled()`. After acquiring the permit, `into_writer` consumes
the submission and writer/permit into an inert `McpSubmissionWrite<W>` future.
No raw request getter or future-returning delegate can detach the proof.

The guard revalidates synchronously immediately before every nonempty
`McpSubmissionWriter::poll_write` and every `poll_flush`, including retries
after `Pending`. It advances a private offset only by acknowledged successful
counts, never replays an accepted prefix, and delegates at most once per poll.
Zero/invalid counts, writer errors and revoked authority permanently terminate
the guard. A caught writer panic also leaves it terminal. Attempted is recorded
before entering the writer: an error or zero acknowledged bytes never proves
that no bytes reached the peer. No consequential request is automatically
replayed after ambiguous submission.

The writer must be the final synchronous plaintext submission boundary, not a
queue or an HTTP request-body producer that drives writes later. The public
guard is appropriate for a direct stdio plaintext sink. Feeding JSON to Hyper
does **not** satisfy HTTP submission authority: headers, TLS and independently
polled connection writes need a separate narrowly scoped native adapter that
retains the proof above TLS. This component makes no HTTP integration claim.

Turn closure first invalidates the scope, removes every slot/tombstone, then
wakes cancellation waiters and drops retained proofs outside the registry
mutex. Runtime retirement likewise invalidates before waking outside its mutex.
Preparation and execution cancellation remain independently live through queue
and writer polls; both waits and final checkpoints independently observe the
original `TurnHandle`, even when supplied tokens are unrelated to its token.
Callbacks and proof/waker destruction never run under the registry mutex.

Feature methods (`resources/read`, `prompts/get`, and others) and startup/control
messages require separate typed native preparation/authority. Arbitrary raw
methods cannot pass through this tool-only boundary. Transport effects,
production permission-preparer routing, HTTP/TLS adaptation, CLI composition
and complete-feature acceptance are separate integration responsibilities.
