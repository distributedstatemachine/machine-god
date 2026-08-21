# Security

The core has no ambient filesystem, process, environment, credential, or
network authority. Native capabilities are supplied explicitly by a host. The
first Milestone 03 native slice only snapshots config/state environment inputs
and reads final-path metadata for status. A second bounded native authority
loads configuration synchronously and read-only. A third, provider-neutral
slice adds capability-aware tool preflight without exercising native authority.
A fourth slice adds a bounded Unix-only `read_file` implementation behind that
preflight seam. A fifth adds bounded one-level Unix-only `list_files`
enumeration behind the same seam. Permission mode remains `ask`; CLI
registration, prompting, and the fail-closed behavior of a production
permission handler remain future work.

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
providers, native session persistence, CLI expansion, native tools other than
the bounded library-level `read_file` and `list_files`, and compatibility or
performance claims remain outside the implemented slices.

Tool preflight closes the representation gap between a model's raw JSON call
and the operation presented to permission policy. The source-compatible default
retains the existing raw `Capability::Tool` and arguments. A tool may instead
prepare a normalized `Capability` and replacement arguments, and an allowed
execution receives exactly those prepared arguments. Preparation is an
effect-free validation and normalization step implemented by trusted host code.
It must be deterministic, synchronous, bounded, and nonblocking, and it must not
open a path, start a process, contact a network destination, mutate state, or
exercise any other unapproved authority. Core checks cancellation immediately
before and after preparation returns but cannot interrupt it in flight.

Core applies its resource bounds before policy or execution. Prepared arguments
retain the exact configured tool-argument byte limit and their JSON depth and
node bounds. Capability depth and node traversal covers only JSON values
embedded in `Capability::Tool` or `Capability::Custom`; every capability variant
is separately serialized as a whole under one total byte cap of
`max_tool_argument_bytes + 1024`. The fixed 1 KiB is headroom within that total
cap, not a separately metered envelope or payload allowance. An invalid or
oversized prepared value fails before either boundary. A tool-reported
preparation error is reduced to a fixed generic durable tool error, does not
consult policy, and causes no tool effect. Policy still sees a fresh request
with the existing critical risk, fixed reason, and deterministic permission ID;
core still does not cache positive grant scopes.

The prepared arguments are not a second grant of authority. A trusted tool may
use them only for effects contained by the exact prepared capability that
policy allowed. Native filesystem, process, and network implementations must not
reinterpret them into a broader path, command, or destination. This slice
supplies that authorization seam. Its first native consumer is `read_file`. The
host injects one absolute workspace root, and Unix construction opens and
retains the final root directory without following a final symlink. Before that
open it rebuilds the host root from lexical path components, removing redundant
separators and `.` components throughout while preserving `..`, without
canonicalizing ancestors or resolving symlinks. The resulting removal of
terminal separators and terminal `.` components ensures that `/root-link/` and
`/root-link/.` cannot bypass final-component no-follow; the equivalent forms of
a real directory remain accepted. Provider input is exactly an object with one
UTF-8 `path` string bounded to 4,096 bytes. Effect-free preflight performs only
strict decoding and lexical normalization; it removes `.` components,
collapses repeated separators, and rejects `/`-rooted paths, `..`, forbidden
control or line/paragraph-separator or bidirectional-formatting characters, or
an empty normalized path before policy. On supported Unix, backslash and
Windows-looking prefixes are ordinary confined filename bytes, not path
separators or prefixes.
Policy receives
`FilesystemAccess::Read` for exactly the normalized workspace-relative path
that execution receives.

Allowed execution starts from the retained root descriptor and opens every path
component descriptor-relatively with no-follow semantics. It retains stable
ancestor descriptors during traversal, opens nonblocking, and authoritatively
checks that the final descriptor is a regular file before reading. A symlink in
any model-selected component, a directory, FIFO, socket, device, or other
non-regular final entry fails closed. Path replacement after a component is
opened cannot redirect later lookup through a different ancestor; replacement
of the final pathname after open cannot change which descriptor is read.
Replacement by another ordinary file before its open may change the bytes at
the same authorized path, so this is path authorization, not an immutable file
identity or snapshot guarantee.

The reader retains at most 8 KiB plus one byte to detect overflow or growth and
never truncates a successful result. It accepts only valid UTF-8 and returns
exactly `{content}`; successful content is intentionally model-visible and
therefore also present in the durable tool result and observer event. Failures
use the fixed constructor and tool-error tables in
[`read-file.md`](read-file.md). Operating-system error numbers may select among
those fixed categories, but no OS text, workspace path, requested path, or file
bytes enter their display, debug, code, or message. Only non-cancellation
preparation and execution errors become core's generic durable tool-error
result. A direct `Tool::execute` observes cancellation as the fixed
`read_file_cancelled` error; when the engine owns the shared token, engine
cancellation wins, terminates the turn, and leaves the unknown placeholder
rather than writing a generic tool error. Cancellation is checked before
traversal, between bounded reads, and after content validation. It closes
retained per-call descriptors and buffers but cannot interrupt an individual
open, metadata, or read syscall already in flight.

`list_files` uses the same authorization and confinement boundary for
`FilesystemAccess::Enumerate`. Strict effect-free preflight accepts only `{}` or
a sole string `path`, defaults omission to `.`, applies the 4,096-byte bound and
the same absolute, parent, control, and bidirectional-character rejection, and
gives policy and execution the exact same normalized path. Backslash and space
are literal Unix filename characters. No preflight filesystem effect occurs.

Allowed enumeration opens every selected component relative to the retained
root with directory and no-follow requirements. It never follows a selected or
entry symlink and never opens a child, resolves an entry target, reads content,
recurses, applies ignore rules, accesses an external path, or discovers a
workspace. It skips only `.` and `..`. Entry kinds come directly from the
directory entry type; an unknown type is `other`. Every observed visible name,
including a truncation witness, must be safe valid UTF-8 or the call fails with
a fixed redacted error.

Enumeration retains no more than 100 entries and 16 KiB of aggregate raw name
bytes. It reads the first extra visible entry needed to prove truncation and
then stops, sorting only what it retained. This limits retained memory and
syscall work after a bound is crossed, but a truncated selection can reflect
filesystem iteration order. It is not a whole-directory ordering or snapshot
claim. At the independent maximums, JSON escaping and fixed entry syntax remain
within 44,130 serialized bytes including core's fixed `ToolOutput` envelope,
below the default 64 KiB result limit.

Failures use the fixed constructor and tool-error tables in
[`list-files.md`](list-files.md). No root, requested path, entry name,
operating-system text, or error number enters the fixed public diagnostic.
Cancellation is checked before traversal, between component opens, before
enumeration, between entry reads, and after result validation and sorting. It
cannot preempt one open or directory-read syscall already in flight and spawns
no detached work.

These tools provide descriptor-rooted confinement of model-selected path
components, not a claim that an untrusted host is sandboxed. The host's
resolution of ancestor components leading to an injected root path and mount
points visible beneath a retained directory are trusted inputs. Hardened
non-Unix workspace construction and traversal remain deferred. The normative
surfaces are [`read-file.md`](read-file.md) and
[`list-files.md`](list-files.md).

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
