# Native `rename_file` contract

Status: **LOCAL GATE GREEN — FORMAL REVIEW PENDING**

This document freezes the twenty-third bounded Milestone 03 slice from exact
delivered base `3d76f2e844312e7f3e809524cb72c1a7957975ff`. That base is
green under exact feature CI `32665981665`, feature benchmark workflow
`32665981641`, main CI `32666261656`, and main benchmark workflow
`32666261525`. Both benchmark workflows retained two nonexpired exact-SHA
artifacts.

The frozen contract is commit
`19cad7d10a8fc885e2e70a7345fc0ba27d76872a`. Exact contract benchmark
workflow `32667647846` is green with both jobs and two exact-SHA artifacts;
contract CI `32667647822` is not yet complete and is not claimed as green.
Production composes on the feature branch at
`d8f73676fcfce2cead385fa5b36598da989abe8f`, and independent evidence
composes at `1dab9a0dfcb4ec2d204625c744171ae923cca458`. Exact composed
local-gate precursor `43847fe5fd405e8b1d28808f0495dac859ebab15`, tree
`80cb9a17d9bb2c1151bc43b72faebcb305dd78c2`, is green. Fresh same-SHA review
cycles, feature delivery, fast-forward main integration, and exact main
workflows remain pending.

`rename_file` moves one existing regular file between two confined names while
preserving the file object. It does not read content, overwrite a destination,
create a parent, move a directory, follow a symlink, access an external path,
or fall back to copy-and-delete. It is library-only in this slice. The product
remains Rust; Zig remains solely a pinned upstream benchmark build input.

## Public API and schema

`machine-god-native` exports `RENAME_FILE_TOOL_NAME`, `RenameFileTool`,
`RenameFileToolOpenError`, `RenameFileToolOpenErrorKind`, and these limits:

| Public constant | Exact value |
| --- | ---: |
| `RENAME_FILE_TOOL_NAME` | `"rename_file"` |
| `MAX_RENAME_FILE_PATH_BYTES` | `4,096` |
| `MAX_RENAME_FILE_PATH_COMPONENTS` | `256` |
| `MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES` | `16,384` |

The exact tool description is
`Rename one existing regular file to an absent path within the configured workspace`.
Property descriptions are `Current workspace-relative regular-file path` and
`New workspace-relative file path` for `old_path` and `new_path`.

The exact input schema is:

```json
{
  "type": "object",
  "properties": {
    "old_path": {
      "type": "string",
      "description": "Current workspace-relative regular-file path"
    },
    "new_path": {
      "type": "string",
      "description": "New workspace-relative file path"
    }
  },
  "required": ["old_path", "new_path"],
  "additionalProperties": false
}
```

Both fields are required strings with no defaults. Unknown fields are invalid.
Each requested and canonical path is independently capped at 4,096 UTF-8 bytes
and 256 canonical components. The complete requested and prepared JSON values
are independently capped at 65,536 serialized bytes. Canonical endpoints must
differ. Direct execution revalidates the same exact shape and requires both
paths already canonical.

Normalization is the delivered mutation-path rule: repeated `/` separators
collapse and exact `.` components disappear. Backslash and space remain literal
Unix filename characters. Empty paths, absolute paths, any `..` component,
C0/C1 controls, Unicode line or paragraph separators, Unicode bidirectional-
formatting characters, and a canonical `.` endpoint are rejected.

Construction accepts one injected absolute workspace directory. The public API
and fixed unsupported result exist on every target; execution is supported only
on Linux and macOS. Construction errors retain only their kind:

| Kind | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native rename_file is unsupported on this platform` |
| `InvalidRoot` | `native rename_file workspace root is invalid` |
| `InvalidFileType` | `native rename_file workspace root is not a directory` |
| `Unavailable` | `native rename_file workspace root is unavailable` |

`Display` and `Debug` never retain an endpoint, injected root, operating-system
text, or raw error number.

## Preparation and authority

Preparation is deterministic, synchronous, bounded, nonblocking, and effect-
free. It performs no lookup, read, creation, mutation, or root inspection.
Successful preparation retains exactly the two canonical fields and returns:

```text
Capability::FilesystemRename {
    old_path: canonical_old_path,
    new_path: canonical_new_path,
}
```

Core serializes this provider-neutral capability with the exact tag
`filesystem_rename`. Both endpoints are part of the policy input because a
single-path filesystem capability cannot authorize the complete effect. Policy
and allowed execution receive the same canonical pair. Denied or failed
preparation has no filesystem effect.

## Supported rename protocol

Allowed Linux/macOS execution uses only the retained workspace descriptor and
per-call owned descriptors. The injected pathname is never reopened as
authority. For one call it performs this bounded sequence:

1. Check cancellation, acquire `.` descriptor-relatively, and validate the
   exact linked workspace identity.
2. Walk the source and destination parent paths through descriptor-relative,
   no-follow, nonblocking directory opens. Both parents must already exist.
3. Record both parent identities. Inspect the source with no-follow metadata
   and require a regular file. Inspect the destination with no-follow metadata
   and require `ENOENT`; every existing entry type is a conflict.
4. Reacquire and revalidate the linked root, completely rewalk both parents,
   require the same parent identities and source device/inode/regular type, and
   require the destination still absent.
5. Perform the final cancellation check immediately before exactly one
   `renameat_with` using `RenameFlags::NOREPLACE`. The rename is never retried,
   including after `EINTR`.
6. After success, ignore later tool cancellation. Inspect the destination with
   no-follow metadata and require the original source device, inode, and
   regular-file type. Sync the source parent, then the destination parent. If
   both identities are equal, sync that directory once. Each unique parent
   allows at most 16 cumulative `fsync` calls, including interrupted calls;
   distinct parents are both attempted even if the first fails.

Success is exactly:

```json
{"old_path":"old/name","new_path":"new/name"}
```

The complete `ToolOutput` is defensively capped at 16,384 serialized bytes.
No content, size, metadata, device, inode, or timing value is returned.

The destination is never overwritten. Missing destination parents are not
created. Cross-filesystem or kernel/filesystem lack of no-replace support fails
without copy/delete fallback. The implementation allocates no content buffer,
temporary name, staging file, backup, or cleanup residue.

## Commit boundary, cancellation, and races

The sole no-replace rename syscall is the irreversible boundary. Before it is
invoked, cancellation or failure publishes no rename. A successful call may
have committed even if post-rename verification or parent sync fails. Such a
failure is nonretryable commit ambiguity. `EINTR` is also ambiguity because it
does not prove the rename was absent; the call is never repeated and both
parents receive the same bounded best-effort sync.

`NOREPLACE` closes the destination replacement race, but portable Linux/macOS
rename has no inode compare-and-swap for the source. A different entry installed
after final validation can be the entry moved. The postcommit identity check
prevents false success but cannot roll back that move; it returns ambiguity.
Symlink or special-file replacements are moved as directory entries without
following referents. The tool does not promise the old name remains absent or
the new name remains unchanged after return because another actor can recreate,
remove, or replace pathnames.

Retained descriptors prevent pathname replacement from redirecting traversal.
A retained source or destination parent moved elsewhere before the syscall can
still receive the descriptor-relative operation; the tool does not claim a
filesystem snapshot or global namespace lock. Mounts visible below the trusted
root remain host-selected authority.

The future is inert until first poll and performs bounded synchronous work on
its polling thread. Cancellation is checked before and after every root/parent
open and metadata call, between both validation passes, and at the exact final
pre-rename point. Cancellation cannot preempt a syscall already in flight.
After success or `EINTR`, cancellation is ignored through verification and
durability work. Drop closes owned descriptors by RAII and starts no detached
task, thread, process, timer, or runtime.

## Fixed tool errors

All failures are fixed and redacted:

| Code | Kind | Retryable | Exact message |
| --- | --- | --- | --- |
| `rename_file_invalid_arguments` | `InvalidInput` | no | `rename_file arguments are invalid` |
| `rename_file_invalid_path` | `InvalidInput` | no | `rename_file path is invalid` |
| `rename_file_unsupported_platform` | `Unavailable` | no | `native rename_file is unsupported on this platform` |
| `rename_file_not_found` | `Unavailable` | no | `rename source is unavailable` |
| `rename_file_permission_denied` | `PermissionDenied` | no | `requested rename is not permitted` |
| `rename_file_path_rejected` | `PermissionDenied` | no | `requested rename path is not confined` |
| `rename_file_destination_exists` | `Execution` | no | `rename destination already exists` |
| `rename_file_unavailable` | `Unavailable` | yes | `requested rename is unavailable` |
| `rename_file_target_changed` | `Execution` | yes | `rename paths changed before execution` |
| `rename_file_unsupported_filesystem` | `Unavailable` | no | `atomic no-replace rename is unavailable` |
| `rename_file_rename_failed` | `Execution` | yes | `requested file could not be renamed` |
| `rename_file_commit_ambiguous` | `Execution` | no | `requested file rename status is uncertain` |
| `rename_file_cancelled` | `Cancelled` | no | `rename_file execution was cancelled` |

Errors retain no endpoint, root, component, entry name, content, metadata,
device/inode, OS diagnostic, or errno. Engine-facing tool failures remain the
delivered generic durable error surface.

## Host composition and compatibility boundary

The native reference host registers exactly nine alphabetical workspace tools:
`delete_file`, `edit_file`, `file_info`, `glob_files`, `grep_files`,
`list_files`, `read_file`, `rename_file`, and `write_file`. Workspace-root
composition consumes the original descriptor for one tool and makes exactly
eight identity-preserving clones.

Pinned fx at `b1774fbf6c7602b503026f96f6e960e946c692ef` uses the same
tool and field names and supports the core rename scenario. Its implementation
also accepts external paths, creates destination parents, permits same-path
calls, and can replace existing destinations or resolve some symlink cases,
despite model-facing guidance warning against overwrites. Machine-god
intentionally rejects those broader behaviors. This slice makes no complete
fx-equivalence or product-performance claim.

## Deferred scope

Destination overwrite, parent creation, directory trees, symlink moves,
external paths, cross-filesystem copy/delete, non-Linux/macOS hardening, CLI
ownership, richer permission modes, benchmark workloads, and performance claims
remain outside this slice. Production, independent evidence, and the exact
composed local gate are green. Three fresh same-SHA adversarial reviews, exact
feature workflows, fast-forward integration, and exact main workflows are still
required before delivery.
