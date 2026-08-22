# Native `file_info` tool

Status: candidate contract for the seventeenth bounded Milestone 03 slice.
Production is present at isolated SHA
`5c2d129a3755dca0c8f7913b27614b70352fe2a4`; independent tests are present at
isolated SHA `ca0091c181d8ffbecda008ee0f981516dc5cff7b` and compose with
production at `f228c06bbda5d01b50905c66f378a2b29e0560bf`, where all 34
focused tests are green. The first fully composed candidate is `8ceef6d`, and
required local gates are green through composed precursor `0973acf`. Three
fresh adversarial review tracks and exact feature and `main` delivery workflows
remain pending. `file_info` is a library
capability in `machine-god-native`; the current CLI does not construct an engine,
register this tool, prompt for permission, or change any invocation or output
byte.

## Workspace authority and platform scope

The host roots each tool in one explicitly selected absolute workspace path. On
the supported Linux and macOS targets, construction opens that path without
following its final component and retains the resulting directory descriptor as
the tool's authority. It rejects a relative root, a final root symlink, or a
non-directory. It does not discover a workspace from process state. Model input
and preflight never select, reopen, canonicalize, or inspect the workspace root.

Before opening the root, construction rebuilds the injected host path from its
lexical components. This removes redundant separators and `.` components while
preserving `..`; it does not canonicalize ancestors or resolve symlinks.
Removing terminal separators and terminal `.` components ensures that forms
such as `/workspace-link/` and `/workspace-link/.` cannot move a final root
symlink out of the no-follow lookup position. The equivalent forms of a real
directory continue to select that directory. This normalization applies only
to the host-supplied root.

The retained descriptor confines every model-selected component beneath that
root. Resolution of ancestors leading to the host-supplied path and mount
points visible below the retained directory remain trusted host boundaries.
This is not a chroot or a sandbox against the host. Hardened root construction
and traversal on targets other than Linux and macOS are deferred, and this
slice makes no security claim for them.

## Provider input and preflight

The registered tool name is `file_info`. Its fixed description is
`Inspect metadata for one path within the configured workspace`. Its advertised
schema and its own preflight accept exactly this object:

```json
{"path":"relative/path.txt"}
```

The `path` property description is `Workspace-relative path to inspect`.
`path` is required, must be a nonempty JSON string, and is bounded to 4,096
UTF-8 bytes. No additional field is accepted.

Preflight is deterministic, synchronous, bounded, nonblocking, and effect-free.
It performs only strict JSON decoding and lexical path handling. It does not
query metadata, resolve a symlink, open a directory, inspect a file, or
otherwise exercise the retained workspace authority.

Lexical handling uses the same component and forbidden-character confinement as
`read_file`. It removes `.` components and collapses repeated `/` separators,
then joins ordinary components into one normalized workspace-relative path. A
nonempty request made entirely of current-directory components, such as `.` or
`./`, explicitly normalizes to `.` so callers can inspect the retained
workspace root. It rejects a path rooted at `/`, any `..` component, an empty
requested string, C0 or C1 control characters, Unicode line or paragraph
separators, and Unicode bidirectional-formatting characters. On the supported
Unix targets, backslash and space are ordinary literal filename characters
rather than separators or trimming syntax. Windows-looking strings such as
`C:\notes` or `\\server\share` are confined Unix names subject to the same
character and byte bounds.

Successful preflight returns both:

- `Capability::Filesystem` with `FilesystemAccess::Metadata` and the normalized
  workspace-relative path; and
- prepared execution arguments of exact shape `{"path":"<normalized>"}` with
  that same path string.

Policy and allowed execution therefore agree on the exact normalized metadata
operation. A preparation error occurs before permission policy or filesystem
access.

## Allowed execution and result

Execution starts only after core allows that exact filesystem-metadata
capability. It consumes the prepared normalized path without reinterpreting it.
It first acquires a fresh `.` descriptor from the retained workspace with
directory, no-follow, nonblocking, and close-on-exec requirements, then
validates that exact acquired descriptor's linked identity. Linux rejects zero
link count. macOS resolves the acquired descriptor's kernel path, opens its
parent descriptor-relatively, and requires the no-follow parent/name metadata
to match the acquired device, inode, and directory type. The filesystem root is
accepted directly on macOS. A validation failure is fixed redacted
`file_info_unavailable`.

Starting from that validated descriptor, execution opens every requested
ancestor component descriptor-relatively with directory and no-follow
requirements. Each opened ancestor remains the stable base for the next lookup.

The final component is inspected with one descriptor-relative, no-follow
metadata operation. It is not opened. A final symlink therefore reports
metadata for the link itself rather than its target. A final FIFO, socket,
device, or other special object is classified as `other` without opening it,
so metadata inspection neither reads content nor blocks waiting for a special
file peer. An ancestor symlink or non-directory fails closed.

When the normalized path is `.`, execution instead obtains the returned
metadata with one `fstat` of the freshly acquired and validated root descriptor.
Its kind is `directory` and its extension is `null`.

Success has exactly this shape:

```json
{
  "path": "src/lib.rs",
  "kind": "file",
  "size_bytes": 1234,
  "modified": {
    "unix_seconds": 1787451000,
    "nanoseconds": 123456789
  },
  "extension": "rs"
}
```

`path` is the prepared normalized path. `kind` is exactly `file`, `directory`,
`symlink`, or `other`, derived from the no-follow metadata. `size_bytes` is the
checked nonnegative metadata size represented as `u64`; for directories,
symlinks, and special objects it remains the operating system's metadata size
and is not a content-length promise. `modified.unix_seconds` is a signed
64-bit Unix timestamp and `modified.nanoseconds` is validated in the inclusive
range `0..=999_999_999`. A negative pre-epoch second remains negative rather
than being rejected or saturated. A negative size, out-of-range nanoseconds,
or value that cannot fit the public representation fails as invalid metadata.

`extension` is non-null only for a regular file whose final basename contains a
non-leading dot followed by a nonempty suffix. It is the suffix after the last
such dot. Thus `.bashrc`, `foo.`, directories, symlinks, and `other` objects
produce `null`; `.config.json` produces `"json"`, and `archive.tar.gz`
produces `"gz"`. Extension classification is lexical and never follows a
target or reads content.

The accepted path is capped at 4,096 UTF-8 bytes, all numeric fields have fixed
integer widths, and `extension` can contain only a suffix of that same bounded
path. Even with worst-case JSON escaping of both the returned path and its
extension, the structured content remains below 17 KiB and therefore below
core's default 64 KiB serialized result limit. This is a structural consequence
of the input and result shape, not a separate tool-side serialized-byte meter.
A host-configured lower core limit still applies after execution.

## Identity, removal, and races

The retained workspace descriptor, not the host path string, is the continuing
authority. Renaming the workspace after construction does not redirect the
tool, and replacing the old host path does not make the tool switch to the
replacement. Each execution nevertheless requires the exact freshly acquired
`.` descriptor to remain linked through the platform validation above. A
stable completed rename retains identity and succeeds. Removal before fresh
acquisition or validation is unavailable. Concurrent rename or removal may
conservatively be unavailable or observe the acquired identity; no path-based
replacement can redirect it to a different workspace.

Replacing an ancestor after its descriptor has been opened cannot redirect the
remaining walk. A different ordinary directory or final object installed
before its lookup may be observed at the same authorized path. For a non-root
path, the returned fields all come from one final no-follow `statat` result; for
`.`, they come from one final `fstat` after the liveness probe. Each is
internally one metadata snapshot, but there is no snapshot from preflight time,
no content snapshot, no symlink-target snapshot, and no promise that the object
still exists after return.

## Errors and cancellation

`FileInfoTool::open` returns this complete fixed taxonomy:

| `FileInfoToolOpenErrorKind` | `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native file_info is unsupported on this platform` |
| `InvalidRoot` | `native file_info workspace root is invalid` |
| `InvalidFileType` | `native file_info workspace root is not a directory` |
| `Unavailable` | `native file_info workspace root is unavailable` |

Preparation and direct execution return these complete fixed `ToolError`
values. `Display` is always `<code>: <message>`.

| `ToolErrorKind` | `code` | `message` | `retryable` |
| --- | --- | --- | --- |
| `Cancelled` | `file_info_cancelled` | `file_info execution was cancelled` | `false` |
| `InvalidInput` | `file_info_invalid_arguments` | `file_info arguments are invalid` | `false` |
| `InvalidInput` | `file_info_invalid_path` | `file_info path is invalid` | `false` |
| `Unavailable` | `file_info_unsupported_platform` | `native file_info is unsupported on this platform` | `false` |
| `Unavailable` | `file_info_not_found` | `requested path is unavailable` | `false` |
| `PermissionDenied` | `file_info_permission_denied` | `requested path metadata cannot be inspected` | `false` |
| `PermissionDenied` | `file_info_path_rejected` | `requested path is not confined to the workspace` | `false` |
| `Unavailable` | `file_info_unavailable` | `requested path metadata is unavailable` | `true` |
| `Execution` | `file_info_metadata_failed` | `requested path metadata could not be inspected` | `true` |
| `Execution` | `file_info_invalid_metadata` | `requested path metadata is invalid` | `false` |

Operating-system error numbers may choose among missing, access-denied,
rejected-path, unavailable, metadata, and invalid-metadata categories, but no
raw number or operating-system text is retained.
No public error `Display` or `Debug` form contains the workspace root,
requested path, metadata values, extension, or operating-system diagnostic.

Non-cancellation errors passing through the engine are reduced to core's
existing generic durable tool-error result before the model sees them. A direct
caller instead receives the fixed `file_info_cancelled` error when its supplied
token is cancelled. When the engine supplies the shared turn token, core's
cancellation checks take precedence: the engine terminates the turn as
cancelled and leaves the durable unknown-result placeholder intact.

Creating the execution future performs no filesystem operation. On first poll,
execution checks cancellation before fresh-root acquisition, after acquisition,
before every ancestor or final component, after every ancestor open,
immediately before and after the final `statat` or `fstat`, and immediately
before return. The platform root-liveness validation occurs after the
post-acquisition check and uses bounded synchronous metadata operations.
Dropping the future before poll is effect-free. Dropping it at a cooperative
boundary closes per-call descriptors and discards the result. Cancellation
cannot preempt an individual open or metadata syscall already in flight.
Execution starts no detached task or thread.

## Deferred scope

This slice does not accept absolute, external, or parent-traversing paths. It
does not follow any selected symlink or inspect a symlink target, read content,
recurse, enumerate children, derive MIME type or a content hash, report
ownership, mode bits, ACLs, extended attributes, birth/access/change times, or
additional timestamps, or mutate anything. It adds no CLI command or output
change, non-Linux/macOS hardening, compatibility or upstream-equivalence
claim, benchmark workload, or product-performance claim.

The pinned fx inventory, benchmark classification, and workflows are unchanged.
Zig remains only the pinned upstream benchmark build input; machine-god is a
Rust product and does not use Zig as a product language or runtime dependency.
