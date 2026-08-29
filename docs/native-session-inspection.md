# Native session inspection

The native layer owns an engine-free, by-ID projection of one current-schema
file session record. Core remains provider-neutral and unchanged; the CLI does
not receive a full `SessionRecord`. Its user-visible consumer is the strict
[`session` CLI contract](session-cli.md). Historical candidates, findings, and
delivery evidence are retained in the
[`session` review ledger](reviews/m03-session-cli-review-01.md); this page states
only the durable native boundary.

## Public boundary

The intended public API is:

```text
inspect_native_session(environment, id)
inspect_process_session(id)
  -> Future<Result<NativeSessionInspection, NativeSessionInspectionError>>
```

Both constructors are inert. The first poll performs the synchronous native
work. The one-pass parser's file bytes, 4 KiB input buffer, fixed-stack token
scratch, duplicate-tracker nodes, two returned ID strings, and retained summary
have finite store-owned ceilings. They are not engine limits, and the operation
has no wall-clock or attempt bound. Linux and macOS are supported. Other
targets return fixed `UnsupportedPlatform` without state selection or
filesystem access.

`NativeSessionInspection` owns validated `SessionId` and
`SessionIncarnationId` values plus `SessionRevision`, `next_turn_sequence`,
`message_count`, and `metadata_entry_count`. It exposes immutable getters only.
Construction is internal so callers cannot manufacture a snapshot that
violates the store-backed invariants. Its `Debug` output may show field names
and bounded structural values but must not contain transcript or metadata
content because none is retained.

The closed native error kinds are:

| Kind | Source |
| --- | --- |
| `UnsupportedPlatform` | No implementation exists for the target. |
| `InvalidEnvironment` | State environment selection is absent or invalid. |
| `UnsafeStateRoot` | Existing state hierarchy violates ownership/mode/ACL/no-follow policy. |
| `NotFound` | Selected root hierarchy or exact record is absent. |
| `Corrupt` | Exact record fails current durable validation. |
| `Unavailable` | Other persistence access failed or was ambiguous. |

Each kind has a stable snake-case name. Error `Debug` shows only the kind;
`Display` is fixed and redacted. The error retains no path, environment value,
record data, filename, OS diagnostic, or source error.

## State-only process capture

The process entrypoint must share the exact state-only capture rule with native
session listing rather than taking the general environment snapshot:

1. request `XDG_STATE_HOME` on first poll;
2. request `HOME` only when XDG state is missing or empty;
3. never request `XDG_CONFIG_HOME`; and
4. pass no workspace or configuration value into `NativeEnvironment`.

Tests use an injected reader to prove request order, lazy fallback, invalid
nonempty XDG behavior, no unrelated reads, and construction-versus-first-poll
timing. A reusable crate-private helper is permitted, but no new public ambient
environment API is required.

## Existing-root streaming inspection and projection

The facade reuses `open_existing_session_store`. Missing selected base or fixed
suffix returns `NotFound` rather than the empty success used by listing.
Existing hierarchy validation and descriptor retention remain unchanged. The
facade invokes a crate-private specialized summary operation with the caller's
already-validated ID. It does not call `FileSessionStore::load` and does not
construct a `SessionRecord`.

The summary operation reads the present no-follow regular record in one forward
pass under the existing per-record advisory lock. Each OS read request is at
most 4 KiB; no full-file buffer exists. Known envelope, record, message,
content, tool, role, and variant tokens are recognized with fixed-stack scratch
space. Long transcript strings and arbitrary JSON scalars are validated and
discarded as they stream. Only the validated session ID and incarnation ID are
retained as payload-sized strings; the remaining result fields are scalars.

The parser must match ordinary strict deserialization, not approximate its
wire grammar. The durable schema is object-only for `StoredEnvelope`,
`StoredRecord`, `StoredMessage`, `StoredToolCall`, and `StoredToolOutput`, and
string-only for `Role`. Ordinary loading/listing and streamed inspection reject
all six noncanonical positional-sequence or externally tagged unit-role map
forms consistently; the canonical writer remains unchanged. Typed envelope
and record fields remain unique and closed.
Numbers use canonical `serde_json::Number` acceptance before positive `u64`
conversion where required. Metadata and nested arbitrary JSON use ordinary
last-value-wins duplicate-key semantics. A fixed-digest tracker records object
key identity and each key's current logical node contribution. Its entries and
buckets grow fallibly in proportion to unique keys encountered rather than
reserving capacity for 65,536 entries on every inspection; a repeated key
replaces its prior contribution. Tracker entries are strictly capped at 65,536,
and aggregate final decoded-tree logical-node accounting is separately capped
at 65,536; shadowed duplicate values can make total parse work visit more nodes.
That total work remains bounded by the 8,651,165-byte file ceiling, and final-
tree container depth is capped at 64. Independently, parsing matches
`serde_json` 1.0.151's 127-active-container recursion accounting even for
values later shadowed by a
duplicate key. Typed parents consume three slots before metadata JSON, six
before JSON content, and seven before tool-call or tool-result JSON. Exact
nested-array accept/reject boundaries are therefore 123/124, 120/121,
119/120, and 119/120. This preserves parse acceptance plus final deserialized-
tree node/count semantics without retaining key text or values. Exact filename/
ID binding, schema,
8,651,165-byte file ceiling, identifier bounds, positive revision and
allocator, and content shape remain authoritative.

These are store-owned persistence constraints. The summary path does not
invoke engine validation and therefore does not enforce the configurable or
default 4,096-message, 8 MiB serialized-transcript, or 256 KiB serialized-
metadata limits. A store-valid historical or differently configured record
over those engine limits remains inspectable.

Store `NotFound` maps to `NotFound`, `Corrupt` maps to `Corrupt`, and every
other store category maps to `Unavailable`. Root categories remain distinct in
the native API so independent tests can prove policy, while the CLI deliberately
collapses invalid/unsafe root details to its `Unavailable` presentation.

## Effects, races, and bounds

Missing hierarchy or record is effect-free and no-create. Inspecting a present
record may create only its permanent owner-only lock sidecar through the
existing store protocol. No record bytes are modified. No engine registry,
live session, provider, event sink, permission policy, workspace, network,
configuration, credential, runtime, migration, recovery, reset, or upstream
state is consulted.

The retained root descriptor prevents path replacement from redirecting an
in-flight inspection. Existing record-lock serialization, no-follow data/lock
checks, exact file cap, and concurrent replacement semantics remain those of
`FileSessionStore`. The bounded parser resources above do not bound exclusive
sidecar-lock acquisition, filesystem latency, or retries after `EINTR`; those
have no wall-clock or attempt ceiling. They execute synchronously and may block
the polling and CLI thread. Inspection is one point-in-time durable snapshot;
a later writer may advance the record after the inspection linearization point.

The operation makes no upstream-equivalence or product-performance claim.
