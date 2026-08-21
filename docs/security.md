# Security

The core has no ambient filesystem, process, environment, credential, or
network authority. Native capabilities are supplied explicitly by a host. The
first Milestone 03 native slice only snapshots config/state environment inputs
and reads final-path metadata for status. A second bounded native authority
loads configuration synchronously and read-only. A third, provider-neutral
slice adds capability-aware tool preflight without exercising native authority.
Permission mode remains `ask`; executable native tools and the
prompt/fail-closed behavior of real permission requests remain future work.

Status resolution recognizes only the `machine-god` namespace. Empty XDG
values fall back to `HOME`; a selected nonempty relative or non-Unicode root is
invalid and cannot be bypassed through a `HOME` fallback. Missing or empty
`HOME` is unavailable. Inspection uses `symlink_metadata`, so a final symlink is
classified as the wrong kind instead of followed. It does not canonicalize,
open or parse `config.json`, create locations, or write state. These constraints
make status a bounded observation surface, not configuration authority.

Status paths are user-visible and may disclose the selected config/state
location when the user explicitly runs the command. JSON output uses JSON
escaping, and human output uses the same JSON-string encoding for paths. C0/C1
controls, Unicode line/paragraph separators, and bidirectional-formatting
controls are emitted as escapes, so environment-derived paths cannot inject
terminal lines or visually reorder the surrounding status fields. Non-UTF-8
CLI arguments are rejected as invalid. Output errors use a fixed diagnostic
rather than reflecting path or OS-error text.

Configuration loading uses the same config-location selection, but missing and
unavailable locations yield explicit built-in schema-v1 `ask` defaults. An
invalid selected environment value does not fall back. On the supported Unix
targets exercised by Milestone 03, a present path is opened no-follow and
nonblocking. A preliminary path-kind check is followed by authoritative
opened-descriptor regularity validation before any bytes are read. Final
symlinks and non-regular entries therefore fail closed without a FIFO open
becoming an unbounded wait. Hardened open semantics for non-Unix targets remain
deferred. The loader does not canonicalize, create, or write anything; its
no-follow guarantee is for the final component and is not a claim that the
complete ancestor path is frozen.

The raw configuration bound is 64 KiB, with at most one additional byte
retained to detect overflow or growth. Accepted bytes must be valid UTF-8 and a
JSON object containing exactly integer `schema_version: 1` and string
`permission_mode: "ask"`. Oversize files, invalid UTF-8, malformed JSON,
unknown or duplicate fields, missing or wrong fields, and unsupported versions
or modes are rejected. Typed failures do not echo environment-derived paths,
configuration contents, or operating-system error text. Inaccessible paths and
read errors are not converted into defaults.

The existing status path remains metadata-only and its CLI output is
byte-stable. Configuration mutation, prompting and modes beyond `ask`, concrete
tools or providers, native session persistence, CLI expansion, and
compatibility or performance claims remain outside the implemented native
configuration slices.

Tool preflight closes the representation gap between a model's raw JSON call
and the operation presented to permission policy. The source-compatible default
retains the existing raw `Capability::Tool` and arguments. A tool may instead
prepare a normalized `Capability` and replacement arguments, and an allowed
execution receives exactly those prepared arguments. Preparation is an
effect-free validation and normalization step: it must not open a path, start a
process, contact a network destination, mutate state, or exercise any other
unapproved authority.

Core applies its resource bounds to the prepared capability and arguments
before policy or execution. An invalid or oversized prepared value fails before
either boundary. A tool-reported preparation error is reduced to a fixed generic
durable tool error, does not consult policy, and causes no tool effect. Policy
still sees a fresh request with the existing critical risk, fixed reason, and
deterministic permission ID; core still does not cache positive grant scopes.
This slice supplies the authorization seam only. Workspace confinement,
symlink-safe filesystem access, and the first native `read_file` implementation
remain planned and must use the same path for authorization and execution.

The threat model must cover workspace escape, symlink races, command injection,
permission confusion, SSRF, secret exposure, corrupted state, denial of service,
and cancellation or shutdown races.

Every logical session lifetime has a validated, host-generated
`SessionIncarnationId` persisted in its `SessionRecord`. Core has no clock or
randomness and never derives a fallback. Reusing, resetting, or rewinding a
`SessionId` requires a new globally unique incarnation; stores reject changing
the incarnation of an existing record, and an engine rejects a different
incarnation while that session ID is live. Permission request IDs use the v2
SHA-256 preimage over length-delimited session ID, incarnation ID, turn ID, and
ordinal. This prevents an ID-keyed permission cache from replaying an allow into
a fresh logical lifetime whose turn numbering and tool-call ordinal restarted.
`ToolContext` and `EngineEvent` carry the same incarnation so tool idempotency
keys and event-sink deduplication or audit keys cannot collide across resets
whose session, turn, call, and event sequence identifiers repeat.

Event sinks are untrusted diagnostic producers. A sink failure crosses the
public API only as `event_sink_failed` / `event sink failed`; the original code
and message are dropped, including when cancellation races a ready sink error.
Engine debug formatting likewise never calls `ModelProvider::name` or includes
provider-controlled text; it exposes only fixed structural fields.

Benchmark CI obtains Zig only from the official Zig 0.16.0 HTTPS archive and
verifies SHA-256 digest
`70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
before extracting into a fresh fixed directory under `RUNNER_TEMP`. The workflow
then fails unless the installed executable reports version `0.16.0`. This keeps
the upstream-reference compiler outside the Rust product's dependency and
authority surfaces while binding its CI bytes without a third-party setup
action.
