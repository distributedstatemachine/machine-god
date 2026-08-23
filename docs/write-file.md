# Native `write_file` contract

Status: **FROZEN CONTRACT — implementation, independent tests, composition,
formal review, and delivery are pending**

`write_file` is the twentieth Milestone 03 slice. It adds one bounded,
permission-gated workspace mutation without granting parent creation,
external-path access, target-content reads, or general filesystem authority.
The product remains Rust. Zig remains only a pinned upstream benchmark build
input.

## Public API and limits

`machine-god-native` will export `WRITE_FILE_TOOL_NAME`, `WriteFileTool`,
`WriteFileToolOpenError`, `WriteFileToolOpenErrorKind`, and these public limits:

| Limit | Value |
| --- | ---: |
| normalized path bytes | 4,096 |
| normalized path components | 256 |
| raw UTF-8 content bytes | 49,152 |
| serialized argument bytes | 65,536 |
| write chunk bytes | 8,192 |
| temporary-name attempts | 8 |
| serialized result bytes | 16,384 |

The API and fixed unsupported error exist on every target. Filesystem
execution is supported only on Linux and macOS. The native reference host is
already limited to those platforms.

The input schema is exactly:

```json
{
  "type": "object",
  "properties": {
    "path": { "type": "string" },
    "content": { "type": "string" }
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

## Preparation and authority

Preparation is effect-free. It requires the exact tool name and exact two-field
shape, checks the serialized input before retaining canonical arguments,
preserves `content` byte-for-byte, and normalizes `path` with the same lexical
workspace confinement used by `read_file`. Empty paths, absolute paths, parent
components, control or bidi characters, a normalized `.` target, paths over
4,096 UTF-8 bytes, and paths over 256 normalized components are rejected.

Successful preparation returns the canonical arguments in the fixed field
shape and exactly this authority:

```text
Capability::Filesystem {
    access: FilesystemAccess::Write,
    path: normalized_path,
}
```

`Write` already exists in core. It does not imply `Read`, `Create`, metadata on
an external path, directory creation, deletion, or symlink-target access.
Execution decodes the same strict shape again and accepts only the prepared
canonical path, preventing direct execution from widening policy.

## Supported effect

Allowed execution writes one regular file at the normalized workspace path.
Every parent directory must already exist. A missing final target is created;
an existing no-follow regular target is atomically replaced at its pathname.
A stable final symlink, directory, FIFO, socket, device, or other special
object is rejected. No selected symlink is followed. Existing target content is
never opened or read, even when the replacement bytes are identical. An
identical write deliberately replaces the inode.

Success is exactly:

```json
{
  "path": "normalized/workspace/path",
  "bytes_written": 123
}
```

The complete serialized `ToolOutput` remains below 16 KiB.

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

The execution future is inert until polled and spawns no detached work.
Cancellation is checked during traversal, every temporary attempt, every write
chunk, verification, and immediately before rename. Rename is the irreversible
boundary. Core can still observe same-poll cancellation after a ready tool
effect and retain its durable unknown-result placeholder; callers must use the
existing recovery model rather than retry an ambiguous mutation automatically.

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
temporary names, credentials, or operating-system text. Final-target absence is
valid create; `not_found` refers to a missing ancestor or final parent.

## Host integration and evidence

The reference host will distribute one retained workspace descriptor plus five
identity-preserving clones and register exactly six tools alphabetically:
`file_info`, `glob_files`, `grep_files`, `list_files`, `read_file`, and
`write_file`.

Independent tests must cover exact schema/exports/descriptions, raw and
serialized boundaries including escape-heavy JSON, normalization and exact
policy/execution agreement, empty/NUL content, create/replace/no-op replacement,
umask-independent `0644`, rwx preservation and special-bit stripping, missing
parents, symlinks/specials, retained-root replacement, atomic descriptor
observations, all eight collisions, cleanup name swaps, target and parent races,
write/chmod/file-sync/rename/directory-sync faults, cancellation at every phase,
engine denial/allow/result recovery, the six-tool catalog, Linux/macOS behavior,
and active unsupported evidence on WASI plus FreeBSD/WASI compilation.

Pinned fx confirms only the required `path` and `content` field names. Its 4 MiB
content allowance, external paths, parent creation, and permissive unknown-field
behavior are deliberate differences. This slice makes no fx-equivalence or
product-performance claim.

