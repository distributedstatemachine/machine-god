# Native session inspection

Status: bounded Milestone 03 slice 32 remains in progress from exact delivered
base `6e687b6872e11845a306c6eaff77b1252a66c393`. Exact cycle-2 candidate
`1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`, passed its complete same-SHA local
gate but is rejected. The three formal verdicts were `0/0/1/2`, `0/0/1/2`, and
`0/0/1/1`; the deduplicated findings are canonical-number mismatch, residual
payload-proportional allocations, duplicate-key mismatch, and stale maintained
documentation. This page now states the synchronized replacement contract.
Replacement source is composed at exact
`f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, and passed the complete required
exact-1.94.1 local gate without fallback. Formal cycle 3, remote workflows,
`main` integration, and delivery remain pending. Focused evidence is green at
21 native inspection, 56 CLI unit, and 54 CLI process tests; complete gate
evidence is recorded in the live ledger. Its sole first consumer is the strict
[`session` CLI contract](session-cli.md).

The native layer owns an engine-free, by-ID projection of one current-schema
file session record. Core remains provider-neutral and unchanged; the CLI does
not receive a full `SessionRecord`.

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
wire grammar. Typed envelope and record fields remain unique and closed.
Numbers use canonical `serde_json::Number` acceptance before positive `u64`
conversion where required. Metadata and nested arbitrary JSON use ordinary
last-value-wins duplicate-key semantics. A fixed-digest tracker records object
key identity and each key's current logical node contribution; a repeated key
replaces its prior contribution. Tracker entries and aggregate arbitrary-JSON
work are strictly capped by the store's 65,536-node limit, and container depth
is capped at 64. This preserves final deserialized-tree node/count semantics
without retaining key text or values. Exact filename/ID binding, schema,
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

Replacement evidence must cover inert construction, state capture, supported
and unsupported targets, missing hierarchy/record, exact projection of a
nonempty record, chunk boundaries, canonical and out-of-range number behavior,
duplicate metadata/nested keys with last-value-wins counts, fixed resource
ceilings, lock-sidecar permissions, invalid and unsafe roots, corrupt/
oversized/wrong-ID/nonregular/symlink records, retained-root replacement,
redaction, and all category mappings. CLI and release-binary evidence is owned
separately. These requirements are green on the composed replacement SHA; the
formal cycle-3 review outcome remains pending.
