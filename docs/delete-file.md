# Native `delete_file` contract

Status: **CONTRACT FROZEN — implementation and formal review pending**

This document freezes the twenty-second bounded Milestone 03 slice from exact
delivered base `719a9bded86fd7ce394d482798b9064c736f43ab`. That base is green
under exact feature CI `32651168514` across all six jobs and feature benchmark
workflow `32651168515` across both jobs with two nonexpired exact-SHA artifacts.
`main` was fast-forwarded without force from
`c1268fdf463e11242b7b916add70675ae91ed115` to that exact base and is green
under exact CI `32651488265` across all six jobs and benchmark workflow
`32651488282` across both jobs with two nonexpired exact-SHA artifacts.

`delete_file` deletes exactly one existing confined regular file or empty
directory. It does not recurse, follow a symlink, remove the workspace root,
read file content, enumerate a directory, create a path, or grant general
filesystem authority. It is library-only in this slice. The product remains
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
The serialized argument object is independently capped at 65,536 bytes before
canonical arguments are retained. Direct execution revalidates the same exact
canonical shape, so bypassing provider preparation cannot widen authority.

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
normalization delivered for `write_file` and `edit_file`; it does not inspect
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
construction. Root construction uses the delivered lexical host-root handling,
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
and at the exact final pre-`unlinkat` boundary. It cannot preempt a syscall
already in flight. Once the delete call has succeeded or returned `EINTR`,
later tool cancellation is ignored while the bounded parent durability attempt
completes. Dropping an unpolled future is effect-free; dropping at a documented
precommit boundary closes owned descriptors without deleting anything.

This protocol is not pathname compare-and-swap. A same-directory actor can
replace the target after final no-follow validation and before `unlinkat`.
Because portable `unlinkat` accepts only a parent descriptor and name, it may
remove a different same-type entry installed in that final window. A type
change generally fails through the flags/type mismatch rather than crossing
the type boundary, but that does not close the different-entry race within a
type. This limitation is disclosed and tested; the slice makes no stronger
adversarial concurrent-mutation claim.

A retained final parent can be renamed outside the public workspace path after
validation. Descriptor-relative `unlinkat` can then remove the entry in that
same retained moved directory. Replacing the old public parent pathname does
not redirect the retained descriptor. Other hard links and already open file or
directory descriptors survive deletion as defined by the operating system. A
concurrent actor may also recreate the pathname immediately after successful
deletion, so success does not promise that a later lookup is absent.

There is no staged file, temporary name, content buffer, ACL operation, chmod,
rename, or cleanup-unlink protocol in this slice. Those `write_file` and
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
are `delete_file_permission_denied`. `unlinkat` interruption and any failure
after successful deletion are `delete_file_commit_ambiguous`; `unlinkat` is
never retried. Other bounded operational failures map to the fixed unavailable,
target-changed, or delete-failed category according to the documented phase.

No public error or tool `Debug` form reflects the requested/canonical path,
workspace root, device or inode, file type, raw errno, operating-system text,
credentials, or another directory entry. Non-cancellation errors passing
through the engine retain core's existing generic durable error behavior.

## Reference-host composition

The reference host distributes one retained workspace descriptor plus seven
identity-preserving clones and registers exactly eight tools in alphabetical
order:

```text
delete_file
edit_file
file_info
glob_files
grep_files
list_files
read_file
write_file
```

The CLI remains byte-unchanged and thin. This slice adds no CLI command,
invocation path, prompt, status field, or output byte.

## Parallel ownership and formal review

Implementation proceeds with three non-overlapping owners:

- production owns native behavior, exports, prepared-root and reference-host
  wiring, and any core contract regression needed for the existing `Delete`
  authority;
- independent tests own direct, engine, host, portability, fault, race, bound,
  cancellation, and unsupported-target evidence; and
- documentation owns this normative contract, maintained indexes and plan
  status, and the review lineage record.

Production should use a statically dispatched evidence seam in the real
pipeline for root and intermediate opens, ordinal-aware descriptor `fstat` and
pathname `statat`, checkpoints after initial and final validation, the actual
`unlinkat` flags and outcome, the checkpoint after a real deletion, and parent
`fsync`. It must add no global mutable evidence state, dynamic dispatch in the
release path, unbounded retry, or behavior-only test fork.

After one exact composed behavior candidate passes focused and complete local
gates, three fresh agents must adversarially review that same SHA for
correctness/API, filesystem/robustness, and performance/concurrency. Every
finding is fixed and all three tracks restart on a new exact candidate until
all report **GREEN** with zero findings. Documentation-only contract, seal, and
delivery commits are exempt from another adversarial cycle under the user's
explicit instruction, but still require their exact remote workflows.

## Required independent evidence

The composed candidate must prove:

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
  public workspace; different-entry final-window races; hard-link/open-
  descriptor survival; and immediate pathname recreation;
- production-routed root/intermediate-open, ordinal `fstat`/`statat`,
  `unlinkat`, and parent-sync faults with exact precommit, committed, and
  ambiguous mappings;
- exact empty versus `REMOVEDIR` flags, exactly one delete call on success and
  `EINTR`, no delete retry, and cumulative 16-interruption parent-sync handling;
- cancellation at every traversal/metadata boundary, immediately after both
  validation phases, immediately before the real delete, and after the real
  delete, plus unpolled/drop and engine same-poll unknown-result recovery;
- the exact eight-tool alphabetical host catalog and original-plus-seven-clone
  retained workspace identity;
- native Linux/macOS execution, FreeBSD/WASI compilation, active unsupported-
  target behavior, no unsafe Rust, and complete regression of all seven
  delivered workspace tools.

Focused suites run first, followed by Rust 1.94.1 formatting, workspace all-
target/all-feature warnings-denied Clippy, workspace tests, workspace doctests,
the repository Python and pinned-compatibility checks, dependency policy and
audit, portability gates, documentation integrity, diff/no-unsafe checks, and
a freshly built locked release CLI smoke. Green local evidence alone is not
delivery: the exact reviewed SHA must pass feature workflows, fast-forward
`main`, and pass exact `main` workflows.

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

This slice adds no recursion, parent or target creation, wildcard or multi-path
deletion, trash/recovery mode, secure erasure, symlink-target mutation, content
read, directory enumeration, external path, CLI change, new dependency,
non-Linux/macOS hardening, compatibility promotion, benchmark workload,
product-performance claim, or fx-equivalence claim.
