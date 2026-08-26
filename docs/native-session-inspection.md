# Native session inspection

Status: bounded Milestone 03 slice 32 remains in progress from exact delivered
base `6e687b6872e11845a306c6eaff77b1252a66c393`. Exact cycle-2 candidate
`1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`, passed its complete same-SHA local
gate but is rejected. The three formal verdicts were `0/0/1/2`, `0/0/1/2`, and
`0/0/1/1`; the deduplicated findings are canonical-number mismatch, residual
payload-proportional allocations, duplicate-key mismatch, and stale maintained
documentation. This page now states the synchronized replacement contract.
Cycle-2 replacement source was composed at exact
`f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, and passed the complete required
exact-1.94.1 local gate without fallback. Its focused evidence was green at 21
native inspection, 56 CLI unit, and 54 CLI process tests. Formal cycle 3 then
rejected exact candidate `9282b404`, tree `6d41f7ee`, with track counts
`0/0/1/0`, `0/0/1/0`, and `0/0/1/1`. The deduplicated `0/0/1/1` union is a
medium context-free recursion-budget mismatch and low self-counted allocation
evidence. Exact remediation `af055ff3`, tree `14eafad`, is composed; its
complete replacement gate is green under exact Rust/Cargo 1.94.1 without
fallback. Formal cycle 4 rejected exact candidate
`df72e08404f1fb92c02d1e1af880430941d6abcc`, tree
`99bf524033c6212a05c22e7417ea6f93c202104f`. All three tracks reported
`0/0/1/0`; the deduplicated `0/0/2/0` union is an ordinary-versus-streamed
wire-form mismatch and eager approximately 8.9 MB duplicate-tracker
reservation. Exact remediation
`1f96c4bf05f93a99b86f0ca549621e739953e520`, tree
`b320f55219ebc808790138dfd293d32e83da77c3`, implements the replacement
contract and passed its complete local gate under exact Rust/Cargo 1.94.1
without fallback. Focused evidence is green at 24 native inspection and 64 CLI
process tests, including 16 differentials. Formal cycle 5 rejected exact
candidate `8f533cdec235660c3e17b70fc5bbd5dd0ab8c1f6`, tree
`8215fb94fa3de08841b26dd9d7c63a2ecb7e8a8d`. Correctness/API, native boundary/
effects, and performance/concurrency/resources reported `0/0/0/1`, `0/0/0/0`,
and `0/0/0/1`, respectively, deduplicated to `0/0/0/1`. The sole low is stale
cycle/allocation status in maintained summary pages, not a native production
finding. Independent correctness evidence completed 312 generated valid/
boundary and 1,200 randomized mutation differentials with zero mismatch; the
native track and performance allocation/resource audit were green. Cross-
document status remediation, fresh cycle-6 reviews, remote workflows, `main`
integration, and delivery remain pending. Its sole first consumer is the strict
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
reserving the full 65,536-node capacity for every inspection; a repeated key
replaces its prior contribution. Tracker entries and aggregate arbitrary-JSON
work are strictly capped by the store's 65,536-node limit, and container depth
is capped at 64. Independently, parsing matches `serde_json` 1.0.151's 127-
active-container recursion accounting even for values later shadowed by a
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

Replacement evidence must cover inert construction, state capture, supported
and unsupported targets, missing hierarchy/record, exact projection of a
nonempty record, chunk boundaries, canonical and out-of-range number behavior,
duplicate metadata/nested keys with last-value-wins counts, fixed resource
ceilings, lock-sidecar permissions, invalid and unsafe roots, corrupt/
oversized/wrong-ID/nonregular/symlink records, retained-root replacement,
redaction, and all category mappings. CLI and release-binary evidence is owned
separately. Replacement allocation evidence must bound allocator high-water
use for an empty or small record and compare long versus short discarded values
at equal structural shape; a near-cap file alone does not prove the absence of
eager maximum-capacity reservation. Cycle-3 remediation focused evidence is
green at 22 native tests
and 58 CLI process tests, including ten exact ordinary-store/listing/session
equivalence cases. Real dev-only allocation instrumentation runs five shapes in
separate child processes; all five have exact totals/current/max of 14/2/8
allocations and 8,913,715/14/8,913,347 bytes. The complete exact-remediation
gate is green: required Rust, Python 135/8 skips, pinned fx, WASI/FreeBSD, docs
85/147/626/81, dependency policy/audit, diff/inventory/no-added-unsafe, release-
hash, the 18/18 session matrix, and direct/native near-cap probes all passed.
Those results belong to the rejected cycle-4 candidate. Exact replacement
`1f96c4bf05f93a99b86f0ca549621e739953e520`, tree
`b320f55219ebc808790138dfd293d32e83da77c3`, passes 24 focused native tests and
the complete exact-1.94.1 local gate without fallback. Its allocator evidence
reports `12/2/7` allocations and `819/14/645` bytes for empty, short-text, and
long-text records; `14/2/8` and `1,427/14/1,059` for short/long JSON; and
`35/2/9` and `2,228,435/14/1,606,083` for 5,000 keys, in total/current/maximum
order. The direct native near-cap probe passed 1/1. Full evidence is retained in
the review ledger. Formal cycle 5 rejected exact candidate `8f533cd`, tree
`8215fb9`, only because maintained summary pages were stale. The native track
reported `0/0/0/0`; production review and the performance allocation/resource
audit were otherwise green. Cross-document status remediation and fresh
cycle-6 review remain pending.
