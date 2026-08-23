# Native `write_file` contract

Status: **FROZEN CONTRACT — implementation, independent tests, composition,
formal review, and delivery are pending**

The contract commit is
`3ee52fd8393bfb86f11048eaa6c624bd18a78798`. Its exact feature CI run
`32626410935` and benchmark-evidence run `32626410931` are green. Those runs
validate only the contract kickoff; they are not implementation, behavior-
review, or final delivery evidence.

`write_file` is the twentieth Milestone 03 slice. It adds one bounded,
permission-gated workspace mutation without granting parent creation,
external-path access, target-content reads, or general filesystem authority.
The product remains Rust. Zig remains only a pinned upstream benchmark build
input.

## Public API, schema, and limits

`machine-god-native` will export `WRITE_FILE_TOOL_NAME`, `WriteFileTool`,
`WriteFileToolOpenError`, `WriteFileToolOpenErrorKind`, and these public limits:

| Public constant | Exact value |
| --- | ---: |
| `WRITE_FILE_TOOL_NAME` | `"write_file"` |
| `MAX_WRITE_FILE_PATH_BYTES` | `4,096` |
| `MAX_WRITE_FILE_PATH_COMPONENTS` | `256` |
| `MAX_WRITE_FILE_CONTENT_BYTES` | `49,152` |
| `MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_WRITE_FILE_CHUNK_BYTES` | `8,192` |
| `MAX_WRITE_FILE_TEMP_ATTEMPTS` | `8` |
| `MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES` | `16,384` |

The exact tool description is
`Write one file within the configured workspace`. The exact property
descriptions are `Workspace-relative file path` for `path` and
`UTF-8 content to write` for `content`.

The API and fixed unsupported error exist on every target. Filesystem
execution is supported only on Linux and macOS. The native reference host is
already limited to those platforms.

The input schema is exactly:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Workspace-relative file path"
    },
    "content": {
      "type": "string",
      "description": "UTF-8 content to write"
    }
  },
  "required": ["path", "content"],
  "additionalProperties": false
}
```

Both fields are required and have no defaults. Empty content and NUL bytes in
the Rust string are valid. The raw-content and serialized-argument limits are
independent: JSON escaping and path overhead can make an input with at most 48
KiB of raw content exceed 64 KiB when serialized. Such an input is rejected as
invalid arguments before permission policy. This mirrors the existing core and
AI Gateway serialized-argument ceiling; it does not promise every raw-valid
direct input is admissible through an engine/provider boundary.

`WriteFileTool::open` accepts the explicitly injected absolute workspace used
by the other confined native tools. Its complete fixed construction taxonomy
is:

| `WriteFileToolOpenErrorKind` | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native write_file is unsupported on this platform` |
| `InvalidRoot` | `native write_file workspace root is invalid` |
| `InvalidFileType` | `native write_file workspace root is not a directory` |
| `Unavailable` | `native write_file workspace root is unavailable` |

The construction error retains only its kind. Its `Display` and `Debug` forms
must not retain the injected workspace path, operating-system text, or a raw
error number. The tool and error types remain public on every target, while a
safe public tool instance cannot be constructed on an unsupported target.

## Preparation and authority

Preparation is effect-free. It requires the exact tool name and exact two-field
shape, checks the serialized input before retaining canonical arguments,
preserves `content` byte-for-byte, and normalizes `path` with the same lexical
workspace confinement used by `read_file`. Repeated `/` separators collapse and
exact `.` components are removed. Backslash and space are literal Unix filename
characters, so Windows-looking input remains one confined Unix path spelling
rather than an external prefix. Empty paths, absolute paths, parent components,
C0/C1 controls,
Unicode line/paragraph separators, bidirectional-formatting characters, and a
normalized `.` target are rejected. The requested and normalized forms are
independently bounded to 4,096 UTF-8 bytes, and the normalized form is bounded
to 256 components.

Successful preparation returns the canonical arguments in the fixed field
shape and exactly this authority:

```text
Capability::Filesystem {
    access: FilesystemAccess::Write,
    path: normalized_path,
}
```

`Write` already exists in core and remains distinct from the core `Create`
access kind. For this exact tool, one allowed `Write` invocation authorizes
either replacement or missing-final-file creation under the contract below; it
does not grant general creation authority to another tool. It grants no read,
external-path metadata, parent-directory creation, deletion, or symlink-target
access. Execution decodes the same strict shape again, reapplies both content
and serialized-argument caps, and accepts only an already canonical path.
Direct execution therefore cannot widen policy or bypass a public resource
limit.

For example, provider input:

```json
{"path":"./notes//today.txt","content":"first line\nsecond line\n"}
```

prepares exact execution arguments:

```json
{"path":"notes/today.txt","content":"first line\nsecond line\n"}
```

and requests only `FilesystemAccess::Write` for `notes/today.txt`. This is a
library example, not a new CLI invocation.

## Supported effect

Allowed execution has two target-state modes and no caller-selectable mode
field:

| Observed final state | Allowed effect | Final ordinary rwx bits |
| --- | --- | --- |
| missing | create one regular file without replacing a raced entry | exact `0644` |
| existing no-follow regular file | atomically replace its pathname | initially observed `st_mode & 0o777` |

Every parent directory must already exist. A stable final symlink, directory,
FIFO, socket, device, or other special object is rejected. No selected symlink
is followed. Existing target content is never opened or read, even when the
replacement bytes are identical. An identical write deliberately replaces the
inode.

Success is exactly:

```json
{
  "path": "normalized/workspace/path",
  "bytes_written": 123
}
```

The complete serialized `ToolOutput` remains below 16 KiB.

For the example above, a successful two-line write reports the byte length of
the complete content rather than a character, line, or serialized JSON count:

```json
{"path":"notes/today.txt","bytes_written":23}
```

## Descriptor-relative commit protocol

On Linux and macOS, execution performs this bounded sequence:

1. Reacquire `.` from the retained workspace descriptor and validate the
   platform-specific linked-root identity.
2. Walk only existing parent components through retained, descriptor-relative,
   no-follow directory opens. Record the final parent device and inode.
3. Inspect the final target with no-follow descriptor-relative metadata. Record
   missing state, or the regular target's device, inode, and nine ordinary rwx
   bits. Do not open or read the target.
4. In that exact parent, try at most eight fixed-short, high-entropy temporary
   basenames that cannot equal the target basename. Open with write-only,
   create, exclusive, no-follow, close-on-exec, and nonblocking flags at mode
   `0600`. A collision is never deleted and consumes one attempt.
5. Write in chunks of at most 8 KiB with cancellation and exact-byte checks.
   Verify the staged descriptor identity and byte count. Apply final ordinary
   rwx bits with `fchmod`, then `fsync` the staged file.
6. Reacquire and validate the root, rewalk the parent, require its device and
   inode to match, and revalidate the target state, regular type, identity, and
   observed rwx bits. Revalidate the staged pathname against the held staged
   descriptor.
7. Perform the final cancellation check. For an initially missing target,
   publish with `renameat_with(..., RenameFlags::NOREPLACE)` and fail closed if
   that primitive is unsupported. For a validated existing target, use an
   ordinary same-parent atomic rename.
8. After rename succeeds, do not return the tool's own cancellation error.
   `fsync` the parent directory. Failure here returns nonretryable
   `write_file_commit_ambiguous`, because the new pathname may already be live.

At the target pathname, rename gives old-or-new visibility rather than a
partial file. The temporary pathname can be visible to another actor that can
enumerate the parent, but remains private `0600` while bytes are written.

New files receive exact POSIX ordinary rwx bits `0644` via `fchmod`, independent
of process umask. Replacements copy only the initially observed
`st_mode & 0o777`; set-id and sticky bits are stripped. Target rwx changes before
revalidation cause `write_file_target_changed`. Replacement does not preserve
the old inode, ownership, ACLs, extended attributes, timestamps, or the effect
of replacement on other hard links.

The observable lifecycle is therefore:

| Last completed stage | Tool outcome | Target-path guarantee |
| --- | --- | --- |
| before publish | fixed precommit error or cancellation | the tool publishes no target-path change |
| rename and parent sync | success | complete new bytes are live and directory entry is synced |
| rename, then parent-sync failure | `write_file_commit_ambiguous` | complete new bytes may already be live |
| ready effect followed by same-poll core cancellation | core cancellation with durable unknown placeholder | caller must recover rather than retry automatically |

Atomicity applies to visibility at the target pathname. It is not transaction
isolation from concurrent writers and does not preserve the replaced inode or
its non-mode metadata.

## Races and cleanup boundary

Creation uses `NOREPLACE`, so a target that appears after the missing
observation is not overwritten. Portable replacement is not inode compare-and-
swap: after final validation, another actor can replace or remove the target
before rename, and the ordinary rename can replace the new entry or create the
name. A final parent can also be renamed outside the workspace after the
identity rewalk; descriptor-relative publication can then land in that retained
moved directory. Rename never follows a symlink, but the slice does not claim
adversarial concurrent-rename confinement.

Before rename, failures leave the target pathname unchanged. Cleanup is
best-effort and identity-checked: the staged name is compared by no-follow
device and inode with the held staged descriptor before unlink is attempted.
The implementation never intentionally removes a mismatched entry or a
preexisting collision. Portable metadata-check-to-unlink still has a final
race, and cleanup failure or identity disagreement may leave a private
temporary residue. Perfect identity-safe unlink would require platform-specific
primitives not shared by Linux and macOS and is not promised.

The execution future is inert until polled, performs bounded synchronous work
on the polling thread, and spawns no task, thread, subprocess, timer, or other
detached work. Cancellation is checked during root/parent traversal, every
temporary attempt, every write chunk, staged and target verification, and
immediately before rename. It cannot preempt an individual open, metadata,
write, chmod, rename, unlink, or sync syscall already in flight.

Rename is the irreversible boundary. Once it succeeds, the tool ignores its own
later cancellation observation, completes parent sync, and returns success or
commit ambiguity. Core can still observe same-poll cancellation after a ready
tool effect and retain its durable unknown-result placeholder; callers must use
the existing recovery model rather than retry an ambiguous mutation
automatically. Dropping an unpolled future is effect-free. A prepublish exit
after staging releases owned descriptors and invokes only the documented best-
effort staging cleanup; the tool publishes no target change.

## Fixed redacted errors

| Kind | Code | Message | Retryable |
| --- | --- | --- | --- |
| invalid input | `write_file_invalid_arguments` | `write_file arguments are invalid` | no |
| invalid input | `write_file_invalid_path` | `write_file path is invalid` | no |
| invalid input | `write_file_content_too_large` | `write_file content exceeds the supported size limit` | no |
| unavailable | `write_file_unsupported_platform` | `native write_file is unsupported on this platform` | no |
| unavailable | `write_file_not_found` | `requested parent directory is unavailable` | no |
| permission denied | `write_file_permission_denied` | `requested file cannot be written` | no |
| permission denied | `write_file_path_rejected` | `requested path is not a confined regular file target` | no |
| unavailable | `write_file_unavailable` | `requested file is unavailable` | yes |
| execution | `write_file_target_changed` | `requested file changed before commit` | yes |
| execution | `write_file_write_failed` | `requested file could not be written` | yes |
| execution | `write_file_commit_ambiguous` | `requested file commit status is uncertain` | no |
| cancelled | `write_file_cancelled` | `write_file execution was cancelled` | no |

Diagnostics and debug output never reflect path, content, errno, generated
temporary names, credentials, or operating-system text. `Display` is exactly
`<code>: <message>`. Final-target absence is valid create; `not_found` refers to
a missing ancestor or final parent. A failure after the atomic rename is never
reported as retryable because blind retry could repeat a mutation whose commit
status is uncertain.

## Reference-host composition

The reference host will distribute one retained workspace descriptor plus five
identity-preserving clones and register exactly six tools alphabetically:
`file_info`, `glob_files`, `grep_files`, `list_files`, `read_file`, and
`write_file`.

This is pending composition. Until the production and independent-test
components are integrated, the delivered reference host continues to expose
the existing five-tool catalog and no documentation may claim otherwise.

## Independent evidence checklist

Every item is pending on this frozen-contract commit:

- [ ] Exact exports, constant values, tool/schema descriptions, strict shape,
  fixed construction/tool errors, `Display`, and debug redaction.
- [ ] Requested and normalized path byte boundaries, component boundary,
  forbidden forms, raw-content exact/one-over, serialized-argument exact/one-
  over including escape-heavy JSON, and serialized-result bound.
- [ ] Preparation is effect-free; denial produces no filesystem effect; policy
  and execution observe the same normalized path and exact canonical content.
- [ ] Empty, NUL-containing, Unicode, and maximum-sized content round-trip
  byte-for-byte with exact `bytes_written`.
- [ ] Missing-target creation, existing-target replacement, identical-content
  inode replacement, old-descriptor/new-path atomic visibility, and exact result
  shapes.
- [ ] New files are `0644` under hostile umask; replacements preserve only the
  nine ordinary rwx bits and strip set-id/sticky bits.
- [ ] Parents are never created; ancestor/final symlinks and directory, FIFO,
  socket, device, and other special targets fail closed and outside sentinels
  remain unchanged.
- [ ] Retained-root removal/replacement, target appearance/replacement, final-
  parent identity change/movement, and the disclosed final validation-to-rename
  races are exercised through deterministic seams.
- [ ] Eight temporary-name collisions exhaust exactly; foreign collisions are
  preserved; staged-name replacement and cleanup-name swaps never intentionally
  remove a mismatched sentinel; possible residue is accounted for.
- [ ] Injected write, chmod, staged-file sync, rename, and parent-directory sync
  failures prove precommit unchanged-target behavior and post-rename commit
  ambiguity.
- [ ] Cancellation is exercised at traversal, temporary-attempt, write,
  verification, and final-precommit boundaries; unpolled/drop behavior and the
  core same-poll durable unknown-result path are covered.
- [ ] Engine deny/allow events and durable results, exact six-tool alphabetical
  catalog, and original-plus-five-clones workspace identity all pass.
- [ ] Linux and macOS execute the native behavior; FreeBSD and WASI compile;
  an active unsupported-target test reaches the fixed public construction
  failure without fabricating an unsafe instance.

Passing only a subset of this list does not establish delivery. The exact
composed behavior SHA must also pass repository gates and three fresh
adversarial tracks as recorded in the
[`write_file` review](reviews/m03-write-file-review-01.md).

## Pinned fx input and deliberate differences

Pinned fx confirms only the required `path` and `content` field names. Its 4 MiB
content allowance, external paths, parent creation, and permissive unknown-field
behavior are deliberate differences. This slice makes no fx-equivalence or
product-performance claim.

Machine-god additionally fixes strict decoding, smaller independent input
bounds, retained descriptor-relative confinement, no-follow handling,
existing-parent-only mutation, same-directory staging, explicit durability,
fixed redacted errors, and the race/cleanup boundary above. None of those
differences should be normalized away merely to resemble upstream behavior.

This slice adds no CLI or slash command, parent creation, external path,
symlink-target write, metadata-only edit, append mode, patch mode, ownership/
ACL/xattr preservation, non-Linux/macOS hardened execution, benchmark workload,
compatibility-status change, or product-performance claim. The component and
delivery lineage remains pending in the
[`write_file` review](reviews/m03-write-file-review-01.md).
