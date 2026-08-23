# Native `edit_file` contract

Status: **IN PROGRESS — contract-only kickoff; no production behavior exists**

This document freezes the twenty-first bounded Milestone 03 slice from exact
delivered base `242adfed4be717baf7cd07275aae40ec8a3637f6`. The kickoff changes
documentation only. Under the user's explicit instruction, documentation-only
kickoff, seal, and delivery commits are exempt from adversarial review. Their
exact feature and `main` workflows remain required, but cannot be self-recorded
before they run.

`edit_file` will replace exactly one occurrence of exact text in one existing
workspace file. It does not grant file creation, a general read capability, or
unbounded filesystem mutation. The product remains Rust. Zig remains only the
pinned upstream fx benchmark build input.

## Public API, schema, and limits

`machine-god-native` will export `EDIT_FILE_TOOL_NAME`, `EditFileTool`,
`EditFileToolOpenError`, `EditFileToolOpenErrorKind`, and these public limits:

| Public constant | Exact value |
| --- | ---: |
| `EDIT_FILE_TOOL_NAME` | `"edit_file"` |
| `MAX_EDIT_FILE_PATH_BYTES` | `4,096` |
| `MAX_EDIT_FILE_PATH_COMPONENTS` | `256` |
| `MAX_EDIT_FILE_OLD_STRING_BYTES` | `49,152` |
| `MAX_EDIT_FILE_NEW_STRING_BYTES` | `49,152` |
| `MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_EDIT_FILE_EXISTING_BYTES` | `49,152` |
| `MAX_EDIT_FILE_RESULTING_BYTES` | `49,152` |
| `MAX_EDIT_FILE_CHUNK_BYTES` | `8,192` |
| `MAX_EDIT_FILE_MATCH_WORK_STEPS` | `393,216` |
| `MAX_EDIT_FILE_TEMP_ATTEMPTS` | `8` |
| `MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES` | `16,384` |

The exact tool description is
`Replace one exact text occurrence in an existing workspace file`. Property
descriptions are `Workspace-relative file path`, `Exact UTF-8 text to replace`,
and `UTF-8 replacement text` for `path`, `old_string`, and `new_string`.

The input schema is exactly:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Workspace-relative file path"
    },
    "old_string": {
      "type": "string",
      "description": "Exact UTF-8 text to replace"
    },
    "new_string": {
      "type": "string",
      "description": "UTF-8 replacement text"
    }
  },
  "required": ["path", "old_string", "new_string"],
  "additionalProperties": false
}
```

All fields are required and have no defaults. Unknown or mistyped values are
invalid. `old_string` must be nonempty, and `old_string == new_string` is
invalid. Empty `new_string`, embedded NUL bytes, and other valid UTF-8 are
accepted in either text field except for the nonempty-old-string rule.

The two raw text limits are independent. The complete serialized arguments have
their own 64 KiB limit, so JSON escaping and field/path overhead can reject an
input even when each raw field is within its individual limit. The requested
and canonical argument forms are both checked before execution can retain or
use them. This does not promise that both 48 KiB fields can coexist in one
admissible provider call.

`EditFileTool::open` accepts one explicitly injected absolute workspace. The
public type and fixed unsupported behavior exist on every target; filesystem
execution is supported only on Linux and macOS. Its redacted construction
taxonomy is:

| `EditFileToolOpenErrorKind` | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native edit_file is unsupported on this platform` |
| `InvalidRoot` | `native edit_file workspace root is invalid` |
| `InvalidFileType` | `native edit_file workspace root is not a directory` |
| `Unavailable` | `native edit_file workspace root is unavailable` |

Construction errors retain only their kind. `Display` and `Debug` never retain
the injected path, operating-system text, or a raw error number. A safe public
tool instance cannot be constructed on an unsupported target.

## Preparation and authority

Preparation is strict, synchronous, bounded, nonblocking, and effect-free. It
must not inspect whether the selected path exists, read its bytes, count
matches, stage output, or compute a diff. It requires the exact tool name and
three-field shape, checks the raw and serialized limits, and normalizes `path`
with the delivered `write_file` confinement rule. Repeated `/` separators
collapse and exact `.` components are removed. Backslash and space remain
literal Unix filename characters. Empty paths, absolute paths, parent
components, C0/C1 controls, Unicode line/paragraph separators, bidirectional-
formatting characters, and a normalized `.` target are rejected. Requested and
normalized path forms are independently bounded to 4,096 UTF-8 bytes; the
normalized form is bounded to 256 components.

Successful preparation preserves `old_string` and `new_string` byte-for-byte,
emits the exact canonical three-field arguments, and requests only:

```text
Capability::Filesystem {
    access: FilesystemAccess::Edit,
    path: normalized_path,
}
```

Core will add `FilesystemAccess::Edit`, serialized exactly as `edit`. This
authority permits one bounded existing-file preimage read, exact-one-match
derivation, and atomic pathname replacement under this contract. It is distinct
from `Read`, `Write`, `Create`, `Delete`, `Metadata`, `Enumerate`,
`EnumerateRecursive`, and `SearchContent`.

Reusing `Write` would contradict the delivered `write_file` contract, which
explicitly grants no target-content read. `Edit` grants no unrelated content
read, file creation, deletion, parent creation, external-path access,
symlink-target access, or general mutation. Execution decodes the same strict
shape, requires an already canonical path, and reapplies every input bound, so
direct invocation cannot widen an approved capability.

Because core preparation is contractually effect-free, this slice cannot
reproduce pinned fx's preapproval preimage read or computed diff preview. Policy
can receive the normalized path and distinct edit authority, not preimage bytes
or a derived patch. Preview-bearing authorization is deferred rather than
weakening `Tool::prepare` or moving an ambient filesystem effect into core.

For example, provider input:

```json
{
  "path": "./notes//today.txt",
  "old_string": "draft",
  "new_string": "final"
}
```

prepares the same text bytes at canonical path `notes/today.txt` and requests
only `FilesystemAccess::Edit` for that path.

## Exact-one matching and result construction

Allowed execution reads one complete existing regular-file preimage of at most
49,152 bytes plus one overflow witness. The complete preimage must be valid
UTF-8; NUL is valid. No lossy decoding or alternate encoding is attempted.

The matcher uses deterministic, worst-case-linear prefix-table matching over
UTF-8 bytes. Exact byte equality defines a match; no normalization, case
folding, regex, locale, or character-boundary expansion is applied. Occurrences
are overlapping. For example, `old_string = "aa"` occurs twice in preimage
`"aaa"`, at byte offsets zero and one, so that edit is ambiguous. Matching may
stop immediately after observing the second occurrence because neither its
position nor an exact total is exposed.

`MAX_EDIT_FILE_MATCH_WORK_STEPS` is a public defensive ceiling. One step is
charged for each prefix-table byte comparison or fallback transition and each
preimage byte comparison or fallback transition, including the post-match
fallback needed to detect overlap. Exactly 393,216 steps are allowed; the next
step fails without publication. Cancellation is checked before matching, after
each bounded batch of at most 1,024 charged steps, after finding a match, and
after establishing exact-one or ambiguity. The linear algorithm keeps every
publicly legal input below the defensive ceiling; private injected-budget tests
must nevertheless prove exact-cap acceptance and one-over rejection. No
accepted input can cause quadratic rescanning.

Zero occurrences returns the fixed no-match error. A second occurrence returns
the fixed ambiguous-match error. Exactly one occurrence constructs the result
once with checked arithmetic as:

```text
preimage before match + new_string + preimage after match
```

The complete result must be at most 49,152 bytes. Construction copies in
batches of at most 8,192 bytes with cancellation checks between batches. Empty
replacement is valid and can produce an empty file when the sole match is the
complete preimage.

Success is exactly:

```json
{"path":"normalized/workspace/path","bytes_written":123}
```

`bytes_written` is the resulting file's UTF-8 byte length, not a character,
match, line, or serialized-JSON count. The complete serialized `ToolOutput`
must remain below 16 KiB.

## Descriptor-relative replacement protocol

On Linux and macOS, allowed execution performs this bounded sequence:

1. Reacquire `.` from the retained workspace descriptor and validate the exact
   platform-specific linked-root identity.
2. Walk only existing parent components through retained descriptor-relative,
   no-follow, close-on-exec, nonblocking directory opens. Record final-parent
   device and inode.
3. Open the final target read-only, no-follow, close-on-exec, and nonblocking.
   It must already be a regular file. Record device, inode, size, nine ordinary
   rwx bits, modification seconds/nanoseconds, and change seconds/nanoseconds.
4. Read at most 49,152 bytes plus one overflow witness in chunks no larger than
   8,192 bytes. Recheck those exact held-descriptor fields after the read; a
   changed initial descriptor view fails before matching. Require valid UTF-8.
5. Find exactly one overlapping-aware exact match and construct one bounded
   postimage as specified above.
6. In the same retained parent, try at most eight high-entropy exclusive
   temporary basenames that cannot equal the target basename. Each stage opens
   read/write, create, exclusive, no-follow, close-on-exec, and nonblocking at
   `0600`. A collision is preserved and consumes one attempt.
7. Write the postimage in at most 8 KiB chunks; verify staged pathname and held-
   descriptor identity and exact size. Apply only the original target's
   `st_mode & 0o777`, sync the staged file, reread it through the held descriptor
   within the same size/chunk bounds, and require exact postimage bytes plus a
   stable identity, size, and ordinary-mode view.
8. Reacquire and validate the root, rewalk the parent, require its recorded
   device and inode, and reopen the target no-follow. Require the original
   device, inode, mode, size, and complete bytes. The verification reread uses
   the same 49,152-byte-plus-witness and 8 KiB-chunk bounds and stable-view
   checks. Revalidate the staged name against the held staged descriptor.
9. Perform the final precommit cancellation check, atomically rename the staged
   regular file over that existing target in the same parent, and cross the
   irreversible boundary.
10. Ignore later tool-level cancellation. Verify by no-follow metadata that the
    published pathname names the staged device/inode with its expected size and
    ordinary mode, sync the parent directory, and return success. A publication-
    verification or parent-sync failure returns nonretryable commit ambiguity
    because new bytes may already be live.

Temporary-name entropy inherits the delivered bounded implementation: Linux
uses direct `rustix` `getrandom` with `NONBLOCK`; macOS uses the pinned one-call
`getentropy` path. Each 16-byte name fill accepts at most 16 cumulative `EINTR`
results and at most 31 calls including partial progress, with cancellation
checked before and after every call. `ENOSYS`, `EPERM`, and `EAGAIN` fail closed
without a fallback or blocking source.

Each initial-read, verification-read, staged-write, staged-file-sync, and post-
rename parent-sync phase accepts at most 16 interrupted syscall results. A
phase's interruption count does not reset after partial progress. Precommit
cancellation checked on the terminal interruption wins; otherwise exhausted
precommit work fails without publication. Once rename succeeds, later
interruption exhaustion is nonretryable commit ambiguity.

At the target pathname, rename provides old-or-new complete visibility rather
than partial bytes. An existing descriptor for the old inode continues to see
the old bytes. Replacing the inode intentionally separates the pathname from
other hard links. The new inode preserves only the original nine ordinary rwx
bits; set-id and sticky bits, ownership, ACLs, extended attributes, timestamps,
and hard-link identity are not preserved.

## Race, cancellation, and cleanup boundary

Target or parent changes observed before the final validation fail closed. The
target is rechecked by identity, ordinary mode, size, stable metadata view, and
complete bytes. This is not portable compare-and-swap isolation: a competing
rename, removal, same-metadata in-place write, or mode change after the final
validation can still be overwritten by the ordinary rename. A retained parent
moved outside the configured workspace after validation can receive
publication. The slice states these limits and does not claim adversarial
concurrent-rename confinement.

Before rename, every failure leaves the target pathname unchanged. Cleanup is
best-effort and identity-checked: one best-effort `fchmod(0600)` is attempted on
the held unpublished staged descriptor before the staged pathname is compared
by no-follow device and inode and unlinked. A preexisting collision or
mismatched pathname is never intentionally removed. Portable metadata-check-to-
unlink retains its final race. If both mode restoration and unlink fail, owned
residue can retain final mode bits; perfect cleanup is not promised.

The execution future is inert until polled, performs bounded synchronous work
on its polling thread, and spawns no detached task, thread, subprocess, timer,
or runtime. Cancellation is checked during both reads, matcher preprocessing
and scan, result construction, root/parent traversal, every staging-name and
entropy attempt, every write chunk, verification, and immediately before
rename. It cannot preempt a native syscall already in flight.

Rename is the irreversible boundary. Once it succeeds, the tool does not
report its own clean cancellation and instead completes parent sync or returns
commit ambiguity. Core's existing same-poll post-effect cancellation recovery
can still persist an unknown-result placeholder; a caller must recover rather
than blindly retry. Dropping an unpolled future is effect-free. Dropping a
polled precommit future publishes no target change and invokes only the bounded
best-effort staging cleanup owned by its retained resources.

## Fixed redacted errors

| Kind | Code | Message | Retryable |
| --- | --- | --- | --- |
| invalid input | `edit_file_invalid_arguments` | `edit_file arguments are invalid` | no |
| invalid input | `edit_file_invalid_path` | `edit_file path is invalid` | no |
| invalid input | `edit_file_text_too_large` | `edit_file text exceeds the supported size limit` | no |
| invalid input | `edit_file_old_string_empty` | `edit_file old_string must not be empty` | no |
| invalid input | `edit_file_strings_identical` | `edit_file old_string and new_string must differ` | no |
| unavailable | `edit_file_unsupported_platform` | `native edit_file is unsupported on this platform` | no |
| unavailable | `edit_file_not_found` | `requested file is unavailable` | no |
| permission denied | `edit_file_permission_denied` | `requested file cannot be edited` | no |
| permission denied | `edit_file_path_rejected` | `requested path is not a confined regular file` | no |
| invalid input | `edit_file_existing_too_large` | `requested file exceeds the supported size limit` | no |
| invalid input | `edit_file_invalid_utf8` | `requested file is not valid UTF-8` | no |
| execution | `edit_file_match_not_found` | `old_string was not found` | no |
| execution | `edit_file_match_ambiguous` | `old_string occurs more than once` | no |
| execution | `edit_file_match_work_exceeded` | `edit_file match work exceeds the supported limit` | no |
| execution | `edit_file_result_too_large` | `edited file exceeds the supported size limit` | no |
| unavailable | `edit_file_unavailable` | `requested file is unavailable` | yes |
| execution | `edit_file_target_changed` | `requested file changed before commit` | yes |
| execution | `edit_file_write_failed` | `requested file could not be edited` | yes |
| execution | `edit_file_commit_ambiguous` | `requested file commit status is uncertain` | no |
| cancelled | `edit_file_cancelled` | `edit_file execution was cancelled` | no |

Preparation-time shape, path, size, empty-old-string, and identical-string
errors occur before policy. Existence, UTF-8, match, result-size, and mutation
errors necessarily occur only after allowed execution reads the target.
Diagnostics and debug output never reflect paths, old/new/preimage/postimage
bytes, match positions, temporary names, errno, credentials, or operating-
system text. `Display` is exactly `<code>: <message>`.

## Reference-host composition

The composed reference host will distribute one retained workspace descriptor
plus six identity-preserving clones and register exactly seven tools in
alphabetical order:

```text
edit_file
file_info
glob_files
grep_files
list_files
read_file
write_file
```

The CLI remains byte-unchanged and thin. The new tool is library-only in this
slice.

## Required independent evidence

Production and independently owned tests must compose before review. Evidence
must cover:

- exact exports, constants, schema and descriptions, error kinds/codes/
  messages/retryability, serialized forms, and debug/display redaction;
- strict shape, exact/one-over path, component, raw text, serialized argument,
  preimage, postimage, matcher-work, chunk, staging-attempt, interruption,
  entropy-call, and serialized-result bounds;
- private production-helper evidence for exact-cap/one-over defensive matcher
  accounting and the 16,384-byte serialized-result guard, because all public
  success payloads are much smaller than that result ceiling;
- effect-free preparation, denial before every filesystem read, exact `Edit`
  capability, canonical policy/execution agreement, and strict direct use;
- beginning/middle/end and Unicode matches, NULs, empty replacement, complete-
  file deletion, zero/one/two matches, and overlapping ambiguity;
- valid maximum, oversize, invalid/growing UTF-8, missing target/parents,
  ancestor/final symlinks, directory, FIFO, socket, device, and other special
  targets without blocking or outside-sentinel changes;
- original rwx preservation under hostile umask, special-bit stripping, inode
  replacement, old-descriptor/new-path visibility, and hard-link behavior;
- retained-root rename/replacement/removal and deterministic target identity,
  mode, size/content, staged-name, and parent race seams;
- every read, match, construction, write, chmod, file-sync, rename, and parent-
  sync fault, including unchanged-target precommit behavior and post-rename
  ambiguity;
- bounded interruptions, entropy partial progress/exhaustion, all eight
  collisions, collision preservation, cleanup swaps, mode restoration, and its
  disclosed dual-failure residue boundary;
- cancellation during both target reads, prefix-table work, matching, result
  construction, traversal, entropy, staging, final verification, immediately
  before rename, unpolled/drop behavior, and engine same-poll recovery;
- exact seven-tool alphabetical host catalog, original-plus-six-clone retained
  identity, native Linux/macOS execution, FreeBSD/WASI compilation, and active
  unsupported-target behavior; and
- complete regression of the delivered `write_file` contract if shared private
  staging/publication mechanics are extracted.

Focused suites run first. The composed behavior candidate must then pass Rust
1.94.1 formatting, workspace all-target/all-feature warnings-denied Clippy,
workspace and documentation tests, repository Python and pinned-compatibility
checks, dependency policy/audit, Linux/macOS native checks, FreeBSD/WASI gates,
documentation links, no-unsafe/diff checks, and a freshly built locked release
CLI smoke.

After production composition, three fresh adversarial agents must review the
same exact behavior SHA for correctness/API, filesystem/robustness, and
performance/concurrency. Every confirmed finding is fixed and restarts all
three fresh tracks on a new exact SHA until all are green. The green behavior
must then receive exact feature-branch workflows, a no-force fast-forward to
`main`, and exact `main` workflows. Documentation-only kickoff, seal, and final
records remain exempt from another adversarial cycle under the user's explicit
instruction.

## Pinned fx input and deliberate differences

Pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` confirms the
required `path`, `old_string`, and `new_string` field names and an exact-text
replacement target. That input is compatibility reconnaissance, not evidence
of current equivalence or delivered behavior.

Machine-god deliberately uses smaller independent bounds, strict unknown-field
rejection, overlapping-match ambiguity, valid-UTF-8 preimages, retained
descriptor-relative confinement, no-follow traversal, existing-file-only
replacement, fixed redacted errors, cancellation/work metering, and explicit
durability and race semantics. It does not adopt pinned fx's 4 MiB limits,
permissive unknown fields, non-overlapping ambiguity rule, invalid-UTF-8 byte
editing, external paths, textual result, or preapproval diff behavior.

This slice adds no external paths, parent creation, missing-file insertion,
binary/alternate-encoding edit, regex, patch/range/multi-edit, append,
formatting, symlink-target access, metadata preservation beyond rwx bits, CLI
change, crash-safe exactly-once recovery, hardened non-Linux/macOS execution,
compatibility-status promotion, benchmark workload, product-performance claim,
or fx-equivalence claim.
