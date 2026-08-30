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
and paths whose first component is a Unicode case-fold alias of the managed
`skills` component are rejected. The alias check includes ordinary ASCII case
variants and non-ASCII folds such as `ſkills` (long s) and `sKills` (Kelvin
sign). This conservative lexical rule prevents managed-source overlap on
case-insensitive filesystems without consulting the filesystem during
preparation. The last normalized component is the destination skill name.
When `skill` is present it must equal that basename byte-for-byte; it cannot
rename a source.

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
are copied before publication. The private tree is then recursively rewalked
no-follow within the same public entry, depth, path, name, file,
aggregate-byte, and operation bounds. That rewalk freezes a staged snapshot
containing every relative entry, entry type and identity, and each regular
file's exact length and content digest. Every staged file and required staged
directory is synchronized before one atomic no-replace rename publishes either
the new child or the new managed root. Concurrent creation is reported as a
collision and never overwritten. Filesystems without the required no-replace
primitive are unsupported.

Immediately before publication, the tool revalidates both the random stage
identity and the workspace's current `skills` identity or absence, reserves
the complete bounded postcommit validation and durability budget, and then
checks cancellation again directly before invoking rename. Cancellation that
arrives during any preceding verification call is therefore observed before
the irreversible operation. Relocation or replacement before commit is a
retryable changed-destination failure.

After publication, the tool first revalidates that the retained stage identity
is visible at the reported managed destination, then recursively rewalks that
destination no-follow and compares it with the complete staged snapshot. No
entry may be missing or unexpected; every path, entry type, retained identity,
regular-file length, and content digest must match. Mutation, replacement,
extra content, invalid traversal, or validation-budget failure after commit is
`install_skill_commit_ambiguous`, never success. This postcommit comparison
does not reinterpret `SKILL.md` or weaken the admitted-content invariant: the
published manifest must still be the exact regular UTF-8 bytes admitted before
staging, and every other file remains exact opaque bytes.

The destination-parent durability sync is attempted within its fixed retry
bound after every committed publication even when identity or recursive
content validation has already selected `install_skill_commit_ambiguous`.
Validation failure cannot short-circuit that durability attempt. Success is
returned only when both validation and synchronization succeed. A later rename
or mutation after the successful postcommit observation remains an ordinary
external filesystem race.

Cancellation is honored before source observation, throughout admission and
staging, after final prepublication verification, and directly before rename.
A cancelled or failed precommit execution attempts identity-checked, entry-,
depth-, name-, and operation-bounded descriptor-relative cleanup and never
reports success. If stage `mkdirat` succeeds but the tool then cannot acquire
the staged descriptor, it still makes one bounded no-follow removal attempt;
before descriptor identity exists this can remove only the current empty
directory and never recursively traverses it. After descriptor acquisition,
recursive cleanup first proves that the pathname still names the retained
identity. A changed or nonempty replacement is not recursively removed.
Cleanup stops without deleting an unproved identity; a failed or contended
cleanup may leave a private random-name residue for host maintenance.

As with the other native workspace tools, an uncoordinated process holding the
same operating-system identity and direct write authority to the workspace is
a trusted external mutator during the platform's non-atomic `mkdirat`-to-open
directory-acquisition interval. After the descriptor is acquired, staged entry
identity and content validation detects pathname replacement or injection both
before and after publication; it cannot produce success for a changed tree.

Once rename succeeds, or its result cannot prove that publication did not
occur, cancellation is ignored while bounded postcommit identity/content
validation and the unconditional bounded parent-sync attempt finish. A
publication, validation, or synchronization error whose final state cannot be
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
| `install_skill_invalid_arguments` | `InvalidInput` | no | `install_skill arguments are invalid` |
| `install_skill_invalid_source` | `InvalidInput` | no | `install_skill source is invalid` |
| `install_skill_invalid_skill` | `InvalidInput` | no | `install_skill skill name is invalid` |
| `install_skill_overlap` | `InvalidInput` | no | `install_skill source overlaps its managed destination` |
| `install_skill_resource_limit` | `InvalidInput` | no | `install_skill resource limit was exceeded` |
| `install_skill_invalid_entry` | `InvalidInput` | no | `install_skill source contains an invalid entry name` |
| `install_skill_invalid_manifest` | `InvalidInput` | no | `install_skill source requires a regular UTF-8 SKILL.md` |
| `install_skill_cancelled` | `Cancelled` | no | `install_skill execution was cancelled` |
| `install_skill_unsupported_platform` | `Unavailable` | no | `native install_skill is unsupported on this platform` |
| `install_skill_source_not_found` | `Unavailable` | no | `install_skill source is unavailable` |
| `install_skill_source_unavailable` | `Unavailable` | yes | `install_skill source is unavailable` |
| `install_skill_source_changed` | `Execution` | yes | `install_skill source changed during installation` |
| `install_skill_path_rejected` | `PermissionDenied` | no | `install_skill path is not confined` |
| `install_skill_permission_denied` | `PermissionDenied` | no | `install_skill filesystem access was denied` |
| `install_skill_destination_exists` | `Execution` | no | `install_skill destination already exists` |
| `install_skill_destination_unavailable` | `Unavailable` | yes | `install_skill destination is unavailable` |
| `install_skill_destination_changed` | `Execution` | yes | `install_skill destination changed before publication` |
| `install_skill_write_failed` | `Execution` | yes | `install_skill staged copy failed` |
| `install_skill_unsupported_filesystem` | `Unavailable` | no | `atomic no-replace skill publication is unavailable` |
| `install_skill_commit_ambiguous` | `Execution` | no | `install_skill publication status is uncertain` |

Errors never include model-provided paths, entry names, file content, OS
diagnostics, temporary names, descriptor values, or host paths. Published
content remains untrusted model-visible workspace data; installation grants no
execution, process, network, MCP, subagent, persistence, or additional
filesystem authority.
