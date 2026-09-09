# Native session listing

Native session listing adds `NativeSessionLifecycle::list_sessions` on supported
Linux and macOS targets. It returns an owned `NativeSessionList` containing
only validated session IDs and a `truncated` flag. The operation is a bounded
observation of the existing flat file-session root; it is not a search index,
registry, pagination protocol, or multi-record snapshot.

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

## ID-only consumers

The original ID-only listing does not interpret native metadata or add rich
summaries, workspace filters, newest selection, cursor, pagination,
session-ID generation, deletion, cleanup, or slash commands. It remains a
separate fail-fast library contract. The top-level command's richer native
catalog consumption and presentation are specified in the
[`CLI contract`](sessions-cli.md); they do not change this legacy API.

## Rich native catalog

`NativeSessionCatalog::new(Arc<FileSessionStore>)` retains an explicitly supplied
store without I/O. Its `list(query)` and `exact(id)` futures are inert until
polled, then perform synchronous native I/O and advisory locking in the polling
context. These Linux/macOS APIs require no HTTP feature, engine, provider,
configuration, clock, workspace access, or live-session registry.

Each `NativeSessionCatalogEntry` owns the validated ID, incarnation, revision,
message count, and authoritative `NativeSessionMetadata` snapshot described in
[native session lifecycle](native-session-lifecycle.md). Missing historical
native metadata stays unknown. The catalog does not infer title, workspace,
origin, or timestamps from transcripts, filesystem modification times, current
working directory, or ID spelling. By default, invalid reserved native metadata
in any reached record fails the entire call with fixed `Corrupt`, even if the
record would not match the query or fit the displayed results. The explicit
reporting mode below counts and omits these records instead. Generic metadata is not
promoted into authoritative native facts.

The separately named `preview` is presentation data, not a title or metadata
fact. It selects the first nonempty canonical user text block that is not a
single-line slash command. It contains at most two nonempty trimmed lines and
240 UTF-8 bytes, with `preview_truncated` reporting omitted text. Assistant,
tool, and arbitrary JSON blocks do not supply a preview. Unknown title remains
unknown even when a preview exists. Preview and metadata contents remain
untrusted display data; terminal consumers must escape controls. Entry `Debug`
omits IDs, metadata contents, and previews; getters deliberately expose them to
the trusted caller.

`NativeSessionCatalogQuery` accepts a presentation limit from 1 through 100
(default 100), an optional exact normalized absolute Unix workspace spelling
(at most 4096 bytes), an optional inclusive update-time lower bound, and search
text bounded to 1024 raw UTF-8 bytes before copying. Unknown workspace or update
time never matches an explicit corresponding predicate. Search trims outer
space/tab/CR/LF and matches ASCII-case-insensitive substrings of known title,
known workspace spelling, or bounded preview; it does not search IDs, generated
unknown-value labels, omitted preview text, or the full transcript.

The rich scanner shares the ID-only scanner's descriptor-relative no-follow
access, acquired-root identity checks, canonical naming, permanent per-ID
locks, strict record decoder, and corruption rules. It uses the same 1,024
non-dot entry and 64 MiB accepted-record aggregate bounds, including the same
single overflow witnesses. Each record still has its existing per-record and
structural limits. Unlike ID-only listing, the presentation limit does not stop
validation: all canonical candidates reached within those scan bounds are
decoded once and projected before filtering and ranking. At most 100 selected
metadata/preview snapshots are retained, plus one candidate projection and one
transient bounded decoded record. Whole transcripts are not retained in the
page, and there is no second deserialize pass or persistent index.

Rows sort by known update time descending, unknown update times last, then ID
descending for ties. `scan_complete` reports whether directory/aggregate bounds
allowed the full observation; `results_truncated` separately reports that more
matching rows were observed than the display limit. The page also exposes
matched count, matching unknown-activity count, scanned record count, and
accepted record bytes. `history_len()` counts canonical user-message groups,
excluding assistant/tool continuation rounds; `message_count()` still counts
all canonical messages. `latest()` refuses `ScanIncomplete` or `UnknownActivity`
when either prevents an authoritative ordering of eligible observed sessions,
including unknown activity on a matching row omitted from the display. A
complete scan with only result truncation can select its newest matching row.
This remains a set of individually locked observations, not an atomic snapshot
or a promise that concurrent writers cannot subsequently change the newest row.

### Continuation and invalid-record reporting

`NativeSessionCatalogCursor` is a portable value on all platforms, independent
of native filesystem support. `new(updated_at_ms, id)` and `parse(text)` retain
an optional authoritative timestamp and validated native ID. Display uses
canonical `v1:<signed-i64>:<id>` for known time, matching the pinned ordinary-ID
cursor spelling; `v1:unknown:<id>` explicitly represents unknown historical
activity. Parsing bounds raw input to 320 bytes before copying and rejects
noncanonical integers (including leading plus, leading zeros, and negative
zero), invalid IDs, or other versions with a fixed redacted error. Native IDs
permit colons, so the entire suffix after the first two separators belongs to
the ID. This is an unambiguous native ID-domain extension, not a restriction of
the existing native identifier alphabet. Cursor `Debug` is redacted; `Display`
deliberately exposes its continuation value.

`query.with_continuation(cursor)` selects rows strictly after that boundary in
the descending optional-time/ID ordering, reapplying workspace, time, and search
predicates during a new bounded scan. The cursor grants no authority and does
not freeze the query or store. Concurrent updates can change later page
membership. `page.next_cursor()` returns the last displayed row's boundary only
when the scan is complete and additional matching valid rows were observed
beyond the display limit. Empty/final pages and incomplete scans return `None`;
there is no fabricated cursor claiming unseen candidates can be exhausted.

`query.with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport)`
opts into omitting candidate-local `Corrupt` outcomes and exposing only their
count through `page.skipped_invalid()`. The default `Fail` and ID-only API retain
their fail-fast behavior. Malformed records, invalid native metadata, wrong
filename/ID identity, oversized files, nonregular or symlink candidates, and
hostile derived lock entries retain their existing corruption classification.
Reporting applies before query filtering and counts each reached invalid
candidate once, without returning its ID, path, content, or raw error. Global
root/enumeration failures and ordinary unavailable I/O still fail the entire
call. Concurrent disappearance remains an omission, not corruption.

All bytes actually read from a rejected candidate consume the existing 64 MiB
aggregate budget before scanning can continue. Metadata-only rejections such
as a stat-proven oversized file read no content and charge no content bytes.
Per-file growth witnesses are charged when continuing; at most one final
aggregate overflow witness can be transferred without acceptance into the
budget. Presentation, raw-entry, per-file, and structural bounds are unchanged.
`scanned_records()` counts successfully validated/projected records, while
`scanned_record_bytes()` includes accepted bytes of rejected records as well.
No repair, quarantine, deletion, or source rewrite is performed. Skipped
corruption does not itself make the enumeration incomplete, so valid rows can
still be paged with an explicit skipped count; however, `latest()` refuses
`SkippedInvalid` whenever any candidate was skipped, including candidates whose
query eligibility cannot be established. Scan incompleteness takes precedence.

`exact(id)` loads only the requested canonical record through the retained
store and returns its projection, independent of enumeration, ranking, or
listing truncation. An absent ID returns `None` without creating a lock.
`list_native_session_catalog` and `inspect_native_session_catalog_entry` accept
an explicit `NativeEnvironment`; their process variants capture only the
existing state-root environment selection on first poll. All four use the
same no-create root facade as ID-only listing: a missing hierarchy returns an
empty page or `None`, while unsafe roots and malformed environment selection
are fixed errors. Successful reads of existing records may create missing
private lock sidecars, but no operation creates a state hierarchy or repairs,
migrates, rewrites, or deletes records.

`list_process_current_workspace_session_catalog(query)` additionally resolves
`.` to its canonical spelling on first poll and applies an exact workspace
filter before the same state scan. Resolution or unusable workspace spelling
returns fixed `Unavailable`; construction and dropping an unpolled future do
not access CWD. This explicitly scoped facade does not infer associations for
unknown historical records. The existing all-workspaces process facade performs
no CWD lookup, including when the process's former directory was removed.

The ordering and bounded preview/search choices follow the pinned upstream
`session_summary_codec.zig` newest comparator, `session_catalog.zig` query
matching, and `session_display_metadata.zig` preview bounds. Native stored
metadata remains the authority instead of adopting upstream title derivation
or unknown-value display labels as facts. The catalog itself adds no CLI
parsing, interactive picker, resume admission, or compatibility promotion.

The `sessions-json` comparison remains non-equivalent, not measured, and
claim-ineligible. This contract establishes no samples, thresholds, product-
performance result, compatibility promotion, or upstream-equivalence claim.

## Owned picker catalog reads

`NativeReferenceHost::session_catalog_reader()` constructs an inert
`NativeSessionCatalogReader` over that host's exact `Arc<FileSessionStore>`,
canonical workspace spelling, and existing owned-worker completion scope.
Legacy hosts without that scope return fixed `Unavailable`; no replacement
scope, state root, working directory, or environment is discovered. Reader
clones share one scan admission and retain no engine or host-lifetime vote.

`reader.list(scope, limit, continuation, cancel)` accepts `CurrentWorkspace` or
`All`, a pure 1–100 presentation limit, and the existing optional catalog cursor.
It constructs the query itself: only current-workspace scope adds the exact
workspace predicate, and neither scope adds search. Picker search operates on
already loaded pages. Both scopes use `SkipAndReport`, preserving explicit
invalid-record counts, ordering, unknown metadata, byte/entry limits and honest
continuation semantics from the ordinary catalog. No second scanner or stored
index exists.

First poll validates the query, checks cancellation, and reserves one scan
before submitting it to the actual host worker scope. The same scanner checks
cancellation before directory iteration, each record and lock, each 8,192-byte
read (including interrupted-read retries), and before/after bounded decoding
and projection. Picker record locks use nonblocking exclusive acquisition;
contention returns fixed `Busy`, not skipped corruption or a successful partial
page. Cancellation returns fixed `Cancelled` with no page. Other catalog errors
retain their fixed categories. Ordinary catalog and ID-only listing keep their
existing synchronous blocking-lock behavior.

Dropping an unpolled request is effect-free. Dropping a polled response requests
private cancellation without cancelling the caller's token or closing the host
scope. The actual worker owns the store, query, scan and admission until scan
cleanup returns, even after response abandonment. Response readiness is not
full worker/thread-local cleanup completion; the host's existing completion
handle remains authoritative. No filesystem or serde operation is promised a
hard wall-clock deadline. Reads can create the existing permanent private lock
sidecars but never modify session records.
