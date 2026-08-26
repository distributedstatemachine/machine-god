# Native session inspection

Status: production and focused evidence composed for in-progress Milestone 03
slice 32 from exact delivered base
`6e687b6872e11845a306c6eaff77b1252a66c393`. Initial composition was
`852fec7`; focused composition-gate remediation is exact
`c0c16a745943a97330223aafd4a6f6a7dce84ca6`, tree
`61bcf619fc9190a9a70ab3a9c643605c88ab1817`. All 12 native inspection tests and
focused native/CLI warnings-denied Clippy are green under exact Rust/Cargo
1.94.1. Python, pinned-fx regeneration, WASI/FreeBSD target, diff/no-unsafe, and
exact-tree release-matrix checks are also green. Exact precursor
`fa099f75277f7ae23a3ac220e66356c45223d1a5`, tree
`64d6a72e66b6df78bc476dadd82ce3e911644b2d`, passed the complete required local
formatting, workspace warnings-denied Clippy, test, and doctest gate under exact
Rust/Cargo 1.94.1. Documentation integrity is 85/146/620/81 with zero errors.
Exact cycle-1 candidate `5381d4b4dda2b609f256ec7237e0c4435b40a165`, tree
`4435bdeac6ffc1df5d5c8f68515082cd167dfc61`, passed its exact same-SHA local
gate but is rejected by formal review. Correctness/API reported `0/0/0/0`,
native boundary/effects reported `0/0/0/1`, and performance/concurrency/
resources reported `0/0/1/2`, in blocker/high/medium/low order. The native low
duplicates one performance low, yielding a deduplicated `0/0/1/2` union.
Remediation, a complete replacement gate, three fresh replacement reviews,
remote workflows, `main` integration, and delivery remain pending. These
documentation corrections do not claim that production remediation exists or
that review is green. Its sole first consumer is the strict
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
work. Successful data transfer and retained values have finite ceilings, but
the operation has no wall-clock or attempt bound. Linux and macOS are
supported. Other targets return fixed `UnsupportedPlatform` without state
selection or filesystem access.

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

## Existing-root load and projection

The facade reuses `open_existing_session_store`. Missing selected base or fixed
suffix returns `NotFound` rather than the empty success used by listing.
Existing hierarchy validation and descriptor retention remain unchanged. The
facade calls `FileSessionStore::load` exactly once with the caller's already-
validated ID and polls it to completion within its own future. A theoretically
pending store future is an `Unavailable` failure; production load is currently
synchronous on poll.

`Ok(None)` becomes `NotFound`. A loaded record is accepted only after the store
has validated exact ID binding, schema, digest, file-byte and aggregate JSON
depth/node ceilings, positive revision, positive next turn sequence, identifier
bounds, and deserialized content shape. This store path does not invoke the
engine's configurable validation and therefore does not enforce the default
4,096-message, 8 MiB serialized-transcript, or 256 KiB serialized-metadata
limits. A store-valid historical or differently configured record over those
engine limits remains inspectable. The facade then moves the two IDs from the
record, copies the scalar values, counts the top-level message vector and
metadata map, and drops the remaining record without exporting it. Projection
performs no serialization and adds no independent scan or aggregate limit.

The rejected cycle-1 implementation first reads the complete capped record
bytes, deserializes the complete envelope, and materializes an owned
`SessionRecord` before discarding all but six summary fields. Output and final
snapshot retention are bounded, but those properties do not make the internal
load summary-oriented. Formal performance review classed that unnecessary raw
JSON and owned-record materialization as a medium finding. Production
remediation is pending and is not claimed by this documentation commit.

Store `NotFound` maps to `NotFound`, `Corrupt` maps to `Corrupt`, and every
other store category maps to `Unavailable`. Root categories remain distinct in
the native API so independent tests can prove policy, while the CLI deliberately
collapses invalid/unsafe root details to its `Unavailable` presentation.

## Effects, races, and bounds

Missing hierarchy or record is effect-free and no-create. Loading a present
record may create only its permanent owner-only lock sidecar through the
existing store protocol. No record bytes are modified. No engine registry,
live session, provider, event sink, permission policy, workspace, network,
configuration, credential, runtime, migration, recovery, reset, or upstream
state is consulted.

The retained root descriptor prevents path replacement from redirecting an
in-flight load. Existing record-lock serialization, no-follow data/lock checks,
exact read cap, strict decoding, and concurrent replacement semantics remain
those of `FileSessionStore`. The retained summary and successful transferred
bytes/work have finite ceilings, but exclusive sidecar-lock acquisition,
filesystem latency, and retries after `EINTR` have no wall-clock or attempt
bound. They execute synchronously and may block the polling and CLI thread.
Inspection is one point-in-time durable snapshot; a later writer may advance
the record after the load linearization point.

Focused evidence must cover inert construction, state capture, supported and
unsupported targets, missing hierarchy/record, exact projection of a nonempty
record, lock-sidecar permissions, invalid and unsafe roots, corrupt/oversized/
wrong-ID/nonregular/symlink records, retained-root replacement, redaction, and
all category mappings. CLI and release-binary evidence is owned separately.
