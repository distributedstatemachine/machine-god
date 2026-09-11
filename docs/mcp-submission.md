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

`prepare` and `prepare_http` accept only an exact existing `Capability::Tool`: its tool, call and
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

The input is one JSON payload without literal CR/LF framing. For `prepare`, the direct writer
guard submits those exact bytes followed by exactly one LF. Callers must not
append another newline. Payload preparation is bounded before copying; it does
not reserve a slot until first poll. An unpolled or pre-cancelled future reserves
nothing.

`prepare_http` instead binds a complete fixed-length HTTP/1.1 POST before
reservation and permission admission. `McpSubmissionHttpHead` derives the
origin-form request target and Host from the explicitly admitted `McpEndpoint`.
It retains bounded explicit headers with canonical lowercase names and exact
byte values, including HTAB and obs-text/non-UTF-8 bytes from resolved header
inputs. Generated Host, Content-Length, Content-Type, Accept and
Connection headers cannot be overridden, including case variants. Duplicate
names, prohibited controls (including CR/LF/NUL/DEL), framing/encoding, proxy and hop-by-hop overrides
are rejected. The generated request uses `Connection: close`,
`Content-Type: application/json`, and an Accept value for JSON/event streams.
Content-Length covers exactly the admitted JSON payload, without NDJSON's LF.
No chunking, trailers, Expect negotiation or arbitrary HTTP methods are admitted.
Endpoint/header/body bytes cannot change after permission. The trusted preparer
must select this head from the pinned runtime's admitted endpoint/authentication
configuration; a syntactically valid endpoint is not network or credential
authority.

Native connectors can inspect the exact immutable HTTP request before opening
the admitted destination; this read-only observation is not a submission grant.
An owned cancellation observer retains execution, preparation, registered-turn,
original core-turn and runtime-retirement signals after the submission writer
is consumed, so response reads remain cancellable. It retains neither a proof
nor a replayable submission. The borrowed queue observer uses the same path.

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

HTTP adds at most 1 MiB of encoded request head to the 128 KiB JSON payload,
with an explicit total wire cap of 1 MiB + 128 KiB. The head accepts up to
256 trusted-composed fields, 16 KiB names/values and 768 KiB summed name/value
bytes, excluding endpoint and framing. The resolved-input validator separately
admits at most 128 fields/512 KiB before trusted native composition appends its
bounded protocol headers; this transport head is not a substitute for that
input validator. Endpoint, generated headers and CRLF framing are charged
against the independent 1 MiB encoded-head cap. A vectored driver call accepts at most 64 slices and never
concatenates them. Header construction, request construction and prefix matching
all charge their bounds before copying/delegating. The HTTP adapter is enabled
on Linux/macOS, matching the existing endpoint type's platform boundary.

## Queueing and the final writer boundary

`McpSubmission` is non-clone and retains the concrete proof, exact request and
runtime while waiting for an exclusive writer permit. Queue admission must be
raced against `cancelled()`. After acquiring the permit, `into_writer` consumes
the NDJSON submission and writer/permit into an inert `McpSubmissionWrite<W>` future.
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
queue or an HTTP request-body producer that drives writes later. `into_writer`
is for the NDJSON direct stdio sink; HTTP-framed submissions reject before any
writer work if routed through it. Conversely, `into_http_driver` consumes and
rejects an NDJSON submission. No transport-mode conversion happens after proof.

## Proof-bearing HTTP connection driver

`into_http_driver` consumes an HTTP-prepared `McpSubmission` and an exclusive
plaintext sink into a non-clone `McpSubmissionHttpDriver<W>`. The native
connection must be dedicated, nonpooled, nonpipelined HTTP/1.1 for exactly that
one request. The wrapper belongs immediately **above TLS**, around the final
connection writer, and keeps the original concrete proof/turn/runtime through
connection-driver scheduling, partial writes and flush retries. Neither polling
an HTTP body nor checking permission before `send_request` satisfies this
boundary. Feeding JSON into Hyper is not by itself proof-bearing submission.

Each scalar/vectored driver write must exactly match a prefix of the remaining
pre-admitted plaintext. All offered slices are validated before any sink call;
only successful acknowledged counts advance the private cursor, including
counts crossing vector boundaries. Changed endpoints, headers, bodies, repeated
prefixes or trailing/pipelined requests fail closed and permanently terminate
the wrapper before delegation. Empty writes perform no sink work. Pending
retries recheck authority; scalar/vectored writes and intermediate/final flushes
use the same cancellation/proof/attempt state as the direct writer. Cancellation
or revocation observed during delegation wins without erasing acknowledged
effects. A final successful flush after all bound bytes closes further writer
operations. Intermediate flushes never reset the cursor.

The wrapper exposes no inner-writer escape or detachable/reusable permission
guard. A crate-private borrowed exact-request accessor is solely data for the
trusted native encoder; it confers no authority. `belongs_to_runtime` on the
claimed submission lets a connection check its explicitly admitted runtime set
by `Arc::ptr_eq`, not server-name or numeric-generation equality. This identity
observation remains true after retirement and is not a live permission check:
the writer independently revalidates before effects. Network destination/DNS,
TLS/ALPN/certificates, dedicated connection ownership, responses and actual
HTTP-driver adaptation remain coordinator-owned integration responsibilities.
No HTTP/2, pooling, redirects, reconnection or request replay is supplied here.

Exact byte matching is not a claim of compatibility with Hyper's opaque header
ordering/casing or another HTTP encoder's serialization. The transport must
send the retained exact bytes (a bounded data copy into its encoder is allowed)
or deliberately align its serialization; differing valid HTTP encodings are
rejected. This component includes no actual Hyper integration. The connection
separately owns its response read half; this proof-bearing adapter controls only
the final plaintext write half above TLS and adds no networking or response
parser.

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
production permission-preparer routing, actual HTTP/TLS connection integration, CLI composition
and complete-feature acceptance are separate integration responsibilities.
