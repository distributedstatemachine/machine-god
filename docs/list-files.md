# Native `list_files` tool

This page is the normative contract for the fifth bounded Milestone 03 slice.
`list_files` is a library capability in `machine-god-native`; the current CLI
does not construct an engine, register the tool, prompt for permission, or load
a provider.

## Workspace authority and platform scope

The host roots each tool in one explicitly selected absolute workspace path. On
the supported Linux and macOS Unix targets, construction opens that path and
retains the resulting directory descriptor as its authority. It rejects a
relative root, a final root symlink, or a non-directory. It does not discover a
workspace from process state. Model input and preflight never select, reopen,
canonicalize, or inspect the workspace root.

Before opening a root path, construction rebuilds the injected host path from
its lexical components. This removes redundant separators and `.` components
throughout the root while preserving `..`; it does not canonicalize ancestors
or resolve symlinks. Removing terminal separators and terminal `.` components
prevents forms such as `/workspace-link/` or `/workspace-link/.` from making
the operating system traverse through the final component before no-follow
protection applies. `/real-workspace`, `/real-workspace/`, and
`/real-workspace/.` therefore open the same real directory; the corresponding
forms of a final root symlink all reject. This normalization applies only to
the host-supplied root.

The retained descriptor confines every model-selected component beneath that
root. Resolution of ancestors leading to the host-supplied root path and mount
points visible below the retained directory are trusted host boundaries. This
is not a chroot or a sandbox against the host. Hardened root construction and
traversal on non-Unix targets are deferred, and this slice makes no non-Unix
security claim.

## Provider input and preflight

The registered tool name is `list_files`. Its fixed description is
`List one directory within the configured workspace`. Its advertised schema and
its own preflight accept exactly either an empty object or an object whose sole
field is a string `path`:

```json
{}
```

```json
{"path":"relative/directory"}
```

The `path` property description is
`Workspace-relative directory path; defaults to the workspace root`. Omission
selects `.`. A present path must be a JSON string; `null` and every additional
field are invalid. Both the requested string and normalized path are bounded to
4,096 UTF-8 bytes.

Preflight is deterministic, synchronous, bounded, nonblocking, and effect-free.
It performs only strict JSON decoding and lexical path handling. It does not
query metadata, resolve a symlink, open a directory, read an entry, or otherwise
exercise the retained directory authority.

Lexical handling joins ordinary components into a normalized
workspace-relative path. It removes `.` components and collapses repeated `/`
separators; a path that denotes the workspace root normalizes to `.`. It rejects
an absolute path rooted at `/`, any `..` component, an empty present path, C0 or
C1 control characters, Unicode line or paragraph separators, and Unicode
bidirectional-formatting characters. On supported Unix targets, backslash and
space are ordinary literal filename characters rather than separators or
trimming syntax. Windows-looking strings such as `C:\notes` or
`\\server\share` are ordinary confined Unix names, subject to the same
character and byte bounds.

Successful preflight returns both:

- `Capability::Filesystem` with `FilesystemAccess::Enumerate` and the normalized
  workspace-relative path; and
- prepared execution arguments of exact shape `{"path":"<normalized>"}` with
  that same path string.

The capability and execution arguments therefore name the same one-level
enumeration. A preparation error occurs before permission policy or filesystem
access.

## Allowed execution

Execution starts only after core allows that exact prepared filesystem-enumerate
capability. It consumes the prepared normalized path without reinterpreting it.
On supported Unix targets it starts from the retained workspace descriptor and
opens each selected component descriptor-relatively with directory and
no-follow requirements. Each opened ancestor descriptor remains the stable base
for the next lookup, and the final selected object must be a directory. A
symlink in any selected component and a regular file, FIFO, socket, device, or
other non-directory final object fail closed.

These descriptor rules prevent replacement of an already opened ancestor from
redirecting later traversal. A different ordinary directory installed at the
same normalized path before its final open may be enumerated. Authorization is
for that path in the retained workspace namespace; the tool does not promise a
directory identity or filesystem snapshot from preflight time.

The tool enumerates only immediate directory entries. It does not recurse,
apply ignore rules, open a child, read file content, follow or resolve an entry
symlink, inspect an external path, or discover a workspace. It skips only the
special `.` and `..` entries. Dotfiles and every other safe visible name remain
eligible.

Every observed visible entry name, including an extra entry used only to prove
truncation, must be valid UTF-8 and pass the same control,
line/paragraph-separator, and bidirectional-formatting rejection used for input
paths. An unsupported name fails the complete call; names are not lossily
encoded or silently skipped. The `kind` comes only from the directory entry's
reported type and is one of `file`, `directory`, `symlink`, or `other`. An
unknown entry type is `other`; classification does not open the child or resolve
its target.

The retained subset contains at most 100 entries and at most 16 KiB of
aggregate raw UTF-8 name bytes. The tool reads the first additional visible
entry that would exceed either bound and sets `truncated` to `true`; if no such
entry is observed, it is `false`. It stops after that witness, so a truncated
selection can reflect filesystem iteration order. Only the retained subset is
sorted in ascending lexicographic `name` order. The result makes no claim that
the entire directory was sorted, that the retained subset is the globally first
100 names, or that enumeration is a stable full-directory snapshot.

Success has exactly this shape, where `path` is the prepared normalized path:

```json
{
  "path": ".",
  "entries": [
    {"name": "Cargo.toml", "kind": "file"},
    {"name": "crates", "kind": "directory"}
  ],
  "truncated": false
}
```

The output remains below core's default 64 KiB serialized tool-result cap even
at its independent worst-case bounds. Safe path and name strings need at most
two JSON bytes per raw UTF-8 byte because only quote and backslash can require
escaping. A 4,096-byte path, 16,384 bytes of names, and the fixed syntax for 100
maximum-length `directory` kind labels conservatively serialize to at most
44,101 bytes of structured content. Including the fixed `ToolOutput` content and
error-status envelope yields at most 44,130 serialized bytes. A host may
configure a lower core result limit, in which case core's ordinary
post-execution limit handling still applies.

## Errors and cancellation

`ListFilesTool::open` returns this complete fixed taxonomy:

| `ListFilesToolOpenErrorKind` | `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native list_files is unsupported on this platform` |
| `InvalidRoot` | `native list_files workspace root is invalid` |
| `InvalidFileType` | `native list_files workspace root is not a directory` |
| `Unavailable` | `native list_files workspace root is unavailable` |

Preparation and direct execution return these complete fixed `ToolError`
values. `Display` is always `<code>: <message>`.

| `ToolErrorKind` | `code` | `message` | `retryable` |
| --- | --- | --- | --- |
| `InvalidInput` | `list_files_invalid_arguments` | `list_files arguments are invalid` | `false` |
| `InvalidInput` | `list_files_invalid_path` | `list_files path is invalid` | `false` |
| `Unavailable` | `list_files_unsupported_platform` | `native list_files is unsupported on this platform` | `false` |
| `Unavailable` | `list_files_not_found` | `requested directory is unavailable` | `false` |
| `PermissionDenied` | `list_files_permission_denied` | `requested directory cannot be listed` | `false` |
| `PermissionDenied` | `list_files_path_rejected` | `requested path is not a confined directory` | `false` |
| `Unavailable` | `list_files_unavailable` | `requested directory is unavailable` | `true` |
| `Execution` | `list_files_read_failed` | `requested directory could not be listed` | `true` |
| `Execution` | `list_files_invalid_entry_name` | `requested directory contains an unsupported entry name` | `false` |
| `Cancelled` | `list_files_cancelled` | `list_files execution was cancelled` | `false` |

Operating-system error numbers select among these fixed categories: missing,
symlink/not-directory, access-denied, and other unavailable failures remain
distinct where supported. Enumeration failures and invalid entry names likewise
map to fixed categories. Raw error numbers and operating-system text are never
retained in public errors. Their kind, code, message, `Display`, and `Debug`
forms contain no workspace root, requested path, entry name, or operating-system
diagnostic.

Non-cancellation preparation or execution errors that pass through the engine
are reduced to core's existing generic durable tool-error result before the
model sees them. Direct callers of `Tool::execute` instead receive the exact
`list_files_cancelled` error above when the supplied token is cancelled. When
the engine supplies its shared turn token, core's cancellation checks take
precedence over the tool result: the engine terminates the turn as cancelled and
does not replace the durable unknown placeholder with a generic tool-error
result.

Execution checks cancellation before traversal, between descriptor-relative
component opens, before enumeration, between directory-entry reads, and after
validating and sorting the retained result. Cancellation closes per-call
descriptors and discards accumulated entries. It cannot preempt an individual
open or directory-read syscall already in flight; the contract is cooperative
at those stated boundaries. Execution spawns no detached task or thread.

## Deferred scope

This slice does not add CLI commands or alter existing CLI bytes. It does not
add a provider, a production permission handler or prompt, permission modes
beyond `ask`, durable native sessions, configuration behavior, workspace
discovery, non-Unix hardened filesystem access, recursive enumeration, globs,
grep or content search, filesystem mutations, or other executable native tools.
It adds no compatibility, upstream-equivalence, or product-performance claim.
It does not change the pinned fx inventory, benchmark workloads or
classification, workflows, or Zig download.
