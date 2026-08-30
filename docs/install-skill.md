# Native `install_skill` tool

This document defines machine-god's bounded local-only `install_skill` tool.
It copies one existing workspace directory into the managed
`skills/<name>` namespace. It does not fetch, clone, unpack, parse, execute, or
trust skill content.

## Input, canonical form, and authority

The advertised schema accepts exactly one required `source` string and one
optional `skill` string:

```json
{"source":"incoming/release-checks"}
{"source":"incoming/release-checks","skill":"release-checks"}
```

`source` is a workspace-relative directory path. Repeated separators and exact
`.` components are removed. Absolute paths, `..`, backslashes, empty normalized
paths, forbidden control or bidirectional characters, oversized components,
and paths beginning with an ASCII-case-insensitive spelling of the managed
`skills` component are rejected. The last normalized component is the
destination skill name. When `skill` is present it must equal that basename
byte-for-byte; it cannot rename a source.

Preparation is bounded, synchronous, and effect-free. It opens no descriptor,
does not inspect either source or destination, and cannot reveal whether they
exist. It expands the optional field and returns exact canonical arguments:

```json
{"source":"incoming/release-checks","skill":"release-checks"}
```

Successful preparation requests exactly one custom capability:

```json
{
  "name":"install_skill",
  "details":{
    "source":"incoming/release-checks",
    "destination":"skills/release-checks"
  }
}
```

The capability is indivisible: approval authorizes reading that exact local
source tree and publishing the one exact managed destination. Direct execution
accepts only canonical expanded arguments and repeats validation. Permission
denial therefore occurs before source or destination observation in engine
use.

## Admitted source and public bounds

The source must be an existing real directory below the retained workspace.
It must contain a regular top-level `SKILL.md` whose complete bytes are valid
UTF-8. The file is validated only as opaque text: Markdown, YAML, frontmatter,
headings, links, code fences, declared names, and references are not parsed or
followed. Missing, nonregular, or non-UTF-8 `SKILL.md` is rejected.

Every other admitted resource is copied opaquely. All descendants must have
valid UTF-8 ordinary component names and be regular files or real directories.
Symlinks, FIFOs, sockets, devices, and other special objects fail closed. Empty
directories are preserved. File modes, timestamps, ownership, ACLs, extended
attributes, and sparse layout are not preserved; installed directories and
files use private requested modes `0700` and `0600`, subject to host policy.

| Limit | Value |
| --- | ---: |
| `MAX_INSTALL_SKILL_SOURCE_BYTES` | 4,096 |
| `MAX_INSTALL_SKILL_PATH_BYTES` | 4,096 |
| `MAX_INSTALL_SKILL_NAME_BYTES` | 128 |
| `MAX_INSTALL_SKILL_COMPONENT_BYTES` | 255 |
| `MAX_INSTALL_SKILL_PATH_COMPONENTS` | 32 |
| `MAX_INSTALL_SKILL_ENTRIES` | 256 |
| `MAX_INSTALL_SKILL_FILE_BYTES` | 1,048,576 |
| `MAX_INSTALL_SKILL_TOTAL_BYTES` | 8,388,608 |
| `MAX_INSTALL_SKILL_ENTRY_NAME_BYTES` | 1,048,576 |
| `MAX_INSTALL_SKILL_CHUNK_BYTES` | 65,536 |
| `MAX_INSTALL_SKILL_IO_ATTEMPTS` | 8,192 |
| `MAX_INSTALL_SKILL_STAGE_ATTEMPTS` | 8 |
| `MAX_INSTALL_SKILL_SERIALIZED_ARGUMENT_BYTES` | 32,768 |
| `MAX_INSTALL_SKILL_SERIALIZED_RESULT_BYTES` | 16,384 |

Entry count includes files and directories below the source root. Aggregate
content counts regular-file bytes. The operation counter charges directory
enumeration and opening, metadata queries, entropy acquisition, staging,
reads, writes, synchronization, and publication. Limit exhaustion fails
without beginning an unbounded retry loop; individual filesystem calls may
still block for a kernel-dependent duration.

Construction failures expose only these stable kinds and messages:

| Kind | Message |
| --- | --- |
| `UnsupportedPlatform` | `native install_skill is unsupported on this platform` |
| `InvalidRoot` | `native install_skill workspace root is invalid` |
| `InvalidFileType` | `native install_skill workspace root is not a directory` |
| `Unavailable` | `native install_skill workspace root is unavailable` |

## Confinement and publication

On Linux and macOS, construction retains an existing absolute workspace
directory without following its final component. Execution never reopens the
host-selected workspace path. Renaming that workspace after construction does
not redirect the tool, and replacement of the former pathname is not observed.

After approval, source traversal begins at that retained descriptor. Every
component is opened descriptor-relatively with no-follow, close-on-exec, and
nonblocking flags, then revalidated by descriptor. Source file descriptors
remain retained between admission and copying. A source file whose length
changes before copying fails as changed rather than silently producing a
different byte count. Mount points visible below the retained workspace and
the host's original ancestor resolution remain trusted boundaries; this is
descriptor confinement, not a general process sandbox.

The destination must be absent. If `skills` already exists it must be a real
directory and a private staged skill directory is built inside it. If `skills`
is absent, a private staged skills tree is built beside it. All admitted bytes
are copied and staged files are synchronized before one atomic no-replace
rename publishes either the new child or the new managed root. Concurrent
creation is reported as a collision and never overwritten. Filesystems without
the required no-replace primitive are unsupported.

Immediately before publication, the tool revalidates both the random stage
identity and the workspace's current `skills` identity or absence. Immediately
after publication, it revalidates that the retained stage identity is visible
at the reported managed destination. Relocation or replacement before commit
is a retryable changed-destination failure; uncertainty after commit is a
nonretryable ambiguous result rather than false success. A later rename after
the successful postcommit observation remains an ordinary external filesystem
race.

Cancellation is honored before source observation, throughout admission and
staging, and at the final prepublication boundary. A cancelled or failed
precommit execution attempts identity-checked, entry-, depth-, name-, and
operation-bounded descriptor-relative cleanup and never reports success.
Cleanup stops without deleting a changed stage identity; a failed or contended
cleanup may leave a private random-name residue for host maintenance. Once the
no-replace rename succeeds, cancellation is ignored
while bounded postcommit synchronization and result construction complete.
A publication or postpublication synchronization error whose state cannot be
proved has the fixed nonretryable `install_skill_commit_ambiguous` result;
blind retry is unsafe because the destination may already exist.

On other targets, public construction returns the fixed unsupported error
without touching the supplied path. Network URLs, Git repositories, archive
files, package collections, global skill roots, process execution, and source
deletion are outside this tool's authority.

## Result and errors

Success returns the canonical source, skill name, destination, admitted entry
count, and aggregate copied file bytes:

```json
{
  "source":"incoming/release-checks",
  "skill":"release-checks",
  "destination":"skills/release-checks",
  "entries":3,
  "total_bytes":32
}
```

The complete `ToolOutput` must fit its serialized bound. The execution error
taxonomy is exact:

| Code | Kind | Retryable | Message |
| --- | --- | --- | --- |
| `install_skill_invalid_arguments` | invalid input | no | `install_skill arguments are invalid` |
| `install_skill_invalid_source` | invalid input | no | `install_skill source is invalid` |
| `install_skill_invalid_skill` | invalid input | no | `install_skill skill name is invalid` |
| `install_skill_overlap` | invalid input | no | `install_skill source overlaps its managed destination` |
| `install_skill_resource_limit` | invalid input | no | `install_skill resource limit was exceeded` |
| `install_skill_invalid_entry` | invalid input | no | `install_skill source contains an invalid entry name` |
| `install_skill_invalid_manifest` | invalid input | no | `install_skill source requires a regular UTF-8 SKILL.md` |
| `install_skill_cancelled` | cancelled | no | `install_skill execution was cancelled` |
| `install_skill_unsupported_platform` | unavailable | no | `native install_skill is unsupported on this platform` |
| `install_skill_source_not_found` | unavailable | no | `install_skill source is unavailable` |
| `install_skill_source_unavailable` | unavailable | yes | `install_skill source is unavailable` |
| `install_skill_source_changed` | execution | yes | `install_skill source changed during installation` |
| `install_skill_path_rejected` | permission denied | no | `install_skill path is not confined` |
| `install_skill_permission_denied` | permission denied | no | `install_skill filesystem access was denied` |
| `install_skill_destination_exists` | execution | no | `install_skill destination already exists` |
| `install_skill_destination_unavailable` | unavailable | yes | `install_skill destination is unavailable` |
| `install_skill_destination_changed` | execution | yes | `install_skill destination changed before publication` |
| `install_skill_write_failed` | execution | yes | `install_skill staged copy failed` |
| `install_skill_unsupported_filesystem` | unavailable | no | `atomic no-replace skill publication is unavailable` |
| `install_skill_commit_ambiguous` | execution | no | `install_skill publication status is uncertain` |

Errors never include model-provided paths, entry names, file content, OS
diagnostics, temporary names, descriptor values, or host paths. Published
content remains untrusted model-visible workspace data; installation grants no
execution, process, network, MCP, subagent, persistence, or additional
filesystem authority.
