# Top-level session command

Status: bounded Milestone 03 slice 32 remains in progress from exact delivered
base `6e687b6872e11845a306c6eaff77b1252a66c393`. Exact cycle-2 candidate
`1d09a0d8a289fd00533e35b975e0b53dff23d0e0`, tree
`72a63c07e4a48356f87c918a85def12b5943dad3`, passed its complete same-SHA local
gate but is rejected. Correctness/API reported `0/0/1/2`, native boundary/
effects `0/0/1/2`, and performance/concurrency/resources `0/0/1/1`, in
blocker/high/medium/low order. The deduplicated findings are two medium themes
(canonical-number mismatch and residual payload-proportional allocations) and
two low themes (duplicate-key mismatch and stale maintained documentation).
The contract below is the synchronized replacement contract. Cycle-2
replacement source was composed at exact
`f4dbe3d576c80f61b671b723eaf92ed5f29c4bbf`, tree
`86971aca0f78e637de55d2a79eda64e88bff8734`, and passed the complete required
exact-1.94.1 local gate without fallback. Its focused evidence was green at 56
CLI unit, 54 CLI process, and 21 native inspection tests; the full Python,
pinned-fx, target, documentation, no-delta, release-hash, and release-matrix
record is in the review ledger. Formal cycle 3 rejected exact candidate
`9282b404`, tree `6d41f7ee`, with track counts `0/0/1/0`, `0/0/1/0`, and
`0/0/1/1`, deduplicated to one medium context-free recursion-budget mismatch
and one low self-counted allocation-evidence finding. Exact remediation
`af055ff3`, tree `14eafad`, uses `serde_json` 1.0.151-equivalent 127-active-
container accounting with typed parent counts 3/6/7 and exact nested-array
boundaries 123/124, 120/121, 119/120, and 119/120. Focused evidence is now 58
CLI process tests, including ten equivalence cases, and 22 native inspection
tests. The complete replacement gate is green on exact `af055ff3`/`14eafad`
under Rust/Cargo 1.94.1 without fallback, including the Python, pinned-fx,
target, documentation, dependency, diff/inventory/no-added-unsafe, release-
hash, and 18/18 release-session matrix recorded in the ledger. Formal cycle 4
rejected exact candidate `df72e08404f1fb92c02d1e1af880430941d6abcc`, tree
`99bf524033c6212a05c22e7417ea6f93c202104f`. The three track counts are
`0/0/1/0`, `0/0/1/0`, and `0/0/1/0`; the deduplicated `0/0/2/0` union is an
ordinary-versus-streamed wire-form mismatch and eager approximately 8.9 MB
duplicate-tracker reservation. Exact remediation
`1f96c4bf05f93a99b86f0ca549621e739953e520`, tree
`b320f55219ebc808790138dfd293d32e83da77c3`, implements the replacement
contract and passed its complete local gate under exact Rust/Cargo 1.94.1
without fallback. Focused evidence is green at 24 native inspection and 64 CLI
process tests, including 16 differentials. The 3,985,216-byte release binary,
SHA-256 `c0e83dbfdfba7c4843a1af4c3689bda568045c84dc87ef4d6098cc7a4cd6975c`,
passed the release matrix recorded in the ledger. Fresh cycle-5 reviews, remote
workflows, `main` integration, and delivery remain pending; this is not a
review-green or delivered claim.
No compatibility or performance claim exists. The live evidence record is the
[`session` review ledger](reviews/m03-session-cli-review-01.md).

This slice adds a strict, engine-free inspection of one current-schema
machine-god session record. It exposes structural summary data only. It does
not expose transcript content or metadata values and does not construct the
native reference host.

## Grammar and exits

The only accepted invocations are:

```text
machine-god session <id>
machine-god session <id> --json
```

`<id>` must satisfy the core portable `SessionId` contract: 1 through 128
ASCII bytes containing only letters, digits, `-`, `_`, `.`, or `:`. The exact
tokens `last`, `--id`, and `--json` are reserved and invalid in ID position for
this slice. `last`, `--id <id>`, `--json <id>`, `--json=true`, reordered or
repeated flags, extra arguments, missing IDs, invalid IDs, and non-Unicode
arguments are rejected before environment or filesystem access. There is no
implicit current, latest, or workspace-relative session.

Invalid syntax writes the one global usage diagnostic to standard error,
writes no standard output, and exits 2. Success writes only standard output and
exits 0. Operational or rendering failure exits 1. Human mode writes a fixed
diagnostic to standard error with empty standard output. JSON mode writes one
compact error object to standard output with empty standard error. A stdout
failure uses the existing exact `machine-god: failed to write output\n`
standard-error diagnostic and exits 1.

The closed, redacted presentation categories are:

| Category | Meaning |
| --- | --- |
| `NotFound` | The selected machine-god root or exact record does not exist. |
| `Corrupt` | The exact canonical record failed the native record contract. |
| `Unavailable` | State selection, root safety, or persistence access failed. |
| `Unsupported` | The target has no supported native inspection implementation. |
| `ResourceLimit` | A returned invariant or complete-output ceiling failed. |

The human failure is exactly
`machine-god session: could not inspect session: <Category>\n`. JSON failure
fixes key order `kind,error,code` and is exactly
`{"kind":"session","error":"could not inspect session: <Category>","code":"<Category>"}\n`.
Neither mode reflects paths, environment values, record content, filenames,
operating-system diagnostics, raw error numbers, or underlying error text.

## Successful output

Successful inspection returns exactly these fields:

| Field | Meaning |
| --- | --- |
| `id` | The validated requested and stored session ID. |
| `incarnation_id` | The validated identity of this logical session lifetime. |
| `revision` | The positive durable optimistic-concurrency revision. |
| `next_turn_sequence` | The positive first never-reserved turn sequence. |
| `message_count` | The number of provider-neutral stored messages, not a turn or history count. |
| `metadata_entry_count` | The number of top-level metadata keys, without keys or values. |

The specialized native inspection parser must prove that the stored ID equals
the requested ID, both identifiers satisfy the core bound, `revision >= 1`,
`next_turn_sequence >= 1`, and the complete stream satisfies the store-owned
current-schema, file-byte, aggregate-JSON-depth, aggregate-JSON-node,
identifier, counter, number, duplicate-key, and content-shape constraints. It
must use canonical `serde_json::Number` acceptance and the same last-value-wins
semantics as ordinary deserialization for duplicate keys in metadata and
nested arbitrary JSON. The durable schema is object-only for `StoredEnvelope`,
`StoredRecord`, `StoredMessage`, `StoredToolCall`, and `StoredToolOutput`, and
string-only for `Role`; ordinary loading/listing and streamed inspection must
reject all six noncanonical sequence/map representations consistently. The
canonical writer remains unchanged. Inspection does not invoke an engine and
therefore does
not prove or enforce its configurable limits, including the default 4,096-
message, 8 MiB serialized-transcript, or 256 KiB serialized-metadata limits. A
store-valid historical record written under different engine limits remains
inspectable even when it exceeds those current defaults. The native boundary
returns only the six fields above and never returns or clones message bodies,
tool arguments/results, reasoning, metadata keys, or metadata values into the
CLI snapshot.

Human success is exactly this shape and order:

```text
[session] alpha
 - incarnation_id: incarnation-alpha
 - revision: 7
 - next_turn_sequence: 4
 - message_count: 3
 - metadata_entry_count: 2
```

Compact JSON fixes top-level key order
`kind,id,incarnation_id,revision,next_turn_sequence,message_count,metadata_entry_count`:

```json
{"kind":"session","id":"alpha","incarnation_id":"incarnation-alpha","revision":7,"next_turn_sequence":4,"message_count":3,"metadata_entry_count":2}
```

Both representations end in exactly one LF. The complete success or JSON
failure representation is assembled before its first byte is written and is
bounded by inclusive `MAX_SESSION_OUTPUT_BYTES = 4,096`, including the final
LF. A violated native/host snapshot invariant or output ceiling becomes
`ResourceLimit`; partial intentional success output is forbidden. Counts and
integers are rendered in canonical unsigned decimal with no padding.

## Native state and effects

Construction of the injected-environment and process-environment futures is
effect-inert. On Linux and macOS, first poll requests `XDG_STATE_HOME`; only a
missing or empty value permits a `HOME` request. It never requests
`XDG_CONFIG_HOME`. A selected nonempty relative or non-Unicode XDG state value
fails without requesting or falling back to `HOME`.

The facade then opens only an already-existing selected state base and fixed
machine-god suffix using the delivered descriptor-relative, no-follow,
effective-user, mode, and macOS ACL policy. Missing selected bases, fixed
suffixes, or exact records are `NotFound`; no missing component is created.
Unsafe, inaccessible, symlink, wrong-kind, or otherwise unavailable roots fail
closed. Unsupported targets fail without filesystem access.

An existing store inspects exactly the requested record through a specialized
summary path, not `FileSessionStore::load`. The path makes one forward pass,
requests at most 4 KiB per read, and never buffers the full file or constructs a
`SessionRecord`. The existing per-record ceiling is 8,651,165 bytes; stored
arbitrary JSON depth is at most 64 and the final deserialized arbitrary-JSON
forest is at most 65,536 nodes. Current strict schema, digest, filename/ID
binding, positive revision/allocator, canonical number acceptance, and all
message/metadata structural validation remain authoritative. Known field,
variant, and role tokens are recognized with fixed-stack scratch space. Only
the returned session and incarnation ID strings are retained as payload-sized
owned strings. Metadata and nested JSON object keys are represented during the
pass by fixed-size digests in a strictly node-capped tracker. Its entries and
buckets grow fallibly in proportion to unique keys actually encountered rather
than reserving the complete 65,536-node capacity for every inspection; repeated
keys replace the prior logical value and node contribution so the result matches
ordinary last-value-wins deserialization. No transcript string, metadata key,
or arbitrary JSON scalar is retained after validation. Before those final-tree
limits, parsing also reproduces `serde_json` 1.0.151's 127 simultaneously
active container budget, including three typed parents for metadata, six for a
JSON content block, and seven for tool-call/result JSON. Exact nested-array
accept/reject boundaries are consequently 123/124, 120/121, 119/120, and
119/120, including values later shadowed by duplicate keys. Corrupt or future
records are not skipped, repaired, migrated, or rewritten.

Allocation evidence for the replacement must bound small-record allocator
high-water use and compare long and short discarded values at equal structural
shape. A file near the byte ceiling is not, by itself, evidence that empty or
small records avoid eager maximum-capacity reservation.

The command creates no state root and does not create, repair, rewrite,
migrate, reset, or delete a record. Inspecting a present canonical record may
create its missing permanent private `0600` advisory-lock sidecar, as already
documented for the file session store. The command is therefore described as
no-root/no-record mutation, not strictly no-write. A missing root or record
creates no sidecar.

All state selection, descriptor work, streaming validation, and projection
occur on the thread polling the future. File bytes, parser depth/nodes,
fixed-size duplicate records, the two returned IDs, and final output are capped
by store-owned limits; these are not engine limits and do not create a latency
or attempt bound. Exclusive sidecar-lock acquisition, filesystem latency, and
retries after `EINTR` have no wall-clock or attempt ceiling and synchronously
block the polling and CLI thread. There is no detached task,
thread, timer, Tokio runtime, configuration read, credential discovery, engine,
provider, permission handler, prompt, workspace access, network access,
terminal replay, resume, migration, recovery, or `.fx` access.

## Deliberate pinned-fx differences

Pinned fx at `b1774fbf6c7602b503026f96f6e960e946c692ef` accepts an explicit
ID, `--id <id>`, or workspace-relative `last`, optionally with JSON. Its exact-
ID detail owns timestamps, language, history length, and full ordered history;
the same command family also participates in resume, migration, and recovery.

Machine-god instead inspects only an explicit positional ID in its global,
separate state namespace and returns the six structural fields above. It has no
authoritative workspace, timestamp, language, title, preview, history-length,
or terminal-tape fields and does not read upstream `.fx` data. `replay` remains
reserved for a future content/privacy contract, while `resume` remains reserved
for future live continuation.

The fixed bootstrap benchmark workload has no `session-json` case and is not
expanded by this slice. Compatibility generation and the existing six workload
records remain byte-stable. This feature is intentionally non-equivalent,
unmeasured, and claim-ineligible: no samples, threshold, product-performance
result, compatibility promotion, or fx-equivalence claim is introduced.
