# Native `read_file` tool

This page is the normative contract for the first executable native tool in
Milestone 03. It is a library capability in `machine-god-native`; the current
CLI does not construct an engine, register the tool, prompt for permission, or
load a provider.

## Workspace authority and platform scope

The host constructs one tool with an explicitly injected absolute workspace
root. Construction is the only ambient path-selection step. On the supported
Linux and macOS Unix targets it opens and retains the root as directory
authority and rejects a final root symlink or non-directory. Model input and
preflight never select, reopen, canonicalize, or inspect a workspace root.

Before that final no-follow open, construction rebuilds the injected host root
from its lexical path components. This removes redundant separators and `.`
components throughout the root while preserving `..`; it does not canonicalize
ancestors or resolve symlinks. In particular, removing terminal separators and
terminal `.` components prevents forms such as `/workspace-link/` or
`/workspace-link/.` from making the operating system traverse through the final
component before applying no-follow protection. `/real-workspace`,
`/real-workspace/`, and `/real-workspace/.` therefore open the same real
directory; the corresponding forms of a final root symlink all reject. This is
not model-controlled path normalization.

The retained descriptor confines every model-selected component beneath that
root. Resolution of ancestor components leading to the host-supplied root and
mount points visible below it are part of the trusted host boundary; this is not
a chroot or a sandbox against the host. Hardened root construction and traversal
on non-Unix targets are deferred. This slice makes no non-Unix security claim.

## Provider input and preflight

The registered tool name is `read_file`. Its advertised input schema and its
own preflight accept exactly this object shape:

```json
{"path":"relative/path.txt"}
```

`path` is required, must be a JSON string, and is bounded to 4,096 UTF-8 bytes.
No other field is accepted. Preflight is deterministic, synchronous, bounded,
nonblocking, and effect-free. It performs only strict JSON decoding and lexical
path handling; it does not query metadata, resolve a symlink, open a file, read
bytes, or otherwise exercise the retained directory authority.

Lexical handling joins ordinary components into a normalized
workspace-relative path. It removes `.` components and collapses repeated
separators, so `./src//./lib.rs` becomes `src/lib.rs`. It rejects an
absolute path rooted at `/`, any `..` component, forbidden control or
line/paragraph-separator or bidirectional-formatting characters, and an empty
normalized path. On supported Unix targets, backslash is an ordinary filename
byte rather than a separator; Windows-looking strings such as `C:\notes.txt`
or `\\server\share` are ordinary confined Unix names, subject to the same
character and byte bounds. The normalized path must also remain within the
4,096-byte bound.

Successful preflight returns both:

- `Capability::Filesystem` with `FilesystemAccess::Read` and the normalized
  workspace-relative path; and
- prepared execution arguments of exact shape `{"path":"<normalized>"}` with
  that same path string.

The capability and execution arguments therefore name one operation. A
preflight error occurs before permission policy or filesystem access.

## Allowed execution

Execution starts only after core has allowed the prepared filesystem-read
capability. It consumes the prepared normalized path without reinterpreting it.
On supported Unix targets it walks from the retained workspace descriptor with
descriptor-relative opens. Every component uses no-follow semantics, and each
opened ancestor directory descriptor remains the stable base for the following
lookup. The final open is nonblocking and its descriptor is authoritatively
required to be a regular file before content is read. A symlink in any selected
component and a directory, FIFO, socket, device, or other non-regular final
entry fail closed.

These descriptor rules close path-redirection races: replacing a previously
opened ancestor cannot redirect later traversal, and replacing the final
pathname after open cannot change the descriptor being read. A different
ordinary file installed at the same normalized path before the final open may
be read. Authorization is for that path in the retained workspace namespace;
the tool does not promise an inode or content snapshot from preflight time.

The tool retains at most 8 KiB of file content plus one byte used only to detect
overflow or growth. Exactly 8 KiB is accepted; one additional byte fails, and a
successful result is never silently truncated. The complete bytes must be valid
UTF-8. Success has exactly this output shape:

```json
{"content":"complete UTF-8 file contents"}
```

Reading does not create, write, rename, delete, enumerate, or change metadata.
Successful file contents are intentionally visible to the model, durable
transcript, `ToolFinished` event, and configured observer.

## Errors and cancellation

`ReadFileTool::open` returns this complete fixed taxonomy:

| `ReadFileToolOpenErrorKind` | `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native read_file is unsupported on this platform` |
| `InvalidRoot` | `native read_file workspace root is invalid` |
| `InvalidFileType` | `native read_file workspace root is not a directory` |
| `Unavailable` | `native read_file workspace root is unavailable` |

Preparation and direct execution return these complete fixed `ToolError`
values. `Display` is always `<code>: <message>`.

| `ToolErrorKind` | `code` | `message` | `retryable` |
| --- | --- | --- | --- |
| `InvalidInput` | `read_file_invalid_arguments` | `read_file arguments are invalid` | `false` |
| `InvalidInput` | `read_file_invalid_path` | `read_file path is invalid` | `false` |
| `Unavailable` | `read_file_unsupported_platform` | `native read_file is unsupported on this platform` | `false` |
| `Unavailable` | `read_file_not_found` | `requested file is unavailable` | `false` |
| `PermissionDenied` | `read_file_permission_denied` | `requested file cannot be read` | `false` |
| `PermissionDenied` | `read_file_path_rejected` | `requested path is not a confined regular file` | `false` |
| `Unavailable` | `read_file_unavailable` | `requested file is unavailable` | `true` |
| `Execution` | `read_file_read_failed` | `requested file could not be read` | `true` |
| `Execution` | `read_file_too_large` | `requested file exceeds the read limit` | `false` |
| `Execution` | `read_file_not_utf8` | `requested file is not valid UTF-8` | `false` |
| `Cancelled` | `read_file_cancelled` | `read_file execution was cancelled` | `false` |

Operating-system error numbers select among these fixed categories: missing,
symlink/not-directory, access-denied, and other unavailable failures remain
distinct where supported. Metadata and read failures likewise map to fixed
unavailable or execution categories. Raw error numbers and operating-system
text are never retained in the public errors. Their kind, code, message,
`Display`, and `Debug` forms contain no workspace root, requested path, file
contents, or operating-system diagnostic.

Non-cancellation preparation or execution errors that pass through the engine
are reduced to core's existing generic durable tool-error result before the
model sees them. Direct callers of `Tool::execute` instead receive the exact
`read_file_cancelled` error above when the supplied token is cancelled. When the
engine supplies its shared turn token, core's cancellation checks take
precedence over the tool result: the engine terminates the turn as cancelled and
does not replace the durable unknown placeholder with a generic tool-error
result.

Execution checks its cancellation token before traversal, between bounded
reads, and after content validation. Cancellation closes per-call descriptors
and discards accumulated bytes. It cannot preempt an individual open, metadata,
or read syscall already in flight; the contract is cooperative at those stated
boundaries. Execution spawns no detached task or thread.

## Deferred scope

This slice does not add CLI commands or alter existing CLI bytes. It does not
add a provider, a production permission handler or prompt, permission modes
beyond `ask`, durable native sessions, workspace discovery, non-Unix hardened
filesystem access, other native tools, binary-file encoding, ranges or line
selection, compatibility evidence, or product performance claims. It does not
change the pinned fx inventory, benchmark workloads, Zig download, or benchmark
classification.
