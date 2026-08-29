# Native `delete_file` contract

`delete_file` deletes exactly one existing confined regular file or empty
directory. It does not recurse, follow a symlink, remove the workspace root,
read file content, enumerate a directory, create a path, or grant general
filesystem authority. It is library-only. The product remains
Rust; Zig remains solely a pinned upstream benchmark build input.

## Public API, schema, and limits

`machine-god-native` exports `DELETE_FILE_TOOL_NAME`, `DeleteFileTool`,
`DeleteFileToolOpenError`, `DeleteFileToolOpenErrorKind`, and these public
limits:

| Public constant | Exact value |
| --- | ---: |
| `DELETE_FILE_TOOL_NAME` | `"delete_file"` |
| `MAX_DELETE_FILE_PATH_BYTES` | `4,096` |
| `MAX_DELETE_FILE_PATH_COMPONENTS` | `256` |
| `MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES` | `16,384` |

The exact tool description is
`Delete one regular file or empty directory within the configured workspace`.
The exact `path` property description is
`Workspace-relative file or empty-directory path`.

The advertised schema and preparation input are exactly:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Workspace-relative file or empty-directory path"
    }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

`path` must be a string. It has no default, and no unknown field is accepted.
The requested path's lexical validity and 4,096-byte bound are checked before
the serialized argument object is independently capped at 65,536 bytes. The
remaining canonical path and component bounds follow normalization before
canonical arguments are retained. Direct execution revalidates the same exact
shape and precedence, so bypassing provider preparation cannot widen authority
or impose unbounded serialized-value work through an over-limit path string.

`DeleteFileTool::open` accepts one explicitly injected absolute workspace root.
Its complete fixed construction taxonomy is:

| `DeleteFileToolOpenErrorKind` | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native delete_file is unsupported on this platform` |
| `InvalidRoot` | `native delete_file workspace root is invalid` |
| `InvalidFileType` | `native delete_file workspace root is not a directory` |
| `Unavailable` | `native delete_file workspace root is unavailable` |

Construction errors retain only their kind. Their `Display` and `Debug` forms
must not retain the injected root, operating-system text, or a raw error number.
The public API and fixed unsupported behavior exist on every target, while
native execution is supported only on Linux and macOS.

## Preparation and authority

Preparation is deterministic, synchronous, bounded, nonblocking, and effect-
free. It performs strict JSON validation and the exact lexical mutation-path
normalization used by `write_file` and `edit_file`; it does not inspect
the retained root or requested target.

Repeated `/` separators collapse and exact `.` components are removed.
Backslash and space remain literal Unix filename characters. Empty paths,
absolute paths, any `..` component, C0/C1 controls, Unicode line or paragraph
separators, Unicode bidirectional-formatting characters, and a canonical `.`
target are rejected. Rejecting `.` makes workspace-root removal impossible.
Both the requested UTF-8 spelling and canonical path are independently capped
at 4,096 bytes, and the canonical path is capped at 256 components.

Successful preparation retains exactly `{"path":"<normalized>"}` and returns:

```text
Capability::Filesystem {
    access: FilesystemAccess::Delete,
    path: normalized_path,
}
```

`FilesystemAccess::Delete` is the existing provider-neutral deletion authority.
Policy and allowed execution receive the exact same canonical path. A denied or
failed preparation performs no filesystem lookup, deletion, or sync.

## Supported deletion protocol

Allowed Linux/macOS execution uses only the workspace descriptor retained at
construction. Root construction uses the existing lexical host-root handling,
final-component no-follow open, and exact linked-root validation. The host path
string is not reopened as authority after construction.

For one allowed call, execution performs this bounded sequence:

1. Check cancellation, acquire `.` descriptor-relatively from the retained
   workspace descriptor, and validate the exact platform-specific linked-root
   identity.
2. Walk every existing parent component through retained descriptor-relative
   directory opens with no-follow, nonblocking, and close-on-exec requirements.
   Record the final parent's device, inode, and directory type.
3. Inspect the target basename once with no-follow descriptor-relative
   metadata. Record its device, inode, and exact regular-file or directory type.
   A symlink or any other special type is rejected. The target is not opened,
   its content is not read, and a directory is not enumerated.
4. Reacquire and revalidate the linked root, completely rewalk the existing
   parent path, and require the final parent identity and directory type to
   equal the initial observation. Reinspect the target with no-follow metadata
   and require the same device, inode, and exact type.
5. Perform the final cancellation check immediately after final validation and
   immediately before one deletion syscall. Invoke exactly one `unlinkat`: a
   regular file uses empty flags and a directory uses the platform `REMOVEDIR`
   flag. Never retry that `unlinkat`, including after `EINTR`.
6. Once `unlinkat` reports success, ignore later tool cancellation and `fsync`
   the retained final-parent descriptor. The complete parent-sync phase accepts
   at most 16 cumulative interrupted results. The sixteenth interruption or
   any other sync failure returns nonretryable commit ambiguity.

An `unlinkat` `EINTR` is itself nonretryable commit ambiguity because the
directory entry may already have been removed. The tool makes no second delete
attempt. It best-effort syncs the retained parent under the same 16-interruption
bound before returning ambiguity, because a deletion that may have committed
still needs a durability attempt. Later cancellation does not replace that
ambiguous result.

A directory is never enumerated to predict emptiness. `unlinkat` with
`REMOVEDIR` is the authoritative empty-directory operation. A nonempty result
maps to the fixed nonretryable directory-not-empty error and leaves the entry
present. Regular-file deletion removes exactly the selected directory entry;
it does not destroy other hard links or invalidate already open descriptors.

Success is exactly:

```json
{"path":"normalized/workspace/path"}
```

No kind, byte count, inode, or deletion count is returned. The result is
defensively checked against the 16,384-byte serialized-result cap even though
the bounded canonical path makes a valid success structurally smaller.

## Commit boundary, cancellation, and races

The sole `unlinkat` call is the irreversible boundary. Before it is invoked,
every tool failure or cancellation publishes no deletion. A successful call
may have removed the entry even when later parent sync fails, so sync failure
is never retryable. Likewise, `EINTR` does not prove the syscall had no effect.
Callers must recover by observation instead of blindly retrying a commit-
ambiguous deletion.

The execution future is inert until polled, performs bounded synchronous work
on its polling thread, and starts no detached task, thread, subprocess, timer,
or runtime. Cancellation is checked before and after root acquisition, around
each parent open and metadata operation, after initial and final validation,
and at the exact final pre-`unlinkat` boundary. Every root/parent open and
metadata after-check runs even when that operation returns an error;
cancellation then takes precedence over the saved noncommit error. It cannot
preempt a syscall already in flight. Once the delete call has succeeded or
returned `EINTR`, later tool cancellation is ignored while the bounded parent
durability attempt completes. Dropping an unpolled future is effect-free;
dropping at a documented precommit boundary closes owned descriptors without
deleting anything.

When `unlinkat` returns a definitive non-`EINTR` failure, the target was not
committed by that outcome. Cancellation is therefore checked immediately after
the syscall/evidence hook and before errno or macOS diagnostic mapping; observed
cancellation wins as `delete_file_cancelled`. The failed call is not retried or
synced. Only success and `EINTR` cross the cancellation-ignore boundary.

This protocol is not pathname compare-and-swap. A same-directory actor can
replace the target after final no-follow validation and before `unlinkat`.
Because portable `unlinkat` accepts only a parent descriptor and name, a
regular-file deletion using empty flags may remove any non-directory entry
installed in that final window, including a different regular file, symlink,
FIFO, or Unix-domain socket. It never follows a replacement symlink, and the
symlink referent and unrelated sentinels remain untouched. A directory
replacement presented to the file-class call fails the flags/type boundary;
the inverse file-class replacement presented to a `REMOVEDIR` call likewise
fails. The directory-class call may still remove a different empty directory.
These portable final-window limits are disclosed and tested; the contract makes
no stronger adversarial concurrent-mutation or final-entry-type claim.

A retained final parent can be renamed outside the public workspace path after
validation. Descriptor-relative `unlinkat` can then remove the entry in that
same retained moved directory. Replacing the old public parent pathname does
not redirect the retained descriptor. Other hard links and already open file or
directory descriptors survive deletion as defined by the operating system. A
concurrent actor may also recreate the pathname immediately after successful
deletion, so success does not promise that a later lookup is absent.

There is no staged file, temporary name, content buffer, ACL operation, chmod,
rename, or cleanup-unlink protocol in this tool. Those `write_file` and
`edit_file` mechanisms are intentionally inapplicable to a one-entry delete.

## Fixed redacted errors

Preparation and direct execution use this complete fixed taxonomy. `Display`
is exactly `<code>: <message>`.

| Kind | Code | Message | Retryable |
| --- | --- | --- | --- |
| invalid input | `delete_file_invalid_arguments` | `delete_file arguments are invalid` | no |
| invalid input | `delete_file_invalid_path` | `delete_file path is invalid` | no |
| unavailable | `delete_file_unsupported_platform` | `native delete_file is unsupported on this platform` | no |
| unavailable | `delete_file_not_found` | `requested path is unavailable` | no |
| permission denied | `delete_file_permission_denied` | `requested path cannot be deleted` | no |
| permission denied | `delete_file_path_rejected` | `requested path is not a confined regular file or empty directory` | no |
| execution | `delete_file_directory_not_empty` | `requested directory is not empty` | no |
| unavailable | `delete_file_unavailable` | `requested path is unavailable` | yes |
| execution | `delete_file_target_changed` | `requested path changed before deletion` | yes |
| execution | `delete_file_delete_failed` | `requested path could not be deleted` | yes |
| execution | `delete_file_commit_ambiguous` | `requested path deletion status is uncertain` | no |
| cancelled | `delete_file_cancelled` | `delete_file execution was cancelled` | no |

An initially absent target or ancestor is `delete_file_not_found`. Absence or a
type mismatch during final revalidation or the delete call is
`delete_file_target_changed`. A nonempty directory is
`delete_file_directory_not_empty`. Permission and read-only-filesystem failures
are `delete_file_permission_denied` at every root, parent, and target operation
in either validation phase. `unlinkat` interruption and any failure after
successful deletion are `delete_file_commit_ambiguous`; `unlinkat` is never
retried. Other bounded operational failures map to the fixed unavailable,
target-changed, or delete-failed category according to the documented phase.

On macOS, empty-flag `unlinkat` reports `EPERM` for both a genuine permission
failure and a final-window file-to-directory replacement. The bounded
diagnostic no-follow metadata operation compares the complete observed target
identity and type with the validated regular-file identity. Cancellation wins;
absence, a type-change errno, or any observed identity/type mismatch is
`delete_file_target_changed`; an exact unchanged identity, diagnostic
`EACCES`/`EPERM`, or another diagnostic OS error preserves the original
`delete_file_permission_denied`. The diagnostic never retries deletion.

No public error or tool `Debug` form reflects the requested/canonical path,
workspace root, device or inode, file type, raw errno, operating-system text,
credentials, or another directory entry. Non-cancellation errors passing
through the engine retain core's existing generic durable error behavior.

## Reference-host composition

Current reference-host composition and retained-workspace descriptor
distribution are maintained in the
[canonical tool catalog](native-reference-host.md#tool-catalog).

The CLI remains byte-unchanged and thin. This tool adds no CLI command,
invocation path, prompt, status field, or output byte.

## Required independent evidence

Evidence must cover:

- exact exports, constants, schema/property descriptions, strict shape,
  construction and tool errors, retryability, result, `Display`, and redaction;
- exact and one-over requested-path, canonical-path, component, serialized-
  argument, and serialized-result boundaries;
- effect-free preparation, denial before any lookup, exact `Delete` authority,
  and policy/direct-execution agreement on the canonical path;
- deletion of regular files and empty directories, including hostile umask,
  Unicode and literal-backslash names, with no content read or enumeration;
- root, missing path/ancestor, nonempty directory, every symlink position,
  FIFO, socket, device, and other special rejection without blocking or outside
  sentinel changes;
- retained-root rename, replacement, and removal; complete initial/final parent
  and target identity/type revalidation; retained-parent movement outside the
  public workspace; final same-class and file-to-symlink/FIFO/socket replacement
  races with referent/sentinel preservation; hard-link/open-descriptor
  survival; and immediate pathname recreation;
- production-routed root/intermediate-open, ordinal `fstat`/`statat`,
  `unlinkat`, and parent-sync faults with exact precommit, committed, and
  ambiguous mappings;
- exact empty versus `REMOVEDIR` flags, exactly one delete call on success and
  `EINTR`, no delete retry, and cumulative 16-interruption parent-sync handling;
- cancellation at every traversal/metadata boundary, immediately after both
  validation phases, immediately before the real delete, and after the real
  delete, plus unpolled/drop and engine same-poll unknown-result recovery;
- canonical reference-host composition and retained-workspace identity;
- native Linux/macOS execution, FreeBSD/WASI compilation, active unsupported-
  target behavior, no unsafe Rust, and complete regression of the existing workspace tools.

## Pinned fx input and deliberate differences

Pinned fx revision `b1774fbf6c7602b503026f96f6e960e946c692ef` was observed
to expose a required delete path and accept both a regular file and an empty
directory. Its implementation is broader and pathname-based. This observation
is compatibility reconnaissance only, not an equivalence or delivery claim.

Machine-god deliberately uses strict unknown-field rejection, smaller
independent limits, effect-free canonical preflight, exact deletion authority,
retained descriptor-relative no-follow confinement, complete bounded
revalidation, explicit non-CAS race disclosure, fixed redacted errors,
cancellation boundaries, one non-retried delete call, and explicit durability
ambiguity. It does not adopt upstream external paths or broader pathname
behavior.

This contract adds no recursion, parent or target creation, wildcard or multi-
path deletion, trash/recovery mode, secure erasure, symlink-target mutation, content
read, directory enumeration, external path, CLI change, new dependency,
non-Linux/macOS hardening, compatibility promotion, benchmark workload,
product-performance claim, or fx-equivalence claim.
