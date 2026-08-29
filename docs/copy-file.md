# Native `copy_file` contract

`copy_file` copies the bounded bytes of one existing confined regular file to
one absent confined destination without modifying the source. It does not
overwrite a destination, create a parent, copy a directory, follow a symlink,
access an external path, or allocate the complete file in memory. It is
library-only. The product remains Rust; Zig remains solely a
pinned upstream benchmark build input.

## Public API and schema

`machine-god-native` exports `COPY_FILE_TOOL_NAME`, `CopyFileTool`,
`CopyFileToolOpenError`, `CopyFileToolOpenErrorKind`, and these limits:

| Public constant | Exact value |
| --- | ---: |
| `COPY_FILE_TOOL_NAME` | `"copy_file"` |
| `MAX_COPY_FILE_PATH_BYTES` | `4,096` |
| `MAX_COPY_FILE_PATH_COMPONENTS` | `256` |
| `MAX_COPY_FILE_SOURCE_BYTES` | `16,777,216` |
| `MAX_COPY_FILE_CHUNK_BYTES` | `65,536` |
| `MAX_COPY_FILE_IO_CALLS` | `4,096` |
| `MAX_COPY_FILE_TEMP_ATTEMPTS` | `8` |
| `MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES` | `65,536` |
| `MAX_COPY_FILE_SERIALIZED_RESULT_BYTES` | `16,384` |

The exact tool description is
`Copy one existing regular file to an absent path within the configured workspace`.
Property descriptions are `Source workspace-relative regular-file path` and
`Destination workspace-relative file path` for `source` and `destination`.

The exact input schema is:

```json
{
  "type": "object",
  "properties": {
    "source": {
      "type": "string",
      "description": "Source workspace-relative regular-file path"
    },
    "destination": {
      "type": "string",
      "description": "Destination workspace-relative file path"
    }
  },
  "required": ["source", "destination"],
  "additionalProperties": false
}
```

Both fields are required strings with no defaults. Unknown fields, including
`overwrite`, are invalid. Each requested and canonical path is independently
capped at 4,096 UTF-8 bytes and 256 canonical components. Complete requested
and prepared JSON values are independently capped at 65,536 serialized bytes.
Canonical endpoints must differ. Direct execution revalidates the same exact
shape and requires both paths already canonical.

Normalization is the existing mutation-path rule: repeated `/` separators
collapse and exact `.` components disappear. Backslash and space remain literal
Unix filename characters. Empty paths, absolute paths, any `..` component,
C0/C1 controls, Unicode line or paragraph separators, Unicode bidirectional-
formatting characters, and a canonical `.` endpoint are rejected.

Construction accepts one injected absolute workspace directory. The public API
and fixed unsupported result exist on every target; execution is supported only
on Linux and macOS. Construction errors retain only their kind:

| Kind | Exact `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native copy_file is unsupported on this platform` |
| `InvalidRoot` | `native copy_file workspace root is invalid` |
| `InvalidFileType` | `native copy_file workspace root is not a directory` |
| `Unavailable` | `native copy_file workspace root is unavailable` |

`Display` and `Debug` never retain an endpoint, injected root, operating-system
text, or raw error number.

## Preparation and authority

Preparation is deterministic, synchronous, bounded, nonblocking, and effect-
free. It performs no lookup, open, read, creation, mutation, staging, or root
inspection. Successful preparation retains exactly the two canonical fields
and returns:

```text
Capability::FilesystemCopy {
    source: canonical_source,
    destination: canonical_destination,
}
```

Core serializes this provider-neutral capability with the exact tag
`filesystem_copy`. Both endpoints are part of policy input because a single-
path read or write capability cannot authorize the combined effect. Policy and
allowed execution receive the same canonical pair. Denial or failed
preparation has no filesystem effect.

The capability grants only bounded metadata and content reads from the selected
source, private staging and absent-only publication in the selected destination
parent, and destination durability work. It grants no overwrite, source
mutation, parent creation, directory-tree copy, symlink following, external
access, or unrelated filesystem authority.

## Supported copy protocol

The source may contain arbitrary binary bytes, including an empty file, and is
capped at 16 MiB. The destination must be absent and both parents must already
exist. The new file receives only the source's nine ordinary `st_mode & 0o777`
permission bits. Set-id and sticky bits are stripped. Ownership, ACLs, extended
attributes, timestamps, flags, sparse layout, and hard-link identity are not
copied. A destination-local stage makes confined cross-mount source reads
possible without a copy/delete fallback.

Allowed Linux/macOS execution uses only the retained workspace descriptor and
per-call owned descriptors. The injected pathname is never reopened as
authority. For one call it performs this bounded sequence:

1. Check cancellation, acquire `.` descriptor-relatively, and validate the
   exact linked workspace identity.
2. Walk the source and destination parents through descriptor-relative,
   no-follow, nonblocking directory opens. Both parents must exist; retain and
   record their identities.
3. Inspect and open the source no-follow and nonblocking, require a regular file,
   and retain the descriptor. Record device, inode, byte size, ordinary mode,
   modification time, and change time. Reject a source over 16 MiB. Require the
   destination absent; every existing entry type is a conflict.
4. In the destination parent, try at most eight fixed-short, high-entropy stage
   basenames. Open at `0600` with create, exclusive, no-follow, close-on-exec,
   and nonblocking flags. A collision is never removed and consumes one attempt.
   Entropy acquisition uses the bounded nonblocking 16-byte protocol,
   with at most 16 cumulative interruptions and 31 total calls per name fill.
5. Stream source bytes to the stage in chunks of at most 64 KiB while computing a
   SHA-256 digest. Each logical source-read and stage-write phase permits at
   most 4,096 native calls and 16 cumulative interrupted results. Zero progress,
   overreported progress, size overflow, or a changing source fails closed.
6. Re-stat the source descriptor and require the recorded stable fingerprint.
   Hash the complete staged descriptor through the same bounded buffer and
   require exact size and digest. Revalidate staged pathname/descriptor identity.
   Clear inherited access ACLs and verify the empty result where the supported
   platform exposes that primitive.
7. Apply the source ordinary mode to the stage, sync the staged file, then
   repeat its identity, size, digest, mode, and ACL checks.
8. Reacquire and validate the root, completely rewalk both parents, require the
   original parent identities, require the source pathname still names the
   retained stable regular-file object, require the destination still absent,
   and require the staged pathname still names the held stage descriptor.
9. Perform the final cancellation check, then invoke exactly one
   `renameat_with(..., RenameFlags::NOREPLACE)` in the destination parent. Never
   retry publication, including after `EINTR`.
10. After success, ignore later tool cancellation. Require the destination name
    to identify the held staged inode, hash the complete destination and require
    the copied size and digest, revalidate the source name and stable descriptor,
    then sync the destination parent with at most 16 cumulative calls. A
    postcommit verification or sync failure is ambiguity.

Success is exactly:

```json
{"source":"src/template.bin","destination":"fixtures/template.bin","bytes_copied":123}
```

The complete `ToolOutput` is defensively capped at 16,384 serialized bytes. No
content, digest, permission bits, timestamp, device, inode, or temporary name is
returned. The implementation allocates fixed streaming and hashing state, not a
source-sized buffer.

## Commit boundary, cancellation, cleanup, and races

The sole no-replace destination rename is the irreversible boundary. Before it,
cancellation or failure publishes no destination entry. A successful rename may
have committed even if verification or parent sync fails. Every publication
`EINTR` is nonretryable ambiguity because it does not prove whether the name is
absent; publication is never repeated.

Owned stage cleanup is best-effort and identity-checked. It first attempts to
restore `0600` through the retained stage descriptor, then compares the staged
name by no-follow identity before unlinking. A preexisting collision or
mismatched replacement is never intentionally deleted. A portable final
metadata-to-unlink race remains, and if mode restoration and unlink both fail,
residue can retain final permission bits.

`NOREPLACE` closes the destination replacement race. Retained descriptors keep
source or parent pathname replacement from redirecting already-open content
I/O. Portable Linux/macOS filesystems provide no source-content compare-and-
swap: a source mutation or source-name replacement after final validation can
make the copied bytes no longer match the path's newest state. Postcommit
source and digest verification prevents that observation from being reported as
success but cannot roll back an already-published destination.

A retained destination parent moved elsewhere after the final identity rewalk
can still receive descriptor-relative staging or publication. Mounts visible
below the trusted root remain host-selected authority. Success is a verified
observation, not a permanent equality guarantee against later concurrent
changes.

The future is inert until first poll and performs bounded synchronous work on
its polling thread. Cancellation is checked before and after every root, parent,
metadata, entropy, read, write, hashing, chmod, ACL, and sync phase and at the
exact final prepublication point. Cancellation cannot preempt a syscall already
in flight. After successful publication or publication `EINTR`, cancellation is
ignored through verification and durability work. Drop closes owned
descriptors by RAII and starts no detached task, thread, process, timer, or
runtime.

## Fixed tool errors

All failures are fixed and redacted:

| Code | Kind | Retryable | Exact message |
| --- | --- | --- | --- |
| `copy_file_invalid_arguments` | `InvalidInput` | no | `copy_file arguments are invalid` |
| `copy_file_invalid_path` | `InvalidInput` | no | `copy_file path is invalid` |
| `copy_file_unsupported_platform` | `Unavailable` | no | `native copy_file is unsupported on this platform` |
| `copy_file_not_found` | `Unavailable` | no | `copy source or destination parent is unavailable` |
| `copy_file_source_too_large` | `InvalidInput` | no | `copy source exceeds the supported size limit` |
| `copy_file_permission_denied` | `PermissionDenied` | no | `requested copy is not permitted` |
| `copy_file_path_rejected` | `PermissionDenied` | no | `requested copy path is not confined` |
| `copy_file_destination_exists` | `Execution` | no | `copy destination already exists` |
| `copy_file_unavailable` | `Unavailable` | yes | `requested copy is unavailable` |
| `copy_file_target_changed` | `Execution` | yes | `copy paths changed before commit` |
| `copy_file_unsupported_filesystem` | `Unavailable` | no | `atomic no-replace copy publication is unavailable` |
| `copy_file_copy_failed` | `Execution` | yes | `requested file could not be copied` |
| `copy_file_commit_ambiguous` | `Execution` | no | `requested file copy status is uncertain` |
| `copy_file_cancelled` | `Cancelled` | no | `copy_file execution was cancelled` |

Errors retain no endpoint, root, component, entry name, content, digest,
metadata, temporary name, OS diagnostic, or errno. Engine-facing tool failures
remain the existing generic durable error surface.

## Host composition and compatibility boundary

Current reference-host composition and retained-workspace descriptor
distribution are maintained in the
[canonical tool catalog](native-reference-host.md#tool-catalog).

Pinned fx at `b1774fbf6c7602b503026f96f6e960e946c692ef` uses the same tool
and field names and supports the core source-preserving copy scenario. Its
implementation also creates destination parents, permits same-path calls,
replaces existing destinations, accepts external paths, and follows a source
symlink. Machine-god intentionally rejects those broader behaviors. This contract
makes no complete fx-equivalence or product-performance claim.

## Required evidence

- Exact public constants, schema, descriptions, result, construction taxonomy,
  errors, redaction, strict argument shape, canonicalization, and every bound.
- Effect-free preparation, exact `FilesystemCopy` serialization, denial before
  lookup, and policy/execution agreement.
- Empty, text, binary, executable, exact-limit, cross-parent, and cross-mount
  copies; unchanged source; absent-only destination; exact bytes and ordinary
  mode; no full-content allocation.
- Source, destination, ancestor, and staged-entry type/race coverage, including
  symlinks, FIFOs, sockets, devices where available, hostile umask and inherited
  ACL behavior, root replacement, moved retained parents, and outside sentinels.
- Exact-one publication, `EINTR`, entropy, partial/zero/overreported I/O,
  source stability, digest mismatch, stage identity, cleanup dual failure,
  parent-sync bounds, cancellation, drop, and same-poll engine recovery.
- Native Linux/macOS, FreeBSD/WASI compilation, active unsupported target,
  canonical reference-host composition, dependency, compatibility,
  documentation, no-unsafe, diff, and fresh release-binary smoke evidence.

## Deferred scope

Destination overwrite, parent creation, directory trees, symlink copies,
external paths, source files above 16 MiB, source sparse-layout preservation,
non-Linux/macOS hardened execution, CLI ownership, richer permission modes,
benchmark workloads, and performance claims remain outside this contract.
