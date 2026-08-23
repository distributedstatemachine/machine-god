# Native `rename_file` contract

Status: **CYCLE 4 GREEN; FEATURE DELIVERY PENDING**

This document freezes the twenty-third bounded Milestone 03 slice from exact
delivered base `3d76f2e844312e7f3e809524cb72c1a7957975ff`. That base is
green under exact feature CI `32665981665`, feature benchmark workflow
`32665981641`, main CI `32666261656`, and main benchmark workflow
`32666261525`. Both benchmark workflows retained two nonexpired exact-SHA
artifacts.

The frozen contract is commit
`19cad7d10a8fc885e2e70a7345fc0ba27d76872a`. Exact contract benchmark
workflow `32667647846` is green with both jobs and two exact-SHA artifacts;
contract CI `32667647822` was cancelled when a later feature push superseded
it and is not claimed as green.
Production composes on the feature branch at
`d8f73676fcfce2cead385fa5b36598da989abe8f`, and independent evidence
composes at `1dab9a0dfcb4ec2d204625c744171ae923cca458`. Exact composed
local-gate precursor `43847fe5fd405e8b1d28808f0495dac859ebab15`, tree
`80cb9a17d9bb2c1151bc43b72faebcb305dd78c2`, is green. Fresh same-SHA review
cycle 1 is **NOT GREEN** in all three tracks on exact candidate
`2bc4f9a8ad809cd38a6b7b36488b27bf9bd531f6`, tree
`44558a0e88019ad9063234642c08097b4123c5f2`. Exact remediation
`a3491cf8d5e6c388c896374e768794d06bf7be0b`, tree
`0b195bdf29e7873a4d77169ec4d031491b1b336a`, passes the complete replacement
local gate. Tree-identical cycle-2 candidate
`4f224a5447a61a76a3cdea5ced035c164240c02c`, tree
`cb75dca76eeec80dc526946c9d39d6e3da882c68`, is green with zero findings in
all three fresh tracks. Feature delivery, fast-forward main integration, and
exact main workflows remain pending. First documentation seal `a03a57b` passed
feature benchmark `32671805335` with both jobs and two exact-SHA artifacts;
feature CI `32671805412` was cancelled after its quality job reproduced a
pre-existing Linux deadlock in an unrelated session-lifecycle test fixture.
Exact test-only remediation `2c771edf3d4385c0c94f2cbbee93427ea9e8b13a`, tree
`5de94a6f90d5316ab84b7f9451e51b7cc25fd6a2`, changes no production or rename
behavior and passes the complete replacement local gate. A tree-identical
cycle-3 candidate `5cc1523ebf1ba20264a80f3e703891ace58e1473`, tree
`99b88ec8653679ca5386c9b0f1c368543f487796`, was **NOT GREEN**: correctness/API
was green, while filesystem/robustness and performance/concurrency independently
found that device/inode identity was not pinned against reuse. Exact remediation
`4cbd46f82d3553009824883de2bc243177459207`, tree
`35f531eb867e1b08375041b3c74fcf1a650ae063`, retains the validated source
descriptor through commit verification and passes the complete replacement
local gate. Exact tree-identical cycle-4 candidate
`13379800ee2ee6eb6802db76c516e81dd087c62b`, tree
`ab2bdc2b719061faa69749360fd1399177748c24`, is green with zero findings in all
three fresh tracks. Replacement feature and main delivery remain pending.

`rename_file` validates and authorizes one existing regular file between two
confined names, and reports success only when that same file object is observed
at the destination. It does not accept a directory, symlink, or special-file
source, read content, overwrite a destination, create a parent, access an
external path, or fall back to copy-and-delete. The unavoidable final source-
replacement race is qualified below. It is library-only in this slice. The
product remains Rust; Zig remains solely a pinned upstream benchmark build
input.

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
3. Record both parent identities. Open and retain the source without reading
   content or following its final component, then `fstat` it and require a
   regular file. Linux uses `O_PATH`; macOS uses `O_EVTONLY`. Inspect the
   destination with no-follow metadata and require `ENOENT`; every existing
   entry type is a conflict.
4. Reacquire and revalidate the linked root, completely rewalk both parents,
   require the same parent identities and require the source path still names
   the retained source device/inode/regular type, and require the destination
   still absent.
5. Perform the final cancellation check immediately before exactly one
   `renameat_with` using `RenameFlags::NOREPLACE`. The rename is never retried,
   including after `EINTR`.
6. After success, ignore later tool cancellation. Inspect the destination with
   no-follow metadata and require the retained source device, inode, and
   regular-file type. The retained descriptor prevents that inode from being
   recycled during the comparison. Sync the source parent, then the destination
   parent. If both identities are equal, sync that directory once. Each unique
   parent allows at most 16 cumulative `fsync` calls, including interrupted
   calls; distinct parents are both attempted even if the first fails.

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

macOS applies access checks to `O_EVTONLY`; if it cannot retain an unreadable
source, execution returns the fixed permission error before rename and preserves
both endpoints. Linux `O_PATH` continues to accept a mode-000 regular source.

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
against the descriptor-pinned source prevents false success but cannot roll back
that move; it returns ambiguity. Retaining the source prevents device/inode
reuse from making a different file appear to be the validated object.
Regular-file, symlink, FIFO or other special-file, and directory replacements
can therefore be moved as directory entries in this final window without
following referents. None can produce success unless its identity is the
original validated regular file. The tool does not promise the old name remains
absent or the new name remains unchanged after return because another actor can
recreate, remove, or replace pathnames.

Retained descriptors prevent pathname replacement from redirecting traversal
and pin the validated source object through commit verification. A retained
source or destination parent moved elsewhere before the syscall can
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
remain outside this slice. Production, independent evidence, and the recorded
precursor local gate were green before formal cycle 1. That exact candidate is
not green: retained evidence was missing for the terminal replacement-race,
`EINTR`/errno, postcommit, sync-bound, late-cancellation, and moved-parent
matrix, and the final directory-replacement wording required clarification.
Exact remediation `a3491cf8d5e6c388c896374e768794d06bf7be0b`, tree
`0b195bdf29e7873a4d77169ec4d031491b1b336a`, expands the private suite to 15
tests, clarifies the directory race, and passes the complete replacement local
gate recorded in the review. Tree-identical cycle-2 candidate `4f224a5`, tree
`cb75dca`, is green with zero findings in all three fresh same-SHA tracks.
First seal `a03a57b` passed the exact feature benchmark workflow, while exact
feature CI reproduced an unrelated Linux session-lifecycle fixture deadlock.
Test-only remediation `2c771ed`, tree `5de94a6`, deterministically removes the
fixture cycle and passes the complete replacement local gate without changing
production. Cycle-3 candidate `5cc1523`, tree `99b88ec`, was not green because
two fresh tracks found the unpinned device/inode reuse race. Exact remediation
`4cbd46f`, tree `35f531e`, retains a non-reading source descriptor through
commit verification, adds direct macOS permission evidence and deterministic
unlinked-source evidence, and passes the complete replacement local gate. A
tree-identical cycle-4 candidate
`1337980`, tree `ab2bdc2`, is green with zero findings in all three fresh
same-SHA tracks. Exact replacement feature workflows, fast-forward integration,
and exact main workflows remain required before delivery.
