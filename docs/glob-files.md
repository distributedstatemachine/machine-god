# Native `glob_files` tool

`glob_files` is a Linux/macOS library capability in `machine-god-native`. The
current CLI does not construct an engine, register or invoke this tool, prompt
for its permission, or change any accepted invocation or output byte.

## Workspace authority and platform scope

The host roots the tool in one explicitly selected absolute workspace path. On
Linux and macOS, construction applies the same lexical host-root cleanup and
final-component no-follow directory open as the existing confined workspace
tools, then retains that directory descriptor as the continuing authority. A
relative root, final root symlink, non-directory, or unavailable root fails
with a fixed redacted construction error. The tool does not discover a
workspace from process state.

Lexical host-root cleanup rebuilds the injected path from its components. It
removes redundant separators and `.` components while preserving `..`; it does
not canonicalize ancestors or resolve symlinks. Removing terminal separators
and terminal `.` components keeps a decorated final root symlink in the
no-follow lookup position, while equivalent forms of a real directory continue
to select that directory. This cleanup applies only to the trusted
host-supplied root.

Each execution begins from the retained identity, acquires a fresh `.`
descriptor with directory, no-follow, nonblocking, and close-on-exec
requirements, and applies the same linked-root liveness validation as
[`file_info`](file-info.md). Linux rejects zero link count. macOS resolves the
exact acquired descriptor's kernel path, opens its parent descriptor-
relatively, and requires no-follow parent/name metadata to match the acquired
device, inode, and directory type; the filesystem root is accepted directly.
The provider-supplied search root and every traversed directory are then opened
descriptor-relatively beneath that validated identity without following any
selected component.

The retained descriptor confines model-selected components beneath the
workspace. Resolution of ancestors leading to the injected host root and mount
points visible below the retained directory remain trusted-host boundaries.
This is not a chroot or a sandbox against the host. Hardened construction and
traversal on targets other than Linux and macOS are deferred, and this contract
makes no security claim for them.

## Public constants and construction errors

The native crate exports `GlobFilesTool`, `GlobFilesToolOpenError`,
`GlobFilesToolOpenErrorKind`, and these fixed constants:

| Constant | Value |
| --- | ---: |
| `GLOB_FILES_TOOL_NAME` | `"glob_files"` |
| `MAX_GLOB_FILES_PATTERN_BYTES` | `4,096` |
| `MAX_GLOB_FILES_PATH_BYTES` | `4,096` |
| `MAX_GLOB_FILES_RESULT_PATH_BYTES` | `4,096` |
| `MAX_GLOB_FILES_MATCHES` | `100` |
| `MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES` | `16,384` |
| `MAX_GLOB_FILES_VISITED_ENTRIES` | `100,000` |
| `MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES` | `16,777,216` |
| `MAX_GLOB_FILES_DEPTH` | `256` |
| `MAX_GLOB_FILES_MATCH_STEPS` | `8,388,608` |

`GlobFilesTool::open` returns this complete fixed construction taxonomy:

| `GlobFilesToolOpenErrorKind` | `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native glob_files is unsupported on this platform` |
| `InvalidRoot` | `native glob_files workspace root is invalid` |
| `InvalidFileType` | `native glob_files workspace root is not a directory` |
| `Unavailable` | `native glob_files workspace root is unavailable` |

The construction error exposes only its kind. Its `Debug` and `Display` forms
retain no workspace path, operating-system text, or raw error number.

## Provider input and effect-free preflight

The registered tool name is `glob_files`. Its fixed description is
`Find file paths matching a glob pattern within the configured workspace`.
Its advertised schema and preflight accept exactly this strict object:

```json
{"pattern":"src/**/*.rs","path":".","mode":"matches"}
```

`pattern` is required and must be a string. `path` and `mode` are optional;
omission defaults them to `.` and `matches`, respectively. A present `path`
must be a string, and a present `mode` must be exactly `matches` or `count`.
`null`, every unknown field, a missing `pattern`, or any wrong type or enum
spelling is invalid. The fixed property descriptions are:

- `pattern`: `Glob pattern relative to the search root, such as src/**/*.rs or
  *.md`
- `path`: `Workspace-relative directory search root; defaults to the workspace
  root`
- `mode`: `Return matching paths or an exact count; defaults to matches`

Both the requested and normalized `path` are independently bounded to 4,096
UTF-8 bytes. Path normalization is exactly the [`file_info`](file-info.md)
lexical rule: collapse repeated `/` separators, remove `.` components, join
ordinary components, and normalize current-directory forms to `.`. It rejects
an empty present string, `/`-rooted path, any `..` component, C0 or C1 control
character, Unicode line or paragraph separator, or Unicode bidirectional-
formatting character. Backslash and space are literal Unix filename
characters. Windows-looking input is a confined Unix name, not an external
prefix.

Both the requested and normalized `pattern` are independently bounded to
4,096 UTF-8 bytes. Pattern normalization uses `/` as its only separator,
collapses repeated separators, and removes exact `.` segments. It rejects an
empty requested or normalized pattern, an absolute pattern, any exact `..`
segment, and the same forbidden characters as path normalization. A pattern
made only of separators or current-directory segments is therefore invalid.
Backslash is literal rather than an escape or separator. Square brackets and
braces are literal bytes rather than character-class, alternation, or expansion
syntax.

Preflight is deterministic, synchronous, bounded, nonblocking, and effect-
free. It performs only strict JSON decoding and lexical normalization. It does
not acquire or validate a root descriptor, open a search directory, read an
entry, inspect a type, follow a link, or exercise any filesystem authority.

Successful preflight returns both:

- `Capability::Filesystem` with the distinct
  `FilesystemAccess::EnumerateRecursive` operation at the normalized selected
  `path`; and
- exact prepared execution arguments
  `{"pattern":"<normalized>","path":"<normalized>","mode":"matches|count"}`,
  including explicit values for both defaults.

Policy and allowed execution therefore agree on the exact selected subtree.
The normalized pattern and mode attenuate only what is returned; they do not
broaden authority and are not substitutes for permission to enumerate that
entire subtree. `EnumerateRecursive` is distinct from the existing one-level
`FilesystemAccess::Enumerate`: neither permission kind implies the other.
Preparation failure occurs before permission policy or filesystem access.

## Frozen bytewise matcher

Matching operates on the UTF-8 bytes of already validated candidate paths.
Ordinary pattern bytes match themselves. The only wildcard rules are:

- `?` matches exactly one UTF-8 byte within one path component;
- `*` matches zero or more UTF-8 bytes within one path component and never
  crosses `/`; and
- a segment that is exactly `**` matches zero or more complete path
  components.

Only an exact `**` segment has cross-component meaning. Backslash, `[`, `]`,
`{`, and `}` remain literal. Because `?` is bytewise, one `?` does not match an
entire multi-byte Unicode scalar; candidate names nevertheless must be valid
UTF-8 and are never lossily decoded.

If the normalized pattern contains no `/`, it is matched against every
candidate's basename while the complete selected subtree is traversed. For
example, `*.md` can match a Markdown basename at any depth. If the normalized
pattern contains `/`, it is matched against the candidate path relative to the
selected search root. Thus a selected `path` of `docs` and a slashful pattern
of `reviews/*.md` match relative spelling `reviews/report.md`, while the
returned path is `docs/reviews/report.md`. An exact `**` can consume zero
components, so `**/*.md` also admits a root-level Markdown candidate. Pattern
normalization happens before deciding whether the pattern is slash-free.

These field names, enum spellings, and bytewise matcher are pinned
compatibility inputs. They do not establish fx equivalence. Strict decoding,
retained-root confinement, literal bracket/brace/backslash behavior, complete
bounded traversal, exact output shapes, and redacted failures are deliberate
machine-god contract choices.

## Descriptor-relative recursive execution

Execution begins only after core allows the exact prepared
`EnumerateRecursive` capability. It consumes the prepared values without
reinterpreting them. After fresh retained-root acquisition and liveness
validation, it opens the selected search root descriptor-relatively with
directory and no-follow requirements. A selected symlink or non-directory
fails closed.

Traversal is iterative and descriptor-relative. The selected search-root
directory has traversal depth `0`. Child directories may be opened through
traversal depth `256`, inclusive. While scanning a directory at depth `256`,
regular-file and final-symlink candidates directly inside it are eligible;
their component depth relative to the selected root is `257`. Encountering a
child directory that would require opening traversal depth `257` fails the
whole call with `glob_files_scan_limit`.

Every visited directory is fully read before any of its entries are processed.
The tool skips only the special `.` and `..` entries. Every other name must be
valid UTF-8 and pass the same control, line/paragraph-separator, and
bidirectional-formatting rejection as input paths. The complete validated entry
set is then sorted in ascending raw UTF-8 byte order before classification and
traversal. Hidden names are included. Stable-tree behavior is therefore
independent of filesystem iteration order.

Only regular files and final symlinks are match candidates. A symlink is
matched and returned as the link path itself, including when its target is
missing, but is never followed or descended through. Directories are used only
for traversal and are not returned. FIFO, socket, device, and other special
objects are ignored as candidates but still consume scan budgets. The tool
reads no file or symlink-target content, performs no symlink descent, applies no
ignore or Git rule, and starts no subprocess.

Candidate output paths are normalized full workspace-relative paths. A search
under `docs` therefore returns `docs/report.md`, not `report.md` or an external
absolute spelling. Constructing any regular-file or final-symlink candidate
whose full workspace-relative path exceeds 4,096 UTF-8 bytes fails the entire
call with `glob_files_scan_limit`; it is never silently omitted or truncated.

## Complete scan budgets

Both modes traverse the complete bounded selected subtree. Across the entire
call, the tool permits at most:

- 100,000 visited non-dot entries of every type;
- 16 MiB of aggregate raw UTF-8 entry-name bytes;
- directory traversal depth 256 under the exact boundary above; and
- 8,388,608 aggregate bytewise matcher-work steps.

Matcher work is charged deterministically across the whole call. One step is
charged for each candidate byte inspected while splitting a slashful candidate,
each pattern-segment loop visit (including a consecutive `**` that is skipped),
each dynamic-programming cell written (including column zero), and each main
loop transition or trailing-star consumption in an invoked component matcher.
Slash-free matching uses only the component-matcher charges. Pattern splitting
and the non-`**` segment count are computed once outside candidate matching and
do not consume the per-call counter. Checked counter overflow is exhaustion.
Exactly 8,388,608 steps are permitted; the next charged step fails closed.

Every non-`.`/`..` entry counts against the visit and name-byte budgets,
including hidden entries, directories, nonmatching candidates, symlinks, and
ignored special objects. Attempting to exceed any scan budget, the matcher-work
budget, or the candidate path bound is `glob_files_scan_limit` in both
`matches` and `count` mode. A
scan-limit failure returns no partial result. Pattern and mode do not permit a
caller to bypass work required to establish a complete bounded result.

## Exact results and truncation

`matches` mode succeeds with exactly this shape:

```json
{
  "path": ".",
  "pattern": "**/*.rs",
  "mode": "matches",
  "matches": ["crates/core/src/lib.rs", "src/main.rs"],
  "truncated": false
}
```

After the complete scan, all observed matches are ordered by their full
workspace-relative path's raw UTF-8 bytes. The emitted array is the longest
prefix of that global order that contains at most 100 paths and at most 16 KiB
of aggregate raw path bytes. If the next ordered path would exceed the byte
budget, that path and all later paths are omitted; the tool does not skip a long
path to admit a later short one. `matches` is always bytewise sorted and is the
globally smallest prefix, not a filesystem-iteration-dependent sample.

`truncated` is `true` if and only if at least one observed match is omitted from
the emitted prefix, equivalently when the total observed match count exceeds
the emitted length. It is not pagination and carries no continuation token.
Because scan-budget failures fail the whole call, `truncated` never means that
traversal stopped early.

`count` mode succeeds with exactly this shape:

```json
{
  "path": ".",
  "pattern": "**/*.rs",
  "mode": "count",
  "count": 42
}
```

The count is exact for the complete bounded traversal and is never a lower
bound or truncated count. In both result shapes, `path`, `pattern`, and `mode`
are the explicit prepared normalized values.

At the independent worst case, 4,096 raw bytes each for `path` and `pattern`,
16 KiB of aggregate raw match-path bytes, 100 array elements, and JSON escaping
remain below core's default 64 KiB serialized result limit. A host-configured
lower core limit still applies after execution. Count mode is substantially
smaller.

## Identity, removal, and races

The retained workspace descriptor, not the original host path string, remains
the authority. A stable rename does not redirect the tool, and replacing the
old host path cannot switch it to a different workspace. Each call still
requires the fresh acquired `.` descriptor to remain linked through the
platform liveness check. Removal before acquisition or validation is
unavailable; a concurrent rename or removal may conservatively fail or observe
the acquired retained identity.

An opened directory descriptor remains the stable base for its children, so
renaming or replacing that already opened ancestor cannot redirect later
lookups. An ordinary entry replaced before its own no-follow classification or
directory open may be observed at the same authorized name. A `NOENT` race
after a directory entry was read may omit that vanished entry or subtree. Any
other traversal, classification, or enumeration failure fails the complete call
through the fixed redacted taxonomy.

A stable tree produces a deterministic result. During concurrent mutation,
the call supplies no point-in-time snapshot: entries can be observed before or
after replacement, a `NOENT` race can omit them, and returned paths need not all
have existed simultaneously. Complete bounded traversal is not filesystem
snapshotting.

## Tool errors, redaction, and cancellation

Preparation and direct execution return these complete fixed `ToolError`
values. `Display` is always `<code>: <message>`.

| `ToolErrorKind` | `code` | `message` | `retryable` |
| --- | --- | --- | --- |
| `Cancelled` | `glob_files_cancelled` | `glob_files execution was cancelled` | `false` |
| `InvalidInput` | `glob_files_invalid_arguments` | `glob_files arguments are invalid` | `false` |
| `InvalidInput` | `glob_files_invalid_path` | `glob_files path is invalid` | `false` |
| `InvalidInput` | `glob_files_invalid_pattern` | `glob_files pattern is invalid` | `false` |
| `Unavailable` | `glob_files_unsupported_platform` | `native glob_files is unsupported on this platform` | `false` |
| `Unavailable` | `glob_files_not_found` | `requested search root is unavailable` | `false` |
| `PermissionDenied` | `glob_files_permission_denied` | `requested search root cannot be enumerated` | `false` |
| `PermissionDenied` | `glob_files_path_rejected` | `requested path is not a confined directory` | `false` |
| `Unavailable` | `glob_files_unavailable` | `requested glob search is unavailable` | `true` |
| `Execution` | `glob_files_read_failed` | `requested glob search could not be completed` | `true` |
| `Execution` | `glob_files_invalid_entry_name` | `requested glob search contains an unsupported entry name` | `false` |
| `Execution` | `glob_files_scan_limit` | `requested glob search exceeds the scan limit` | `false` |

The selected-root lookup maps missing, access-denied, rejected-path, and other
unavailable conditions to the fixed applicable categories. Invalid UTF-8 or a
forbidden entry name fails the complete call as `invalid_entry_name`. Scan caps
always use `scan_limit`; enumeration and other traversal failures use the fixed
read or unavailable categories. No public construction or tool error retains
the root, requested values, normalized values, candidate or entry name,
operating-system diagnostic, or raw error number.

Non-cancellation preparation or execution errors passing through the engine
are reduced to core's existing generic durable tool-error result before the
model sees them. A direct caller receives the fixed `glob_files_cancelled`
error when its supplied token is cancelled. When the engine supplies the shared
turn token, core cancellation takes precedence, terminates the turn, and leaves
the durable unknown-result placeholder intact.

Creating the execution future performs no filesystem operation. Its first poll
performs the complete bounded synchronous traversal and returns ready; it does
not internally yield pending. Cancellation is checked before and after fresh-
root acquisition and liveness validation, between selected path-component
opens, before and after each directory read, before each entry classification
or child-directory open, after each such syscall, before match accounting, and
immediately before return. It cannot preempt one filesystem syscall already in
flight. Dropping before first poll is effect-free. The future starts no task,
thread, channel, subprocess, timer, or detached work.

## Reference-host composition

`glob_files` is part of the native reference host. The canonical current
inventory, ordering, and descriptor distribution live only in the
[native reference-host tool catalog](native-reference-host.md#tool-catalog).
Prepared-root composition does not reopen the workspace path. A workspace
descriptor-clone failure remains the host's fixed redacted `WorkspaceRoot`
construction stage and occurs before engine construction. No constructor
arguments, configuration fields, credential, provider, transport, permission,
session, runtime, or CLI authority change.

## Deferred scope

This tool adds no CLI or slash command, external-path access, workspace
discovery, non-Linux/macOS hardening, ignore-file or Git behavior, Git or shell
subprocess, content or symlink-target read, mutation, pagination, snapshot,
watcher, new dependency, benchmark workload, product-performance claim, or fx-
equivalence claim.
