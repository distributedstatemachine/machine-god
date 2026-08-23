# Native `grep_files` tool

Status: **DELIVERED — formal fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is green with zero findings
across correctness/API, filesystem/robustness, and performance/concurrency;
documentation seal `0f48806` is exact-feature-green and fast-forwarded without
force to `main`, where exact CI and benchmark-evidence workflows are green**
nineteenth bounded Milestone 03 slice.
The exact base is `f6aa458bb875d6cb26565adc878703fe140916d3`.
The tree-identical integration kickoff is
`f6ab594c928bead48b48ab080ac12a7ce9c0d3f4`. Production, independent tests,
and this maintained documentation are owned in parallel, non-overlapping
worktrees. Production component
`27eec2f3c25ffecd1ba8ff3c0a4fe0129dbeeac3` and initial independent-test
component `6eaee93398de8fbf6e87e77cf4d3e7de56e2a8cb` exist. They compose initially
through `9057feb24fd3f24657148ca8e78198b88c9dbab4` and
`44e33d7e24c6650a1e375cd095eb9efae31f4e78`, respectively. Reference-host
fixture fix `bdbb677161322e249aea95a12bfb1b2169ff5b48` makes focused production-
and-test composition green. Documentation component
`b04151a7d958875118eebddd67526d74e2ea9526` produces first fully composed
behavior candidate `42e4793b27902da7390dc54ef6bedb169da7e1bc`. Lint fix and
exact precursor local gates are green at
`45ad91fa2689250c47c79d2105f5e3c261cea638`. All three first-cycle formal
reviews are **NOT GREEN** on exact candidate
`355a11a6055b0053dff80e71011d7633e8a6ce97`. Isolated production, independent-
test, documentation, deterministic-evidence, and unsupported-target-test
remediation commits `012f14d273b15085713fba9092e93486d4e6f0e4`,
`646286203ab665e9dc9d0a86f7de6d036b7c5c86`,
`771a3e34816d2d67cd1e08d73abdac7c807313a3`,
`842d36ae5f7cc8aa5a3011a41bba209e0f35172c`, and
`325350ce558a7e6f21ef4cf2d4d030e30cc4f740` compose in order through
`35defb5c7cee021064411535070b9ecd62387e2f`,
`58550734b77a5a44c4b9452438e34f265013c40b`,
`3cd282fafb26bb069ac73407fde0fd30c7d1ff82`,
`630acbb384f2e1b79b6916a10baaac26acafbf41`, and final code/test local-gate
precursor `275d263dd3c7981e66f6a0f90f3779c271eb4cc3`. Its replacement local
gates are green. First replacement formal candidate
`ae87bf1454b1527b2e55ed5e517c21fd7410c980` is **NOT GREEN**: correctness found
one low literal-compilation ordering mismatch; filesystem review found one low
deep-FIFO evidence overclaim; and performance review found one medium slashful
candidate/false-DP cancellation gap plus one low unmetered slashful selected-
file decision. Second production remediation
`ac5d7726411744e4f85344edf966d26a3cdb0a26` composes at
`d67221021aa173299f8f2e99d2574a15870cd5c8`; second documentation remediation
`7ad0863885d28b7b7a1d6f89d35f525cdd2dd3fa` produces fully composed exact
local-gate precursor `b498ba06fa808dc9453a7644727cf8166b6f8e87`.
`ae87bf1` remains historically **NOT GREEN**. Formal second replacement
candidate `5aeddc1b4cb210b00cb967b938db8d5232062916` has correctness/API and
filesystem/robustness **GREEN** with zero findings. Performance/concurrency is
**NOT GREEN**: repeated eligible files can each allocate a 204,801-byte content
buffer; 10,000 empty files accumulated approximately 2,048,010,000 allocated
bytes, with an observed 6.10-second diagnostic run that is not a contractual
timing result. Its two low findings correct the Markdown inventory from 57 to
58 files and reject the claim that deterministic evidence already exercises
the recursive dynamic-programming branch. Third production remediation
`8777825b1b8b8c97dd4eb4bb31c0d8dbed9a7741` composes at
`ab1c13385a475ac34e8df2180e8c4cbb3b0ee3e9`; independent regression
`dcf57ad35150b86c84a3f6c1127d9e379f3840fc` composes at
`d7526d4dcd7f41be1b8d4c95d640da061088517c`; review-findings documentation
`44afb232f2b8418c0b61eec7d1dab46bbe8e3667` composes at
`f08c5f2e35befb5e533ef4bb80a4b342dc5ffa46`; lint follow-up
`1f13f9ae04ee3307d13a363ed28b156d7ee2421f` produces exact fully composed
local-gate precursor `a8f61794ee5e279558856220b5789526b908015a`.
Exact Rust 1.94.1 formatting, warnings-denied workspace Clippy, 598 non-
documentation tests plus two doctests, 25 private native tests, 40 direct
`grep_files` tests, four engine tests, and diff checks are green. Exact a8f
cross-target/dependency/link and compatibility/release validators are green.
Formal third-cycle candidate
`0bfe68a9692837187c057b5b4efa08ebe3dee058` has filesystem/robustness
**GREEN** with zero findings. Correctness/API and performance/concurrency are
**NOT GREEN** only for the same LOW documentation contract mismatch; reviewers
confirmed zero production defects. In diagnostic allocator instrumentation,
two 10,000-file boundary scans at `5aeddc1` requested approximately
4,103,462,456 bytes and made 20,000 allocations of exactly 204,801 bytes;
`0bfe68a` requested approximately 7,459,007 bytes and made zero maximum-sized
allocations. Its maximum-plus-384 regression requested approximately 3,349,064
bytes and made one high-water allocation. Allocation and timing instrumentation
is diagnostic only, not a contract or product-performance result. Isolated
wording remediation `993b618bf78d30f6a68f3b248b572e33e4de1126` composes at
exact `f87f6bef4016aa4ce3cd49e2c795d15bff3e84f4`. Exact f87 documentation
gates are green: formatting and two doctests pass; 58 Markdown files contain
420 inline links, including 270 repository-relative links with zero missing;
and diff, added-line-length, exact 11-file ownership, and clean-worktree checks
pass. Its behavior tree remains `a8f6179` except for documentation. Formal
fourth-cycle exact behavior SHA
`8e5fccea1b12483d2de2448e7a4ece0dc840ba39` is **GREEN** with zero findings
on each fresh correctness/API, filesystem/robustness, and performance/
concurrency track. Exact reviewers verified formatting, warnings-denied
workspace Clippy/tests, Linux/FreeBSD cross-target and WASI gates, two doctests,
25 private tests, 40 direct `grep_files` tests, four engine tests, and the
58/420/270/0 documentation inventory. All historical findings are closed,
including the attempted-read-window storage wording, so behavior is green.
This commit is a documentation-only seal and is exempt from another adversarial
review under the user's explicit instruction. Documentation seal
`0f48806310882caf3c668c72fe1b9d211cae744b` passed exact feature CI run
`32623585346` with all six jobs green and benchmark-evidence run `32623585349`
with both jobs and artifacts green. Both runs report exact `0f`. `main` was
fast-forwarded without force from `f6ab594` to exact `0f`. Main CI run
`32623904784` is **GREEN** for exact `0f`: all six jobs and every step passed
without reruns. Main benchmark-evidence run `32623904800` is **GREEN** on
attempt 1 for exact `0f`: both jobs and every step passed, with two valid non-
expired exact-SHA artifacts retained. The `grep_files` slice is delivered; the
remaining native-tool inventory remains pending.
This final delivery record is documentation-only and explicitly exempt from
another adversarial review. Its own exact feature and `main` workflows remain
required after push and cannot be self-recorded in this commit.

This document freezes the behavior that production, independent tests, and
documentation must compose into one exact behavior candidate before formal
review. A documentation-only seal or final delivery record made after behavior
is green is exempt from another adversarial cycle under the user's explicit
instruction, but its exact feature and `main` workflows must still pass and be
recorded. Milestone 03 remains `IN PROGRESS`, and its combined native-tool
checklist remains unchecked.

`grep_files` is a Linux/macOS library capability in `machine-god-native`. It
searches bounded eligible UTF-8 regular-file content beneath one retained
workspace identity. The current CLI does not construct an engine, register or
invoke this tool, prompt for its permission, or change any accepted invocation
or output byte.

## Workspace authority and permission semantics

The host supplies one explicit absolute workspace. Construction applies the
existing lexical host-root cleanup and final-component no-follow directory open,
then retains that opened directory descriptor as the continuing authority. A
relative root, final root symlink, non-directory, or unavailable root fails with
a fixed redacted construction error. The tool performs no workspace discovery.

Each execution reacquires a fresh descriptor-relative `.` with read-only,
directory, no-follow, close-on-exec, and nonblocking requirements. It applies
the delivered linked-root liveness validation: Linux rejects zero link count;
macOS requires the acquired descriptor's parent/name no-follow identity to
match its device, inode, and directory type, with the filesystem root handled
directly. The provider-selected root is then reached only through
descriptor-relative no-follow operations beneath that acquired identity.

Successful preflight prepares a distinct
`Capability::Filesystem { access: FilesystemAccess::SearchContent, path }`.
`SearchContent` authorizes bounded recursive entry-name observation and bounded
regular-file content inspection at the exact normalized path: that selected
object if it is a regular file, or eligible regular files below it if it is a
directory. It neither implies nor is implied by `Read`, `Metadata`, one-level
`Enumerate`, or `EnumerateRecursive`. It grants no mutation, external-path,
symlink-target, process, environment, or network authority.

The literal pattern, include filter, case option, output mode, pagination, and
context request attenuate what one allowed invocation returns. They do not
replace authorization for content search throughout the selected file or
subtree. Core does not infer a relationship among filesystem access kinds.

The retained descriptor confines model-selected components, not an untrusted
host. Ancestor resolution leading to the injected root, hard links already
inside it, and mounts visible beneath it remain trusted-host boundaries. This
is not a chroot or sandbox. Hardened construction and traversal outside Linux
and macOS are deferred.

## Public constants and construction errors

The candidate exports `GrepFilesTool`, `GrepFilesToolOpenError`,
`GrepFilesToolOpenErrorKind`, and these exact public constants:

| Constant | Value |
| --- | ---: |
| `GREP_FILES_TOOL_NAME` | `"grep_files"` |
| `MAX_GREP_FILES_PATTERN_BYTES` | `4,096` |
| `MAX_GREP_FILES_PATH_BYTES` | `4,096` |
| `MAX_GREP_FILES_INCLUDE_BYTES` | `4,096` |
| `MAX_GREP_FILES_RESULT_PATH_BYTES` | `4,096` |
| `MAX_GREP_FILES_HEAD_LIMIT` | `100` |
| `MAX_GREP_FILES_OFFSET` | `67,108,864` |
| `MAX_GREP_FILES_CONTEXT_LINES` | `5` |
| `MAX_GREP_FILES_FILE_BYTES` | `204,800` |
| `MAX_GREP_FILES_RESULT_LINE_BYTES` | `4,096` |
| `MAX_GREP_FILES_VISITED_ENTRIES` | `100,000` |
| `MAX_GREP_FILES_TOTAL_ENTRY_NAME_BYTES` | `16,777,216` |
| `MAX_GREP_FILES_CANDIDATE_FILES` | `10,000` |
| `MAX_GREP_FILES_TOTAL_CONTENT_BYTES` | `67,108,864` |
| `MAX_GREP_FILES_INCLUDE_MATCH_STEPS` | `8,388,608` |
| `MAX_GREP_FILES_CONTENT_MATCH_STEPS` | `268,435,456` |
| `MAX_GREP_FILES_DEPTH` | `256` |
| `MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES` | `8,192` |
| `MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES` | `8,192` |
| `MAX_GREP_FILES_SERIALIZED_RESULT_BYTES` | `49,152` |

The line bound was corrected during contract review from an initially proposed
256 bytes to 4,096 bytes. It now equals the pattern bound, so a bounded matching
excerpt can contain the complete first matched substring. The aggregate text
and serialized-result bounds remain unchanged.

`GrepFilesTool::open` has this complete fixed construction taxonomy:

| `GrepFilesToolOpenErrorKind` | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native grep_files is unsupported on this platform` |
| `InvalidRoot` | `native grep_files workspace root is invalid` |
| `InvalidFileType` | `native grep_files workspace root is not a directory` |
| `Unavailable` | `native grep_files workspace root is unavailable` |

The error retains only its kind. Its exact debug shape is
`GrepFilesToolOpenError { kind: ... }`; `Display` is the corresponding fixed
table entry. `GrepFilesTool` debug is exactly `GrepFilesTool { .. }`. None
contains a root path, operating-system diagnostic, or raw error number.

## Strict provider input and effect-free preflight

The advertised schema and preflight accept exactly these eight pinned field
names:

```json
{
  "pattern": "needle",
  "path": ".",
  "include": "src/**/*.rs",
  "case_insensitive": false,
  "mode": "matches",
  "head_limit": 100,
  "offset": 0,
  "context_lines": 0
}
```

`pattern` is required and must be a string. Every other field is optional.
Omission has these exact defaults:

| Field | Default and accepted value |
| --- | --- |
| `path` | `"."`; otherwise a string |
| `include` | `null` in prepared arguments; provider input, when present, must be a string |
| `case_insensitive` | `false`; otherwise a boolean |
| `mode` | `"matches"`; otherwise exactly `"matches"`, `"files_with_matches"`, or `"count"` |
| `head_limit` | `100`; otherwise an integer in `1..=100` |
| `offset` | `0`; otherwise an integer in `0..=67108864` |
| `context_lines` | `0`; otherwise an integer in `0..=5` |

Unknown fields, explicit provider `null`, wrong types, invalid enum spellings,
fractional values, and integers outside those ranges are invalid rather than
ignored or clamped. `head_limit` and `offset` affect only the two list modes.
`context_lines` affects only `matches`. The exact canonical values are still
prepared and echoed in every mode, including where they do not alter output.

Requested and normalized `path` forms are independently bounded to 4,096 UTF-8
bytes. Normalization is the delivered confined workspace rule: collapse
repeated `/`, remove `.` components, join ordinary components, and normalize
current-directory forms to `.`. It rejects an empty present string, absolute
path, any `..` component, C0 or C1 control, Unicode line or paragraph separator,
and Unicode bidirectional-formatting character. Backslash and space remain
literal Unix filename characters.

`pattern` is a nonempty literal UTF-8 byte string bounded to 4,096 bytes. It is
not a regular expression and receives no path normalization or metacharacter
interpretation. Empty strings, C0 or C1 control characters (including NUL, CR,
LF, and tab), Unicode line or paragraph separators, and Unicode
bidirectional-formatting characters reject as an invalid pattern. Backslash,
spaces, and regex punctuation remain literal.

An `include` string is independently bounded to 4,096 bytes and normalized
under exactly the delivered [`glob_files`](glob-files.md) grammar and forbidden-
character rules. Only `/` separates components; repeated separators and exact
`.` components normalize away; absolute, parent, empty-normalized, control,
line/paragraph-separator, and bidirectional-formatting forms reject. Backslash,
brackets, and braces are literal. `?`, `*`, and an exact `**` segment retain
their bytewise delivered meanings.

Execution constructs the fixed literal pattern table and charges that literal
work before selected-root resolution. It then compiles the optional normalized
include exactly once per call before candidate filtering. Slashful patterns
retain their parsed segments and non-recursive-segment count for reuse; they are
not split, counted, or allocated again per entry. The aggregate include-work
meter charges the complete once-per-call parse/compile work as well as every
per-candidate basename or path match operation, including candidate splitting,
dynamic-programming cells, and segment transitions. Slashful candidate
splitting checks cancellation at least every 1,024 candidate bytes. Recursive
and non-recursive dynamic-programming branches route their checks through the
same scan-local cancellation interface, including deterministic injected-
checker evidence for each branch. Checked overflow or the first unit beyond the
include-work cap fails the complete call.

Preflight is deterministic, synchronous, bounded, nonblocking, and effect-free.
It performs only strict decoding, range validation, and lexical normalization.
It opens no descriptor, reads no entry or content, inspects no type, and starts
no process. Successful preflight supplies policy with `SearchContent` at the
normalized selected path and supplies execution these exact canonical
arguments, including all defaults:

```json
{
  "pattern": "needle",
  "path": ".",
  "include": null,
  "case_insensitive": false,
  "mode": "matches",
  "head_limit": 100,
  "offset": 0,
  "context_lines": 0
}
```

Direct execution strictly decodes that exact eight-field canonical object and
revalidates every normalized value. It cannot reinterpret allowed arguments
into broader authority.

## Literal matching and eligible text

Search is line-oriented literal substring matching. Regex syntax, captures,
backreferences, look-around, alternation, and character classes have no special
meaning. Case-sensitive mode compares UTF-8 bytes exactly. Case-insensitive
mode folds only ASCII `A` through `Z`; all non-ASCII bytes remain exact. Match
records identify matching lines rather than every occurrence. The first match
on a line supplies its byte offset.

The matcher must be worst-case linear in bounded pattern and content bytes,
using KMP or an equivalently auditable failure-function algorithm. Content work
charges pattern-table construction before selected-root resolution, then each
per-file candidate-byte comparison and failure transition across the whole
call. Checked overflow exhausts the work budget. This deliberately does not
reproduce an unmetered naive sliding-window search.

Logical lines are split on LF. LF is not part of the line, while a preceding CR
remains content. A final LF creates no synthetic trailing line; real empty lines
between separators remain lines. Line numbers are one-based. An empty file has
zero real lines. The required nonempty pattern cannot match across line
boundaries.

An include-selected regular file is eligible text only when the complete
observed content is at most 204,800 bytes, contains no NUL, and is valid UTF-8.
The tool validates the complete bounded file before retaining matches, so a NUL
or invalid sequence after an apparent match disqualifies the entire file.

A file whose opened metadata already exceeds 204,800 bytes is counted as
skipped oversized without being read. Otherwise the reader retains at most
204,800 bytes plus one overflow witness; a witness caused by initial size
mismatch or concurrent growth also classifies the file as oversized. Invalid
UTF-8 or NUL classifies it as non-text. These expected exclusions are successful
scan statistics, not errors. A different open, metadata, or read failure fails
the complete call rather than silently weakening a count.

`candidate_files` counts include-selected regular candidates attempted.
`searched_files` counts eligible text files actually searched.
`skipped_oversized_files` and `skipped_non_text_files` disclose the two
eligibility exclusions. In a stable tree, candidate files partition into
searched and skipped files. A documented `NOENT` race may consume candidate
work before omission, so that equality is not a concurrent-scan invariant.
Matching totals are exact for eligible text observed within a successfully
completed bounded scan, not for excluded binary or oversized files.

## Descriptor-relative selected roots and traversal

The normalized selected `path` may name either one regular file or one
directory. Execution classifies its final component without following it. A
selected directory is then opened with directory-specific no-follow,
close-on-exec, and nonblocking flags. For a selected regular file, include
filtering occurs after that no-follow stat classification but before any content
open. Only an include-selected file is opened with content-specific no-follow,
close-on-exec, and nonblocking flags. Opened-descriptor metadata is
authoritative. A selected symlink or special object fails closed without being
followed or read.

For a selected regular file, an absent `include` selects it and a slash-free
`include` applies to its basename. A slashful include has no path beneath a
single-file search root to match and therefore excludes that file through one
charged, cancellation-checked include decision. Every selected-file call has
already consumed the fixed literal pattern-table construction and include
compile/decision work. An excluded selected file is not content-opened and
consumes no candidate, content-byte, or per-file literal-matching work. An
included selected file is opened and authoritatively revalidated as regular
before it consumes those latter budgets. Its returned path is the full
normalized workspace-relative selected path.

For a selected directory, traversal is iterative and descriptor-relative. The
selected directory is depth zero. Each directory is fully read; only `.` and
`..` are skipped. Every other entry consumes visit and raw name-byte budgets,
including hidden names, directories, symlinks, nonmatching regular files, and
special objects. Names must be valid UTF-8 and pass the confined path-character
checks. Every entry is no-follow classified before processing. The complete
validated set is sorted by a bytewise path-order key: a nondirectory uses its
raw UTF-8 name, while a directory uses its raw name followed by `/`. Traversal
uses deterministic depth-first order over those sorted entries, yielding
bytewise path order, with each eligible file's lines considered in increasing
line-number order.

Directories through depth 256 are opened and scanned. Encountering a child
directory requiring depth 257 fails the call. For every observed child, checked
arithmetic computes the complete workspace-relative descendant length before a
full descendant string is allocated, before entry-kind dispatch can retain or
use it, and before include matching. A length above 4,096 bytes fails the
complete call. Thus every constructed, retained, matched, or returned full
workspace-relative descendant path is bounded to 4,096 bytes, including paths
for directories, symlinks, and special objects.

Only no-follow regular files can become content candidates. Directory symlinks
are not descended; file symlinks, including links whose targets are inside the
workspace, are never followed or read. A stable FIFO, socket, device, or other
special classification is skipped without open. Concurrent replacement after
a regular-file classification may cause a content-specific nonblocking open to
acquire a special object at that same authorized name; authoritative opened-
descriptor type validation rejects it before content is read. No raced symlink
is followed. All observed entries still consume entry/name/path budgets. This
slice never uses a path-based context reread.

The once-compiled optional include pattern is tested against each regular
candidate before content open. A slash-free include matches the basename
recursively. A slashful include matches the path relative to the selected
search root. It filters files only and never prunes directory traversal. Its
single compile plus every invocation charge the aggregate delivered glob-
matcher work budget completely. Candidate path splitting checks cancellation
at intervals of at most 1,024 candidate bytes, including inputs that ultimately
fail to match. Both recursive and non-recursive dynamic-programming branches
route through the scan-local cancellation checker.

Hidden files are included. No ignore file, Git repository state, subprocess,
shell, external path, home expansion, or ambient environment participates.

## Complete scan and retained-output bounds

Every mode completes the same bounded candidate discovery and eligible-content
scan or fails without a partial result. Across one call:

- at most 100,000 non-dot entries and 16 MiB of raw entry-name bytes are
  observed;
- at most 10,000 include-selected regular candidates are attempted;
- at most 64 MiB of bytes returned by content reads are accepted in aggregate,
  including per-file overflow witnesses;
- include compilation and matching together receive at most 8,388,608 charged
  steps;
- literal content matching receives at most 268,435,456 charged steps;
- traversal opens directories only through depth 256; and
- every constructed descendant/result path remains at most 4,096 bytes.

All counters use checked arithmetic. Exactly the named cap is permitted; the
next charged unit is `grep_files_scan_limit`. A fired scan, content, candidate,
path, depth, include-work, or content-work cap fails every mode without partial
structured output. An output `truncated` flag never represents incomplete
scanning.

One content buffer belongs to one scan and is never shared between concurrent
or reentrant scans. Each file begins with a logical length of zero and reuses
the same allocation at its prior high-water mark. Initialized storage is
acquired or grown before each read in an attempted-read window of at most 8 KiB,
and its high-water length never exceeds 204,801 bytes: the 204,800-byte eligible-
file ceiling plus one overflow witness. A short or zero-byte read therefore may
initialize storage beyond the bytes that read returns. Only the logical length,
visible file slice, and charged content-byte counters advance by bytes actually
read. Logical reset prevents stale bytes from entering a later file view;
actual per-file and aggregate-overflow witnesses remain charged exactly as
before.

The two list modes retain at most the requested `head_limit`, never more than
100 mode records, after discarding the first `offset` logical results from the
deterministic result stream. Across returned records, raw path bytes are at
most 8 KiB and raw match/context text bytes are at most 8 KiB. One line or
context value is at most 4,096 UTF-8 bytes. The complete serialized
`ToolOutput`, including its envelope and JSON escaping, is at most 48 KiB. A
host-configured lower core result limit still applies afterward.

For a matching line no longer than 4,096 bytes, `line` is the complete line and
`excerpt_start_byte` is zero. For a longer line, `line` is a deterministic
UTF-8-boundary window no longer than 4,096 bytes that contains the entire first
matched substring; `excerpt_start_byte` identifies that window in source-line
bytes and `line_truncated` is true. The equality between maximum pattern and
line-result bytes makes the complete first match representable.

Context is extracted from the same validated per-file view of the scan-local
reusable buffer that produced the match. There is no second open or race-prone
reread. A context line record is
the complete line when it fits, otherwise its longest UTF-8-safe prefix of at
most 4,096 bytes with `line_truncated: true`.

For each retained match, output budgets admit the match excerpt first, then
existing requested context lines in increasing source-line order:
`context_before` chronologically, followed by `context_after` chronologically.
Only complete bounded context records are admitted while aggregate text and
serialized-result budgets allow. Omitted requested context sets the match's
`context_truncated` to true. Missing lines at the beginning or end of a file are
not omissions. If the match record itself cannot fit, it and every later mode
record are omitted from the returned prefix.

## Exact structured results

Every mode echoes all eight canonical request fields. A nullable include is the
explicit prepared `null`, not an omitted field.

### `matches`

```json
{
  "path": ".",
  "pattern": "needle",
  "include": null,
  "case_insensitive": false,
  "mode": "matches",
  "head_limit": 100,
  "offset": 0,
  "context_lines": 1,
  "matches": [
    {
      "path": "src/lib.rs",
      "line_number": 12,
      "match_start_byte": 8,
      "excerpt_start_byte": 0,
      "line": "let x = needle();",
      "line_truncated": false,
      "context_before": [
        {"line_number": 11, "line": "fn example() {", "line_truncated": false}
      ],
      "context_after": [
        {"line_number": 13, "line": "}", "line_truncated": false}
      ],
      "context_truncated": false
    }
  ],
  "total_matches": 1,
  "matching_files": 1,
  "candidate_files": 1,
  "searched_files": 1,
  "skipped_oversized_files": 0,
  "skipped_non_text_files": 0,
  "next_offset": null,
  "truncated": false
}
```

One match record is produced per matching line and refers to the first literal
match on that line. `total_matches` is the exact eligible matching-line total;
`matching_files` is the exact number of eligible files with at least one match.
Context records do not affect either total, pagination, or `head_limit`.

### `files_with_matches`

```json
{
  "path": ".",
  "pattern": "needle",
  "include": null,
  "case_insensitive": false,
  "mode": "files_with_matches",
  "head_limit": 100,
  "offset": 0,
  "context_lines": 0,
  "files": ["src/lib.rs"],
  "matching_lines": 1,
  "total_files": 1,
  "candidate_files": 1,
  "searched_files": 1,
  "skipped_oversized_files": 0,
  "skipped_non_text_files": 0,
  "next_offset": null,
  "truncated": false
}
```

Each eligible matching file appears at most once. `matching_lines` is the exact
eligible matching-line total; `total_files` is the exact number of eligible
files with matches.

For both list modes, `offset` discards that many logical mode records and
`head_limit` bounds the returned page. `next_offset` is `null` when no later
logical result exists; otherwise it is exactly `offset + emitted_records`.
`truncated` is true whenever the returned list is not the complete logical
result: `offset > 0`, a later result exists, or a path/text/serialized-output
bound omitted a result. Output-bound omission stops the returned prefix rather
than skipping a nonfitting result to admit a later one. The 67,108,864 offset
maximum equals the aggregate content-byte bound. Because a matching line must
contain the required nonempty pattern, a successful scan cannot contain more
than 67,108,864 logical matching-line records; matching-file totals are bounded
more tightly by candidate count. Every non-null emitted `next_offset` is
therefore accepted unchanged by a subsequent prepared call.

### `count`

```json
{
  "path": ".",
  "pattern": "needle",
  "include": null,
  "case_insensitive": false,
  "mode": "count",
  "head_limit": 100,
  "offset": 0,
  "context_lines": 0,
  "matching_lines": 1,
  "matching_files": 1,
  "candidate_files": 1,
  "searched_files": 1,
  "skipped_oversized_files": 0,
  "skipped_non_text_files": 0
}
```

Count mode has neither `next_offset` nor `truncated`. Its two matching totals
are exact for eligible text in the successfully completed bounded scan.
`head_limit`, `offset`, and `context_lines` are echoed canonical inputs but do
not alter count execution.

## Identity, removal, and concurrent mutation

The retained workspace descriptor, not the original host path, remains the
authority. Renaming the workspace and replacing its old pathname cannot switch
the tool to a different directory. Each call still requires the fresh acquired
root identity to remain linked. Removal before acquisition or validation is
unavailable; a concurrent rename or removal can conservatively fail or observe
the acquired retained identity.

An opened ancestor remains the stable base of later child lookup. Replacement
before an entry's no-follow classification/open may be observed at the same
authorized name. Replacement after a regular file opens cannot redirect its
descriptor. A stable special object is skipped without open. Replacement of a
classified regular file with a special object may race into a nonblocking open,
but authoritative descriptor validation rejects it without reading content;
symlink targets are never followed. A `NOENT` race after entry observation may
omit that entry or subtree and still produce a successful bounded result. Other
traversal, classification, open, metadata, and read failures fail the complete
call.

File contents are not an atomic snapshot. Concurrent writes, growth, shrink,
or replacement before open can affect observed bytes; the opened descriptor
still cannot be redirected afterward. The size sentinel and aggregate read
budget prevent growth from becoming an unbounded read. Context and matching
derive from the same validated logical file view, so they agree with each other
even though the scan-local allocation is reused afterward and is not a
filesystem snapshot. Retained result strings own their bytes independently of
that later reuse.

A stable tree and stable file bytes produce deterministic traversal, totals,
pagination, excerpts, and context. A concurrent scan is not a multi-file or
multi-entry point-in-time snapshot, and returned paths and contents need not
all have coexisted simultaneously.

## Fixed errors, redaction, cancellation, and drop

Preparation and direct execution expose only these fixed categories, messages,
and retry flags:

| `ToolErrorKind` | `code` | Exact message | Retryable |
| --- | --- | --- | --- |
| `Cancelled` | `grep_files_cancelled` | `grep_files execution was cancelled` | no |
| `InvalidInput` | `grep_files_invalid_arguments` | `grep_files arguments are invalid` | no |
| `InvalidInput` | `grep_files_invalid_path` | `grep_files path is invalid` | no |
| `InvalidInput` | `grep_files_invalid_pattern` | `grep_files pattern is invalid` | no |
| `InvalidInput` | `grep_files_invalid_include` | `grep_files include pattern is invalid` | no |
| `Unavailable` | `grep_files_unsupported_platform` | `native grep_files is unsupported on this platform` | no |
| `Unavailable` | `grep_files_not_found` | `requested search root is unavailable` | no |
| `PermissionDenied` | `grep_files_permission_denied` | `requested search root cannot be searched` | no |
| `PermissionDenied` | `grep_files_path_rejected` | `requested path is not a confined regular file or directory` | no |
| `Unavailable` | `grep_files_unavailable` | `requested content search is unavailable` | yes |
| `Execution` | `grep_files_read_failed` | `requested content search could not be completed` | yes |
| `Execution` | `grep_files_invalid_entry_name` | `requested content search contains an unsupported entry name` | no |
| `Execution` | `grep_files_scan_limit` | `requested content search exceeds the scan limit` | no |

No constructor or tool error `Display`, `Debug`, nested source, code, or
message retains or reflects the workspace root, selected path, pattern,
include, entry name, file bytes, matching line, metadata, operating-system text,
or raw error number. Only fixed error categories may be selected from native
failure classes. Core continues mapping non-cancellation preparation/execution
errors to its generic durable tool error.

Successful paths, literal request values, line excerpts, context, and counts
are intentionally model-visible. They enter the durable tool result and
observer events and must be treated as potentially sensitive workspace data.
This success disclosure is not an error-redaction failure.

Creating the execution future is inert. Its first poll performs bounded
synchronous native work. Cancellation is checked before and after fresh-root
acquisition/liveness, around every selected-component and descendant open,
directory read, classification, file open and metadata operation, before and
after bounded content reads, at fixed intervals through include compilation,
include/content matching, and line-index construction, before every serialized-
size trimming reconstruction/serialization attempt, and immediately before
return. Slashful selected-file rejection has its own charged cancellation check;
slashful candidate splitting checks at intervals of at most 1,024 candidate
bytes; and both recursive and non-recursive dynamic-programming branches route
through the scan-local cancellation checker. The fixed byte-processing interval
is at most 1,024 source or candidate bytes; directory-entry loops retain their
per-entry checks, and every
output-trimming iteration begins with a check. Cancellation cannot preempt one
syscall already in flight.

No task, thread, process, timer, cache, indexer, or producer is detached.
Dropping an unpolled future performs no filesystem work. Dropping after work
has begun releases every owned per-call descriptor and the one scan-local
content buffer and publishes no partial result.

## Pinned upstream input and deliberate differences

The pinned fx revision advertises the same eight provider field names, literal
matching, three modes, ASCII case-insensitive option, pagination, and bounded
context. That is a compatibility input only. Its implementation may use Git,
ignore behavior, path-based access, internal-workspace symlink targets,
permissive decoding, different output formatting, a different result cap, or a
naive fallback matcher. Machine-god does not inherit those authority or
resource choices.

This candidate deliberately uses strict input, workspace-only normalized paths,
the distinct `SearchContent` authorization kind, retained descriptor-relative
no-follow access, no symlink targets, deterministic sorted traversal, one
linear and explicitly metered literal engine, explicit eligibility/skip
statistics, structured bounded output, fixed diagnostics, and no subprocess.
No scenario evidence yet establishes observable fx equivalence.

Zig remains solely the pinned upstream benchmark build input. The machine-god
product and this implementation remain Rust.

## Deferred scope and nonclaims

This candidate does not add:

- regular expressions, captures, Unicode case folding, occurrence counts, or
  cross-line matching;
- binary/byte output, alternate encodings, memory mapping, streaming results,
  indexing, caching, watchers, or snapshots;
- Git, ignore files, repository discovery, subprocesses, shell fallback, or
  symlink-target search;
- external paths, `~` expansion, workspace escape, or ambient environment
  discovery;
- a CLI `grep_files` command, workspace command, slash command, prompt UI, or
  release-binary behavior;
- non-Linux/macOS hardened traversal;
- a benchmark workload, latency/throughput/allocation result, product-
  performance claim, compatibility-status change, or fx-equivalence claim; or
- completion of the native-tool inventory or Milestone 03.

The existing core argument/result limits, permission-request identity and risk,
grant-cache absence, generic durable tool-error mapping, provider/transport,
session store and lifecycle, configuration, credentials, CLI bytes, benchmark
evidence, compatibility inventory, workflows, and release thresholds remain
unchanged by this bounded candidate.
