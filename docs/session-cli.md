# Top-level session command

Status: production and independent evidence composed for bounded Milestone 03
slice 32 from exact delivered base
`6e687b6872e11845a306c6eaff77b1252a66c393`. Initial composition was
`852fec7`; focused composition-gate remediation is exact
`c0c16a745943a97330223aafd4a6f6a7dce84ca6`, tree
`61bcf619fc9190a9a70ab3a9c643605c88ab1817`. Focused exact-1.94.1 evidence is
green for 12 native inspection tests, 56 CLI unit tests, 46 independent CLI
process tests, and native/CLI warnings-denied Clippy. Python, pinned-fx
regeneration, WASI/FreeBSD target, diff/no-unsafe, and exact-tree release-matrix
checks are also green. The full workspace gate passed on exact precursor
`fa099f75277f7ae23a3ac220e66356c45223d1a5`,
tree `64d6a72e66b6df78bc476dadd82ce3e911644b2d`, under exact Rust/Cargo 1.94.1;
documentation integrity is 85/146/620/81 with zero errors. Exact cycle-1
candidate `5381d4b4dda2b609f256ec7237e0c4435b40a165`, tree
`4435bdeac6ffc1df5d5c8f68515082cd167dfc61`, passed its exact same-SHA local
gate but is rejected by formal review. Correctness/API reported `0/0/0/0`,
native boundary/effects reported `0/0/0/1`, and performance/concurrency/
resources reported `0/0/1/2`, in blocker/high/medium/low order. One native low
duplicates the performance track's engine-limit-documentation low, so the
deduplicated union is `0/0/1/2`. Remediation, its replacement local gate, and a
three-fresh-review cycle remain pending, as do remote workflows, `main`
integration, and delivery. This documentation correction does not claim that
production remediation exists or that review is green. No compatibility or
performance claim exists. The live evidence record is the
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

The native store must already have proved that the stored ID equals the
requested ID, both identifiers satisfy the core bound, `revision >= 1`,
`next_turn_sequence >= 1`, and the full record satisfies the store-owned
current-schema, file-byte, aggregate-JSON-depth, aggregate-JSON-node,
identifier, counter, and content-shape constraints. Loading for inspection does
not invoke an engine and therefore does not prove or enforce its configurable
limits, including the default 4,096-message, 8 MiB serialized-transcript, or
256 KiB serialized-metadata limits. A store-valid historical record written
under different engine limits remains inspectable even when it exceeds those
current defaults. The inspection facade projects only the six fields above.
It does not return or clone message bodies, tool arguments/results, reasoning,
metadata keys, or metadata values into the CLI snapshot.

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

An existing store loads exactly the requested record through
`FileSessionStore::load`. The existing per-record ceiling is 8,651,165 bytes;
stored JSON depth is at most 64 and stored JSON nodes at most 65,536. Current
strict schema, digest, filename/ID binding, positive revision/allocator, and
all message/metadata structural validation remain authoritative. Corrupt or
future records are not skipped, repaired, migrated, or rewritten.

The command creates no state root and does not create, repair, rewrite,
migrate, reset, or delete a record. Loading a present canonical record may
create its missing permanent private `0600` advisory-lock sidecar, as already
documented for the file session store. The command is therefore described as
no-root/no-record mutation, not strictly no-write. A missing root or record
creates no sidecar.

All state selection, descriptor work, record loading, and projection occur on
the thread polling the future. The retained summary and successful transferred
bytes/work are capped by the store and output ceilings, but this is not a
latency or attempt bound. Exclusive sidecar-lock acquisition, filesystem
latency, and retries after `EINTR` have no wall-clock or attempt ceiling and
synchronously block the polling and CLI thread. There is no detached task,
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
