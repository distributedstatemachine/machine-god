# Top-level sessions command

The thin CLI presents the native rich session catalog without constructing an
engine, provider, permission handler, tool registry, or network runtime.

## Grammar and exits

```text
machine-god sessions [--all] [--limit <1-100>] [--cursor <cursor>] [--json]
```

Flags can occur in any order, once each. The default limit is 100. Limits use
unsigned decimal digits; zero, values above 100, overflow, missing values,
unknown/repeated flags, positional extras, and non-Unicode arguments are
invalid. Parsing finishes before any host/environment/filesystem effects.
Invalid syntax writes global usage to standard error, leaves standard output
empty, and exits 2.

Default scope is the canonical current working directory's exact native
workspace association. `--all` lists the entire selected native namespace,
including records whose workspace is unknown. Neither scope infers workspace
ownership from titles, previews, filenames, or environment text.

Complete, paginated, skipped-invalid, and resource-incomplete observations exit
0 with empty standard error. Operational/render failures exit 1. Human errors
use standard error; JSON errors use standard output. The closed categories are
`Corrupt`, `Unavailable`, `Unsupported`, and `ResourceLimit`. Error text never
includes record contents, filenames, paths, environment values, or OS details:

```text
machine-god sessions: could not list sessions: <Category>
```

```json
{"kind":"sessions","error":"could not list sessions: <Category>","code":"<Category>"}
```

Output failure reports the existing fixed
`machine-god: failed to write output` diagnostic and exits 1.

## Rich output

Rows retain native newest-first ordering: descending known update time,
descending ID for ties, then unknown times with descending ID. Human output
contains a title followed by ID, actual user-message turn count, optional
language label, and UTC update time. Missing, negative, or out-of-calendar-range
human timestamps render as `unknown`; JSON preserves known signed timestamps.
The title fallback `Untitled session` is presentation only and is not persisted.

```text
[sessions] 1 saved
 - Investigate rendering
   id=alpha | 1 turn | English | updated 2026-01-01 00:00:00.000 UTC
```

Complete empty output is exactly `[sessions] no saved sessions\n`.
JSON uses the pinned rich row fields in this order:

```json
{"kind":"sessions","count":1,"sessions":[{"id":"alpha","title":"Untitled session","preview":null,"workspace_root":null,"origin_workspace_root":null,"created_at_ms":null,"updated_at_ms":null,"history_len":0,"conversation_language":null}]}
```

Unknown metadata is null. `history_len` counts actual canonical user messages,
not all transcript messages. Preview is the bounded native excerpt, not trusted
user authority or a generated title. `origin_workspace_root` is the separately
recorded creation workspace, preserved across later workspace rebindings; it is
not the native provenance enum. Absent historical origin remains null even when
the current association is known. A non-UTF-8 workspace has its corresponding
text field null and an additional final `workspace_root_hex` or
`origin_workspace_root_hex` field with exact lowercase native-byte hex; no
replacement-character path is invented. Current-workspace hex precedes origin
hex when both are present.
Workspace filtering still compares exact native bytes.

Both formats escape terminal controls, C1 characters, bidi controls, and line
separators. JSON decoding preserves the original strings. Human strings also
escape quotes and backslashes. Each representation ends with one LF.

The complete success output is bounded and constructed before any write:
6,988,096 bytes including LF. This allows 100 rows with maximum-length IDs
(128 bytes), titles/previews (240 each), two workspace paths (4096 each), language
tags (24), up to six output bytes per input byte, optional 8192-byte hex per path,
fixed fields, numbers, and page diagnostics. Invalid host projections fail
closed as `ResourceLimit`, without intentionally emitting partial success.

## Pagination and damaged records

Cursors are native checked values, not filenames:
`v1:<signed-update-ms>:<id>` or `v1:unknown:<id>`. Timestamps use canonical
decimal representation; only the first two colons delimit components because
native IDs may contain colons. Raw cursors are bounded to 320 bytes.

A fully scanned page with an additional eligible result includes
`has_more:true` and `next_cursor` before `sessions`. Human output supplies a
`machine-god sessions` continuation command preserving `--all` and the actual
`--limit`. A cursor is an ordering boundary, not a snapshot or frozen index;
concurrent changes can affect later observations.

The native `SkipAndReport` policy retains healthy rows and counts unreadable
records without exposing their names. Positive `skipped_invalid` appears
before pagination fields in JSON; human output adds a count-only warning and
`machine-god doctor` hint. Complete scans with no readable rows and skipped records use
`[sessions] no readable saved sessions`, not the complete-empty sentence.

A resource-incomplete scan never promises a continuation. JSON includes
`scan_complete:false,truncated:true`; human output adds
`[sessions] listing incomplete: a resource limit was reached`. An empty
incomplete scan uses a counted zero-row header. Complete empty JSON with no
invalid records remains exactly:

```json
{"kind":"sessions","count":0,"sessions":[]}
```

## Native effects and platforms

Linux/macOS use the [native catalog](native-session-listing.md) facade. Default
scope canonicalizes `.` in native code; `--all` does not require a workspace.
State selection uses nonempty `XDG_STATE_HOME`, otherwise `HOME/.local/state`,
with the fixed `machine-god` namespace and existing native root safety rules.
Missing state roots produce empty observations; unsafe/inaccessible roots fail.
No config, credential, model, or permission state is loaded.

Future construction is inert. Canonicalization, environment selection,
descriptor operations, bounded scans, reads, and advisory locks execute
synchronously on first poll, without detached tasks. An injected pending
future is polled once, dropped, and reported as `Unavailable`. Unsupported
targets fail with the fixed `Unsupported` category without filesystem work.

Native scan limits remain 1,024 processed directory entries plus an overflow
witness, 64 MiB aggregate record bytes plus a byte witness, and per-record
bounds. Listing does not create state roots or rewrite/delete/migrate records.
Observing an existing record can create its missing permanent private advisory
lock sidecar, as documented by the native store. No product-performance claim
follows from this command.
