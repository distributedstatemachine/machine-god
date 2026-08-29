# Native `semantic_search` tool

This document defines the durable contract for machine-god's bounded
`semantic_search` tool. The name and keyword-ranking behavior are compatibility
inputs from the pinned fx revision. The implementation is a local lexical
concept-keyword search: it uses no embeddings, model call, network request,
vector database, persistent index, cache, watcher, Git process, or shell
command.

## Provider input and authority

The advertised schema and preflight accept exactly one required `query` string
and one optional `path` string:

```json
{"query":"session recovery state","path":"crates"}
```

Unknown fields, missing `query`, explicit `null`, and wrong field types are
invalid. `path` omission defaults to `.`. The requested query is bounded to
4,096 UTF-8 bytes and must contain a byte other than ASCII space or tab.
Preparation otherwise preserves the query bytes: it performs no Unicode
normalization, stemming, synonym expansion, or case rewriting. NUL, other C0
or C1 controls, unsafe formatting controls, and an overlong query are
`semantic_search_invalid_query`. Tab remains valid because it is one of the
pinned keyword separators.

Requested and normalized paths are independently bounded to 4,096 UTF-8 bytes.
Path normalization collapses repeated `/`, removes exact `.` components, joins
ordinary components, and normalizes current-directory spellings to `.`. It
rejects an empty present path, an absolute path, any `..` component, C0 or C1
controls, Unicode line or paragraph separators, and Unicode bidirectional-
formatting controls. Backslash is an ordinary Unix filename character, not a
separator or escape.

Preflight is deterministic, synchronous, bounded, nonblocking, and effect-free.
It strictly decodes and normalizes the arguments and returns:

- `Capability::Filesystem` with `FilesystemAccess::SearchContent` at the exact
  normalized selected path; and
- canonical execution arguments containing the preserved query and normalized
  explicit path.

`SearchContent` authorizes bounded entry-name observation and regular-file
content inspection at that one path: the selected object when it is a regular
file, or eligible regular files beneath it when it is a directory. It does not
imply `Read`, `Metadata`, `Enumerate`, `EnumerateRecursive`, mutation,
external-path access, or symlink-target access. Preparation opens no
descriptor, reads no entry or content, and consults no process state. Execution
extracts the bounded keyword list before it reacquires the retained workspace
root, so a stopword-only query performs no filesystem operation after
permission succeeds.

## Keyword extraction

Keyword extraction scans the preserved query from left to right. Exactly these
single ASCII bytes split tokens: space, tab, comma, period, semicolon, colon,
question mark, and exclamation mark. New token boundaries are not inferred
from other ASCII or Unicode punctuation or whitespace.

Tokens shorter than two bytes are discarded. The following stop words are
discarded with ASCII-case-insensitive comparison:

```text
a an the is are was were in on at to for of and or not it this that with from
by as do does how what where when why which
```

The first sixteen remaining tokens are retained in encounter order. Tokens are
not deduplicated, stemmed, normalized, translated, or reordered; repeated
tokens therefore remain repeated scoring inputs. Once sixteen have been
retained, later query bytes do not add keywords. A syntactically valid query
that contains only splitters, short tokens, or stop words succeeds with empty
`keywords` and `results`, all counters zero, `incomplete: false`, and no
filesystem effect during execution.

## Workspace confinement and traversal

The host supplies one explicit absolute workspace root. On Linux and macOS,
construction opens its final component as a no-follow directory and retains
that descriptor as the tool's only filesystem authority. Other targets expose
the fixed redacted unsupported-platform failure and perform no workspace
access.

After permission, execution reparses the canonical arguments and reacquires the
retained root identity. It resolves every selected component descriptor-
relatively without following symbolic links. The selected object must be a
regular file or directory. A selected symlink, intermediate symlink, special
object, path escape, or mismatched revalidation fails closed. Replacing a
previously opened ancestor cannot redirect later lookups outside the retained
identity.

Directory traversal is iterative and deterministic. A directory is admitted
atomically: execution stages no more than the global remaining non-dot entry
budget plus one overflow witness. Exact EOF commits the whole staged batch to
`visited_entries`; its names are then validated, its entries are classified,
and the batch is ordered by raw path bytes before processing and descent. If
the extra witness exists, execution charges all remaining visit slots, charges
the bounded raw-name bytes including the witness, discards the entire staged
batch without UTF-8 decoding, metadata inspection, file processing, or
descent, marks `traversal_cap`, and stops the traversal. It never selects a
kernel-`readdir`-order subset of an overflowing directory. An empty directory
still probes EOF when no visit slots remain, so reaching exactly 2,000 entries
does not by itself make the result incomplete.

Directory input has a separate global 12,288-attempt ceiling. On Linux, an
8 KiB safe `RawDir` buffer exposes and charges every `getdents64` refill before
the call, including exact EOF and an interrupted refill that will be retried.
On macOS, the libc directory stream does not expose buffer boundaries, so the
bounded conservative equivalent charges every `readdir` call; an interrupted
macOS stream is a read failure because that stream becomes terminal and cannot
be retried as though its next `None` were exact EOF. At the 2,000-entry visit
ceiling, even the most call-heavy normal macOS shape requires at most 8,003
charged calls: 2,000 admitted entries, two dot records for each of at most
2,001 directories, and an EOF probe for each directory
(`2,000 + 2×2,001 + 2,001`). The remaining 4,285 attempts admit bounded
incidental Linux interruptions without permitting an interrupt loop. Attempt
exhaustion is the nonretryable `semantic_search_scan_limit` hard failure and
returns no partial result.

Regular files are candidates. Symbolic-link entries are counted and skipped;
they are never opened, resolved, scored, or descended through. FIFOs, sockets,
devices, and other special objects are not content candidates. Hidden entries
are otherwise eligible.

During recursive traversal, directories with any of these exact, case-sensitive
basenames are not descended:

```text
.git .zig-cache zig-out node_modules .next dist build coverage target
```

The ignored set is fixed; `.gitignore`, other ignore files, repository state,
and environment configuration cannot change it. In particular, Rust's
`target` directory is excluded without running Git or Cargo.

The selected root is depth zero and descendant directory depth is capped at
256. One call visits at most 2,000 non-dot entries. Each raw entry name and each
constructed selected/result path is capped at 4,096 bytes; aggregate raw entry-
name bytes are capped at 8,388,608. An invalid UTF-8 or forbidden entry name
fails with the fixed invalid-entry-name category rather than being lossily
decoded. A structural path, name, or depth violation never produces an
unbounded allocation or a path-derived diagnostic.

## File admission and text safety

Only descriptor-verified regular files are read. One file may contribute at
most 102,400 bytes plus a single transient overflow witness. A larger file is
not partially scored: it increments `skipped_oversized_files`. Across the call,
at most 67,108,864 file bytes are admitted for text classification and scoring,
including bounded bytes from a file later classified as non-text.

Every content-read attempt is charged before the operating-system call. The
global 12,288-attempt ceiling includes successful full or short reads, EOF
probes, aggregate-overflow witnesses, and interrupted reads that will be
retried. The existing 64 MiB aggregate and 2,000-candidate ceilings require at
most 12,191 attempts when each ordinary regular-file call returns all bytes
currently available up to the request: at most 10,191 data reads after
per-file chunk-boundary fragmentation plus one EOF probe per candidate. The
remaining fixed allowance tolerates incidental interruptions without
permitting an interrupt loop or one-byte source to make unbounded calls.
Attempt exhaustion is the nonretryable
`semantic_search_scan_limit` hard failure and returns no partial result.

Eligible content must be valid UTF-8 and contain no NUL byte. Invalid UTF-8 and
NUL-containing files increment `skipped_non_text_files` and contribute no
score. No lossy decoding, binary-to-text conversion, MIME or extension filter,
secret scan, or content rewriting runs. Successful line excerpts are
intentionally authorized workspace content and may enter the durable tool
result and observer events.

Files are read through their retained descriptors. Descriptor type and initial
size are verified before the bounded read, and the read admits one overflow
witness, so a symlink or special-object race cannot redirect content access or
silently bypass the file-size bound. A concurrent rename of the opened file
does not redirect its descriptor; a disappearance, growth, replacement, or
inconsistent observation may instead select a fixed redacted unavailable,
path-rejected, or read-failed category. The operation is not a multi-file
snapshot.

## Scoring and ordering

Matching is an ASCII-case-insensitive substring test. Non-ASCII bytes compare
exactly; there is no Unicode case folding. Each retained keyword contributes
at most one point to a given line when present anywhere in that line. The file
score is the checked sum of those per-line keyword-presence counts across every
line. A keyword present in the file's basename adds three more points,
independently of its line matches. Duplicate retained keywords are scored as
separate entries.

Lines are numbered from one and split only on LF; LF is excluded while a
preceding CR remains content. The sample is the first line whose line score is
strictly greater than every earlier line score. Equal later lines do not
replace it. A filename-only match has line number zero and an empty sample.
The terminal split segment is always dispatched, including the empty segment
of an empty file or content ending in LF, matching the pinned scalar-split
behavior. It cannot change a score when empty, but it remains charged matcher
work. Only files with a positive total score are matching results.

Matching work is charged across the call and uses checked arithmetic. At most
67,108,864 work steps are admitted. One step is charged before every
line-keyword and basename-keyword matcher dispatch, including an empty line or
basename, and one step is charged before every ASCII-folded KMP byte
comparison in keyword compilation and content matching. Thus newline-heavy
content cannot create unmetered fixed dispatch work, while the ceiling remains
a fixed synchronous-work bound independent of the 64 MiB content-admission
ceiling. Exhaustion cannot wrap a counter or start an unmetered fallback; it is
the nonretryable `semantic_search_scan_limit` hard failure and returns no
partial result.

Results are ordered by descending score, then by ascending raw UTF-8 path
bytes. A worst-first bounded heap retains the globally best 200 matching files
in `O(m log 200)` selection work for `m` matching files; later matches never
trigger a linear scan of all retained records. Replacement updates aggregate
accounting exactly, retaining at most 819,200 aggregate path bytes and 400,000
aggregate sample-line bytes. At most the first 100 ranked results are shown. A
shown line is the longest valid UTF-8 prefix no longer than 2,000 bytes, and
`line_truncated` reports whether bytes were omitted.

## Structured result and incomplete scans

Success returns structured content with this shape:

```json
{
  "query": "session recovery state",
  "path": "crates",
  "keywords": ["session", "recovery", "state"],
  "results": [
    {
      "path": "crates/machine-god-native/src/session_store.rs",
      "score": 8,
      "line_number": 42,
      "line": "structural session recovery state",
      "line_truncated": false
    }
  ],
  "visited_entries": 120,
  "candidate_files": 70,
  "searched_files": 60,
  "skipped_oversized_files": 2,
  "skipped_non_text_files": 3,
  "skipped_symlink_entries": 5,
  "matching_files": 7,
  "incomplete": false,
  "incomplete_reasons": []
}
```

`query` is the preserved prepared query and `path` is the normalized selected
path. `visited_entries` counts atomically admitted non-dot entries plus visit
slots conservatively charged to one overflowing directory; its overflow
witness is not an additional visit. `candidate_files` counts bounded regular-
file candidates considered for admission; an atomically discarded directory
contributes none. `searched_files` counts admitted model-safe text files that
were scored. Skip counters identify deliberately excluded oversized, non-text,
and symlink entries. `matching_files` counts positive-score files observed by
the bounded scan, including a ranked match omitted from `results` by a later
cap.

The serialized complete `ToolOutput` is capped at 49,152 bytes. Result
selection and serialization never truncate JSON or split UTF-8. If every
otherwise showable result does not fit, a ranked prefix is emitted and the
result reports the omission. Prefix fitting is monotonic and uses binary
search over the at-most-100-result display set, including JSON escaping and the
complete `ToolOutput` envelope in every size decision; it does not repeatedly
remove and reserialize one result at a time.

`incomplete_reasons` contains only these stable values, in this order when
more than one applies:

1. `traversal_cap` — a directory had more non-dot entries than the remaining
   part of the pinned-compatible 2,000-entry visit ceiling, so that whole
   directory was discarded and traversal stopped before every otherwise
   eligible candidate was searched;
2. `result_cap` — more than 200 matching files existed, so only the best 200
   were retained; and
3. `output_cap` — one or more retained results were omitted by the 100-result
   display ceiling or the complete serialized-output ceiling.

`incomplete` is exactly whether that array is nonempty. Ordinary skips for an
oversized file, non-text file, ignored directory, symlink, or special object do
not by themselves make the scan incomplete; their fixed eligibility rules and
counters make those exclusions explicit. A stopword-only query has no
filesystem observations and therefore reports zero statistics.

Aggregate content or name bytes, directory-read attempts, content-read
attempts, keyword-work steps, path or depth bounds, and checked counter
overflow are hard scan-limit failures. They return no partial result and are
never represented by `traversal_cap`. Raw-name accounting includes the one
bounded overflow witness even though that witness does not increment
`visited_entries`. The partial traversal reason is reserved solely for atomic
directory rejection at the 2,000-entry ceiling.

## Errors, lifecycle, and cancellation

Construction distinguishes unsupported platform, invalid root, non-directory
root, and unavailable root. The error retains only that fixed category.
Preparation and direct execution use the following fixed redacted codes:

| Code | Meaning | Retryable |
| --- | --- | --- |
| `semantic_search_invalid_arguments` | The strict object shape or field type is invalid. | no |
| `semantic_search_invalid_query` | The query violates its text or byte contract. | no |
| `semantic_search_invalid_path` | The selected path violates lexical normalization. | no |
| `semantic_search_unsupported_platform` | Hardened native execution is unavailable. | no |
| `semantic_search_not_found` | The selected file or directory is absent. | no |
| `semantic_search_permission_denied` | The selected search root cannot be searched. | no |
| `semantic_search_path_rejected` | The path is not a confined regular file or directory. | no |
| `semantic_search_unavailable` | The bounded native search is temporarily unavailable. | yes |
| `semantic_search_read_failed` | The bounded content search could not be completed. | yes |
| `semantic_search_invalid_entry_name` | A traversed entry name is unsupported. | no |
| `semantic_search_scan_limit` | A hard scan bound was exceeded without a safe partial result. | no |
| `semantic_search_cancelled` | Execution was cancelled. | no |

Error `Display`, `Debug`, code, message, and nested state contain no workspace
root, selected path, query, keyword, entry name, file bytes, sample line,
operating-system text, or raw error number. Successful paths, keywords, lines,
and counts are authorized model-visible content, not error diagnostics. Core's
ordinary generic durable error mapping remains unchanged.

Creating an execution future is inert. The first poll begins bounded
synchronous native work. Cancellation is checked before root acquisition,
around selected-component and descendant opens, between entry reads, before
and after each bounded file read, at fixed intervals through keyword matching,
before result reconstruction, and immediately before publication. One
operating-system open, metadata, directory-read, or file-read call already in
flight cannot be preempted; cancellation is cooperative when that call returns.

If descriptor-relative reacquisition of the retained workspace root fails
with operating-system access or permission denial, execution returns the
nonretryable `semantic_search_permission_denied` category. Other reacquisition
failures remain retryable `semantic_search_unavailable`; neither mapping
includes a raw path, error number, or operating-system diagnostic.
On macOS this taxonomy also covers every retained-root linkedness-validation
boundary: descriptor metadata, `getpath`, parent reacquisition, and parent
entry metadata.

No task, thread, process, timer, producer, cache, or indexer is detached.
Dropping an unpolled future performs no filesystem effect. Dropping a polled
future closes owned per-call descriptors, releases buffers and retained
results, and publishes no partial tool result.

## Pinned compatibility and deliberate differences

Pinned fx supplies the field names, splitter and stop-word table, sixteen-token
selection, line-presence scoring, basename bonus, first-best sample, ranking,
and bounded result concept. Those are compatibility inputs, not evidence of
complete observable equivalence.

Machine-god deliberately uses strict decoding, explicit `SearchContent`
authority, retained descriptor-relative traversal, no selected symlink
following, a fixed ignored-directory set that includes Rust `target`, explicit
byte/work/output ceilings, structured counters, and fixed redacted failures.
Pinned fx may resolve and read a symlink whose target remains inside its allowed
scope; machine-god skips or rejects that link instead. This authority
divergence is intentional.

This contract does not add embeddings, fuzzy or vector similarity, query
expansion, language-aware tokenization, Unicode case folding, indexing,
watchers, ignore-file interpretation, repository discovery, external paths,
non-Linux/macOS hardened traversal, a CLI command, or a product-performance or
fx-equivalence claim.
