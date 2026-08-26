# Native session listing

Status: delivered sixteenth bounded Milestone 03 library slice. Production,
documentation, and 13 initial independently owned tests were
composed through `dec98e0`, but each first-round formal review track reported
confirmed findings. Acquire-first production fix `4b8d8b0`, test hardening
`446b495`, and the corrected contract are composed in this replacement
behavior candidate `3fa54635dab00ebba78b233c69fd39e04e9be57e`. Its 18 focused
tests, full all-target/all-feature workspace suite, and all three replacement
review tracks are green. First remote CI run `32599591900` exposed a Linux-only
removed-root liveness gap. Exact portable-fix candidate
`17f1884c20e84574561eb3cedd96b9aee6d37284` adds linked-descriptor validation
and is green under both executable review tracks. Documentation seal
`d3312d7f402a162289524e987a4a1793c18528f5` resolves the remaining exact-lineage
finding, passed exact feature CI `32600292770` and benchmark evidence
`32600292779`, was fast-forwarded without force to `main`, and passed exact main
CI `32600567094` and benchmark evidence `32600567090`. This documentation-only
commit is the final delivery record; its workflows are reported at handoff.

The exact first-candidate lineage is: integrated base `9ada4b5`; isolated
production `0accfbf`, composed as `1bffac9`; isolated documentation `63d589c`,
composed as `87d7de0`; isolated tests `1b531297`, composed as `4b4e468`; and the
removed-root finding fix plus first formal review candidate `dec98e0`. The
isolated replacement source fix is `4b8d8b0`; isolated finding-test hardening is
`446b495`; both are composed in exact behavior candidate
`3fa54635dab00ebba78b233c69fd39e04e9be57e`. The Linux portable-fix behavior is
exact candidate `17f1884c20e84574561eb3cedd96b9aee6d37284`. Full lineage and
findings are recorded in the
[`native session-listing review`](reviews/m03-native-session-listing-review-01.md).

This slice adds `NativeSessionLifecycle::list_sessions` on supported Linux and
macOS targets. It returns an owned `NativeSessionList` containing only validated
session IDs and a `truncated` flag. The operation is a bounded observation of
the existing flat file-session root; it is not a search index, registry,
pagination protocol, or multi-record snapshot.

## Result contract

The result owns:

- no more than 100 validated session IDs;
- IDs sorted in ascending lexical order and containing no duplicates; and
- `truncated`, which is true when any scan, aggregate-byte, or result bound
  prevents a complete observation of the directory.

`truncated` means only that the bounded observation is incomplete. It is not
`has_more`, does not promise another page, and supplies no cursor or continuation
token. It also does not prove that another valid session exists: ignored entries
consume the scan budget, and the bound can be reached before another canonical
record is found.

Recognized canonical candidate filenames are sorted before record validation.
Result-count and aggregate-byte truncation therefore select deterministically
from the sorted scanned candidate set. The filesystem chooses raw enumeration
order, however, so only a fired 1,024-entry scan cap can make that candidate set
iteration-order-dependent. Returned IDs are sorted after selection. Because
candidate filename order is digest order, even an otherwise deterministic
truncated result is not promised to contain the globally first 100 session IDs,
the newest IDs, or any other semantic ranking.

An empty accepted root returns an empty, non-truncated result. A complete
non-truncated result means every visible entry observed during that enumeration
fit within the fixed budgets and every canonical candidate still present at its
locked read was validated. A candidate that concurrently vanishes can be
omitted as described below. The result is still only a non-atomic observation;
another process can change another record before, during, or after the call.

## Fixed bounds

One call has three independent ceilings:

| Bound | Maximum | What consumes it |
| --- | ---: | --- |
| Returned IDs | 100 | Each distinct validated canonical record selected for the result. |
| Non-dot directory entries processed or selected | 1,024 | Every non-dot entry within the scan budget, including unrelated, lock, temporary, and noncanonical names. The iterator may fetch and inspect the name of one additional non-dot entry solely as the overflow witness. |
| Accepted and decoded aggregate canonical record bytes | 64 MiB | Bytes accepted from recognized canonical record candidates. A bounded read may transiently transfer one additional byte solely to detect concurrent growth past the remaining aggregate budget; that witness is not accepted or decoded. |

The implementation stops bounded observation when continuing would exceed a
ceiling and returns the accepted subset with `truncated: true`. The existing
per-record `MAX_FILE_SESSION_BYTES` limit remains authoritative: one recognized
candidate that exceeds that fixed file bound is corrupt, not a benign aggregate
truncation case.

Counting all non-dot entries prevents an attacker from hiding unbounded scan
work behind ignored names. Dot entries supplied by the directory API itself are
not user-visible children and are not candidates. The implementation processes
or selects at most 1,024 non-dot entries and may fetch and name-inspect only the
first additional non-dot entry needed to prove overflow. It accepts and decodes
at most 64 MiB of aggregate canonical record bytes and may transiently transfer
only one additional byte to detect concurrent growth beyond the remaining byte
budget. Successful work and retained application data are bounded; directory,
file, and advisory-lock latency and the store's documented interrupted-system-
call retries have no wall-clock bound.

## Candidate recognition and validation

Only an exact canonical record basename is a candidate:

```text
session-<64 lowercase hexadecimal ASCII characters>.json
```

The prefix, digest width, lowercase encoding, and `.json` suffix must all match.
Lock sidecars, temporary artifacts, unrelated files, uppercase hashes,
wrong-width hashes, nested directories, and other noncanonical names are
ignored after consuming scan budget. Ignoring a name is not permission to
follow it or interpret it as another record format.

Each canonical candidate is validated through the same file-session invariants
as a by-ID load:

- descriptor-relative, no-follow access under the retained session root;
- an authoritative regular-file check;
- the current strict compact schema-v1 envelope and structural bounds;
- positive revision and next-turn counters;
- the fixed per-record byte ceiling; and
- exact agreement between the decoded `SessionId` and the digest in the
  candidate filename.

The replacement contract acquires the fresh `.` descriptor relative to the
retained root first, then checks that exact acquired descriptor's linked
identity before constructing its enumeration stream. Linux rejects an acquired
descriptor with zero links. On macOS, where an unlinked retained descriptor can
still reopen `.`, the linked-identity check resolves the acquired descriptor's
parent/name identity rather than checking the earlier retained descriptor.
A stable completed rename preserves directory identity and remains valid.
Removal before acquisition or before the identity check is `Unavailable`.
Concurrent rename or removal may conservatively return `Unavailable` or may
return an observation of the exact acquired identity; it never redirects to a
replacement and does not create a global snapshot.

The operation uses the store's permanent per-ID advisory lock while validating
each candidate. A successful listing can therefore create a missing fixed
`.lock` sidecar with private `0600` mode. It never writes, repairs, replaces,
deletes, migrates, or quarantines a record. Noncanonical entries are not opened
as records and do not cause sidecars to be created.

A canonical symlink, directory, FIFO, device, socket, oversized record,
malformed or unsupported envelope, invalid counter, or filename/decoded-ID
mismatch is `Corrupt`. A hostile or nonregular derived lock entry for a present
exact data candidate is also `Corrupt`. Corruption reached within the bounded
selected set fails the complete call; the API does not skip it or return a
partial successful list. A candidate omitted beyond a truncation boundary is
not inspected and cannot poison that successful partial result. Directory
enumeration, record open/read, metadata, or ordinary lock I/O failures are
`Unavailable`. These categories reuse the native lifecycle's fixed redacted
operation-error boundary.

## Concurrency and snapshot semantics

Listing takes no root-wide lock and creates no multi-record transaction. Each
candidate is locked and validated independently. A returned result can
therefore contain IDs observed at different instants, and concurrent create or
reset operations can race between candidates. Per-ID locking prevents a
cooperating writer from exposing a partial record under that candidate's
linearization point; it does not turn the directory into a consistent global
snapshot.

If a recognized candidate disappears between enumeration or probing and its
locked record read, listing may omit it rather than fail the complete call. The
lock acquisition can create or leave that candidate's permanent private
sidecar even though no ID is returned. This rule is narrowly about concurrent
absence; a still-present canonical candidate must pass the validation and
failure rules above.

IDs are deduplicated defensively before return even though canonical digest
names normally provide one candidate per ID. The final ID sort is deterministic
for the selected set. Candidate selection is filesystem-iteration-dependent
only when the raw directory scan cap fires.

## Polling and authority

Constructing the returned future performs no directory read, record read,
metadata call, lock creation or acquisition, allocation proportional to the
unbounded directory, provider call, permission prompt, tool call, network
request, registry access, runtime construction, or background work. Dropping
the future before first poll is effect-free.

The first poll performs the bounded directory enumeration, synchronous
candidate I/O, and advisory locking on the polling thread. The implementation
starts no task, thread, timer, retry worker, or detached effect. Once a
synchronous call is running, dropping the future cannot preempt it. Hosts that
must keep an asynchronous executor responsive must choose a suitable polling
context.

The listing operation receives only the `FileSessionStore` already retained by
`NativeSessionLifecycle`. It does not inspect the engine's live-session
registry or source, ask the provider for history, call tools, consult permission
policy, read configuration or environment, access the workspace, allocate an
incarnation, or discover another root. Returned IDs are deliberately visible to
the trusted caller. `NativeSessionList` and its derived `Debug` deliberately
expose those IDs and `truncated`; callers must treat them as session identity
data. Only lifecycle error `Display` and `Debug` are redacted: they retain no
session ID, digest, filename, root or child path, record bytes, schema contents,
operating-system diagnostic, or raw error number.

The standalone lifecycle listing API is available on Linux and macOS without
the optional HTTP feature. Observation through `NativeReferenceHost` inherits
that wrapper's stricter `ai-gateway-http`, non-WebAssembly, Linux/macOS gate.

## Deliberately absent semantics

The current record schema does not contain authoritative workspace, title,
preview, language, creation time, update time, or display-order fields, and the
store has no authoritative session index. This slice does not derive those
values from filesystem modification times, message contents, metadata maps,
live registry state, or directory order.

Consequently the delivered library slice adds no rich summaries, workspace
filter, newest or latest selection, ordering other than lexical ID order,
cursor, pagination, session-ID generation, deletion, cleanup, or slash command.
In-progress bounded slice 31 now consumes this result through strict top-level
`sessions [--json]`, without adding any of those absent semantics. Its separate
[`CLI contract`](sessions-cli.md) uses a no-create, engine-free native process
facade and preserves the exact scan bounds and per-record lock-sidecar effect.
It does not implement fx's richer `sessions` behavior and makes no compatibility
or upstream-equivalence claim.

The in-progress CLI changes the `sessions-json` performance workload only to
implemented/non-equivalent/not-measured/claim-ineligible. It adds no samples,
threshold, workflow, product-performance result, or compatibility promotion.
Zig remains only the pinned toolchain used to build the upstream fx benchmark
reference; machine-god remains a Rust product.

The combined Milestone 03 root plus native create/list/resume/replay/reset
library boundary is delivered: replacement regressions, all three formal
rereviews, exact feature workflows, fast-forward integration, and exact `main`
workflows are green. Milestone 03 remains in progress for the remaining native
tools, top-level CLI and
slash-command ownership, and composed freshly built release-binary end-to-end
evidence.
