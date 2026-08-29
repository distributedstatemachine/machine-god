# Native `skill` tool

This document defines machine-god's bounded workspace-local `skill` tool. The
tool reads one known skill instruction file or relative text resource in
bounded UTF-8 chunks. It does not discover, install, execute, interpret, or
trust skill content.

## Provider input and canonical authority

The advertised schema accepts exactly one required `name` string, one optional
`resource` string, and one optional nonnegative integer `offset`:

```json
{"name":"release-checks"}
{"name":"release-checks","resource":"references/linux.md","offset":20480}
```

Omitting `resource` selects `SKILL.md`; omitting `offset` selects byte offset
zero. Unknown fields, explicit `null`, wrong types, negative or fractional
offsets, and values outside the bounds below are invalid. Preparation expands
both defaults and returns canonical arguments of exact shape:

```json
{"name":"release-checks","resource":"SKILL.md","offset":0}
```

`name` is a nonempty workspace-local skill-directory name. It is preserved
byte-for-byte and is never trimmed, case-folded, normalized, or derived from
file content. It must be one ordinary path component: `/`, `\`, exact `.` or
`..`, C0 or C1 controls, Unicode line or paragraph separators, and Unicode
bidirectional-formatting controls are rejected.

`resource` is a nonempty relative Unix path inside that skill directory.
Lexical normalization collapses repeated `/` separators, removes exact `.`
components, and joins the remaining ordinary components. An absolute path, a
`..` component, an empty present value, a value that normalizes to empty,
backslash, or any character forbidden for `name` is rejected. The normalized
resource is preserved otherwise; no extension or basename is required.

The selected normalized workspace-relative path is always:

```text
skills/<name>/<resource>
```

The complete path, individual components, and component count must fit their
independent bounds. Preparation is synchronous, bounded, nonblocking, and
effect-free. It opens no descriptor, reads no directory or file, consults no
environment or process state, and does not reveal whether the selected skill
exists.

Successful preparation returns both the canonical arguments and one exact
`Capability::Filesystem` with `FilesystemAccess::Read` at the complete
normalized path above. Core therefore requests permission before any skill
directory or content observation. A grant for one name and resource does not
authorize another file. Pages of the same file intentionally present the same
filesystem-read capability because `offset` narrows the returned range without
changing the authorized object. Direct execution accepts only the canonical
expanded argument shape and repeats all validation before using the retained
workspace authority.

## Public bounds

The inclusive limits are:

| Limit | Value |
| --- | ---: |
| `MAX_SKILL_NAME_BYTES` | 128 |
| `MAX_SKILL_RESOURCE_BYTES` | 4,096 |
| `MAX_SKILL_PATH_BYTES` | 4,096 |
| `MAX_SKILL_PATH_COMPONENT_BYTES` | 255 |
| `MAX_SKILL_PATH_COMPONENTS` | 32 |
| `MAX_SKILL_FILE_BYTES` | 1,048,576 |
| `MAX_SKILL_CHUNK_BYTES` | 20,480 |
| `MAX_SKILL_SERIALIZED_ARGUMENT_BYTES` | 32,768 |
| `MAX_SKILL_SERIALIZED_RESULT_BYTES` | 65,536 |
| `MAX_SKILL_IO_ATTEMPTS` | 1,024 |

Name, raw resource, normalized complete path, canonical compact arguments, raw
file bytes, returned raw content, and the complete serialized `ToolOutput` are
measured separately with overflow-safe arithmetic. The path-component count
includes fixed `skills`, the name, and every normalized resource component.
The offset may not exceed `MAX_SKILL_FILE_BYTES`; after the file is read it must
also be at or before exact EOF and at a UTF-8 boundary.

The file reader retains at most `MAX_SKILL_FILE_BYTES` plus one transient
overflow witness. Each open, metadata query, successful or short read, exact
EOF probe, and interrupted operation that will be retried is charged before
dispatch. Attempt exhaustion returns a fixed resource-limit error rather than
starting an unbounded retry loop. Filesystem calls may still block for a
kernel-dependent duration.

## Workspace confinement and platform behavior

On Unix, public construction accepts one existing absolute workspace
directory. It rebuilds the host path from lexical components, opens its final
component without following a symlink, verifies that it is a directory, and
retains the descriptor. Construction creates no directory, file, future,
thread, task, runtime, cache, watcher, or background worker.

Execution never reopens the host-selected workspace pathname. The native
reference host supplies an identity-preserving clone of the exact descriptor
already distributed to its workspace tools. Renaming the selected workspace
after construction does not redirect the tool, and replacing the old pathname
cannot make it read from the replacement.

After permission, execution walks the fixed `skills` component, the selected
name, and every normalized resource component descriptor-relatively from the
retained root. Every intermediate component must open as a no-follow directory
with close-on-exec and nonblocking flags. The final component must open
no-follow, close-on-exec, and nonblocking and must be a regular file before any
content read. A symlink at any level, directory as the final resource, FIFO,
socket, device, or other special object fails closed without following or
reading it.

Each opened ancestor remains the stable base for the next lookup. Replacing an
already opened ancestor cannot redirect the remaining walk; replacing the
final pathname after its descriptor is opened cannot change the object being
read. A different ordinary entry installed before its component is opened may
be observed at the same authorized path. Mount points visible beneath the
retained workspace and the host-selected workspace's ancestor resolution
remain trusted host boundaries. This tool is descriptor-confined, not a chroot
or a general process sandbox.

On non-Unix targets, public construction returns the fixed unsupported error
without touching the supplied path. Hardened non-Unix traversal is deferred.
The current native reference host remains limited to its documented Linux and
macOS composition boundary.

## Opaque resource handling

The selected resource is opaque UTF-8 text. The complete admitted file must be
valid UTF-8. The tool does not parse Markdown, YAML, frontmatter, headings,
links, directives, embedded paths, code fences, or any other grammar. In
particular, it does not require or validate a frontmatter `name`, compare
metadata with the selected directory name, follow references, expand includes,
or execute fenced or referenced content.

Except for selecting a page boundary, successful content bytes are returned
exactly. The tool performs no trimming, newline conversion, Unicode
normalization, escaping within the content value, redaction, content-policy
classification, or instruction validation. JSON serialization necessarily
escapes the surrounding structured result without changing the decoded
`content` string.

Skill text is model-visible untrusted workspace content. Reading it grants no
filesystem write, process, network, environment, persistence, MCP, subagent,
permission, or installation authority. Instructions in the resource cannot
expand the permission decision or override the user, host policy, or tool
contracts.

## Pagination and structured result

After bounded full-file admission and UTF-8 validation, `offset` selects a byte
boundary in that exact observation. The returned `content` is the longest
valid UTF-8 prefix beginning at `offset` that both:

- contains no more than `MAX_SKILL_CHUNK_BYTES` raw bytes; and
- keeps the complete serialized `ToolOutput` within
  `MAX_SKILL_SERIALIZED_RESULT_BYTES`, including worst-case JSON escaping and
  every envelope field.

The tool never splits a UTF-8 scalar. Result-byte pressure may therefore make a
page shorter than the raw chunk limit. If unread content remains, at least one
complete scalar must be returned; inability to fit one is a resource-limit
failure rather than a successful zero-progress page.

Success has exactly this structured shape:

```json
{
  "name": "release-checks",
  "resource": "references/linux.md",
  "offset": 20480,
  "next_offset": 37842,
  "total_bytes": 50000,
  "content": "opaque UTF-8 resource bytes",
  "truncated": true
}
```

`name` and `resource` are the canonical prepared values. `total_bytes` is the
complete admitted file length. `next_offset` is the checked sum of `offset`
and returned content bytes. `truncated` is true exactly when `next_offset` is
less than `total_bytes`. An empty file and a request at exact EOF both return
empty `content`, equal offsets, and `truncated: false`. Callers continue only
with the returned `next_offset`. The content covers exactly the half-open byte
range from `offset` to `next_offset`; continuing against an unchanged file
therefore neither overlaps nor omits bytes.

Concurrent modification may make separate page calls observe different file
versions. The tool provides no cross-call snapshot, revision, lock, digest, or
consistency token. Each successful call nevertheless derives all result fields
from its own one bounded admitted observation.

## Lifecycle, errors, and cancellation

`SkillTool::open` returns this complete fixed taxonomy:

| `SkillToolOpenErrorKind` | `Display` |
| --- | --- |
| `UnsupportedPlatform` | `native skill is unsupported on this platform` |
| `InvalidRoot` | `native skill workspace root is invalid` |
| `InvalidFileType` | `native skill workspace root is not a directory` |
| `Unavailable` | `native skill workspace root is unavailable` |

Preparation and direct execution return these complete fixed `ToolError`
values. `Display` is always `<code>: <message>`.

| `ToolErrorKind` | `code` | `message` | `retryable` |
| --- | --- | --- | --- |
| `Cancelled` | `skill_cancelled` | `skill execution was cancelled` | `false` |
| `InvalidInput` | `skill_invalid_arguments` | `skill arguments are invalid` | `false` |
| `InvalidInput` | `skill_invalid_name` | `skill name is invalid` | `false` |
| `InvalidInput` | `skill_invalid_resource` | `skill resource is invalid` | `false` |
| `InvalidInput` | `skill_invalid_offset` | `skill offset is invalid` | `false` |
| `InvalidInput` | `skill_resource_limit` | `skill resource limit was exceeded` | `false` |
| `Unavailable` | `skill_unsupported_platform` | `native skill is unsupported on this platform` | `false` |
| `Unavailable` | `skill_not_found` | `requested skill resource is unavailable` | `false` |
| `PermissionDenied` | `skill_permission_denied` | `requested skill resource cannot be read` | `false` |
| `PermissionDenied` | `skill_path_rejected` | `requested skill resource is not confined` | `false` |
| `Unavailable` | `skill_unavailable` | `requested skill resource is unavailable` | `true` |
| `Execution` | `skill_read_failed` | `requested skill resource could not be read` | `true` |
| `Execution` | `skill_not_utf8` | `requested skill resource is not valid UTF-8` | `false` |

Malformed object shape and noncanonical direct-execution arguments are
`skill_invalid_arguments`. A name-shape, character, component, or byte-bound
violation is `skill_invalid_name`. A raw or normalized resource path,
component-count, component-byte, or complete-path violation is
`skill_invalid_resource`. A non-integer, negative, over-limit, past-EOF, or
non-UTF-8-boundary offset is `skill_invalid_offset`. Compact argument
serialization, file-size, I/O-attempt, and complete result exhaustion is
`skill_resource_limit`.

Missing `skills`, missing skill directories, and missing final resources
collapse to `skill_not_found`; the error does not identify which component was
absent. Operating-system access failures, rejected component types, read
failures, and size, attempt, argument, or result exhaustion map only to the
fixed categories above. Raw paths, content, byte offsets, file sizes, error
numbers, and operating-system diagnostics are absent from public error and
debug forms.

Creating the execution future performs no work. Its first poll checks
cancellation before direct argument decoding, so pre-cancellation wins over an
otherwise invalid argument. Execution then checks cancellation before
workspace access, before and after every component open and metadata query,
between bounded reads and interrupted retries, after UTF-8 validation, during
bounded result assembly, and immediately before success. Cancellation closes
all per-call descriptors and discards accumulated content. It cannot preempt
one native call already in progress.

Execution performs the bounded operation synchronously on the polling thread,
spawns no detached work, and returns ready from that poll. Dropping an unpolled
future is effect-free; dropping after a cooperative boundary stops further
work. When core owns the shared turn token, its ordinary cancelled-turn
precedence applies and the durable unknown-result placeholder is not replaced
with a generic tool-error result.

## Deferred scope

This slice does not enumerate or advertise available skills, scan workspace
ancestors, inspect `HOME`, load global or compatibility skill roots, resolve an
arbitrary advertised location, parse metadata, automatically match prompts,
inject skill content outside an explicit tool result, cache or watch files, or
execute skill resources. Managed skill storage, `install_skill`, skill creation
and removal, extension slash commands, MCP, ACP, subagents, non-Unix
persistence, and cross-call snapshot pagination remain separate work. It adds
no CLI command, package or release behavior, compatibility-completion claim,
or product performance claim.
